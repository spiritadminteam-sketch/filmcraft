//! Effect Controls' keyframe lane zooms like Premiere's: a zoom scroller under the lane (drag an
//! end to zoom, drag the thumb to scroll, double-click for the whole clip again), Option/Alt +
//! wheel zooms about the pointer, and the view follows the playhead when it leaves the view.
//! Keyframes outside the view are neither drawn nor hit. Driven headless through the control
//! channel.
//!
//! With `FILMCRAFT_UI_SHOTS=<dir>` the tests also render the UI with wgpu and write PNGs there.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
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
    /// The demo project with its first video clip selected and Effect Controls showing.
    fn demo() -> (Self, Clip) {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let shots = std::env::var_os("FILMCRAFT_UI_SHOTS").map(std::path::PathBuf::from);
        let mut b = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000);
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
        d.ok("ui.panel.show", json!({"panel": "EffectControls"}));
        d.frames(3);
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
    /// The element's rect `[x, y, w, h]`, or `None` when it is not on screen.
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
    /// The lane's zoom and start.
    fn view(&mut self) -> (f32, f32) {
        self.frames(2);
        let v = self.harness.state().ui.keyframe_view;
        (v.zoom, v.start)
    }
    fn seek_fraction(&mut self, clip: &Clip, f: f64) {
        let t = clip.start + (clip.duration as f64 * f) as i64;
        self.exec("playhead.set", json!({"time": t}));
        self.frames(3);
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

/// Scale keyframes at `fractions` of the clip; their media times.
fn scale_keyframes(d: &mut Driver, clip: &Clip, fractions: &[f64]) -> Vec<i64> {
    for f in fractions {
        let t = clip.start + (clip.duration as f64 * f) as i64;
        d.exec("effects.addKeyframe", json!({"clip": clip.id, "effect": "motion", "param": "scale", "time": t}));
    }
    d.frames(3);
    let s = &d.harness.state().session;
    let it = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip.id)).unwrap().1;
    it.effect("motion").unwrap().param("scale").unwrap().keyframes.iter().map(|k| k.time.0).collect()
}

/// The scroller starts out showing the whole clip; dragging its right end to a quarter of the
/// track zooms in about 4×, and keyframes outside the view leave the lane. Double-clicking the
/// thumb shows the whole clip again.
#[test]
fn zoom_scroller_end_zooms_and_hides_keyframes_out_of_view() {
    let (mut d, clip) = Driver::demo();
    let keys = scale_keyframes(&mut d, &clip, &[0.1, 0.9]);
    assert_eq!(keys.len(), 2);
    d.seek_fraction(&clip, 0.05);
    let track = d.rect("effectControls.zoom");
    let thumb = d.rect("effectControls.zoom.thumb");
    assert!((thumb[2] - track[2]).abs() < 2.0, "the whole clip at first: thumb {thumb:?}, track {track:?}");
    assert_eq!(d.view(), (1.0, 0.0));
    let early = format!("effectControls.motion.scale.keyframe.{}", keys[0]);
    let late = format!("effectControls.motion.scale.keyframe.{}", keys[1]);
    assert!(d.find(&early).is_some() && d.find(&late).is_some(), "both keyframes in the lane");
    d.shot("ec_zoom_1_whole_clip");

    // drag the right end to a quarter of the track
    let end = d.rect("effectControls.zoom.end");
    let to_x = track[0] + track[2] * 0.25;
    d.ok("ui.drag", json!({"from": {"x": end[0] + end[2] / 2.0, "y": end[1] + end[3] / 2.0}, "to": {"x": to_x, "y": end[1] + end[3] / 2.0}}));
    d.frames(4);
    let (zoom, start) = d.view();
    // (egui may hold back the first few points of a drag)
    assert!((2.8..=4.6).contains(&zoom), "about 4× after dragging the end to a quarter: {zoom}");
    assert_eq!(start, 0.0);
    let thumb = d.rect("effectControls.zoom.thumb");
    assert!((thumb[2] - track[2] / zoom as f64).abs() < 3.0, "the thumb is the visible stretch: {thumb:?}");
    assert!(d.find(&early).is_some(), "the keyframe at 10 % is in view");
    assert!(d.find(&late).is_none(), "the keyframe at 90 % is out of view, so not in the lane");
    d.shot("ec_zoom_2_zoomed_in");

    // a click on the track (right of the thumb) centres the view there
    let before = d.view();
    let thumb = d.rect("effectControls.zoom.thumb");
    let elements = d.ok("ui.elements", json!({"prefix": "effectControls"}));
    d.ok("ui.click", json!({"id": "effectControls.zoom", "fx": 0.8}));
    d.frames(4);
    let after = d.view();
    assert!(after.1 > before.1, "a click on the track scrolls: {before:?} -> {after:?}; track {track:?} thumb {thumb:?}; elements {elements}");
    // double-click the scroller: the whole clip again
    d.ok("ui.click", json!({"id": "effectControls.zoom", "fx": 0.05, "count": 2}));
    d.frames(4);
    assert_eq!(d.view(), (1.0, 0.0), "double-click shows the whole clip");
}

/// Dragging the thumb scrolls the view; the keyframe that comes into view is in the lane.
#[test]
fn zoom_scroller_thumb_scrolls() {
    let (mut d, clip) = Driver::demo();
    let keys = scale_keyframes(&mut d, &clip, &[0.1, 0.9]);
    d.seek_fraction(&clip, 0.05);
    // 4× at the start of the clip
    d.harness.state_mut().ui.keyframe_view = filmcraft_ui_egui::state::KeyframeLaneView { clip: clip.id, zoom: 4.0, start: 0.0 };
    d.frames(3);
    let track = d.rect("effectControls.zoom");
    let thumb = d.rect("effectControls.zoom.thumb");
    let y = thumb[1] + thumb[3] / 2.0;
    let from_x = thumb[0] + thumb[2] / 2.0;
    // (past the end: the view stops at the clip's end)
    d.ok("ui.drag", json!({"from": {"x": from_x, "y": y}, "to": {"x": from_x + track[2], "y": y}}));
    d.frames(4);
    let (zoom, start) = d.view();
    assert_eq!(zoom, 4.0, "scrolling keeps the zoom");
    assert!((start - 0.75).abs() < 1e-4, "scrolled to the end: {start}");
    let late = format!("effectControls.motion.scale.keyframe.{}", keys[1]);
    assert!(d.find(&late).is_some(), "the keyframe at 90 % came into view");
}

/// Option/Alt + wheel over the lane zooms about the pointer; the wheel alone does not zoom.
#[test]
fn alt_wheel_zooms_the_lane() {
    let (mut d, _clip) = Driver::demo();
    d.ok("ui.scroll", json!({"id": "effectControls.lane", "dy": 120.0}));
    d.frames(3);
    assert_eq!(d.view().0, 1.0, "the wheel alone scrolls the effect list, not the zoom");
    d.ok("ui.scroll", json!({"id": "effectControls.lane", "fx": 0.5, "dy": 120.0, "alt": true}));
    d.frames(3);
    let (zoom, start) = d.view();
    assert!(zoom > 1.5, "Alt + wheel zooms in: {zoom}");
    // about the pointer (the middle): the middle of the clip stays in the middle of the view
    let mid = start + 0.5 / zoom;
    assert!((mid - 0.5).abs() < 0.05, "zoomed about the pointer: start {start}, zoom {zoom}");
    d.ok("ui.scroll", json!({"id": "effectControls.lane", "fx": 0.5, "dy": -2000.0, "alt": true}));
    for _ in 0..6 {
        d.ok("ui.scroll", json!({"id": "effectControls.lane", "fx": 0.5, "dy": -400.0, "alt": true}));
    }
    assert_eq!(d.view(), (1.0, 0.0), "zooming out stops at the whole clip");
}

/// When the playhead moves out of the view (a click in the Timeline, playback), the view follows.
#[test]
fn view_follows_the_playhead() {
    let (mut d, clip) = Driver::demo();
    d.seek_fraction(&clip, 0.05);
    d.harness.state_mut().ui.keyframe_view = filmcraft_ui_egui::state::KeyframeLaneView { clip: clip.id, zoom: 5.0, start: 0.0 };
    d.frames(3);
    d.seek_fraction(&clip, 0.8);
    let (zoom, start) = d.view();
    assert_eq!(zoom, 5.0);
    assert!(start <= 0.8 && 0.8 <= start + 1.0 / zoom, "the playhead (80 %) is in view: start {start}");
    assert!(d.find("effectControls.playhead").is_some(), "its handle is on the ruler");
    // another clip: the whole clip again
    let seq = d.exec("sequence.inspect", json!({}));
    let other = seq["video"][0]["items"][1]["clip"].as_u64().unwrap();
    d.exec("timeline.select", json!({"clips": [other]}));
    d.frames(3);
    assert_eq!(d.view(), (1.0, 0.0), "selecting another clip resets the zoom");
}
