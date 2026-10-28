//! Odin Pro, and other preset packs made for Premiere Pro, in FilmCraft.
//!
//! A pack is a folder: Premiere projects whose sequences are presets (transitions and effects built
//! from adjustment layers, overlay footage and sound effects), the footage and sounds they use, and
//! a sound effects library. FilmCraft reads the projects ([`filmcraft_interchange::prproj`]) and
//! drops a preset onto the active sequence at the playhead: its cut marker lands on the playhead,
//! its picture tracks go right above the footage there (below captions and titles; new tracks are
//! made when those are taken), its sounds on free audio tracks above A1. Media the projects point to
//! at their author's paths are found in the pack by file name. Motion graphics templates (`.mogrt`)
//! made in After Effects need After Effects to render and are only counted.
//!
//! | Id | Params | Notes |
//! |---|---|---|
//! | `odin.scan` | `{root?}` | the pack's presets, sounds and footage; `root` is remembered (`mediaBrowser.odinRoot`) |
//! | `odin.insertPreset` | `{file, name, time?}` | one undo step |
//! | `odin.insertMedia` | `{path, time?}` | a sound or a piece of footage at the playhead |

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use filmcraft_interchange::Report;
use filmcraft_interchange::prproj::{self, Preset, PresetClip, PresetSource};
use filmcraft_project::{ClipId, ItemId, ItemKind, Label, MediaRef, ParamValue, TrackId, TrackKind};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq, str_p};
use crate::{EngineError, Result, Session};

/// Directory levels and entries walked in a pack.
const MAX_DEPTH: usize = 12;
const MAX_ENTRIES: usize = 100_000;
/// Largest project file read.
const MAX_PROJECT_BYTES: u64 = 64 << 20;
/// The bin presets' media and adjustment layers go in.
const BIN: &str = "Odin Pro";

const SOUND_EXTS: [&str; 6] = ["wav", "mp3", "aif", "aiff", "flac", "ogg"];
const FOOTAGE_EXTS: [&str; 4] = ["mp4", "mov", "webm", "mkv"];

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "odin.scan",
            label: "Scan Odin Pro Folder",
            menu: &[],
            shortcut: None,
            params: r#"{"root":str?}"#,
            enabled: always,
            run: scan,
            journal: false,
        },
        CommandSpec {
            id: "odin.insertPreset",
            label: "Insert Odin Pro Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"file":str,"name":str,"time":ticks?}"#,
            enabled: has_seq,
            run: insert_preset,
            journal: true,
        },
        CommandSpec {
            id: "odin.insertMedia",
            label: "Insert Odin Pro Media",
            menu: &[],
            shortcut: None,
            params: r#"{"path":str,"time":ticks?}"#,
            enabled: has_seq,
            run: insert_media,
            journal: true,
        },
    ]
}

fn ext_of(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default()
}

/// Every file under `root` (bounded), in a stable order.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        for p in entries.into_iter().rev() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return out;
            }
            if p.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((p, depth + 1));
                }
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The folders between `root` and `file`, joined with " / ".
fn group_of(root: &Path, file: &Path) -> String {
    let rel = file.parent().and_then(|d| d.strip_prefix(root).ok()).map(Path::to_path_buf).unwrap_or_default();
    rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join(" / ")
}

fn root_of(s: &Session, p: &Value) -> Option<String> {
    str_p(p, "root").map(str::to_string).or_else(|| Some(s.prefs.media_browser.odin_root.clone())).filter(|r| !r.trim().is_empty())
}

/// `odin.scan`: list the pack's presets, sound effects and footage.
fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "odin.scan";
    let root = root_of(s, p).ok_or_else(|| bad(CMD, "choose the Odin Pro folder (`root`)"))?;
    let root_path = PathBuf::from(&root);
    if !root_path.is_dir() {
        return Err(bad(CMD, format!("{root} is not a folder")));
    }
    if s.prefs.media_browser.odin_root != root {
        let mut next = s.prefs.clone();
        next.media_browser.odin_root = root.clone();
        if let Err(e) = s.set_prefs(next) {
            log::warn!("saving the Odin Pro folder: {e}");
        }
    }
    let files = walk(&root_path);
    let mut presets = Vec::new();
    let mut sounds = Vec::new();
    let mut footage = Vec::new();
    let mut templates = 0usize;
    for f in &files {
        let ext = ext_of(f);
        let group = group_of(&root_path, f);
        let path = f.to_string_lossy().to_string();
        let name = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        match ext.as_str() {
            "prproj" => {
                let names = read_project(f).and_then(|xml| prproj::sequence_names(&xml).map_err(|e| e.to_string()));
                match names {
                    Ok(names) if !names.is_empty() => presets.push(json!({"file": path, "group": group, "names": names})),
                    Ok(_) => {}
                    Err(e) => log::warn!("{path}: {e}"),
                }
            }
            "mogrt" => templates += 1,
            e if SOUND_EXTS.contains(&e) => sounds.push(json!({"path": path, "group": group, "name": name})),
            e if FOOTAGE_EXTS.contains(&e) && !group.contains("Preview") => footage.push(json!({"path": path, "group": group, "name": name})),
            _ => {}
        }
    }
    Ok(json!({
        "root": root,
        "presets": presets,
        "sounds": sounds,
        "footage": footage,
        "templates": templates,
        "templatesNote": "Motion graphics templates (.mogrt) built in After Effects need After Effects to render; FilmCraft lists the presets, sounds and footage instead.",
    }))
}

fn read_project(path: &Path) -> std::result::Result<String, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_PROJECT_BYTES {
        return Err("the project is too large".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    prproj::decompress(&bytes).map_err(|e| e.to_string())
}

/// Files of a pack by lower-case file name; footage under `_Assets` wins over previews.
fn file_index(root: &Path) -> HashMap<String, PathBuf> {
    let mut index: HashMap<String, PathBuf> = HashMap::new();
    for f in walk(root) {
        let Some(name) = f.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { continue };
        let preview = f.to_string_lossy().contains("Preview");
        match index.get(&name) {
            Some(old) if !old.to_string_lossy().contains("Preview") || preview => {}
            _ => {
                index.insert(name, f);
            }
        }
    }
    index
}

/// Where a project's media lives on this machine: the path itself, else a file of that name in the
/// pack (the projects keep their author's paths).
fn locate(path: &str, index: &mut Option<HashMap<String, PathBuf>>, root: &Path) -> Option<PathBuf> {
    let direct = PathBuf::from(path);
    if direct.is_file() {
        return Some(direct);
    }
    // a Windows path read on another system has backslashes in its "file name"
    let name = path.rsplit(['\\', '/']).next()?.to_lowercase();
    let index = index.get_or_insert_with(|| file_index(root));
    index.get(&name).cloned()
}

/// The pack folder for a project: the remembered one when the project is inside it, else the
/// project's folder three levels up.
fn pack_root(s: &Session, file: &Path) -> PathBuf {
    let remembered = PathBuf::from(&s.prefs.media_browser.odin_root);
    if !s.prefs.media_browser.odin_root.is_empty() && file.starts_with(&remembered) {
        return remembered;
    }
    let mut dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    for _ in 0..3 {
        match dir.parent() {
            Some(p) if !p.as_os_str().is_empty() => dir = p.to_path_buf(),
            _ => break,
        }
    }
    dir
}

/// The project item for a media file: one already in the project, else imported into the Odin Pro
/// bin (a numbered still as an image sequence at `frame_ticks` per frame).
fn media_item(s: &mut Session, path: &Path, frame_ticks: Option<i64>, bin: Option<filmcraft_project::BinId>) -> Result<ItemId> {
    let p = path.to_string_lossy().to_string();
    let existing = s.project.items.values().find(|i| matches!(&i.kind, ItemKind::Media(m) if matches!(&m.media, MediaRef::File { path } if *path == p)));
    if let Some(it) = existing {
        return Ok(it.id);
    }
    let numbered = filmcraft_media::sequence::Numbered::parse(&p).is_some();
    if numbered && crate::media_pool::image_sequence_frames(&p, &*s.services).is_ok_and(|f| f.len() > 1) {
        let rate = frame_ticks.filter(|t| *t > 0).map(|t| FrameRate::from_f64(TICKS_PER_SECOND as f64 / t as f64)).unwrap_or(FrameRate::FPS_25);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.clone());
        let src = crate::media_pool::open_image_sequence(&p, &*s.services, rate, &name)?;
        let identity = crate::relink::identity_of(&*s.services, &p).map_err(|e| EngineError::Other(format!("{p}: {e}")))?;
        return crate::commands::import_source(s, &p, name, src, identity, bin);
    }
    crate::commands::import_streamed(s, &p, bin)
}

/// The Odin Pro bin, made when missing.
fn odin_bin(s: &mut Session) -> Result<filmcraft_project::BinId> {
    let found = s.project.root.children.iter().find_map(|c| match c {
        filmcraft_project::BinEntry::Bin(b) if b.name == BIN => Some(b.id),
        _ => None,
    });
    match found {
        Some(b) => Ok(b),
        None => s.edit("New Bin", |p, _| Ok(p.add_bin(BIN, None))),
    }
}

fn overlaps(t: &filmcraft_project::Track, a: Tick, b: Tick) -> bool {
    t.items.iter().any(|i| i.start < b && a < i.end())
}

/// Picture tracks for `n` layers over `[a, b)`: right above the highest track with footage there,
/// made (pushing captions and titles up) when those tracks are taken.
fn video_tracks(s: &mut Session, n: usize, a: Tick, b: Tick) -> Result<Vec<TrackId>> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let project = &s.project;
    let footage = |i: &filmcraft_project::TrackItem| {
        matches!(project.item(i.item).map(|x| &x.kind), Some(ItemKind::Media(_) | ItemKind::Sequence(_) | ItemKind::Subclip { .. }))
    };
    let top = q.video_tracks.iter().enumerate().filter(|(_, t)| t.items.iter().any(|i| i.start < b && a < i.end() && footage(i))).map(|(k, _)| k).max();
    let base = top.map_or(0, |k| k + 1);
    let free = (base..base + n).all(|k| q.video_tracks.get(k).is_none_or(|t| !overlaps(t, a, b)));
    let have = q.video_tracks.len();
    if !free {
        s.execute("sequence.addTracks", json!({"video": n, "audio": 0, "videoAfter": base}))?;
    } else if base + n > have {
        s.execute("sequence.addTracks", json!({"video": base + n - have, "audio": 0}))?;
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    Ok((base..base + n).filter_map(|k| q.video_tracks.get(k).map(|t| t.id)).collect())
}

/// Sound tracks for `n` layers over `[a, b)`: free tracks above A1, made at the end when none is.
fn audio_tracks(s: &mut Session, n: usize, a: Tick, b: Tick) -> Result<Vec<TrackId>> {
    let mut out = Vec::new();
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    for t in q.audio_tracks.iter().skip(1) {
        if out.len() == n {
            break;
        }
        if !overlaps(t, a, b) {
            out.push(t.id);
        }
    }
    if out.len() < n {
        let need = n - out.len();
        s.execute("sequence.addTracks", json!({"video": 0, "audio": need}))?;
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let len = q.audio_tracks.len();
        out.extend(q.audio_tracks.iter().skip(len.saturating_sub(need)).map(|t| t.id));
    }
    Ok(out)
}

fn time_p(s: &Session, p: &Value) -> Tick {
    p.get("time").and_then(Value::as_i64).map(Tick).unwrap_or_else(|| s.playhead())
}

/// Give a placed clip the preset's name and effects (intrinsic ones by parameter, the others
/// appended in order). Points the preset leaves automatic are centred on the source.
fn dress(s: &mut Session, clip: ClipId, c: &PresetClip, center: (f64, f64), label: &str) -> Result<()> {
    s.edit_sequence(label, |q, _, _| {
        let Some((_, ti)) = q.find_item_mut(clip) else { return Ok(()) };
        ti.name = c.name.clone();
        ti.enabled = c.enabled;
        for e in &c.effects {
            let mut e = e.clone();
            for param in e.params.values_mut() {
                param.map_values(|v| match v {
                    ParamValue::Vec2(p) if p.x.is_nan() || p.y.is_nan() => {
                        ParamValue::Vec2(filmcraft_geom::Vec2::new(if p.x.is_nan() { center.0 } else { p.x }, if p.y.is_nan() { center.1 } else { p.y }))
                    }
                    other => other,
                });
            }
            if e.def().is_some_and(|d| d.intrinsic) {
                if let Some(slot) = ti.effect_mut(&e.effect) {
                    slot.enabled = e.enabled;
                    slot.params.extend(e.params);
                }
            } else {
                ti.effects.push(e);
            }
        }
        Ok(())
    })
}

fn report_json(r: &Report) -> Vec<String> {
    r.entries.iter().map(|e| if e.count > 1 { format!("{} (×{})", e.message, e.count) } else { e.message.clone() }).collect()
}

/// `odin.insertPreset`: drop preset `name` of project `file` at the playhead (or `time`).
fn insert_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "odin.insertPreset";
    let file = str_p(p, "file").ok_or_else(|| bad(CMD, "need `file` (a .prproj)"))?.to_string();
    let name = str_p(p, "name").ok_or_else(|| bad(CMD, "need `name` (a preset of the project)"))?.to_string();
    let xml = read_project(Path::new(&file)).map_err(|e| bad(CMD, format!("{file}: {e}")))?;
    let (frame, rate) = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        ((q.settings.width.max(1), q.settings.height.max(1)), q.settings.frame_rate)
    };
    let mut report = Report::default();
    let preset: Preset = prproj::read_preset(&xml, &name, frame, &mut report).map_err(|e| bad(CMD, e.to_string()))?;
    let (lo, hi) = preset.extent().ok_or_else(|| bad(CMD, "the preset has no clips"))?;
    let anchor = preset.anchor.unwrap_or(Tick((lo.0 + hi.0) / 2));
    let at = time_p(s, p);
    let mut offset = at - anchor;
    if lo + offset < Tick::ZERO {
        offset = Tick::ZERO - lo;
    }
    let (a, b) = (lo + offset, hi + offset);
    let label = format!("Insert {name}");
    let n0 = s.history.undo.len();
    let result = place_preset(s, &preset, &file, offset, (a, b), frame, rate, &label, &mut report);
    crate::clip_ops::collapse_history(s, n0, &label);
    let clips = result?;
    Ok(json!({"preset": name, "clips": clips.iter().map(|c| c.0).collect::<Vec<_>>(), "start": a.0, "end": b.0, "report": report_json(&report)}))
}

#[allow(clippy::too_many_arguments)]
fn place_preset(
    s: &mut Session,
    preset: &Preset,
    file: &str,
    offset: Tick,
    (a, b): (Tick, Tick),
    frame: (u32, u32),
    rate: FrameRate,
    label: &str,
    report: &mut Report,
) -> Result<Vec<ClipId>> {
    let root = pack_root(s, Path::new(file));
    let bin = Some(odin_bin(s)?);
    let mut index = None;
    // one adjustment layer item covers every adjustment clip of the preset
    let needs_adjustment = preset.tracks.iter().flat_map(|t| &t.clips).any(|c| c.source == PresetSource::AdjustmentLayer);
    let adjustment = if needs_adjustment {
        let duration = (b - a) + Tick(TICKS_PER_SECOND);
        let item = s.edit("New Adjustment Layer", |pr, _| {
            Ok(pr.add_item(
                &format!("{} (Odin Pro)", preset.name),
                Label::Lavender,
                ItemKind::AdjustmentLayer { width: frame.0, height: frame.1, rate, duration },
                bin,
            ))
        })?;
        Some(item)
    } else {
        None
    };
    let video: Vec<_> = preset.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
    let audio: Vec<_> = preset.tracks.iter().filter(|t| t.kind == TrackKind::Audio).collect();
    let vt = if video.is_empty() { Vec::new() } else { video_tracks(s, video.len(), a, b)? };
    let at = if audio.is_empty() { Vec::new() } else { audio_tracks(s, audio.len(), a, b)? };
    let mut placed = Vec::new();
    for (tracks, ids, is_video) in [(&video, &vt, true), (&audio, &at, false)] {
        for (track, tid) in tracks.iter().zip(ids.iter()) {
            for c in &track.clips {
                let (item, center) = match &c.source {
                    PresetSource::AdjustmentLayer => match adjustment {
                        Some(i) => (i, (f64::from(frame.0) / 2.0, f64::from(frame.1) / 2.0)),
                        None => continue,
                    },
                    PresetSource::File { path, frame_ticks } => {
                        let Some(found) = locate(path, &mut index, &root) else {
                            report.warn(format!("{} was not found in the pack", path.rsplit(['\\', '/']).next().unwrap_or(path)));
                            continue;
                        };
                        let item = match media_item(s, &found, *frame_ticks, bin) {
                            Ok(i) => i,
                            Err(e) => {
                                report.warn(format!("{}: {e}", found.display()));
                                continue;
                            }
                        };
                        let size =
                            s.project.item(item).and_then(|i| i.as_media()).and_then(|m| m.info.video.as_ref().map(|v| (v.width, v.height))).unwrap_or(frame);
                        (item, (f64::from(size.0) / 2.0, f64::from(size.1) / 2.0))
                    }
                };
                let range = TimeRange::new(c.source_in, c.duration);
                let (vdest, adest) = if is_video { (Some(*tid), None) } else { (None, Some(*tid)) };
                match crate::commands::place_item(s, crate::commands::ItemPlacement { item, range, at: c.start + offset, vdest, adest, insert: false, label, audio_split: None }) {
                    Ok(ids) => {
                        if let Some(&clip) = ids.first() {
                            dress(s, clip, c, center, label)?;
                            placed.push(clip);
                        }
                    }
                    Err(e) => report.warn(format!("\"{}\" could not be placed: {e}", c.name)),
                }
            }
        }
    }
    if placed.is_empty() {
        return Err(EngineError::Other(report_json(report).into_iter().next().unwrap_or_else(|| "nothing of the preset could be placed".into())));
    }
    s.state.selection = placed.clone();
    Ok(placed)
}

/// `odin.insertMedia`: a sound (on a free audio track above A1) or footage (right above the
/// footage at the playhead) from the pack, at the playhead (or `time`).
fn insert_media(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "odin.insertMedia";
    let path = PathBuf::from(str_p(p, "path").ok_or_else(|| bad(CMD, "need `path`"))?);
    if !path.is_file() {
        return Err(bad(CMD, format!("{} is not a file", path.display())));
    }
    let at = time_p(s, p);
    let label = format!("Insert {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let n0 = s.history.undo.len();
    let result = (|| -> Result<Vec<ClipId>> {
        let bin = Some(odin_bin(s)?);
        let item = media_item(s, &path, None, bin)?;
        let rate = s.active_sequence().map(|q| q.settings.frame_rate).ok_or(EngineError::NoSequence)?;
        let still = s.prefs.timeline.still_duration(rate);
        let range = crate::clip_ops::item_range(&s.project, item, still).ok_or_else(|| bad(CMD, "the media has no duration"))?;
        let (video, _) = s.project.source_streams(item);
        let (a, b) = (at, at + range.duration);
        let (vdest, adest) = if video { (video_tracks(s, 1, a, b)?.first().copied(), None) } else { (None, audio_tracks(s, 1, a, b)?.first().copied()) };
        crate::commands::place_item(s, crate::commands::ItemPlacement { item, range, at, vdest, adest, insert: false, label: &label, audio_split: None })
    })();
    crate::clip_ops::collapse_history(s, n0, &label);
    let clips = result?;
    s.state.selection = clips.clone();
    Ok(json!({"clips": clips.iter().map(|c| c.0).collect::<Vec<_>>()}))
}
