//! Clip Mismatch Warning (Settings ▸ Timeline ▸ Show Clip Mismatch Warning dialog).
//!
//! As in Premiere: when the first clip goes into an empty sequence and its frame size, frame rate
//! or pixel aspect ratio differ from the sequence's, the editor asks whether to change the
//! sequence to match the clip. [`mismatch`] answers whether to ask; `sequence.matchClip` makes the
//! change (one undo step) and leaves everything else in the sequence settings as it was.
//!
//! | Id | Params | Notes |
//! |---|---|---|
//! | `sequence.matchClip` | `{item}` | frame size, rate and pixel aspect of the media item |

use filmcraft_project::{ItemId, ItemKind, SequenceSettings};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq};
use crate::{EngineError, Result, Session};

const CMD: &str = "sequence.matchClip";

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Change Sequence Settings to Match Clip",
        menu: &[],
        shortcut: None,
        params: r#"{"item":id}"#,
        enabled: has_seq,
        run: match_clip,
        journal: true,
    }]
}

/// What the warning tells the editor: the sequence's settings and the clip's.
#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub item: ItemId,
    pub clip: SequenceSettings,
    pub sequence: SequenceSettings,
}

/// Sequence and clip disagree on what makes a clip play natively: size, rate, pixel aspect.
fn differs(seq: &SequenceSettings, clip: &SequenceSettings) -> bool {
    seq.width != clip.width || seq.height != clip.height || seq.frame_rate != clip.frame_rate || seq.par != clip.par
}

/// Whether placing `item` into the active sequence should ask first: the warning is on, the
/// sequence has no clips yet, and the item is a moving picture whose settings differ. Stills,
/// audio-only media and sequences never ask (their size or rate does not set a timeline's).
pub fn mismatch(s: &Session, item: ItemId) -> Option<Mismatch> {
    if !s.prefs.timeline.clip_mismatch_warning {
        return None;
    }
    let seq = s.active_sequence()?;
    if seq.video_tracks.iter().chain(&seq.audio_tracks).any(|t| !t.items.is_empty()) {
        return None;
    }
    let media = match &s.project.item(item)?.kind {
        ItemKind::Media(m) if m.info.video.is_some() && !matches!(m.info.kind, filmcraft_media::MediaKind::Still) => m,
        _ => return None,
    };
    let clip = crate::commands::default_seq_settings_for(media);
    differs(&seq.settings, &clip).then(|| Mismatch { item, clip, sequence: seq.settings.clone() })
}

// ---------------------------------------------------------------- the command

/// Change the active sequence's frame size, rate and pixel aspect to the item's.
fn match_clip(s: &mut Session, p: &Value) -> Result<Value> {
    let item = p.get("item").and_then(Value::as_u64).map(ItemId).ok_or_else(|| bad(CMD, "needs `item`"))?;
    let id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let clip = match s.project.item(item).map(|i| &i.kind) {
        Some(ItemKind::Media(m)) if m.info.video.is_some() => crate::commands::default_seq_settings_for(m),
        Some(_) => return Err(bad(CMD, "the item has no picture to match")),
        None => return Err(EngineError::Other(format!("{CMD}: no item {}", item.0))),
    };
    s.edit("Change Sequence Settings", |pr, _| {
        let q = pr.sequence_mut(id).ok_or(EngineError::NoSequence)?;
        q.settings.width = clip.width;
        q.settings.height = clip.height;
        q.settings.frame_rate = clip.frame_rate;
        q.settings.par = clip.par;
        q.settings.preset = clip.preset.clone();
        // drop-frame timecode only exists at 29.97 / 59.94 / 119.88
        if !q.settings.frame_rate.supports_drop_frame() {
            q.settings.drop_frame = false;
        }
        q.settings.validate().map_err(|e| bad(CMD, e))
    })?;
    Ok(json!({"width": clip.width, "height": clip.height, "fps": clip.frame_rate.label()}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use filmcraft_media::generators::GeneratorSource;
    use filmcraft_media::{DemoScene, Generator, MediaKind, MediaSource};
    use filmcraft_project::{ItemId, ItemKind, Label, MediaClip, MediaRef, SequenceSettings, TrackKind};
    use filmcraft_time::{FrameRate, Tick, TimeRange};
    use serde_json::json;

    use crate::Session;

    /// A session with an empty 1920×1080 23.976 sequence and a 1280×720 60 fps clip (not on it).
    fn setup() -> (Session, ItemId) {
        let mut s = Session::default();
        let mut p = (*s.project).clone();
        let src = GeneratorSource::new(Generator::Demo(DemoScene::Aurora), 1280, 720, FrameRate::FPS_60, FrameRate::FPS_60.tick_of(60));
        let clip = MediaClip {
            media: MediaRef::Generator(Generator::Demo(DemoScene::Aurora)),
            info: src.info().clone(),
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        };
        let item = p.add_item("clip", Label::Iris, ItemKind::Media(Box::new(clip)), None);
        let settings = SequenceSettings { width: 1920, height: 1080, frame_rate: FrameRate::FPS_23_976, ..Default::default() };
        let seq = p.new_sequence("Seq", settings, 1, 1, None);
        s.project = Arc::new(p);
        s.state.active_sequence = Some(seq);
        s.state.open_sequences = vec![seq];
        (s, item)
    }

    #[test]
    fn asks_for_a_mismatched_first_clip() {
        let (s, item) = setup();
        let m = super::mismatch(&s, item).unwrap();
        assert_eq!((m.clip.width, m.clip.height, m.clip.frame_rate), (1280, 720, FrameRate::FPS_60));
        assert_eq!((m.sequence.width, m.sequence.frame_rate), (1920, FrameRate::FPS_23_976));
    }

    #[test]
    fn stays_quiet_when_off_matching_filled_or_still() {
        // the warning switched off in Settings
        let (mut s, item) = setup();
        s.prefs.timeline.clip_mismatch_warning = false;
        assert!(super::mismatch(&s, item).is_none());
        // the sequence already has a clip: its settings were chosen, don't ask again
        let (mut s, item) = setup();
        let mut p = (*s.project).clone();
        let seq = s.state.active_sequence.unwrap();
        let v = p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FrameRate::FPS_60.tick_of(10)), FrameRate::FPS_23_976).unwrap();
        p.sequence_mut(seq).unwrap().video_tracks[0].items.push(v);
        s.project = Arc::new(p);
        assert!(super::mismatch(&s, item).is_none());
        // a still never sets the sequence
        let (mut s, item) = setup();
        let mut p = (*s.project).clone();
        if let Some(ItemKind::Media(m)) = p.items.get_mut(&item).map(|i| &mut i.kind) {
            m.info.kind = MediaKind::Still;
        }
        s.project = Arc::new(p);
        assert!(super::mismatch(&s, item).is_none());
        // once matched, nothing to ask
        let (mut s, item) = setup();
        s.execute("sequence.matchClip", json!({"item": item.0})).unwrap();
        assert!(super::mismatch(&s, item).is_none());
    }

    #[test]
    fn match_clip_changes_size_and_rate_in_one_undo() {
        let (mut s, item) = setup();
        let before = s.active_sequence().unwrap().settings.clone();
        let r = s.execute("sequence.matchClip", json!({"item": item.0})).unwrap();
        assert_eq!(r["width"], 1280);
        let after = s.active_sequence().unwrap().settings.clone();
        assert_eq!((after.width, after.height, after.frame_rate), (1280, 720, FrameRate::FPS_60));
        assert_eq!(after.sample_rate, before.sample_rate, "audio settings are left alone");
        s.undo();
        assert_eq!(s.active_sequence().unwrap().settings, before);
        assert!(s.execute("sequence.matchClip", json!({})).is_err());
    }
}
