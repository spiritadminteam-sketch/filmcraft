//! The Command Palette (Window ▸ Command Palette…): type, Enter runs the top match on the
//! selection; text fill colours and effects apply to every selected clip in one undo step.
//! Driven headless through the control channel.
#![allow(dead_code)]

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::{Label, Marker};
use filmcraft_time::Tick;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    /// The demo project with one marker at frame 30, the Timeline at 60 points a second.
    fn demo() -> (Self, u64) {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d.exec("markers.clearAll", json!({}));
        let id = d.exec("markers.add", json!({"frame": 30}))["marker"].as_u64().unwrap();
        d.exec("sequence.snap", json!({"on": false}));
        d.exec("playhead.set", json!({"frame": 0}));
        let v = &mut d.harness.state_mut().ui.timeline;
        (v.pps, v.target_pps, v.scroll, v.target_scroll) = (60.0, 60.0, 0.0, 0.0);
        d.frames(4);
        (d, id)
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
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
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
    fn center(&mut self, id: &str) -> (f64, f64) {
        let r = self.find(id).unwrap_or_else(|| panic!("no element {id}"));
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    }
    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }
    fn marker(&mut self, id: u64) -> Option<Marker> {
        self.frames(1);
        self.harness.state().session.active_sequence().unwrap().markers.iter().find(|m| m.id.0 == id).cloned()
    }
    fn frame(&self, n: i64) -> Tick {
        self.harness.state().session.sequence_rate().tick_of(n)
    }
}

fn fill_of(d: &mut Driver, clip: u64) -> String {
    d.frames(1);
    let s = &d.harness.state().session;
    let it = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).unwrap().1;
    let e = it.effects.iter().find(|e| e.effect == "graphic_text").unwrap();
    format!("{:?}", e.param("fill_color").unwrap().value)
}

/// Type "fill red", Enter: the selected text clips turn red, in one undo step; the palette closes.
#[test]
fn the_palette_colours_selected_text() {
    let (mut d, _) = Driver::demo();
    let a = d.exec("graphics.newText", json!({"text": "one", "newClip": true, "seconds": 1.0}))["clip"].as_u64().expect("clip");
    let t = d.frame(200).0;
    let b = d.exec("graphics.newText", json!({"text": "two", "newClip": true, "seconds": 1.0, "time": t}))["clip"].as_u64().expect("clip");
    assert_ne!(a, b);
    d.exec("timeline.select", json!({"clips": [a, b]}));
    let before = fill_of(&mut d, a);
    let undo = d.harness.state().session.history.undo.len();
    d.exec("window.commandPalette", json!({}));
    d.frames(3);
    assert!(d.harness.state().ui.palette.is_some(), "open");
    d.ok("ui.type", json!({"text": "fill red"}));
    d.frames(2);
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    assert!(d.harness.state().ui.palette.is_none(), "closed");
    let (fa, fb) = (fill_of(&mut d, a), fill_of(&mut d, b));
    assert_ne!(fa, before);
    assert_eq!(fa, fb, "both");
    assert!(fa.contains("1.0") && fa.contains("0.0"), "red: {fa}");
    assert_eq!(d.harness.state().session.history.undo.len(), undo + 1, "one undo step");
    // the colour ranks first next time
    assert_eq!(d.harness.state().ui.palette_recent.first().map(String::as_str), Some("fill:#FF0F0F"));
}

/// An effect by name, applied to the selected clip; Esc closes without running anything.
#[test]
fn the_palette_applies_an_effect_and_esc_closes() {
    let (mut d, _) = Driver::demo();
    let seq = d.exec("sequence.inspect", json!({}));
    let clip = seq["video"][0]["items"][0]["clip"].as_u64().unwrap();
    d.exec("timeline.select", json!({"clips": [clip]}));
    d.exec("window.commandPalette", json!({}));
    d.frames(3);
    d.ok("ui.type", json!({"text": "gaussian"}));
    d.frames(2);
    assert!(d.find("palette.item.0").is_some());
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    let has = |d: &mut Driver| {
        let s = &d.harness.state().session;
        s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).unwrap().1.effects.iter().any(|e| e.effect == "gaussian_blur")
    };
    assert!(has(&mut d), "Gaussian Blur applied");
    let undo = d.harness.state().session.history.undo.len();
    d.exec("window.commandPalette", json!({}));
    d.frames(3);
    d.ok("ui.type", json!({"text": "ripple delete"}));
    d.ok("ui.key", json!({"key": "Escape"}));
    d.frames(3);
    assert!(d.harness.state().ui.palette.is_none());
    assert_eq!(d.harness.state().session.history.undo.len(), undo, "nothing ran");
}

/// "label yellow" labels the selected clips Yellow (Excalibur's Label), not the Labels settings;
/// the palette is a window with a title to drag it by.
#[test]
fn the_palette_labels_selected_clips() {
    let (mut d, _) = Driver::demo();
    let seq = d.exec("sequence.inspect", json!({}));
    let clip = seq["video"][0]["items"][0]["clip"].as_u64().unwrap();
    d.exec("timeline.select", json!({"clips": [clip]}));
    d.exec("window.commandPalette", json!({}));
    d.frames(3);
    assert!(d.find("palette.window").is_some(), "a window");
    d.ok("ui.type", json!({"text": "label"}));
    d.frames(2);
    let top = d.ok("ui.elements", json!({"prefix": "palette.item.0"}));
    assert!(top.to_string().contains("Label: "), "a label first, not the Labels settings: {top}");
    d.ok("ui.type", json!({"text": " yellow"}));
    d.frames(2);
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    let s = &d.harness.state().session;
    let it = s.active_sequence().unwrap().find_item(filmcraft_project::ClipId(clip)).unwrap().1;
    assert_eq!(it.label, filmcraft_project::Label::Yellow);
    assert!(d.harness.state().ui.palette.is_none());
}
