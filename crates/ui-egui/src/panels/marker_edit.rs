//! Editing a sequence marker, as in Premiere: its right-click menu (a colour, Edit Marker…,
//! Clear Selected Marker, Clear Markers) and the Marker dialog (name, comments, colour, duration)
//! opened from it or by double-clicking the marker on the Timeline ruler or in the Markers panel.
//!
//! Automation ids: `<prefix>.color.<Label>`, `<prefix>.edit`, `<prefix>.markers.clearCurrent`,
//! `<prefix>.markers.clearAll` for the menu (`prefix` is `timeline.markerMenu` or
//! `markers.menu`); `markerDialog.{name,comment,duration,color.<Label>,delete,cancel,ok}` for the
//! dialog.

use egui::{Align2, Color32, Rect, Sense, vec2};
use filmcraft_project::{Label, MarkerId};
use serde_json::json;

use crate::FilmcraftApp;
use crate::panels::misc::MARKER_FILTER;
use crate::state::MarkerDialog;

fn swatch(ui: &mut egui::Ui, l: Label, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    let c = l.marker_rgb();
    let col = Color32::from_rgb(c[0], c[1], c[2]);
    ui.painter().rect_filled(r.shrink(2.0), 3.0, col);
    if on {
        ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(1.0, Color32::from_gray(200)), egui::StrokeKind::Inside);
    }
    resp.on_hover_text(crate::i18n::t(l.name()))
}

/// The Marker dialog's draft of marker `id` of the active sequence.
pub(crate) fn open_dialog(app: &mut FilmcraftApp, id: u64) {
    let rate = app.session.sequence_rate();
    let Some(m) = app.session.active_sequence().and_then(|q| q.markers.iter().find(|m| m.id == MarkerId(id))) else { return };
    app.ui.marker_dialog = Some(MarkerDialog {
        marker: id,
        name: m.name.clone(),
        comment: m.comment.clone(),
        color: m.color.name().to_string(),
        duration_frames: rate.frame_at(m.duration).max(0),
    });
}

/// The right-click menu of marker `id` (the playhead is already on it, where Clear Selected
/// Marker looks for it).
pub(crate) fn menu(app: &mut FilmcraftApp, ctx: &egui::Context, ui: &mut egui::Ui, id: u64, prefix: &str) {
    let current = app.session.active_sequence().and_then(|q| q.markers.iter().find(|m| m.id == MarkerId(id))).map(|m| m.color.marker_rgb());
    let mut elems: Vec<(String, Rect, String)> = Vec::new();
    // a colour, one click
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for l in MARKER_FILTER {
            let r = swatch(ui, l, current == Some(l.marker_rgb()));
            elems.push((format!("{prefix}.color.{}", l.name()), r.rect, l.name().to_string()));
            if r.clicked() {
                if let Err(e) = app.session.execute("markers.edit", json!({"marker": id, "color": l.name()})) {
                    app.ui.status = e.to_string();
                }
                ui.close();
            }
        }
    });
    let r = ui.button(crate::i18n::t("Edit Marker…"));
    elems.push((format!("{prefix}.edit"), r.rect, "Edit Marker…".into()));
    if r.clicked() {
        open_dialog(app, id);
        ui.close();
    }
    ui.separator();
    for (label, cmd) in [("Clear Selected Marker", "markers.clearCurrent"), ("Clear Markers", "markers.clearAll")] {
        let r = ui.button(crate::i18n::t(label));
        elems.push((format!("{prefix}.{cmd}"), r.rect, label.into()));
        if r.clicked() {
            // the marker itself, wherever the playhead is
            let res = if cmd == "markers.clearCurrent" {
                app.session.execute("markers.remove", json!({"marker": id})).map_err(|e| e.to_string())
            } else {
                crate::menus::invoke(app, ctx, cmd, json!({}))
            };
            if let Err(e) = res {
                app.ui.status = e;
            }
            ui.close();
        }
    }
    for (eid, r, l) in elems {
        app.auto.add(&eid, r, &l);
    }
}

/// The Marker dialog, while one is open.
pub fn dialog(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.marker_dialog.clone() else { return };
    // gone (undone, cleared) while open
    if !app.session.active_sequence().is_some_and(|q| q.markers.iter().any(|m| m.id == MarkerId(d.marker))) {
        app.ui.marker_dialog = None;
        return;
    }
    let mut elems: Vec<(String, Rect, String)> = Vec::new();
    let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let (mut ok, mut delete) = (false, false);
    egui::Window::new(crate::i18n::t("Marker"))
        .id(egui::Id::new("marker-dialog"))
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Grid::new("marker-dialog-grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label(tl!("Name"));
                let r = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(240.0));
                elems.push(("markerDialog.name".into(), r.rect, "Name".into()));
                ui.end_row();
                ui.label(tl!("Duration"));
                let r = ui.add(egui::DragValue::new(&mut d.duration_frames).range(0..=10_000_000).suffix(" f"));
                elems.push(("markerDialog.duration".into(), r.rect, "Duration".into()));
                ui.end_row();
                ui.label(tl!("Comments"));
                let r = ui.add(egui::TextEdit::multiline(&mut d.comment).desired_rows(4).desired_width(240.0));
                elems.push(("markerDialog.comment".into(), r.rect, "Comments".into()));
                ui.end_row();
                ui.label(tl!("Color"));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let on = Label::from_name(&d.color).map(Label::marker_rgb);
                    for l in MARKER_FILTER {
                        let r = swatch(ui, l, on == Some(l.marker_rgb()));
                        elems.push((format!("markerDialog.color.{}", l.name()), r.rect, l.name().to_string()));
                        if r.clicked() {
                            d.color = l.name().to_string();
                        }
                    }
                });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let r = ui.button(tl!("Delete"));
                elems.push(("markerDialog.delete".into(), r.rect, "Delete".into()));
                delete |= r.clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.button(tl!("OK"));
                    elems.push(("markerDialog.ok".into(), r.rect, "OK".into()));
                    ok |= r.clicked();
                    let r = ui.button(tl!("Cancel"));
                    elems.push(("markerDialog.cancel".into(), r.rect, "Cancel".into()));
                    close |= r.clicked();
                });
            });
        });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if delete {
        if let Err(e) = app.session.execute("markers.remove", json!({"marker": d.marker})) {
            app.ui.status = e.to_string();
        }
        app.ui.marker_dialog = None;
    } else if ok {
        let p = json!({"marker": d.marker, "name": d.name, "comment": d.comment, "color": d.color, "durationFrames": d.duration_frames});
        if let Err(e) = app.session.execute("markers.edit", p) {
            app.ui.status = e.to_string();
        }
        app.ui.marker_dialog = None;
    } else if close {
        app.ui.marker_dialog = None;
    } else {
        app.ui.marker_dialog = Some(d);
    }
}
