//! Editing sequence markers on the Timeline ruler, as in Premiere Pro: a marker drags along the
//! ruler (onto a frame, one undo step, the playhead following it); its right-click menu sets its
//! colour; a double-click opens the Marker dialog (name, comments, colour, Delete).
//! Driven headless through the control channel.

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

/// Dragging a marker moves it along the ruler by the time the pointer moved, onto a frame, in one
/// undo step; the playhead goes with it (it stays the selected marker).
#[test]
fn a_marker_drags_along_the_ruler() {
    let (mut d, id) = Driver::demo();
    let start = d.marker(id).unwrap().start;
    let undo = d.harness.state().session.history.undo.len();
    let (x, y) = d.center(&format!("timeline.marker.{id}"));
    // 120 points at 60 points a second: two seconds later
    d.ok("ui.drag", json!({"from": {"x": x, "y": y}, "to": {"x": x + 120.0, "y": y}, "steps": 10}));
    d.frames(4);
    let m = d.marker(id).unwrap();
    let rate = d.harness.state().session.sequence_rate();
    assert_eq!(rate.snap_nearest(m.start), m.start, "on a frame");
    let moved = (m.start - start).seconds();
    assert!((moved - 2.0).abs() < 0.1, "moved about two seconds: {moved}");
    assert_eq!(d.harness.state().session.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(d.harness.state().session.playhead(), m.start, "the playhead follows it");
    d.exec("edit.undo", json!({}));
    assert_eq!(d.marker(id).unwrap().start, start, "undo puts it back");
    // a plain click still only selects it (no move)
    d.ok("ui.click", json!({"id": format!("timeline.marker.{id}")}));
    d.frames(3);
    assert_eq!(d.marker(id).unwrap().start, start);
    assert_eq!(d.harness.state().session.playhead(), start);
}

/// Right-click a marker: its Label menu sets its colour; Clear Selected Marker clears that marker.
#[test]
fn the_marker_menu_sets_its_colour() {
    let (mut d, id) = Driver::demo();
    assert_eq!(d.marker(id).unwrap().color, Label::Green);
    d.ok("ui.click", json!({"id": format!("timeline.marker.{id}"), "button": "right"}));
    d.frames(3);
    d.click("timeline.markerMenu.label");
    d.click("timeline.markerMenu.color.Yellow");
    assert_eq!(d.marker(id).unwrap().color, Label::Yellow);
    // the menu has closed
    assert!(d.find("timeline.markerMenu.color.Rose").is_none());
    let other = d.exec("markers.add", json!({"frame": 90}))["marker"].as_u64().unwrap();
    d.ok("ui.click", json!({"id": format!("timeline.marker.{id}"), "button": "right"}));
    d.frames(3);
    d.click("timeline.markerMenu.markers.clearCurrent");
    assert!(d.marker(id).is_none(), "cleared");
    assert!(d.marker(other).is_some(), "the other one stays");
}

/// Double-click a marker: the Marker dialog edits its name, comments and colour (OK), or deletes it.
#[test]
fn double_click_opens_the_marker_dialog() {
    let (mut d, id) = Driver::demo();
    let (x, y) = d.center(&format!("timeline.marker.{id}"));
    d.ok("ui.click", json!({"x": x, "y": y, "count": 2}));
    d.frames(4);
    assert!(d.harness.state().ui.marker_dialog.is_some(), "the dialog is open");
    d.click("markerDialog.name");
    d.ok("ui.type", json!({"text": "Random"}));
    d.click("markerDialog.comment");
    d.ok("ui.type", json!({"text": "speaking"}));
    d.click("markerDialog.color.Rose");
    d.click("markerDialog.ok");
    let m = d.marker(id).unwrap();
    assert_eq!((m.name.as_str(), m.comment.as_str(), m.color), ("Random", "speaking", Label::Rose));
    assert!(d.harness.state().ui.marker_dialog.is_none(), "closed");
    // Edit Marker… from the menu opens it too; Delete deletes the marker
    d.ok("ui.click", json!({"id": format!("timeline.marker.{id}"), "button": "right"}));
    d.frames(3);
    d.click("timeline.markerMenu.edit");
    assert_eq!(d.harness.state().ui.marker_dialog.as_ref().map(|m| m.name.clone()), Some("Random".to_string()));
    d.click("markerDialog.delete");
    assert!(d.marker(id).is_none());
}

/// With snapping on, a marker dragged near a cut (a clip's end, e.g. one made with Add Edit)
/// lands exactly on it.
#[test]
fn a_dragged_marker_snaps_to_a_cut() {
    let (mut d, id) = Driver::demo();
    d.exec("sequence.snap", json!({"on": true}));
    let start = d.marker(id).unwrap().start;
    // an Add Edit a little after the marker, on every track
    let cut = start + d.frame(70);
    d.exec("playhead.set", json!({"time": cut.0}));
    d.exec("sequence.addEdit", json!({}));
    d.exec("playhead.set", json!({"frame": 0}));
    d.frames(3);
    let (x, y) = d.center(&format!("timeline.marker.{id}"));
    // to 3 points short of the cut, at 60 points a second
    let dx = (cut - start).seconds() * 60.0 - 3.0;
    d.ok("ui.drag", json!({"from": {"x": x, "y": y}, "to": {"x": x + dx, "y": y}, "steps": 10}));
    d.frames(4);
    assert_eq!(d.marker(id).unwrap().start, cut, "on the cut");
}
