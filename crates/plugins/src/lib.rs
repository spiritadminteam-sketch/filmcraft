//! Third-party audio plug-ins (VST3) for FilmCraft (layer L2).
//!
//! Plug-ins never run inside the editor: a helper process, `filmcraft-plugin-host`, loads them and
//! processes audio for FilmCraft over pipes ([`proto`]). If a plug-in crashes or hangs, only the
//! helper goes; the effect then passes audio through and FilmCraft keeps running
//! (docs/adr/0002-plugin-hosting.md).
//!
//! - [`scan`] looks through plug-in folders (the system VST3 folders by default), asks the helper
//!   what each `.vst3` module contains, saves the list and registers each audio effect as a
//!   FilmCraft effect (`vst3:<class id>`, Effects panel ▸ Audio Effects ▸ VST3). Every plug-in
//!   parameter becomes a keyframeable 0–100 % slider in Effect Controls.
//! - [`init`] loads the saved list at startup, so plug-ins are there without rescanning.
//! - [`create_effect`] is the factory the audio engine calls for a `vst3:` effect.

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

pub mod client;
mod effect;
pub mod proto;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use filmcraft_audio_dsp::{AudioEffect, ParamSpec, Unit};
use filmcraft_project::{EffectDef, EffectKind, ParamDef, ParamKind, ParamValue};
use serde::{Deserialize, Serialize};

pub use effect::PluginEffect;
pub use proto::{ParamDesc, PluginInfo};

/// Effects-panel folder of plug-in effects.
pub const CATEGORY: &[&str] = &["Audio Effects", "VST3"];
/// Most modules one scan looks at.
const MAX_MODULES: usize = 2000;
/// Most parameters exposed per plug-in.
const MAX_PARAMS: usize = 512;

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    plugins: Vec<PluginInfo>,
    /// Modules that failed to load (path, reason), shown in the scan report.
    #[serde(default)]
    failed: Vec<(String, String)>,
}

struct State {
    cache_path: Option<PathBuf>,
    cache: Cache,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { cache_path: None, cache: Cache::default() }))
}

/// Intern a string for the `'static` effect definitions (each distinct string is kept once).
fn leak(s: &str) -> &'static str {
    static I: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut m = I.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(v) = m.get(s) {
        return v;
    }
    let v: &'static str = Box::leak(s.to_string().into_boxed_str());
    m.insert(s.to_string(), v);
    v
}

fn user_params(info: &PluginInfo) -> impl Iterator<Item = &ParamDesc> {
    info.params.iter().take(MAX_PARAMS)
}

/// The FilmCraft effect definition of a plug-in: every parameter as a 0–100 % slider.
pub fn effect_def(info: &PluginInfo, name: &str) -> EffectDef {
    let params = user_params(info)
        .map(|p| ParamDef {
            id: leak(&format!("p{}", p.id)),
            label: leak(if p.title.is_empty() { "Parameter" } else { &p.title }),
            kind: ParamKind::Float { min: 0.0, max: 100.0, soft_min: 0.0, soft_max: 100.0, unit: "%", decimals: 1 },
            default: ParamValue::Float((p.default.clamp(0.0, 1.0) * 1000.0).round() / 10.0),
            animatable: true,
            group: None,
        })
        .collect();
    EffectDef {
        id: leak(&info.effect_id()),
        name: leak(name),
        kind: EffectKind::Audio,
        category: CATEGORY,
        params,
        intrinsic: false,
        accelerated: false,
        float32: true,
        yuv: false,
    }
}

/// The processor-side parameter specs of a plug-in (same ids and ranges as [`effect_def`]).
fn specs_of(info: &PluginInfo) -> &'static [ParamSpec] {
    static C: OnceLock<Mutex<HashMap<String, &'static [ParamSpec]>>> = OnceLock::new();
    let key = format!("{}|{}", info.effect_id(), info.params.iter().map(|p| format!("{}:{}", p.id, p.default)).collect::<Vec<_>>().join(","));
    let mut m = C.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(s) = m.get(&key) {
        return s;
    }
    let v: Vec<ParamSpec> = user_params(info)
        .map(|p| {
            ParamSpec::new(leak(&format!("p{}", p.id)), leak(&p.title), 0.0, 100.0, ((p.default.clamp(0.0, 1.0) * 1000.0).round() / 10.0) as f32, Unit::Percent)
        })
        .collect();
    let s: &'static [ParamSpec] = Box::leak(v.into_boxed_slice());
    m.insert(key, s);
    s
}

/// Display names, made unique (two plug-ins called "Compressor" become "Compressor (Vendor)").
fn names(plugins: &[PluginInfo]) -> Vec<String> {
    plugins
        .iter()
        .map(|p| {
            let dup = plugins.iter().filter(|q| q.name.eq_ignore_ascii_case(&p.name)).count() > 1
                || filmcraft_project::effect_defs().iter().any(|d| d.name.eq_ignore_ascii_case(&p.name));
            if dup && !p.vendor.is_empty() { format!("{} ({})", p.name, p.vendor) } else { p.name.clone() }
        })
        .collect()
}

fn register(plugins: &[PluginInfo]) {
    let names = names(plugins);
    filmcraft_project::register_plugin_effects(plugins.iter().zip(&names).map(|(p, n)| effect_def(p, n)).collect());
}

/// Load the saved plug-in list from `cache_path` (if any) and register its effects. Call once at
/// startup; later calls only change where scans are saved.
pub fn init(cache_path: Option<PathBuf>) -> usize {
    let mut st = state().lock().unwrap_or_else(PoisonError::into_inner);
    st.cache_path = cache_path.clone();
    if let Some(c) = cache_path.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Cache>(&b).ok()) {
        st.cache = c;
    }
    register(&st.cache.plugins);
    st.cache.plugins.len()
}

/// The known plug-ins (from the last scan).
pub fn plugins() -> Vec<PluginInfo> {
    state().lock().unwrap_or_else(PoisonError::into_inner).cache.plugins.clone()
}

/// Modules the last scan could not load, with the reason.
pub fn failures() -> Vec<(String, String)> {
    state().lock().unwrap_or_else(PoisonError::into_inner).cache.failed.clone()
}

/// The folders scanned last (empty: never scanned).
pub fn scanned_folders() -> Vec<String> {
    state().lock().unwrap_or_else(PoisonError::into_inner).cache.folders.clone()
}

/// The system's standard VST3 folders.
pub fn default_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if cfg!(windows) {
        let common = std::env::var_os("CommonProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Common Files"));
        v.push(common.join("VST3"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Programs").join("Common").join("VST3"));
        }
    } else if cfg!(target_os = "macos") {
        v.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join("Library/Audio/Plug-Ins/VST3"));
        }
    } else {
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join(".vst3"));
        }
        v.push(PathBuf::from("/usr/lib/vst3"));
        v.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    v
}

/// The `.vst3` modules (bundle folders or files) under `folder`, without descending into bundles.
pub fn find_modules(folder: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 8 || out.len() >= MAX_MODULES {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vst3")) {
                out.push(p);
            } else if p.is_dir() {
                walk(&p, depth + 1, out);
            }
            if out.len() >= MAX_MODULES {
                return;
            }
        }
    }
    let mut out = Vec::new();
    if folder.extension().is_some_and(|e| e.eq_ignore_ascii_case("vst3")) {
        out.push(folder.to_path_buf());
    } else {
        walk(folder, 0, &mut out);
    }
    out
}

/// What a scan found.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ScanReport {
    pub folders: Vec<String>,
    pub modules: usize,
    pub plugins: Vec<PluginInfo>,
    pub failed: Vec<(String, String)>,
}

/// Scan `folders` (the system's VST3 folders when empty) for VST3 audio effects, save the list
/// and register the effects. `progress(done, total, module)` is called before each module.
/// Each module is opened in a helper process of its own scan host; a module that crashes the
/// helper is reported as failed and the scan goes on.
pub fn scan(folders: &[PathBuf], progress: &mut dyn FnMut(usize, usize, &str)) -> ScanReport {
    let folders: Vec<PathBuf> = if folders.is_empty() { default_folders() } else { folders.to_vec() };
    let mut modules: Vec<PathBuf> = folders.iter().flat_map(|f| find_modules(f)).collect();
    modules.sort();
    modules.dedup();
    modules.truncate(MAX_MODULES);
    let mut report = ScanReport { folders: folders.iter().map(|f| f.display().to_string()).collect(), modules: modules.len(), ..Default::default() };
    let mut host: Option<client::Host> = None;
    for (i, m) in modules.iter().enumerate() {
        let path = m.display().to_string();
        progress(i, modules.len(), &path);
        if host.as_ref().is_none_or(|h| !h.is_alive()) {
            match client::Host::spawn() {
                Ok(h) => host = Some(h),
                Err(e) => {
                    report.failed.push((path, e));
                    continue;
                }
            }
        }
        let Some(h) = host.as_ref() else { continue };
        match h.request(&proto::Request::Scan { path: path.clone() }, client::SCAN_TIMEOUT) {
            Ok(proto::Response::Scanned { plugins }) => report.plugins.extend(plugins),
            Ok(proto::Response::Error { message }) => report.failed.push((path, message)),
            Ok(other) => report.failed.push((path, format!("unexpected answer {other:?}"))),
            Err(e) => report.failed.push((path, e)),
        }
    }
    progress(modules.len(), modules.len(), "");
    // one entry per effect id (a module listed twice, or the same class in two folders)
    let mut seen = std::collections::HashSet::new();
    report.plugins.retain(|p| seen.insert(p.effect_id()));
    report.plugins.sort_by_key(|p| p.name.to_ascii_lowercase());
    let mut st = state().lock().unwrap_or_else(PoisonError::into_inner);
    st.cache = Cache { version: proto::VERSION, folders: report.folders.clone(), plugins: report.plugins.clone(), failed: report.failed.clone() };
    if let Some(p) = st.cache_path.clone() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(&st.cache) {
            Ok(b) => {
                if let Err(e) = std::fs::write(&p, b) {
                    log::warn!("plug-in list: {}: {e}", p.display());
                }
            }
            Err(e) => log::warn!("plug-in list: {e}"),
        }
    }
    register(&st.cache.plugins);
    report
}

/// The audio engine's factory for `vst3:` effects (see `filmcraft_render::audio_fx`). `None` for
/// an unknown plug-in (the engine then passes audio through).
pub fn create_effect(effect_id: &str, sample_rate: f32, _channels: usize) -> Option<Box<dyn AudioEffect>> {
    let info = state().lock().unwrap_or_else(PoisonError::into_inner).cache.plugins.iter().find(|p| p.effect_id() == effect_id).cloned()?;
    let def = filmcraft_project::find_effect(effect_id)?;
    Some(Box::new(PluginEffect::new(def.id, specs_of(&info), &info, sample_rate)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, cid: &str) -> PluginInfo {
        PluginInfo {
            path: "/x/Gain.vst3".into(),
            cid: cid.into(),
            name: name.into(),
            vendor: "Acme".into(),
            category: "Fx".into(),
            version: "1.0".into(),
            params: vec![ParamDesc { id: 0, title: "Gain".into(), units: "dB".into(), steps: 0, default: 0.5 }],
        }
    }

    #[test]
    fn plug_ins_become_effects() {
        let p = info("Test Gain", &"ab".repeat(16));
        let d = effect_def(&p, "Test Gain");
        assert_eq!(d.id, format!("vst3:{}", "ab".repeat(16)));
        assert_eq!(d.category, CATEGORY);
        assert_eq!(d.params.len(), 1);
        assert_eq!(d.params[0].id, "p0");
        assert_eq!(d.params[0].default, ParamValue::Float(50.0));
        let s = specs_of(&p);
        assert_eq!((s[0].id, s[0].default, s[0].max), ("p0", 50.0, 100.0));
        // the same specs are reused
        assert!(std::ptr::eq(s, specs_of(&p)));
    }

    #[test]
    fn duplicate_names_get_the_vendor() {
        let a = info("Compressor", &"01".repeat(16));
        let b = info("Shiny", &"02".repeat(16));
        let n = names(&[a, b]);
        // "Compressor" is also a built-in effect name
        assert_eq!(n, vec!["Compressor (Acme)".to_string(), "Shiny".to_string()]);
    }

    #[test]
    fn modules_are_found_without_entering_bundles() {
        let dir = std::env::temp_dir().join(format!("fc-vst3-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Vendor/Comp.vst3/Contents/x86_64-win")).unwrap();
        std::fs::write(dir.join("Vendor/Comp.vst3/Contents/x86_64-win/Comp.vst3"), b"x").unwrap();
        std::fs::write(dir.join("Old.vst3"), b"x").unwrap();
        std::fs::write(dir.join("readme.txt"), b"x").unwrap();
        let found = find_modules(&dir);
        assert_eq!(found, vec![dir.join("Old.vst3"), dir.join("Vendor/Comp.vst3")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_plug_in_has_no_processor() {
        assert!(create_effect("vst3:nope", 48000.0, 2).is_none());
    }
}
