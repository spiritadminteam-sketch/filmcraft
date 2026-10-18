//! The Marker dialog, as in Premiere (name, comments, colour, duration, Delete), opened by
//! double-clicking a marker on the Timeline ruler or in the Markers panel, or with Edit Marker… in
//! its right-click menu (`panels::misc::marker_edit_item`).
//!
//! Automation ids: `markerDialog.{name,comment,duration,color.<Label>,delete,cancel,ok}`.

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

/// The Marker dialog's preset names, with the colour each one sets (none: the colour stays).
pub(crate) const MARKER_PRESETS: [(&str, Option<&str>); 8] = [
    ("bad word", Some("Rose")),
    ("replace", None),
    ("add SFX", None),
    ("add GIF", None),
    ("add image", None),
    ("add visual", None),
    ("add caption", None),
    ("cut", None),
];

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
                // one-click names for the markers an editor drops while reviewing (a bad word to
                // bleep is red, as the editor colour-codes them)
                ui.label(crate::i18n::t("Presets"));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for (i, (name, color)) in MARKER_PRESETS.iter().enumerate() {
                        let on = d.name == *name;
                        let r = ui.add(egui::Button::new(egui::RichText::new(*name).size(11.5)).selected(on).corner_radius(9.0));
                        elems.push((format!("markerDialog.preset.{i}"), r.rect, (*name).to_string()));
                        if r.clicked() {
                            d.name = (*name).to_string();
                            if let Some(c) = color {
                                d.color = (*c).to_string();
                            }
                        }
                    }
                });
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
