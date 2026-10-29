//! Clip Mismatch Warning, as in Premiere: the first clip edited into an empty sequence whose frame
//! size, rate or pixel aspect differ asks whether to change the sequence to match the clip. The
//! engine decides when to ask (`filmcraft_engine::clip_mismatch::mismatch`, off with Settings ▸
//! Timeline ▸ Show Clip Mismatch Warning dialog); this module holds the edit back and shows the
//! choice. Automation ids: `clipMismatch.change`, `clipMismatch.keep`, `clipMismatch.cancel`.

use egui::Align2;
use filmcraft_engine::clip_mismatch::{Mismatch, mismatch};
use filmcraft_project::{ItemId, SequenceSettings};
use serde_json::{Value, json};

use crate::FilmcraftApp;

const KEY: &str = "clip mismatch warning";

/// The edit waiting on the answer.
#[derive(Clone)]
struct Pending {
    cmd: String,
    params: Value,
    m: Mismatch,
}

/// The item an edit command puts into the sequence (the first one when it places several).
fn placed_item(app: &FilmcraftApp, cmd: &str, params: &Value) -> Option<ItemId> {
    match cmd {
        "timeline.place" => params.get("item").or_else(|| params.get("items").and_then(|v| v.get(0))).and_then(Value::as_u64).map(ItemId),
        "source.insert" | "source.overwrite" => app.session.state.source_item,
        _ => None,
    }
}

/// Holds `cmd` back and opens the warning when it would put a mismatched first clip into an empty
/// sequence. Returns false (the caller runs the command) in every other case.
pub fn intercept(app: &mut FilmcraftApp, ctx: &egui::Context, cmd: &str, params: &Value) -> bool {
    let Some(m) = placed_item(app, cmd, params).and_then(|item| mismatch(&app.session, item)) else {
        return false;
    };
    let pending = Pending { cmd: cmd.to_string(), params: params.clone(), m };
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(KEY), pending));
    true
}

/// Runs `cmd` unless the warning takes it, reporting a failure on the status line.
pub fn execute(app: &mut FilmcraftApp, ctx: &egui::Context, cmd: &str, params: Value) {
    if intercept(app, ctx, cmd, &params) {
        return;
    }
    if let Err(e) = app.session.execute(cmd, params) {
        app.ui.status = e.to_string();
    }
}

/// "1920×1080, 23.976 fps" (with the pixel aspect when it is not square).
fn describe(s: &SequenceSettings) -> String {
    let (w, h, fps) = (s.width, s.height, s.frame_rate.label());
    match s.par {
        (1, 1) => tlf!("{w}×{h}, {fps} fps", w, h, fps),
        (n, d) => tlf!("{w}×{h}, {fps} fps, pixel aspect {n}:{d}", w, h, fps, n, d),
    }
}

// ---------------------------------------------------------------- the dialog

/// The warning window: change the sequence and then make the edit, keep the settings and make
/// the edit, or cancel the edit.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let key = egui::Id::unique(KEY);
    let Some(p) = ctx.data(|d| d.get_temp::<Pending>(key)) else {
        return;
    };
    // (change sequence first, then run the edit)
    let mut choice: Option<bool> = None;
    let mut close = false;
    egui::Window::new(tl!("Clip Mismatch Warning"))
        .id(key.with("window"))
        .collapsible(false)
        .resizable(false)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(crate::widgets::dialog_center(ctx))
        .show(ctx, |ui| {
            ui.label(tl!("This clip does not match the sequence's settings. Change the sequence to match the clip's settings?"));
            ui.add_space(4.0);
            ui.label(tlf!("Sequence: {s}", s = describe(&p.m.sequence)));
            ui.label(tlf!("Clip: {c}", c = describe(&p.m.clip)));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let r = ui.button(tl!("Change sequence settings"));
                app.auto.add("clipMismatch.change", r.rect, "Change sequence settings");
                if r.clicked() {
                    choice = Some(true);
                }
                let r = ui.button(tl!("Keep existing settings"));
                app.auto.add("clipMismatch.keep", r.rect, "Keep existing settings");
                if r.clicked() {
                    choice = Some(false);
                }
                let r = ui.button(tl!("Cancel"));
                app.auto.add("clipMismatch.cancel", r.rect, "Cancel");
                close |= r.clicked();
            });
        });
    if let Some(change) = choice {
        if change && let Err(e) = app.session.execute("sequence.matchClip", json!({"item": p.m.item.0})) {
            app.ui.status = e.to_string();
        }
        if let Err(e) = app.session.execute(&p.cmd, p.params.clone()) {
            app.ui.status = e.to_string();
        }
        close = true;
    }
    if close || crate::widgets::escape_closes(ctx) {
        ctx.data_mut(|d| d.remove::<Pending>(key));
    }
}
