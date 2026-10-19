use super::*;

/// A small project shaped like a preset pack's: one transition (an adjustment layer with a
/// keyframed Transform, an overlay file blended Linear Dodge, a sound effect) and a second preset.
const PROJECT: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
	<Sequence ObjectUID="seq-1" ClassID="x" Version="11">
		<MarkerOwner Version="1"><Markers ObjectRef="50"/></MarkerOwner>
		<TrackGroups Version="1">
			<TrackGroup Version="1" Index="0"><First>v</First><Second ObjectRef="10"/></TrackGroup>
			<TrackGroup Version="1" Index="1"><First>a</First><Second ObjectRef="11"/></TrackGroup>
		</TrackGroups>
		<Name>Zoom IN</Name>
	</Sequence>
	<Sequence ObjectUID="seq-2" ClassID="x" Version="11">
		<Name>Zoom OUT</Name>
	</Sequence>
	<Markers ObjectID="50" Version="4"><Markers Version="1">
		<Marker Version="1" Index="0"><First>m2</First><Second ObjectRef="52"/></Marker>
		<Marker Version="1" Index="1"><First>m1</First><Second ObjectRef="51"/></Marker>
	</Markers></Markers>
	<Marker ObjectID="51"><DVAMarker>{"DVAMarker":{"mStartTime":{"ticks":93139200000},"mType":"Comment"}}</DVAMarker></Marker>
	<Marker ObjectID="52"><DVAMarker>{"DVAMarker":{"mStartTime":{"ticks":120000000000}}}</DVAMarker></Marker>
	<VideoTrackGroup ObjectID="10"><TrackGroup Version="1"><Tracks Version="1">
		<Track Index="0" ObjectURef="vt-1"/>
		<Track Index="1" ObjectURef="vt-2"/>
		<Track Index="2" ObjectURef="vt-3"/>
	</Tracks></TrackGroup></VideoTrackGroup>
	<AudioTrackGroup ObjectID="11"><TrackGroup Version="1"><Tracks Version="1">
		<Track Index="0" ObjectURef="at-1"/>
	</Tracks></TrackGroup></AudioTrackGroup>
	<VideoClipTrack ObjectUID="vt-1"><ClipTrack Version="2"><Track Version="3"><ID>1</ID></Track><ClipItems Version="3"></ClipItems></ClipTrack></VideoClipTrack>
	<VideoClipTrack ObjectUID="vt-2"><ClipTrack Version="2"><Track Version="3"><ID>2</ID></Track>
		<ClipItems Version="3"><TrackItems Version="1"><TrackItem Index="0" ObjectRef="20"/></TrackItems></ClipItems>
	</ClipTrack></VideoClipTrack>
	<VideoClipTrack ObjectUID="vt-3"><ClipTrack Version="2"><Track Version="3"><ID>3</ID></Track>
		<ClipItems Version="3"><TrackItems Version="1"><TrackItem Index="0" ObjectRef="60"/></TrackItems></ClipItems>
	</ClipTrack></VideoClipTrack>
	<AudioClipTrack ObjectUID="at-1"><ClipTrack Version="2"><Track Version="3"><ID>1</ID></Track>
		<ClipItems Version="3"><TrackItems Version="1"><TrackItem Index="0" ObjectRef="80"/></TrackItems></ClipItems>
	</ClipTrack></AudioClipTrack>

	<VideoClipTrackItem ObjectID="20"><ClipTrackItem Version="8">
		<ComponentOwner Version="1"><Components ObjectRef="30"/></ComponentOwner>
		<TrackItem Version="3"><Start>42336000000</Start><End>93139200000</End></TrackItem>
		<SubClip ObjectRef="21"/>
	</ClipTrackItem></VideoClipTrackItem>
	<SubClip ObjectID="21"><Clip ObjectRef="22"/><Name>Zoom IN</Name></SubClip>
	<VideoClip ObjectID="22"><Clip Version="18"><Source ObjectRef="23"/><InPoint>914499936000000</InPoint><OutPoint>914550739200000</OutPoint></Clip><AdjustmentLayer>true</AdjustmentLayer></VideoClip>
	<VideoComponentChain ObjectID="30"><ComponentChain Version="3"><Components Version="1">
		<Component Index="0" ObjectRef="31"/>
		<Component Index="1" ObjectRef="32"/>
		<Component Index="2" ObjectRef="33"/>
	</Components></ComponentChain></VideoComponentChain>
	<VideoFilterComponent ObjectID="31"><Component Version="6">
		<Params Version="1">
			<Param Index="0" ObjectRef="40"/>
			<Param Index="1" ObjectRef="41"/>
			<Param Index="2" ObjectRef="42"/>
			<Param Index="3" ObjectRef="43"/>
		</Params>
		<Bypass>false</Bypass><DisplayName>Transform</DisplayName>
	</Component><MatchName>AE.ADBE Geometry2</MatchName></VideoFilterComponent>
	<PointComponentParam ObjectID="40"><Name>Position</Name><IsTimeVarying>false</IsTimeVarying><StartKeyframe>-91445760000000000,0.25:0.5,0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe><ParameterID>2</ParameterID></PointComponentParam>
	<VideoComponentParam ObjectID="41"><IsTimeVarying>false</IsTimeVarying><StartKeyframe>-91445760000000000,false,0,0,0,0,0,0</StartKeyframe><ParameterID>11</ParameterID></VideoComponentParam>
	<VideoComponentParam ObjectID="42"><Name>Scale Height</Name><StartKeyframe>-91445760000000000,100.,0,0,0,0,0,0</StartKeyframe><Keyframes>914499936000000,200.,5,0,0,0.16666666666666666,17.8,1;914550739200000,300.,5,0,88.2,0.066,0,0.16666666666666666;</Keyframes><CurrentValue>300</CurrentValue><LowerBound>-30000</LowerBound><UpperBound>30000</UpperBound><ParameterID>3</ParameterID></VideoComponentParam>
	<VideoComponentParam ObjectID="43"><Name>Rotation</Name><Keyframes>914499936000000,0.,4,0,0,0.16,0,0.33;914510000000000,-43.,0,0,0,0.16,0,0.33;</Keyframes><ParameterID>7</ParameterID></VideoComponentParam>
	<VideoFilterComponent ObjectID="32"><Component Version="6"><Params Version="1"></Params><DisplayName>Mask</DisplayName></Component><MatchName>AE.ADBE AEMask</MatchName></VideoFilterComponent>
	<VideoFilterComponent ObjectID="33"><Component Version="6"><Params Version="1"></Params><DisplayName>Digital Glitch</DisplayName></Component><MatchName>AE.Mettle SkyBox Digital Glitch</MatchName></VideoFilterComponent>

	<VideoClipTrackItem ObjectID="60"><ClipTrackItem Version="8">
		<ComponentOwner Version="1"><Components ObjectRef="70"/></ComponentOwner>
		<TrackItem Version="3"><Start>0</Start><End>254016000000</End></TrackItem>
		<SubClip ObjectRef="61"/>
	</ClipTrackItem></VideoClipTrackItem>
	<SubClip ObjectID="61"><Clip ObjectRef="62"/><Name>Leak 02.mp4</Name></SubClip>
	<VideoClip ObjectID="62"><Clip Version="18"><Source ObjectRef="63"/><InPoint>8467200000</InPoint><OutPoint>262483200000</OutPoint></Clip></VideoClip>
	<VideoMediaSource ObjectID="63"><MediaSource Version="4"><Media ObjectURef="media-1"/></MediaSource></VideoMediaSource>
	<Media ObjectUID="media-1"><VideoStream ObjectRef="64"/><FilePath>D:\Packs\Leaks\Leak 02.mp4</FilePath><Title>Leak 02.mp4</Title><ActualMediaFilePath>D:\Packs\Leaks\Leak 02.mp4</ActualMediaFilePath></Media>
	<VideoStream ObjectID="64"><FrameRate>8467200000</FrameRate></VideoStream>
	<VideoComponentChain ObjectID="70"><ComponentChain Version="3"><Components Version="1">
		<Component Index="0" ObjectRef="71"/>
	</Components></ComponentChain></VideoComponentChain>
	<VideoFilterComponent ObjectID="71"><Component Version="6">
		<Params Version="1">
			<Param Index="0" ObjectRef="72"/>
			<Param Index="1" ObjectRef="73"/>
			<Param Index="2" ObjectRef="74"/>
			<Param Index="3" ObjectRef="75"/>
		</Params>
		<DisplayName>Opacity</DisplayName><Intrinsic>true</Intrinsic>
	</Component><MatchName>AE.ADBE Opacity</MatchName></VideoFilterComponent>
	<VideoComponentParam ObjectID="72"><Name>Opacity</Name><CurrentValue>0</CurrentValue><LowerBound>-3.4028234663852886e+38</LowerBound><UpperBound>3.4028234663852886e+38</UpperBound><ParameterID>1</ParameterID></VideoComponentParam>
	<VideoComponentParam ObjectID="73"><Name>Opacity</Name><CurrentValue>80</CurrentValue><LowerBound>0</LowerBound><UpperBound>100</UpperBound><ParameterID>1</ParameterID></VideoComponentParam>
	<VideoComponentParam ObjectID="74"><Name>Blend Mode</Name><ParameterControlType>10</ParameterControlType><StartKeyframe>-91445760000000000,14,0,0,0,0,0,0</StartKeyframe><ParameterID>2</ParameterID></VideoComponentParam>
	<VideoComponentParam ObjectID="75"><Name>Blend Mode</Name><ParameterControlType>7</ParameterControlType><StartKeyframe>-91445760000000000,12,0,0,0,0,0,0</StartKeyframe><ParameterID>3</ParameterID></VideoComponentParam>

	<AudioClipTrackItem ObjectID="80"><ClipTrackItem Version="8">
		<ComponentOwner Version="1"><Components ObjectRef="90"/></ComponentOwner>
		<TrackItem Version="3"><Node Version="1"><ID>1002713</ID></Node><Start>8467200000</Start><End>127008000000</End></TrackItem>
		<SubClip ObjectRef="81"/>
	</ClipTrackItem></AudioClipTrackItem>
	<SubClip ObjectID="81"><Clip ObjectRef="82"/><Name>whoosh.wav</Name></SubClip>
	<AudioClip ObjectID="82"><Clip Version="18"><Source ObjectRef="83"/><InPoint>50803200000</InPoint><OutPoint>169344000000</OutPoint></Clip></AudioClip>
	<AudioMediaSource ObjectID="83"><MediaSource Version="4"><Media ObjectURef="media-2"/></MediaSource></AudioMediaSource>
	<Media ObjectUID="media-2"><FilePath>D:\Packs\SFX\[T]_Zoom - Basic.wav</FilePath><Title>[T]_Zoom - Basic.wav</Title><ActualMediaFilePath>D:\Packs\SFX\[T]_Zoom - Basic.wav</ActualMediaFilePath></Media>
	<AudioComponentChain ObjectID="90"><ComponentChain Version="3"><Components Version="1">
		<Component Index="0" ObjectRef="91"/>
	</Components></ComponentChain></AudioComponentChain>
	<AudioFilterComponent ObjectID="91"><AudioComponent Version="3"><Component Version="6">
		<Params Version="1"><Param Index="0" ObjectRef="92"/><Param Index="1" ObjectRef="93"/></Params>
		<DisplayName>Volume</DisplayName>
	</Component></AudioComponent></AudioFilterComponent>
	<AudioComponentParam ObjectID="92"><Name>Bypass</Name></AudioComponentParam>
	<AudioComponentParam ObjectID="93"><Name>Level</Name><CurrentValue>0.56234133243560791</CurrentValue><IsTimeVarying>false</IsTimeVarying></AudioComponentParam>
</PremiereData>
"#;

fn preset() -> (Preset, Report) {
    let mut report = Report::default();
    let p = read_preset(PROJECT, "Zoom IN", (1920, 1080), &mut report).unwrap();
    (p, report)
}

fn float(p: &Param) -> f64 {
    match p.value {
        ParamValue::Float(v) => v,
        ref other => panic!("{other:?}"),
    }
}

#[test]
fn lists_the_presets_of_a_project() {
    assert_eq!(sequence_names(PROJECT).unwrap(), ["Zoom IN", "Zoom OUT"]);
    assert_eq!(decompress(PROJECT.as_bytes()).unwrap(), PROJECT, "plain XML passes through");
}

#[test]
fn reads_tracks_clips_and_the_cut_marker() {
    let (p, _) = preset();
    assert_eq!(p.anchor, Some(Tick(93_139_200_000)), "the earliest marker");
    let kinds: Vec<(TrackKind, usize)> = p.tracks.iter().map(|t| (t.kind, t.index)).collect();
    assert_eq!(kinds, [(TrackKind::Video, 1), (TrackKind::Video, 2), (TrackKind::Audio, 0)], "empty tracks are left out");
    let adj = &p.tracks[0].clips[0];
    assert_eq!(
        (adj.source.clone(), adj.start, adj.duration, adj.source_in),
        (PresetSource::AdjustmentLayer, Tick(42_336_000_000), Tick(50_803_200_000), Tick(0))
    );
    let leak = &p.tracks[1].clips[0];
    assert_eq!(leak.source, PresetSource::File { path: r"D:\Packs\Leaks\Leak 02.mp4".into(), frame_ticks: Some(8_467_200_000) });
    assert_eq!(leak.source_in, Tick(8_467_200_000), "files keep their media in point");
    assert_eq!(p.extent(), Some((Tick(0), Tick(254_016_000_000))));
}

#[test]
fn maps_effects_parameters_and_keyframes() {
    let (p, report) = preset();
    let adj = &p.tracks[0].clips[0];
    assert_eq!(adj.effects.len(), 1, "only the Transform: the mask and the plug-in are skipped");
    let t = &adj.effects[0];
    assert_eq!(t.effect, "transform");
    assert_eq!(t.params["position"].value, ParamValue::Vec2(Vec2::new(480.0, 540.0)), "fractions of the frame become pixels");
    assert_eq!(t.params["uniform_scale"].value, ParamValue::Bool(false), "the unnamed switch by its parameter id");
    let sh = &t.params["scale_height"];
    assert_eq!(sh.keyframes.len(), 2);
    assert_eq!(sh.keyframes[0].time, Tick(0), "keyframes move with the adjustment layer's in point");
    assert_eq!(sh.keyframes[1].time, Tick(914_550_739_200_000 - 914_499_936_000_000));
    assert_eq!(sh.keyframes[1].value, ParamValue::Float(300.0));
    assert_eq!(sh.keyframes[0].interp, Interpolation::Bezier);
    assert!((sh.keyframes[0].out_influence - 1.0).abs() < 1e-9, "the ease is kept");
    let rot = &t.params["rotation"];
    assert_eq!(rot.keyframes[0].interp, Interpolation::Hold);
    assert_eq!(rot.keyframes[1].interp, Interpolation::Linear);
    let text = format!("{report:?}");
    assert!(text.contains("masks"), "{text}");
    assert!(text.contains("Digital Glitch"), "{text}");
}

#[test]
fn opacity_blend_mode_and_sound_volume() {
    let (p, _) = preset();
    let leak = &p.tracks[1].clips[0];
    let o = leak.effects.iter().find(|e| e.effect == "opacity").unwrap();
    assert_eq!(float(&o.params["opacity"]), 80.0, "the unbounded duplicate is skipped");
    // the menu's Linear Dodge (Add) sits after two separators
    let blend = filmcraft_project::effect::BLEND_MODES.iter().position(|m| *m == "Linear Dodge (Add)").unwrap() as u32;
    assert_eq!(o.params["blend"].value, ParamValue::Choice(blend));
    let sfx = &p.tracks[2].clips[0];
    assert!(matches!(&sfx.source, PresetSource::File { path, .. } if path.ends_with("[T]_Zoom - Basic.wav")));
    let v = sfx.effects.iter().find(|e| e.effect == "volume").unwrap();
    assert!((float(&v.params["level"]) + 5.0).abs() < 0.01, "a 0.562 gain is -5 dB");
}

#[test]
fn hostile_input_is_an_error_not_a_panic() {
    let mut report = Report::default();
    assert!(read_preset("not xml", "x", (1920, 1080), &mut report).is_err());
    assert!(read_preset(PROJECT, "No Such Preset", (1920, 1080), &mut report).is_err());
    assert!(read_preset(PROJECT, "Zoom OUT", (1920, 1080), &mut report).is_err(), "a preset without clips");
    assert!(decompress(&[0x1f, 0x8b, 8, 0, 1, 2]).is_err());
    // damaged numbers and keyframe lists
    let broken = PROJECT
        .replace("<End>93139200000</End>", "<End>-5</End>")
        .replace("914499936000000,200.,5", "x,y,z")
        .replace("0.25:0.5", "nan:inf")
        .replace("<StartKeyframe>-91445760000000000,12,", "<StartKeyframe>-91445760000000000,99999999999,");
    let p = read_preset(&broken, "Zoom IN", (1920, 1080), &mut report).unwrap();
    assert!(p.tracks.iter().all(|t| t.clips.iter().all(|c| c.duration > Tick(0))));
    for cut in [10, 100, 1000, 5000] {
        let _ = read_preset(&PROJECT[..cut.min(PROJECT.len())], "Zoom IN", (1920, 1080), &mut report);
    }
}
