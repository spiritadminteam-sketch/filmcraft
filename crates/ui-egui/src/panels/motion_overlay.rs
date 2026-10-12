//! Direct manipulation of a clip's Motion in the Program monitor, as in Premiere Pro: with the
//! Selection tool, the selected video clip shows its bounding box (its picture through Motion),
//! corner handles and its anchor point. Drag inside the box to move the clip (Position), a corner
//! handle to scale it about the anchor point (Scale; Scale Width too when Uniform Scale is off),
//! and just outside a corner to rotate it (Rotation). A click on a clip in the picture selects it;
//! a drag on a clip that is not selected selects it and moves it.
//!
//! Each drag is one undo step. A parameter with keyframes gets one at the playhead, exactly as when
//! its value is typed in Effect Controls; one without keyframes just takes the new value. Graphic
//! clips keep their own layer handles (`graphics.rs`): over a graphic layer, the graphics overlay
//! gets the pointer.

use egui::{Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use filmcraft_geom::{Affine, Vec2};
use filmcraft_project::{ClipId, ItemKind, TrackItem};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::state::Tool;

/// How far (screen points) from a corner its scale handle reaches; rotation is grabbed from there
/// out to [`ROTATE_REACH`].
const HANDLE_REACH: f32 = 7.0;
const ROTATE_REACH: f32 = 24.0;

/// A video clip in the picture at the playhead: its picture's corners and anchor point on screen.
#[derive(Clone)]
pub(crate) struct ClipBox {
    pub clip: ClipId,
    pub item: TrackItem,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub quad: [Pos2; 4],
    pub anchor: Pos2,
}

impl ClipBox {
    /// Whether `p` is inside the box (it is convex: a parallelogram).
    pub(crate) fn contains(&self, p: Pos2) -> bool {
        let q = &self.quad;
        let mut sign = 0.0f32;
        for i in 0..4 {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            let c = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            if c.abs() < 1e-6 {
                continue;
            }
            if sign == 0.0 {
                sign = c.signum();
            } else if c.signum() != sign {
                return false;
            }
        }
        sign != 0.0
    }
    /// The corner handle at `p`, if any.
    fn corner_at(&self, p: Pos2) -> Option<usize> {
        self.quad.iter().position(|c| (*c - p).length() <= HANDLE_REACH)
    }
    /// Whether `p` is in the ring just outside a corner, where a drag rotates.
    fn rotate_at(&self, p: Pos2) -> bool {
        !self.contains(p) && self.quad.iter().any(|c| (*c - p).length() <= ROTATE_REACH)
    }
}

fn screen(a: &Affine, v: Vec2) -> Pos2 {
    let s = a.apply(v);
    pos2(s.x as f32, s.y as f32)
}

/// The video clips in the picture at the playhead, front-most first (graphics, which have their
/// own handles, left out).
pub(crate) fn clip_boxes(app: &FilmcraftApp, pic: Rect, frame: (u32, u32)) -> Vec<ClipBox> {
    let Some(seq) = app.session.active_sequence() else { return Vec::new() };
    let t = app.session.playhead();
    let to_screen = crate::panels::graphics::frame_to_screen(pic, frame);
    let mut out = Vec::new();
    for tr in seq.video_tracks.iter().rev() {
        if !tr.enabled {
            continue;
        }
        let Some(it) = tr.item_at(t) else { continue };
        if !it.enabled {
            continue;
        }
        let project = &app.session.project;
        if project.item(it.item).is_none_or(|p| matches!(p.kind, ItemKind::Graphic { .. })) {
            continue;
        }
        let Some(size) = project.source_size(it.item).filter(|s| s.0 > 0 && s.1 > 0) else { continue };
        let mt = it.source_time_at(t);
        let m = to_screen.then_apply(&filmcraft_render::motion_matrix(seq, it, size, project.source_par(it.item), mt));
        let (w, h) = (size.0 as f64, size.1 as f64);
        let quad = [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].map(|v| screen(&m, v));
        if quad.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
            continue;
        }
        let a = it.effect("motion").filter(|e| e.enabled && e.param("anchor").is_some()).map(|e| e.vec2_at("anchor", mt));
        let a = a.filter(|a| a.x.is_finite() && a.y.is_finite()).unwrap_or(Vec2::new(w / 2.0, h / 2.0));
        out.push(ClipBox { clip: it.id, item: it.clone(), quad, anchor: screen(&m, a) });
    }
    out
}

/// What a drag on the picture changes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Move,
    Scale,
    Rotate,
}

/// A drag in progress: the clip, what it changes, and the values when it began.
#[derive(Clone, Copy, Debug)]
struct Drag {
    clip: ClipId,
    grab: Grab,
    press: Pos2,
    anchor: Pos2,
    /// Screen points per sequence pixel, across and down.
    k: (f64, f64),
    position: (f64, f64),
    scale: f64,
    scale_width: f64,
    rotation: f64,
    uniform: bool,
    /// Whether the drag has changed anything yet (its first change starts a new undo step).
    started: bool,
}

/// The selected video clip that is in the picture at the playhead.
fn selected_box<'a>(app: &FilmcraftApp, boxes: &'a [ClipBox]) -> Option<&'a ClipBox> {
    boxes.iter().find(|b| app.session.state.selection.contains(&b.clip))
}

/// Draw the selected clip's box and handle the pointer on the Program monitor picture `pic`.
pub fn monitor_overlay(app: &mut FilmcraftApp, ui: &mut egui::Ui, pic: Rect, frame: (u32, u32)) {
    // only with the Selection tool, and not while a mask, a text or the eyedropper has the picture
    if app.ui.tool != Tool::Selection || app.ui.eyedropper.is_some() || app.ui.gfx_edit.is_some() || app.session.state.selected_mask.is_some() {
        return;
    }
    let Some(seq) = app.session.active_sequence() else { return };
    let (sw, sh) = (seq.settings.width.max(1) as f64, seq.settings.height.max(1) as f64);
    let t = app.tokens;
    let boxes = clip_boxes(app, pic, frame);
    let sel = selected_box(app, &boxes).cloned();
    let id = egui::Id::new("motion-overlay");
    let drag_id = id.with("drag");
    let drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id));

    // ---- the selected clip's box, handles and anchor point
    if let Some(b) = &sel {
        let painter = ui.painter().with_clip_rect(pic.expand(12.0));
        painter.add(egui::Shape::closed_line(b.quad.to_vec(), Stroke::new(1.0, t.accent)));
        for (n, c) in b.quad.iter().enumerate() {
            let hr = Rect::from_center_size(*c, vec2(7.0, 7.0));
            painter.rect_filled(hr, 0.0, egui::Color32::WHITE);
            painter.rect_stroke(hr, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Middle);
            app.auto.add(&format!("program.motion.handle.{n}"), hr.expand(3.0), "scale handle");
        }
        let a = b.anchor;
        painter.circle_stroke(a, 4.0, Stroke::new(1.0, t.accent));
        painter.line_segment([a - vec2(6.0, 0.0), a + vec2(6.0, 0.0)], Stroke::new(1.0, t.accent));
        painter.line_segment([a - vec2(0.0, 6.0), a + vec2(0.0, 6.0)], Stroke::new(1.0, t.accent));
        let r = Rect::from_points(&b.quad);
        app.auto.add("program.motion.box", r.intersect(pic), "clip");
        app.auto.add("program.motion.anchor", Rect::from_center_size(a, vec2(12.0, 12.0)), "anchor point");
    }

    // ---- the pointer: only over a clip of the picture (or the selected clip's handles), and never
    // over a graphic layer, which keeps its own handles
    let hover = ui.ctx().pointer_hover_pos().filter(|p| pic.expand(ROTATE_REACH).contains(*p));
    let over_graphic = |p: Pos2| crate::panels::graphics::visible_layers(app, pic, frame).iter().any(|v| v.hit(p));
    let wants = drag.is_some()
        || hover.is_some_and(|p| {
            let on_sel = sel.as_ref().is_some_and(|b| b.contains(p) || b.corner_at(p).is_some() || b.rotate_at(p));
            let on_clip = pic.contains(p) && boxes.iter().any(|b| b.contains(p));
            (on_sel || on_clip) && !over_graphic(p)
        });
    if !wants {
        return;
    }
    let area = pic.expand(ROTATE_REACH);
    let resp = ui.interact(area, id, Sense::click_and_drag());
    // cursors
    if drag.is_none()
        && let Some(p) = hover
    {
        let icon = match &sel {
            Some(b) if b.corner_at(p).is_some_and(|n| n % 2 == 0) => Some(egui::CursorIcon::ResizeNwSe),
            Some(b) if b.corner_at(p).is_some() => Some(egui::CursorIcon::ResizeNeSw),
            Some(b) if b.rotate_at(p) => Some(egui::CursorIcon::Alias),
            Some(b) if b.contains(p) => Some(egui::CursorIcon::Move),
            _ => None,
        };
        if let Some(icon) = icon {
            ui.ctx().set_cursor_icon(icon);
        }
    }

    // ---- a click: select the clip under the pointer (the front-most), or let go of graphics
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        match boxes.iter().find(|b| b.contains(p)) {
            Some(b) if !app.session.state.selection.contains(&b.clip) => run(app, "timeline.select", json!({"clips": [b.clip.0]})),
            Some(_) => {}
            None => crate::panels::graphics::deselect_on_monitor(app, ui),
        }
    }

    // ---- a drag begins: on the selected clip's handles or box, else on another clip (selected first)
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let target = match &sel {
            Some(b) if b.corner_at(p).is_some() => Some((b.clone(), Grab::Scale)),
            Some(b) if b.rotate_at(p) => Some((b.clone(), Grab::Rotate)),
            Some(b) if b.contains(p) => Some((b.clone(), Grab::Move)),
            _ => boxes.iter().find(|b| b.contains(p)).map(|b| (b.clone(), Grab::Move)),
        };
        if let Some((b, grab)) = target {
            if !app.session.state.selection.contains(&b.clip) {
                run(app, "timeline.select", json!({"clips": [b.clip.0]}));
            }
            let mt = b.item.source_time_at(app.session.playhead());
            let m = b.item.effect("motion");
            let f = |id: &str, default: f64| m.filter(|e| e.param(id).is_some()).map(|e| e.f64_at(id, mt)).filter(|v| v.is_finite()).unwrap_or(default);
            let pos = m.filter(|e| e.param("position").is_some()).map(|e| e.vec2_at("position", mt)).filter(|v| v.x.is_finite() && v.y.is_finite());
            let pos = pos.unwrap_or(Vec2::new(sw / 2.0, sh / 2.0));
            let uniform = m.and_then(|e| e.param("uniform_scale")).and_then(|p| p.value.as_bool()).unwrap_or(true);
            let d = Drag {
                clip: b.clip,
                grab,
                press: p,
                anchor: b.anchor,
                k: (pic.width() as f64 / frame.0.max(1) as f64, pic.height() as f64 / frame.1.max(1) as f64),
                position: (pos.x, pos.y),
                scale: f("scale", 100.0),
                scale_width: f("scale_width", 100.0),
                rotation: f("rotation", 0.0),
                uniform,
                started: false,
            };
            ui.data_mut(|data| data.insert_temp(drag_id, d));
        }
    }

    // ---- while dragging: the new values, one undo step for the whole drag
    let drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id));
    if let Some(mut d) = drag
        && resp.dragged()
        && let Some(cur) = resp.interact_pointer_pos()
    {
        let shift = ui.input(|i| i.modifiers.shift);
        let mut changes: Vec<(&str, Value)> = Vec::new();
        match d.grab {
            Grab::Move => {
                let (mut dx, mut dy) = ((cur.x - d.press.x) as f64, (cur.y - d.press.y) as f64);
                // Shift: along one axis only
                if shift {
                    if dx.abs() > dy.abs() {
                        dy = 0.0;
                    } else {
                        dx = 0.0;
                    }
                }
                let (kx, ky) = (d.k.0.max(1e-9), d.k.1.max(1e-9));
                let np = [d.position.0 + dx / kx, d.position.1 + dy / ky];
                if np.iter().all(|v| v.is_finite()) {
                    changes.push(("position", json!([round2(np[0]), round2(np[1])])));
                }
            }
            Grab::Scale => {
                let r0 = (d.press - d.anchor).length();
                if r0 > 1.0 {
                    let f = ((cur - d.anchor).length() / r0) as f64;
                    if f.is_finite() {
                        changes.push(("scale", json!(round2((d.scale * f).clamp(0.0, 10_000.0)))));
                        if !d.uniform {
                            changes.push(("scale_width", json!(round2((d.scale_width * f).clamp(0.0, 10_000.0)))));
                        }
                    }
                }
            }
            Grab::Rotate => {
                let a0 = (d.press.y - d.anchor.y).atan2(d.press.x - d.anchor.x);
                let a1 = (cur.y - d.anchor.y).atan2(cur.x - d.anchor.x);
                let mut deg = d.rotation + ((a1 - a0) as f64).to_degrees();
                // Shift: in 45° steps
                if shift {
                    deg = (deg / 45.0).round() * 45.0;
                }
                if deg.is_finite() {
                    changes.push(("rotation", json!(round2(deg))));
                }
            }
        }
        for (param, value) in changes {
            let begin = !d.started;
            d.started = true;
            run(app, "effects.setParam", json!({"clip": d.clip.0, "effect": "motion", "param": param, "value": value, "merge": true, "begin": begin}));
        }
        ui.data_mut(|data| data.insert_temp(drag_id, d));
    }
    if resp.drag_stopped() || (drag.is_some() && !resp.dragged() && !resp.drag_started()) {
        ui.data_mut(|data| data.remove::<Drag>(drag_id));
    }
}

/// Two decimals: what Effect Controls shows, and no float noise in the project file.
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn run(app: &mut FilmcraftApp, command: &str, params: Value) {
    if let Err(e) = app.session.execute(command, params) {
        app.ui.status = e.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> ClipBox {
        let mut s = filmcraft_engine::Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        let it = s.active_sequence().unwrap().video_tracks[0].items[0].clone();
        ClipBox { clip: it.id, item: it, quad: [pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 50.0), pos2(0.0, 50.0)], anchor: pos2(50.0, 25.0) }
    }

    #[test]
    fn box_contains_and_handles() {
        let b = square();
        assert!(b.contains(pos2(50.0, 25.0)));
        assert!(!b.contains(pos2(150.0, 25.0)));
        assert_eq!(b.corner_at(pos2(99.0, 49.0)), Some(2));
        assert_eq!(b.corner_at(pos2(50.0, 25.0)), None);
        assert!(b.rotate_at(pos2(110.0, 60.0)), "just outside a corner rotates");
        assert!(!b.rotate_at(pos2(90.0, 40.0)), "inside the box moves, it doesn't rotate");
        // a degenerate box contains nothing (and doesn't panic)
        let mut z = square();
        z.quad = [pos2(5.0, 5.0); 4];
        assert!(!z.contains(pos2(5.0, 5.0)));
        let mut n = square();
        n.quad = [pos2(f32::NAN, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let _ = n.contains(pos2(0.5, 0.5));
    }

    #[test]
    fn rounding() {
        assert_eq!(round2(960.004), 960.0);
        assert_eq!(round2(162.456), 162.46);
    }
}
