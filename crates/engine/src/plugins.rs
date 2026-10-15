//! Third-party audio plug-ins (VST3): scanning and listing (`plugins.*`), and wiring the
//! plug-in effect factory into the audio engine. The plug-ins run in a separate helper process
//! (`filmcraft-plugins`, docs/adr/0002-plugin-hosting.md).

use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad};
use crate::narration::{finish, new_job, spawn};
use crate::{Result, Session};

/// Where the scan results are kept: `<data dir>/plugins/vst3.json`.
pub fn cache_path() -> Option<std::path::PathBuf> {
    crate::autosave::default_data_dir().map(|d| d.join("plugins").join("vst3.json"))
}

/// Load the saved plug-in list and register the plug-in effect factory (once per process).
pub fn ensure_init() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        filmcraft_render::audio_fx::set_external_factory(filmcraft_plugins::create_effect);
        let n = filmcraft_plugins::init(cache_path());
        if n > 0 {
            log::info!("{n} VST3 plug-in effect(s) available");
        }
    });
}

fn info_json(p: &filmcraft_plugins::PluginInfo) -> Value {
    let def = filmcraft_project::find_effect(&p.effect_id());
    json!({
        "effect": p.effect_id(),
        "name": def.map_or(p.name.as_str(), |d| d.name),
        "vendor": p.vendor,
        "category": p.category,
        "version": p.version,
        "path": p.path,
        "params": p.params.iter().map(|q| json!({"param": format!("p{}", q.id), "name": q.title, "units": q.units, "default": q.default * 100.0})).collect::<Vec<_>>(),
    })
}

/// `plugins.list`: the plug-in effects from the last scan.
fn list(_: &mut Session, _: &Value) -> Result<Value> {
    ensure_init();
    let plugins = filmcraft_plugins::plugins();
    Ok(json!({
        "plugins": plugins.iter().map(info_json).collect::<Vec<_>>(),
        "failed": filmcraft_plugins::failures().iter().map(|(p, e)| json!({"path": p, "error": e})).collect::<Vec<_>>(),
        "folders": filmcraft_plugins::scanned_folders(),
        "defaultFolders": filmcraft_plugins::default_folders().iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
        "hostFound": filmcraft_plugins::client::host_exe().is_some(),
    }))
}

/// `plugins.scan`: look for VST3 audio effects (in the system's VST3 folders, or `folders`) in
/// the background; they appear in Effects ▸ Audio Effects ▸ VST3.
fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugins.scan";
    ensure_init();
    if filmcraft_plugins::client::host_exe().is_none() {
        return Err(bad(cmd, format!("{} is missing next to FilmCraft; reinstall FilmCraft", filmcraft_plugins::client::host_exe_name())));
    }
    let folders: Vec<std::path::PathBuf> = match p.get("folders") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(std::path::PathBuf::from).collect(),
        Some(Value::String(f)) => vec![std::path::PathBuf::from(f)],
        Some(_) => return Err(bad(cmd, "`folders` must be a list of folder paths")),
    };
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(false);
    let job = new_job(s, "Scanning audio plug-ins".into());
    let (prog, res) = (job.progress.clone(), job.result.clone());
    let id = spawn(s, job, wait, "filmcraft-plugin-scan", move || {
        use std::sync::atomic::Ordering;
        let t0 = web_time::Instant::now();
        let mut progress = |done: usize, total: usize, module: &str| {
            prog.done.store(done as u64, Ordering::Relaxed);
            prog.total.store(total as u64, Ordering::Relaxed);
            let name = std::path::Path::new(module).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            *prog.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = format!("Scanning {name}");
        };
        let r = filmcraft_plugins::scan(&folders, &mut progress);
        let msg = match (r.plugins.len(), r.failed.len()) {
            (0, 0) if r.modules == 0 => "No VST3 plug-ins found".to_string(),
            (n, 0) => format!("{n} VST3 plug-in effect(s) found"),
            (n, f) => format!("{n} VST3 plug-in effect(s) found; {f} module(s) could not be loaded"),
        };
        finish(&prog, &res, Ok(msg), t0);
    })?;
    let mut out = json!({"job": id});
    if wait {
        out["plugins"] = json!(filmcraft_plugins::plugins().iter().map(info_json).collect::<Vec<_>>());
        out["failed"] = json!(filmcraft_plugins::failures().iter().map(|(p, e)| json!({"path": p, "error": e})).collect::<Vec<_>>());
    }
    Ok(out)
}

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "plugins.scan",
            label: "Scan for Audio Plug-ins",
            menu: &[],
            shortcut: None,
            params: r#"{"folders":[str]? (default: the system's VST3 folders),"wait":bool?}"#,
            enabled: always,
            run: scan,
            journal: false,
        },
        CommandSpec { id: "plugins.list", label: "List Audio Plug-ins", menu: &[], shortcut: None, params: "{}", enabled: always, run: list, journal: false },
    ]
}
