//! Direct manipulation of a video clip's Motion in the Program monitor, as in Premiere Pro: the
//! selected clip has a box with corner handles; dragging inside moves it (Position), a corner
//! scales it (Scale), and a drag is one undo step. A Position with keyframes gets one at the
//! playhead. Driven headless through the control channel.
//!
//! With `FILMCRAFT_UI_SHOTS=<dir>` the tests also render the UI with wgpu and write PNGs there.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::ClipId;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
    shots: Option<std::path::PathBuf>,
}

struct Clip {
    id: u64,
    start: i64,
    duration: i64,
}

impl Driver {
    /// The demo project with its first video clip selected and the playhead a second into it.
    fn demo() -> (Self, Clip) {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let shots = std::env::var_os("FILMCRAFT_UI_SHOTS").map(std::path::PathBuf::from);
        let mut b = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000);
        if shots.is_some() {
            b = b.wgpu().with_pixels_per_point(1.0);
        }
        let harness = b.build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx, shots };
        d.frames(4);
        let seq = d.exec("sequence.inspect", json!({}));
        let c = &seq["video"][0]["items"][0];
        let clip = Clip { id: c["clip"].as_u64().unwrap(), start: c["start"].as_i64().unwrap(), duration: c["duration"].as_i64().unwrap() };
        d.exec("timeline.select", json!({"clips": [clip.id]}));
        d.seek(&clip, 0.3);
        (d, clip)
    }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }
    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params.clone());
        assert_eq!(r["ok"], true, "{method} {params}: {r}");
        r["result"].clone()
    }
    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }
    fn find(&mut self, id: &str) -> Option<[f64; 4]> {
        self.frames(2);
        let v = self.ok("ui.elements", json!({"prefix": id}));
        let e = v.as_array().unwrap().iter().find(|e| e["id"] == id)?;
        let r: Vec<f64> = e["rect"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        Some([r[0], r[1], r[2], r[3]])
    }
    fn rect(&mut self, id: &str) -> [f64; 4] {
        self.find(id).unwrap_or_else(|| panic!("no element {id}"))
    }
    fn seek(&mut self, clip: &Clip, f: f64) {
        let t = clip.start + (clip.duration as f64 * f) as i64;
        self.exec("playhead.set", json!({"time": t}));
        self.frames(3);
    }
    /// Motion's (position, scale, position keyframe count) on the clip.
    fn motion(&mut self, clip: &Clip) -> ((f64, f64), f64, usize) {
        self.frames(2);
        let s = &self.harness.state().session;
        let it = s.active_sequence().unwrap().find_item(ClipId(clip.id)).unwrap().1;
        let m = it.effect("motion").unwrap();
        let mt = it.source_time_at(s.playhead());
        let p = m.vec2_at("position", mt);
        let p = if p.x.is_nan() { (960.0, 540.0) } else { (p.x, p.y) };
        (p, m.f64_at("scale", mt), m.param("position").map_or(0, |p| p.keyframes.len()))
    }
    /// Sequence pixels per screen point across the picture.
    fn per_point(&mut self) -> f64 {
        let pic = self.rect("program.picture");
        let w = self.harness.state().session.active_sequence().unwrap().settings.width as f64;
        w / pic[2]
    }
    fn drag(&mut self, from: (f64, f64), to: (f64, f64)) {
        self.ok("ui.drag", json!({"from": {"x": from.0, "y": from.1}, "to": {"x": to.0, "y": to.1}, "steps": 8}));
        self.frames(4);
    }
    fn shot(&mut self, name: &str) {
        let Some(dir) = self.shots.clone() else { return };
        for _ in 0..40 {
            self.frames(1);
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let img = self.harness.render().expect("wgpu render");
        std::fs::create_dir_all(&dir).unwrap();
        img.save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// Dragging inside the selected clip's box moves it by the same distance in sequence pixels, in
/// one undo step.
#[test]
fn drag_in_the_picture_moves_the_clip() {
    let (mut d, clip) = Driver::demo();
    let bx = d.rect("program.motion.box");
    d.shot("motion_1_selected");
    let (p0, s0, _) = d.motion(&clip);
    let k = d.per_point();
    let c = (bx[0] + bx[2] / 2.0, bx[1] + bx[3] / 2.0);
    d.drag(c, (c.0 + 80.0, c.1 + 40.0));
    let (p1, s1, keys) = d.motion(&clip);
    d.shot("motion_2_moved");
    // (egui may hold back the first few points of a drag)
    assert!((p1.0 - p0.0 - 80.0 * k).abs() <= 8.0 * k, "moved right by 80 points: {p0:?} -> {p1:?} (k {k})");
    assert!((p1.1 - p0.1 - 40.0 * k).abs() <= 8.0 * k, "moved down by 40 points: {p0:?} -> {p1:?}");
    assert_eq!(s1, s0, "moving doesn't scale");
    assert_eq!(keys, 0, "a Position without keyframes just takes the new value");
    // the whole drag is one undo step
    d.exec("edit.undo", json!({}));
    let (p2, _, _) = d.motion(&clip);
    assert!((p2.0 - p0.0).abs() < 0.01 && (p2.1 - p0.1).abs() < 0.01, "one undo puts it back: {p0:?} vs {p2:?}");
}

/// A corner handle scales the clip about its anchor point.
#[test]
fn corner_handle_scales_the_clip() {
    let (mut d, clip) = Driver::demo();
    let (_, s0, _) = d.motion(&clip);
    let h = d.rect("program.motion.handle.2");
    let a = d.rect("program.motion.anchor");
    let (hx, hy) = (h[0] + h[2] / 2.0, h[1] + h[3] / 2.0);
    let (ax, ay) = (a[0] + a[2] / 2.0, a[1] + a[3] / 2.0);
    // pull the bottom-right corner halfway back towards the anchor point: about 50 %
    d.drag((hx, hy), (ax + (hx - ax) * 0.5, ay + (hy - ay) * 0.5));
    let (_, s1, _) = d.motion(&clip);
    d.shot("motion_3_scaled");
    assert!(s1 < s0 * 0.6 && s1 > s0 * 0.4, "about half the scale: {s0} -> {s1}");
}

/// With keyframes on Position, a drag at another time adds a keyframe at the playhead, as
/// typing the value in Effect Controls does.
#[test]
fn drag_with_keyframes_adds_one_at_the_playhead() {
    let (mut d, clip) = Driver::demo();
    d.exec("effects.addKeyframe", json!({"clip": clip.id, "effect": "motion", "param": "position"}));
    assert_eq!(d.motion(&clip).2, 1);
    d.seek(&clip, 0.7);
    let bx = d.rect("program.motion.box");
    let c = (bx[0] + bx[2] / 2.0, bx[1] + bx[3] / 2.0);
    d.drag(c, (c.0 - 60.0, c.1));
    let (p, _, keys) = d.motion(&clip);
    assert_eq!(keys, 2, "a keyframe at the playhead");
    assert!(p.0 < 960.0, "moved left there: {p:?}");
    // and the first keyframe still has the old position
    d.seek(&clip, 0.3);
    let (p0, _, _) = d.motion(&clip);
    assert!((p0.0 - 960.0).abs() < 0.01, "the earlier keyframe is untouched: {p0:?}");
}

/// A click in the picture on a clip that isn't selected selects it.
#[test]
fn click_in_the_picture_selects_the_clip() {
    let (mut d, clip) = Driver::demo();
    d.exec("timeline.select", json!({"clips": []}));
    d.frames(3);
    assert!(d.find("program.motion.box").is_none(), "no box without a selected clip");
    let pic = d.rect("program.picture");
    d.ok("ui.click", json!({"x": pic[0] + pic[2] * 0.5, "y": pic[1] + pic[3] * 0.5}));
    d.frames(4);
    let sel = d.exec("sequence.inspect", json!({}))["selection"].clone();
    assert!(sel.as_array().is_some_and(|s| s.iter().any(|c| c.as_u64() == Some(clip.id))), "the clip under the pointer is selected: {sel}");
    assert!(d.find("program.motion.box").is_some(), "and has its box");
}

/// A click on the Program monitor around the picture lets go of the selected clip (its box goes).
#[test]
fn a_click_around_the_picture_deselects_the_clip() {
    let (mut d, _clip) = Driver::demo();
    assert!(d.find("program.motion.box").is_some());
    let pic = d.rect("program.picture");
    let panel = d.rect("panel.Program");
    let outside = if pic[0] - panel[0] > 20.0 { (pic[0] - 8.0, pic[1] + pic[3] / 2.0) } else { (pic[0] + pic[2] / 2.0, pic[1] + pic[3] + 8.0) };
    d.ok("ui.click", json!({"x": outside.0, "y": outside.1}));
    d.frames(4);
    assert!(d.harness.state().session.state.selection.is_empty(), "deselected");
    assert!(d.find("program.motion.box").is_none(), "no box");
}
