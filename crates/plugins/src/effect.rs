//! [`PluginEffect`]: a plug-in instance in the helper process, seen by the audio engine as an
//! ordinary [`AudioEffect`].

use std::sync::Arc;
use std::time::Duration;

use filmcraft_audio_dsp::{AudioEffect, ParamSpec};

use crate::client::{CREATE_TIMEOUT, Host, host};
use crate::proto::{PluginInfo, ProcessRequest, Request, Response};

/// A running plug-in. When the plug-in cannot run (helper missing, plug-in crashed or hung) the
/// audio passes through unchanged and the reason is logged once.
pub struct PluginEffect {
    id: &'static str,
    specs: &'static [ParamSpec],
    /// VST3 parameter id of each spec.
    vst_ids: Vec<u32>,
    values: Vec<f32>,
    dirty: Vec<bool>,
    inst: Option<(Arc<Host>, u32)>,
    latency: usize,
}

impl PluginEffect {
    pub(crate) fn new(id: &'static str, specs: &'static [ParamSpec], info: &PluginInfo, sample_rate: f32) -> PluginEffect {
        let vst_ids: Vec<u32> = specs.iter().filter_map(|s| s.id.strip_prefix('p').and_then(|n| n.parse().ok())).collect();
        let values: Vec<f32> = specs.iter().map(|s| s.default).collect();
        let mut fx = PluginEffect { id, specs, dirty: vec![false; values.len()], values, vst_ids, inst: None, latency: 0 };
        let params: Vec<(u32, f64)> = fx.vst_ids.iter().zip(&fx.values).map(|(i, v)| (*i, f64::from(*v) / 100.0)).collect();
        let req = Request::Create { path: info.path.clone(), cid: info.cid.clone(), sample_rate: f64::from(sample_rate), params };
        match host().and_then(|h| h.request(&req, CREATE_TIMEOUT).map(|r| (h, r))) {
            Ok((h, Response::Created { inst, latency })) => {
                fx.inst = Some((h, inst));
                fx.latency = (latency as usize).min(1 << 20);
            }
            Ok((_, Response::Error { message })) => log::warn!("plug-in {}: {message}", info.name),
            Ok((_, other)) => log::warn!("plug-in {}: unexpected answer {other:?}", info.name),
            Err(e) => log::warn!("plug-in {}: {e}", info.name),
        }
        fx
    }

    /// Whether the plug-in is running (false: audio passes through).
    pub fn is_running(&self) -> bool {
        self.inst.as_ref().is_some_and(|(h, _)| h.is_alive())
    }
}

/// Samples per message to the host.
const CHUNK: usize = 4096;

impl PluginEffect {
    fn process_chunk(&mut self, channels: &mut [&mut [f32]], at: usize, n: usize) {
        let Some((h, inst)) = self.inst.as_ref().map(|(h, i)| (h.clone(), *i)) else { return };
        if !h.is_alive() {
            self.inst = None;
            return;
        }
        let left = channels.first().and_then(|c| c.get(at..at + n)).map(<[f32]>::to_vec).unwrap_or_default();
        let right = channels.get(1).and_then(|c| c.get(at..at + n)).map_or_else(|| left.clone(), <[f32]>::to_vec);
        let changes: Vec<(u32, f64)> =
            self.dirty.iter().zip(self.vst_ids.iter().zip(&self.values)).filter(|(d, _)| **d).map(|(_, (id, v))| (*id, f64::from(*v) / 100.0)).collect();
        let req = ProcessRequest { inst, changes, left, right };
        match h.process(&req) {
            Ok(r) if r.error.is_none() && r.left.len() >= n && r.right.len() >= n => {
                self.dirty.iter_mut().for_each(|d| *d = false);
                for (ci, src) in [&r.left, &r.right].into_iter().enumerate() {
                    if let (Some(c), Some(s)) = (channels.get_mut(ci), src.get(..n)) {
                        if let Some(dst) = c.get_mut(at..at + n) {
                            dst.copy_from_slice(s);
                        }
                    }
                }
            }
            Ok(r) => {
                log::warn!("plug-in {}: {}", self.id, r.error.unwrap_or_else(|| "short answer".into()));
                self.inst = None;
            }
            Err(e) => {
                log::warn!("plug-in {}: {e}", self.id);
                self.inst = None;
            }
        }
    }
}

impl AudioEffect for PluginEffect {
    fn id(&self) -> &'static str {
        self.id
    }
    fn params(&self) -> &'static [ParamSpec] {
        self.specs
    }
    fn set_param(&mut self, id: &str, value: f32) -> bool {
        let Some(i) = self.specs.iter().position(|s| s.id == id) else { return false };
        let v = self.specs.get(i).map_or(value, |s| s.sanitize(value));
        if let (Some(cur), Some(d)) = (self.values.get_mut(i), self.dirty.get_mut(i)) {
            if *cur != v {
                *cur = v;
                *d = true;
            }
        }
        true
    }
    fn param(&self, id: &str) -> Option<f32> {
        let i = self.specs.iter().position(|s| s.id == id)?;
        self.values.get(i).copied()
    }
    fn reset(&mut self) {}
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let n = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        let mut at = 0;
        while at < n && self.inst.is_some() {
            let m = (n - at).min(CHUNK);
            self.process_chunk(channels, at, m);
            at += m;
        }
    }
    fn latency(&self) -> usize {
        self.latency
    }
}

impl Drop for PluginEffect {
    fn drop(&mut self) {
        if let Some((h, inst)) = self.inst.take() {
            if h.is_alive() {
                let _ = h.request(&Request::Destroy { inst }, Duration::from_secs(2));
            }
        }
    }
}
