//! The Odin Pro window (Window ▸ Odin Pro…): the presets, sounds and footage of an Odin Pro pack
//! (or another preset pack made for Premiere Pro), one click to drop one at the playhead. The
//! engine reads the pack (`odin.scan`) and places what is clicked (`odin.insertPreset`,
//! `odin.insertMedia`); see `filmcraft_engine::odin`.
//!
//! Automation ids: `odin.window`, `odin.root`, `odin.scan`, `odin.tab.<presets|sounds|footage>`,
//! `odin.filter`, `odin.item.<n>` (n from 0, top first, in the open tab).

use egui::{Rect, RichText};
use serde_json::{Value, json};

use crate::FilmcraftApp;

/// Rows shown at most (the filter narrows the rest).
const SHOWN: usize = 400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Presets,
    Sounds,
    Footage,
}

/// The window while it is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OdinState {
    /// The pack folder being typed.
    pub root: String,
    pub filter: String,
    pub tab: Tab,
    /// The last result or error.
    pub status: String,
}

/// One row of a tab: what it shows and what clicking it runs.
pub struct Row {
    pub group: String,
    pub label: String,
    pub command: &'static str,
    pub params: Value,
}

/// The rows of `tab` in a scan result.
pub fn rows(catalog: &Value, tab: Tab) -> Vec<Row> {
    let list = |key: &str| catalog.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    match tab {
        Tab::Presets => {
            for p in list("presets") {
                let file = p["file"].as_str().unwrap_or("").to_string();
                let group = p["group"].as_str().unwrap_or("").to_string();
                for n in p["names"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    out.push(Row { group: group.clone(), label: n.to_string(), command: "odin.insertPreset", params: json!({"file": file, "name": n}) });
                }
            }
        }
        Tab::Sounds | Tab::Footage => {
            for m in list(if tab == Tab::Sounds { "sounds" } else { "footage" }) {
                out.push(Row {
                    group: m["group"].as_str().unwrap_or("").to_string(),
                    label: m["name"].as_str().unwrap_or("").to_string(),
                    command: "odin.insertMedia",
                    params: json!({"path": m["path"]}),
                });
            }
        }
    }
    out
}

/// Whether every word of `filter` is in the row's name or folder.
pub fn matches(row: &Row, filter: &str) -> bool {
    let hay = format!("{} {}", row.label, row.group).to_lowercase();
    filter.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// Open the window (or close it when it is open). The remembered pack is listed on first open.
pub fn toggle(app: &mut FilmcraftApp) {
    if app.ui.odin.take().is_some() {
        return;
    }
    let root = app.session.prefs.media_browser.odin_root.clone();
    let mut st = OdinState { root: root.clone(), ..Default::default() };
    if app.ui.odin_catalog.is_none() && !root.is_empty() {
        st.status = scan(app, &root);
    }
    app.ui.odin = Some(st);
}

/// Read the pack at `root` into the catalog. Returns a status line.
pub fn scan(app: &mut FilmcraftApp, root: &str) -> String {
    match app.session.execute("odin.scan", json!({"root": root})) {
        Ok(v) => {
            let n = |k: &str| v.get(k).and_then(Value::as_array).map_or(0, Vec::len);
            let presets: usize = v["presets"].as_array().into_iter().flatten().map(|p| p["names"].as_array().map_or(0, Vec::len)).sum();
            let status = format!("{presets} presets in {} projects, {} sounds, {} footage clips", n("presets"), n("sounds"), n("footage"));
            app.ui.odin_catalog = Some(v);
            status
        }
        Err(e) => e.to_string(),
    }
}

/// The window, while open.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut st) = app.ui.odin.clone() else { return };
    let t = app.tokens;
    let catalog = app.ui.odin_catalog.clone().unwrap_or(Value::Null);
    let all = rows(&catalog, st.tab);
    let shown: Vec<&Row> = all.iter().filter(|r| matches(r, &st.filter)).take(SHOWN).collect();
    let count = |tab: Tab| rows(&catalog, tab).len();
    let counts = [
        (Tab::Presets, "presets", "Presets", count(Tab::Presets)),
        (Tab::Sounds, "sounds", "Sounds", count(Tab::Sounds)),
        (Tab::Footage, "footage", "Footage", count(Tab::Footage)),
    ];
    let templates = catalog["templates"].as_u64().unwrap_or(0);
    let mut elems: Vec<(String, Rect, String)> = Vec::new();
    let mut open = true;
    let (mut do_scan, mut run): (bool, Option<(&'static str, Value, String)>) = (false, None);
    let screen = ctx.content_rect();
    let win = egui::Window::new("Odin Pro")
        .id(egui::Id::new("odin-pro"))
        .collapsible(false)
        .resizable(true)
        .open(&mut open)
        .default_size([460.0, 560.0])
        .default_pos(egui::pos2(screen.max.x - 500.0, screen.min.y + 80.0))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Pack folder");
                let r = ui.add(egui::TextEdit::singleline(&mut st.root).desired_width((ui.available_width() - 60.0).max(80.0)).hint_text("E:\\Odin Pro\\…"));
                elems.push(("odin.root".into(), r.rect, "Pack folder".into()));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let b = ui.button("Scan");
                elems.push(("odin.scan".into(), b.rect, "Scan".into()));
                do_scan = b.clicked() || enter;
            });
            ui.horizontal(|ui| {
                for (tab, id, name, n) in counts {
                    let r = ui.selectable_label(st.tab == tab, format!("{name} ({n})"));
                    elems.push((format!("odin.tab.{id}"), r.rect, name.into()));
                    if r.clicked() {
                        st.tab = tab;
                    }
                }
            });
            let r = ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text("Search (zoom, glitch, whoosh…)").desired_width(f32::INFINITY));
            elems.push(("odin.filter".into(), r.rect, "Search".into()));
            if templates > 0 && st.tab == Tab::Presets {
                ui.label(
                    RichText::new(format!("{templates} motion graphics templates (.mogrt) need After Effects to render and are not listed."))
                        .size(11.0)
                        .color(t.text_faint),
                );
            }
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 24.0).show(ui, |ui| {
                if catalog.is_null() {
                    ui.label(RichText::new("Choose your Odin Pro folder and press Scan.").color(t.text_faint));
                }
                let mut last_group = None;
                for (i, row) in shown.iter().enumerate() {
                    if last_group != Some(&row.group) {
                        ui.add_space(4.0);
                        ui.label(RichText::new(&row.group).size(11.0).color(t.text_faint));
                        last_group = Some(&row.group);
                    }
                    let r = ui.selectable_label(false, &row.label).on_hover_text("Click to put it at the playhead");
                    elems.push((format!("odin.item.{i}"), r.rect, row.label.clone()));
                    if r.clicked() {
                        run = Some((row.command, row.params.clone(), row.label.clone()));
                    }
                }
                if !catalog.is_null() && shown.is_empty() {
                    ui.label(RichText::new("Nothing matches").color(t.text_faint));
                }
            });
            if !st.status.is_empty() {
                ui.label(RichText::new(&st.status).size(11.0).color(t.text_faint));
            }
        });
    if let Some(w) = &win {
        elems.push(("odin.window".into(), w.response.rect, "Odin Pro".into()));
    }
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if do_scan {
        let root = st.root.trim().to_string();
        st.status = scan(app, &root);
    }
    if let Some((cmd, params, label)) = run {
        st.status = match app.session.execute(cmd, params) {
            Ok(v) => {
                let notes: Vec<&str> = v["report"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if notes.is_empty() { format!("Added {label}") } else { format!("Added {label} ({})", notes.join("; ")) }
            }
            Err(e) => e.to_string(),
        };
    }
    app.ui.odin = if open { Some(st) } else { None };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_filter() {
        let cat = json!({
            "presets": [{"file": "a.prproj", "group": "Transitions / Zoom", "names": ["Zoom IN", "Zoom OUT"]}],
            "sounds": [{"path": "w.wav", "group": "Sound FX / Glitch", "name": "Whoosh"}],
        });
        let p = rows(&cat, Tab::Presets);
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].params, json!({"file": "a.prproj", "name": "Zoom OUT"}));
        assert!(matches(&p[0], "zoom in"));
        assert!(matches(&p[0], "transitions"));
        assert!(!matches(&p[0], "glitch"));
        assert_eq!(rows(&cat, Tab::Sounds)[0].command, "odin.insertMedia");
        assert!(rows(&cat, Tab::Footage).is_empty());
        assert!(rows(&Value::Null, Tab::Presets).is_empty());
    }
}
