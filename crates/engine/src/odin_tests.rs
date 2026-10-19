//! Odin Pro packs: scanning a pack folder and dropping a preset on the timeline.

use serde_json::json;

use filmcraft_project::{ItemKind, ParamValue};
use filmcraft_time::Tick;

use crate::Session;

/// A preset: an adjustment layer with a keyframed Transform from 0.2 s to 0.4 s, a sound from 0.03 s,
/// cut marker at 0.3 s. The sound's path is the pack author's: it is found in the pack by name.
const PROJECT: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
	<Sequence ObjectUID="seq-1"><MarkerOwner><Markers ObjectRef="50"/></MarkerOwner>
		<TrackGroups>
			<TrackGroup Index="0"><First>v</First><Second ObjectRef="10"/></TrackGroup>
			<TrackGroup Index="1"><First>a</First><Second ObjectRef="11"/></TrackGroup>
		</TrackGroups>
		<Name>Zoom IN</Name>
	</Sequence>
	<Markers ObjectID="50"><Markers><Marker Index="0"><First>m</First><Second ObjectRef="51"/></Marker></Markers></Markers>
	<Marker ObjectID="51"><DVAMarker>{"DVAMarker":{"mStartTime":{"ticks":76204800000}}}</DVAMarker></Marker>
	<VideoTrackGroup ObjectID="10"><TrackGroup><Tracks><Track Index="0" ObjectURef="vt-1"/><Track Index="1" ObjectURef="vt-2"/></Tracks></TrackGroup></VideoTrackGroup>
	<AudioTrackGroup ObjectID="11"><TrackGroup><Tracks><Track Index="0" ObjectURef="at-1"/></Tracks></TrackGroup></AudioTrackGroup>
	<VideoClipTrack ObjectUID="vt-1"><ClipTrack><ClipItems></ClipItems></ClipTrack></VideoClipTrack>
	<VideoClipTrack ObjectUID="vt-2"><ClipTrack><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="20"/></TrackItems></ClipItems></ClipTrack></VideoClipTrack>
	<AudioClipTrack ObjectUID="at-1"><ClipTrack><ClipItems><TrackItems><TrackItem Index="0" ObjectRef="80"/></TrackItems></ClipItems></ClipTrack></AudioClipTrack>
	<VideoClipTrackItem ObjectID="20"><ClipTrackItem>
		<ComponentOwner><Components ObjectRef="30"/></ComponentOwner>
		<TrackItem><Start>50803200000</Start><End>101606400000</End></TrackItem>
		<SubClip ObjectRef="21"/>
	</ClipTrackItem></VideoClipTrackItem>
	<SubClip ObjectID="21"><Clip ObjectRef="22"/><Name>Zoom IN</Name></SubClip>
	<VideoClip ObjectID="22"><Clip><Source ObjectRef="23"/><InPoint>914400000000000</InPoint><OutPoint>914450803200000</OutPoint></Clip><AdjustmentLayer>true</AdjustmentLayer></VideoClip>
	<VideoComponentChain ObjectID="30"><ComponentChain><Components><Component Index="0" ObjectRef="31"/></Components></ComponentChain></VideoComponentChain>
	<VideoFilterComponent ObjectID="31"><Component><Params><Param Index="0" ObjectRef="40"/></Params><DisplayName>Transform</DisplayName></Component><MatchName>AE.ADBE Geometry2</MatchName></VideoFilterComponent>
	<VideoComponentParam ObjectID="40"><Name>Scale Height</Name><Keyframes>914400000000000,100.,0,0,0,0.16,0,0.16;914450803200000,300.,0,0,0,0.16,0,0.16;</Keyframes><ParameterID>3</ParameterID></VideoComponentParam>
	<AudioClipTrackItem ObjectID="80"><ClipTrackItem>
		<TrackItem><Start>8467200000</Start><End>59270400000</End></TrackItem>
		<SubClip ObjectRef="81"/>
	</ClipTrackItem></AudioClipTrackItem>
	<SubClip ObjectID="81"><Clip ObjectRef="82"/><Name>whoosh.wav</Name></SubClip>
	<AudioClip ObjectID="82"><Clip><Source ObjectRef="83"/><InPoint>0</InPoint><OutPoint>50803200000</OutPoint></Clip></AudioClip>
	<AudioMediaSource ObjectID="83"><MediaSource><Media ObjectURef="media-2"/></MediaSource></AudioMediaSource>
	<Media ObjectUID="media-2"><FilePath>D:\Packs\SFX\whoosh.wav</FilePath><ActualMediaFilePath>D:\Packs\SFX\whoosh.wav</ActualMediaFilePath></Media>
</PremiereData>
"#;

/// A pack folder: the project, its sound under `_Assets`, a sound library and an After Effects template.
fn pack(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("fc-odin-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dirs = ["Odin Pro Premiere Pro/Transitions/Zoom", "Odin Pro Premiere Pro/_Assets", "Sound FX/Clicks", "Odin Pro Premiere Pro/Titles"];
    for d in dirs {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("Odin Pro Premiere Pro/Transitions/Zoom/Zoom.prproj"), PROJECT).unwrap();
    // half a second of a 440 Hz tone, 48 kHz 16-bit stereo
    let mut pcm = Vec::new();
    for i in 0..24_000 {
        let v = ((i as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 8000.0) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    let wav = filmcraft_interchange::wav::wav_file(&pcm, 2, 48_000, 16);
    std::fs::write(root.join("Odin Pro Premiere Pro/_Assets/whoosh.wav"), &wav).unwrap();
    std::fs::write(root.join("Sound FX/Clicks/Click 1.wav"), &wav).unwrap();
    std::fs::write(root.join("Odin Pro Premiere Pro/Titles/Big Title 01.mogrt"), b"PK").unwrap();
    root
}

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

#[test]
fn scan_lists_presets_sounds_and_counts_templates() {
    let root = pack("scan");
    let mut s = demo();
    let r = s.execute("odin.scan", json!({"root": root.to_string_lossy()})).unwrap();
    assert_eq!(r["presets"][0]["names"], json!(["Zoom IN"]));
    assert_eq!(r["presets"][0]["group"], "Odin Pro Premiere Pro / Transitions / Zoom");
    assert_eq!(r["sounds"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["templates"], 1);
    assert_eq!(s.prefs.media_browser.odin_root, root.to_string_lossy(), "the folder is remembered");
    // and used when not given
    assert!(s.execute("odin.scan", json!({})).is_ok());
    assert!(s.execute("odin.scan", json!({"root": root.join("nope").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_preset_lands_on_the_playhead_above_the_footage_in_one_undo_step() {
    let root = pack("insert");
    let mut s = demo();
    let file = root.join("Odin Pro Premiere Pro/Transitions/Zoom/Zoom.prproj");
    let at = Tick(5 * filmcraft_time::TICKS_PER_SECOND);
    s.set_playhead(at);
    let undo = s.history.undo.len();
    let (v0, a0) = {
        let q = s.active_sequence().unwrap();
        (q.video_tracks.len(), q.audio_tracks.len())
    };
    let r = s.execute("odin.insertPreset", json!({"file": file.to_string_lossy(), "name": "Zoom IN"})).unwrap();
    assert_eq!(r["clips"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let q = s.active_sequence().unwrap();
    // the adjustment layer: 0.2 s → 0.4 s of the preset, its marker (0.3 s) on the playhead
    let (adj, track) = q
        .video_tracks
        .iter()
        .enumerate()
        .find_map(|(k, t)| t.items.iter().find(|i| matches!(s.project.item(i.item).map(|x| &x.kind), Some(ItemKind::AdjustmentLayer { .. }))).map(|i| (i, k)))
        .expect("an adjustment layer clip");
    let marker = Tick(76_204_800_000);
    assert_eq!(adj.start, at - marker + Tick(50_803_200_000));
    assert_eq!(adj.duration, Tick(50_803_200_000));
    let below_has_footage = q.video_tracks[..track].iter().any(|t| t.items.iter().any(|i| i.start < adj.end() && adj.start < i.end()));
    assert!(below_has_footage, "it sits above the footage it changes");
    let t = adj.effect("transform").expect("Transform");
    let k = &t.params["scale_height"].keyframes;
    assert_eq!((k[0].time, k[1].time), (Tick(0), Tick(50_803_200_000)));
    assert_eq!(k[1].value, ParamValue::Float(300.0));
    assert!(!t.params["position"].value.as_vec2().is_some_and(|p| p.x.is_nan()), "automatic points are centred");
    // the sound, found in the pack by name, on a track above A1
    let (sfx_track, sfx) =
        q.audio_tracks.iter().enumerate().find_map(|(k, t)| t.items.iter().find(|i| i.name == "whoosh.wav").map(|i| (k, i))).expect("the sound");
    assert!(sfx_track >= 1);
    assert_eq!(sfx.start, at - marker + Tick(8_467_200_000));
    assert!(q.video_tracks.len() >= v0 && q.audio_tracks.len() >= a0);
    // undo takes all of it away
    s.undo();
    let q = s.active_sequence().unwrap();
    assert!(!q.audio_tracks.iter().any(|t| t.items.iter().any(|i| i.name == "whoosh.wav")));
    assert_eq!((q.video_tracks.len(), q.audio_tracks.len()), (v0, a0));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bad_requests_are_errors() {
    let root = pack("bad");
    let mut s = demo();
    let file = root.join("Odin Pro Premiere Pro/Transitions/Zoom/Zoom.prproj");
    assert!(s.execute("odin.insertPreset", json!({"file": file.to_string_lossy(), "name": "Nope"})).is_err());
    assert!(s.execute("odin.insertPreset", json!({"file": root.join("x.prproj").to_string_lossy(), "name": "Zoom IN"})).is_err());
    assert!(s.execute("odin.insertPreset", json!({})).is_err());
    let sound = root.join("Sound FX/Clicks/Click 1.wav");
    let r = s.execute("odin.insertMedia", json!({"path": sound.to_string_lossy()})).unwrap();
    assert_eq!(r["clips"].as_array().unwrap().len(), 1);
    assert!(s.execute("odin.insertMedia", json!({"path": root.join("none.wav").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(root);
}
