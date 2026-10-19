//! Window ▸ Odin Pro…: scan a preset pack, click a preset and it lands at the playhead; the
//! Command Palette lists the pack's presets too. Driven headless through the control channel.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_project::ItemKind;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

/// One preset: an adjustment layer with a Gaussian Blur, cut marker at 0.3 s.
const PROJECT: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
	<Sequence ObjectUID="seq-1"><MarkerOwner><Markers ObjectRef="50"/></MarkerOwner>
		<TrackGroups><TrackGroup Index="0"><First>v</First><Second ObjectRef="10"/></TrackGroup></TrackGroups>
		<Name>Simple Blur_01</Name>
	</Sequence>
	<Markers ObjectID="50"><Markers><Marker Index="0"><First>m</First><Second ObjectRef="51"/></Marker></Markers></Markers>
	<Marker ObjectID="51"><DVAMarker>{"DVAMarker":{"mStartTime":{"ticks":76204800000}}}</DVAMarker></Marker>
	<VideoTrackGroup ObjectID="10"><TrackGroup><Tracks><Track Index="0" ObjectURef="vt-1"/></Tracks></TrackGroup></VideoTrackGroup>
	<VideoClipTrack ObjectUID="vt-1"><ClipTrack><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="20"/></TrackItems></ClipItems></ClipTrack></VideoClipTrack>
	<VideoClipTrackItem ObjectID="20"><ClipTrackItem>
		<ComponentOwner><Components ObjectRef="30"/></ComponentOwner>
		<TrackItem><Start>50803200000</Start><End>101606400000</End></TrackItem>
		<SubClip ObjectRef="21"/>
	</ClipTrackItem></VideoClipTrackItem>
	<SubClip ObjectID="21"><Clip ObjectRef="22"/><Name>Blur</Name></SubClip>
	<VideoClip ObjectID="22"><Clip><Source ObjectRef="23"/><InPoint>0</InPoint><OutPoint>50803200000</OutPoint></Clip><AdjustmentLayer>true</AdjustmentLayer></VideoClip>
	<VideoComponentChain ObjectID="30"><ComponentChain><Components><Component Index="0" ObjectRef="31"/></Components></ComponentChain></VideoComponentChain>
	<VideoFilterComponent ObjectID="31"><Component><Params><Param Index="0" ObjectRef="40"/></Params><DisplayName>Gaussian Blur</DisplayName></Component><MatchName>AE.ADBE Gaussian Blur 2</MatchName></VideoFilterComponent>
	<VideoComponentParam ObjectID="40"><Name>Blurriness</Name><Keyframes>0,0.,0,0,0,0.16,0,0.16;50803200000,40.,0,0,0,0.16,0,0.16;</Keyframes><ParameterID>1</ParameterID></VideoComponentParam>
</PremiereData>
"#;

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new() -> Self {
        let mut session = Session::default();
        session.execute("file.openDemoProject", json!({})).expect("demo project");
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(session).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_step_dt(1.0 / 60.0).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
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
    fn ok(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
                return v["result"].clone();
            }
        }
        panic!("no reply to {method} {params}");
    }
    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }
    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }
    fn adjustment_clips(&self) -> usize {
        let s = &self.harness.state().session;
        let q = s.active_sequence().unwrap();
        q.video_tracks
            .iter()
            .flat_map(|t| &t.items)
            .filter(|i| matches!(s.project.item(i.item).map(|x| &x.kind), Some(ItemKind::AdjustmentLayer { .. })))
            .count()
    }
}

#[test]
fn a_clicked_preset_lands_at_the_playhead() {
    let root = std::env::temp_dir().join(format!("fc-odin-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Transitions/Blur")).unwrap();
    std::fs::write(root.join("Transitions/Blur/Simple.prproj"), PROJECT).unwrap();

    let mut d = Driver::new();
    d.exec("playhead.set", json!({"time": 5 * filmcraft_time::TICKS_PER_SECOND}));
    d.exec("window.odinPro", json!({}));
    d.frames(3);
    assert!(d.harness.state().ui.odin.is_some(), "the window is open");
    // type the pack folder and scan
    d.harness.state_mut().ui.odin.as_mut().unwrap().root = root.to_string_lossy().to_string();
    d.frames(2);
    d.click("odin.scan");
    assert!(d.harness.state().ui.odin_catalog.is_some(), "{:?}", d.harness.state().ui.odin);
    let before = d.adjustment_clips();
    d.click("odin.item.0");
    assert_eq!(d.adjustment_clips(), before + 1, "{:?}", d.harness.state().ui.odin);
    // the palette lists it too
    d.exec("window.commandPalette", json!({}));
    d.frames(3);
    d.ok("ui.type", json!({"text": "odin simple blur"}));
    d.frames(2);
    let top = d.ok("ui.elements", json!({"prefix": "palette.item.0"}));
    assert!(top.to_string().contains("Odin: Simple Blur_01"), "{top}");
    d.ok("ui.key", json!({"key": "Enter"}));
    d.frames(3);
    assert_eq!(d.adjustment_clips(), before + 2);
    // closing
    d.exec("window.odinPro", json!({}));
    d.frames(2);
    assert!(d.harness.state().ui.odin.is_none());
    let _ = std::fs::remove_dir_all(root);
}
