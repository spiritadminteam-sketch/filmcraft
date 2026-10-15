//! The pipe protocol between FilmCraft and `filmcraft-plugin-host`.
//!
//! Every message is a frame: `u32` little-endian length of what follows, one kind byte, then the
//! payload. Control messages ([`Request`] / [`Response`]) are JSON; audio goes as raw `f32`
//! samples ([`ProcessRequest`] / [`ProcessResponse`]) so a block costs one copy each way.
//!
//! Everything read is treated as hostile: lengths, counts and sizes are capped before anything is
//! allocated, and a malformed frame is an error, never a panic.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

/// Protocol version; the host and FilmCraft must agree.
pub const VERSION: u32 = 1;
/// Largest frame accepted (64 MB).
pub const MAX_FRAME: usize = 64 << 20;
/// Most samples per channel in one process message.
pub const MAX_FRAMES: usize = 65_536;
/// Most parameter changes in one process message.
pub const MAX_CHANGES: usize = 4096;

pub const KIND_JSON: u8 = 1;
pub const KIND_PROCESS: u8 = 2;
pub const KIND_PROCESSED: u8 = 3;

/// One plug-in parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamDesc {
    pub id: u32,
    pub title: String,
    #[serde(default)]
    pub units: String,
    /// 0 = continuous, otherwise the number of steps.
    #[serde(default)]
    pub steps: i32,
    /// Normalised default (0…1).
    pub default: f64,
}

/// One audio effect class found in a `.vst3` module.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    /// The module (bundle folder or file).
    pub path: String,
    /// Class id, 32 hex digits.
    pub cid: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub params: Vec<ParamDesc>,
}

impl PluginInfo {
    /// The FilmCraft effect id: `vst3:<class id>`.
    pub fn effect_id(&self) -> String {
        format!("vst3:{}", self.cid.to_ascii_lowercase())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    Hello,
    /// List the audio effect classes of one module, with their parameters.
    Scan {
        path: String,
    },
    /// Create an instance; `params` are initial normalised values.
    Create {
        path: String,
        cid: String,
        #[serde(rename = "sampleRate")]
        sample_rate: f64,
        #[serde(default)]
        params: Vec<(u32, f64)>,
    },
    Destroy {
        inst: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Response {
    Hello { version: u32 },
    Scanned { plugins: Vec<PluginInfo> },
    Created { inst: u32, latency: u32 },
    Ok,
    Error { message: String },
}

/// Process `frames` samples of planar stereo audio through instance `inst`, after applying
/// `changes` (parameter id, normalised value).
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessRequest {
    pub inst: u32,
    pub changes: Vec<(u32, f64)>,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProcessResponse {
    /// `None` on success, otherwise why the block failed (the audio is then the input).
    pub error: Option<String>,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

/// A decoded frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Json(Vec<u8>),
    Process(ProcessRequest),
    Processed(ProcessResponse),
}

fn err(m: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, m.into())
}

fn write_frame(w: &mut dyn Write, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(payload.len() + 1).map_err(|_| err("frame too large"))?;
    if payload.len() + 1 > MAX_FRAME {
        return Err(err("frame too large"));
    }
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&[kind])?;
    w.write_all(payload)?;
    w.flush()
}

pub fn write_json<T: Serialize>(w: &mut dyn Write, v: &T) -> std::io::Result<()> {
    let b = serde_json::to_vec(v).map_err(|e| err(e.to_string()))?;
    write_frame(w, KIND_JSON, &b)
}

fn push_samples(out: &mut Vec<u8>, s: &[f32]) {
    out.reserve(s.len() * 4);
    for v in s {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

pub fn write_process(w: &mut dyn Write, r: &ProcessRequest) -> std::io::Result<()> {
    let n = r.left.len().min(r.right.len());
    if n > MAX_FRAMES || r.changes.len() > MAX_CHANGES {
        return Err(err("process request too large"));
    }
    let mut b = Vec::with_capacity(12 + r.changes.len() * 12 + n * 8);
    b.extend_from_slice(&r.inst.to_le_bytes());
    b.extend_from_slice(&(r.changes.len() as u32).to_le_bytes());
    b.extend_from_slice(&(n as u32).to_le_bytes());
    for (id, v) in &r.changes {
        b.extend_from_slice(&id.to_le_bytes());
        b.extend_from_slice(&v.to_le_bytes());
    }
    push_samples(&mut b, &r.left[..n]);
    push_samples(&mut b, &r.right[..n]);
    write_frame(w, KIND_PROCESS, &b)
}

pub fn write_processed(w: &mut dyn Write, r: &ProcessResponse) -> std::io::Result<()> {
    let n = r.left.len().min(r.right.len()).min(MAX_FRAMES);
    let msg = r.error.as_deref().unwrap_or("").as_bytes();
    let msg = &msg[..msg.len().min(4096)];
    let mut b = Vec::with_capacity(8 + msg.len() + n * 8);
    b.extend_from_slice(&(msg.len() as u32).to_le_bytes());
    b.extend_from_slice(msg);
    b.extend_from_slice(&(n as u32).to_le_bytes());
    push_samples(&mut b, &r.left[..n]);
    push_samples(&mut b, &r.right[..n]);
    write_frame(w, KIND_PROCESSED, &b)
}

/// A little-endian cursor over a payload that fails (instead of panicking) when it runs out.
struct Cur<'a>(&'a [u8]);

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> std::io::Result<&'a [u8]> {
        if self.0.len() < n {
            return Err(err("truncated frame"));
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f64(&mut self) -> std::io::Result<f64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_le_bytes(a))
    }
    fn samples(&mut self, n: usize) -> std::io::Result<Vec<f32>> {
        let b = self.take(n.checked_mul(4).ok_or_else(|| err("bad length"))?)?;
        Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

fn decode(kind: u8, payload: &[u8]) -> std::io::Result<Frame> {
    let mut c = Cur(payload);
    match kind {
        KIND_JSON => Ok(Frame::Json(payload.to_vec())),
        KIND_PROCESS => {
            let inst = c.u32()?;
            let nc = c.u32()? as usize;
            let n = c.u32()? as usize;
            if nc > MAX_CHANGES || n > MAX_FRAMES {
                return Err(err("process request too large"));
            }
            let mut changes = Vec::with_capacity(nc);
            for _ in 0..nc {
                let id = c.u32()?;
                let v = c.f64()?;
                changes.push((id, if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }));
            }
            let left = c.samples(n)?;
            let right = c.samples(n)?;
            Ok(Frame::Process(ProcessRequest { inst, changes, left, right }))
        }
        KIND_PROCESSED => {
            let ml = c.u32()? as usize;
            if ml > 4096 {
                return Err(err("message too long"));
            }
            let msg = String::from_utf8_lossy(c.take(ml)?).to_string();
            let n = c.u32()? as usize;
            if n > MAX_FRAMES {
                return Err(err("process response too large"));
            }
            let left = c.samples(n)?;
            let right = c.samples(n)?;
            Ok(Frame::Processed(ProcessResponse { error: (!msg.is_empty()).then_some(msg), left, right }))
        }
        k => Err(err(format!("unknown frame kind {k}"))),
    }
}

/// Read one frame; `Ok(None)` at a clean end of stream.
pub fn read_frame(r: &mut dyn Read) -> std::io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(err(format!("bad frame length {len}")));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    let (kind, payload) = buf.split_first().ok_or_else(|| err("empty frame"))?;
    decode(*kind, payload).map(Some)
}

/// Hex of a 16-byte class id.
pub fn cid_hex(cid: &[u8; 16]) -> String {
    cid.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parse 32 hex digits back into a class id.
pub fn cid_parse(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(f: impl Fn(&mut Vec<u8>) -> std::io::Result<()>) -> Frame {
        let mut b = Vec::new();
        f(&mut b).unwrap();
        read_frame(&mut b.as_slice()).unwrap().unwrap()
    }

    #[test]
    fn frames_round_trip() {
        let req = Request::Create { path: "C:/x.vst3".into(), cid: "00".repeat(16), sample_rate: 48000.0, params: vec![(3, 0.5)] };
        let Frame::Json(j) = round(|b| write_json(b, &req)) else { panic!() };
        assert_eq!(serde_json::from_slice::<Request>(&j).unwrap(), req);
        let p = ProcessRequest { inst: 7, changes: vec![(1, 0.25)], left: vec![0.5, -0.5], right: vec![1.0, 0.0] };
        assert_eq!(round(|b| write_process(b, &p)), Frame::Process(p.clone()));
        let r = ProcessResponse { error: Some("nope".into()), left: vec![0.1], right: vec![0.2] };
        assert_eq!(round(|b| write_processed(b, &r)), Frame::Processed(r.clone()));
        let ok = ProcessResponse { error: None, left: vec![], right: vec![] };
        assert_eq!(round(|b| write_processed(b, &ok)), Frame::Processed(ok.clone()));
    }

    #[test]
    fn hostile_frames_are_errors_not_panics() {
        // end of stream
        assert!(read_frame(&mut [].as_slice()).unwrap().is_none());
        // huge length
        assert!(read_frame(&mut [255u8, 255, 255, 255].as_slice()).is_err());
        // zero length
        assert!(read_frame(&mut [0u8, 0, 0, 0].as_slice()).is_err());
        // a process frame claiming more samples than it carries
        let mut b = vec![];
        let mut p = vec![KIND_PROCESS];
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&1000u32.to_le_bytes());
        b.extend_from_slice(&(p.len() as u32).to_le_bytes());
        b.extend_from_slice(&p);
        assert!(read_frame(&mut b.as_slice()).is_err());
        // unknown kind
        assert!(read_frame(&mut [1u8, 0, 0, 0, 99].as_slice()).is_err());
        // every truncation of a valid frame fails cleanly
        let mut full = Vec::new();
        write_process(&mut full, &ProcessRequest { inst: 1, changes: vec![(2, 0.5)], left: vec![0.0; 4], right: vec![0.0; 4] }).unwrap();
        for cut in 1..full.len() {
            let _ = read_frame(&mut &full[..cut]);
        }
    }

    #[test]
    fn class_ids() {
        let id = [0x6e, 0x33, 0x22, 0x52, 0x54, 0x22, 0x4a, 0x00, 0xaa, 0x69, 0x30, 0x1a, 0xf3, 0x18, 0x79, 0x7d];
        assert_eq!(cid_parse(&cid_hex(&id)), Some(id));
        assert_eq!(cid_parse("zz"), None);
        assert_eq!(cid_parse(&"é".repeat(16)), None);
    }
}
