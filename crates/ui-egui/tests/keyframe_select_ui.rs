//! Selecting several keyframes in Effect Controls, as in Premiere: Ctrl/Cmd+A with the panel
//! focused selects all of the clip's keyframes (not the Timeline's clips), Delete deletes them in
//! one undo step, a marquee on the lane selects the keyframes inside it, Shift + click adds one,
//! and dragging one of several selected keyframes moves them all. Driven headless through the
//! control channel.
//!
//! With `FILMCRAFT_UI_SHOTS=<dir>` the tests also render the UI with wgpu and write PNGs there.

#![allow(dead_code)]

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

/// Keyframes on Motion ▸ Scale and Position at `fractions` of the clip.
fn keyframes(d: &mut Driver, clip: &Clip, fractions: &[f64]) {
    for f in fractions {
        let t = clip.start + (clip.duration as f64 * f) as i64;
        for param in ["scale", "position"] {
            d.exec("effects.addKeyframe", json!({"clip": clip.id, "effect": "motion", "param": param, "time": t}));
        }
    }
    d.frames(3);
}

/// The clip's keyframe count on Motion ▸ `param`.
fn count(d: &Driver, clip: &Clip, param: &str) -> usize {
    let s = &d.harness.state().session;
    let it = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip.id)).unwrap().1;
    it.effect("motion").unwrap().param(param).map_or(0, |p| p.keyframes.len())
}

fn selected(d: &Driver) -> usize {
    d.harness.state().ui.keyframe_selection.len()
}

/// Ctrl/Cmd+A in Effect Controls selects every keyframe of the clip, and the Timeline's selection
/// stays as it was; Delete deletes them all in one undo step.
#[test]
fn select_all_and_delete_keyframes() {
    let (mut d, clip) = Driver::demo();
    keyframes(&mut d, &clip, &[0.2, 0.5, 0.8]);
    assert_eq!((count(&d, &clip, "scale"), count(&d, &clip, "position")), (3, 3));
    let before = d.exec("sequence.inspect", json!({}))["selection"].clone();
    d.ok("ui.set", json!({"focused": "EffectControls"}));
    d.ok("ui.key", json!({"key": "Cmd+A"}));
    d.frames(3);
    assert_eq!(selected(&d), 6, "all six keyframes");
    assert_eq!(d.exec("sequence.inspect", json!({}))["selection"], before, "the Timeline's selection is untouched");
    d.shot("ec_select_1_all");
    d.ok("ui.key", json!({"key": "Delete"}));
    d.frames(3);
    assert_eq!((count(&d, &clip, "scale"), count(&d, &clip, "position")), (0, 0), "all deleted");
    assert_eq!(selected(&d), 0);
    assert!(
        d.exec("sequence.inspect", json!({}))["video"][0]["items"].as_array().unwrap().iter().any(|c| c["clip"].as_u64() == Some(clip.id)),
        "the clip itself is still there"
    );
    d.exec("edit.undo", json!({}));
    d.frames(2);
    assert_eq!((count(&d, &clip, "scale"), count(&d, &clip, "position")), (3, 3), "one undo brings them all back");
}
