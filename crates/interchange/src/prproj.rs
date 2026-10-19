//! Premiere Pro projects (`.prproj`) read as effect presets.
//!
//! Template packs such as Odin Pro ship their transitions and effects as Premiere projects: each
//! preset is a sequence of adjustment layers carrying keyframed effects, overlay media (light
//! leaks, burns, paper frames, image sequences) and sound effects, cut around a marker where the
//! user's two shots meet. [`read_preset`] turns one such sequence into FilmCraft clips with
//! FilmCraft effects, ready to be dropped on a timeline; [`sequence_names`] lists the presets of a
//! file.
//!
//! The file is gzip-compressed XML ([`decompress`]). Its top-level elements are objects that refer
//! to each other with `ObjectRef` (a numeric `ObjectID`) or `ObjectURef` (an `ObjectUID`). Times
//! are Premiere ticks, which are FilmCraft ticks (254 016 000 000 per second). Effects are matched
//! by their display name (or match name), parameters by name in order, with a few unnamed ones by
//! parameter id; keyframes keep their times and interpolation. Points are stored as fractions of
//! the frame and come out in pixels of the frame size the caller gives. Masks and effects
//! FilmCraft does not have (third-party plug-ins) are skipped and reported.

use std::collections::{HashMap, HashSet};

use filmcraft_geom::Vec2;
use filmcraft_project::{
    EffectDef, EffectInstance, EffectKind, Interpolation, Keyframe, Param, ParamKind, ParamValue, TrackKind, effect::find_effect_by_name, find_effect,
};
use filmcraft_time::Tick;
use roxmltree::{Document, Node};

use crate::xml::{child, child_text};
use crate::{Error, Report, Result};

const FORMAT: &str = "Premiere Pro project";
/// Largest uncompressed project read (a preset pack's projects are a few megabytes).
const MAX_XML: usize = 256 << 20;
/// Keyframes kept per parameter (a camera shake has a few hundred).
const MAX_KEYFRAMES: usize = 20_000;
/// Clips read per preset.
const MAX_CLIPS: usize = 2_000;

fn parse_error(message: impl Into<String>) -> Error {
    Error::Parse { format: FORMAT, message: message.into() }
}

/// The project's XML: gunzipped when the file is compressed (as Premiere writes it), else as is.
pub fn decompress(bytes: &[u8]) -> Result<String> {
    let raw = if filmcraft_media::gzip::is_gzip(bytes) { filmcraft_media::gzip::decompress(bytes, MAX_XML).map_err(parse_error)? } else { bytes.to_vec() };
    String::from_utf8(raw).map_err(|_| parse_error("the project is not UTF-8 XML"))
}

/// What a preset clip shows or plays.
#[derive(Clone, Debug, PartialEq)]
pub enum PresetSource {
    /// An adjustment layer: its effects apply to the tracks below.
    AdjustmentLayer,
    /// A media file. `frame_ticks` is the frame duration Premiere played a still sequence at.
    File { path: String, frame_ticks: Option<i64> },
}

/// One clip of a preset.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetClip {
    pub name: String,
    /// Start in the preset's sequence.
    pub start: Tick,
    pub duration: Tick,
    /// Media time of the clip's first frame (zero for adjustment layers).
    pub source_in: Tick,
    pub source: PresetSource,
    /// Effects in render order; `motion`, `opacity` and `volume` replace the clip's intrinsic
    /// ones. Keyframe times are media times, like the clip's `source_in`.
    pub effects: Vec<EffectInstance>,
    pub enabled: bool,
}

/// A track of a preset, with its place among the preset's tracks of that kind.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetTrack {
    pub kind: TrackKind,
    pub index: usize,
    pub clips: Vec<PresetClip>,
}

/// A preset: one sequence of the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    /// Where the preset meets the cut (its first sequence marker), if it says.
    pub anchor: Option<Tick>,
    /// Tracks that have clips, video then audio, each from the bottom up.
    pub tracks: Vec<PresetTrack>,
}

impl Preset {
    /// The span its clips cover in the preset's sequence.
    pub fn extent(&self) -> Option<(Tick, Tick)> {
        let clips = self.tracks.iter().flat_map(|t| &t.clips);
        let start = clips.clone().map(|c| c.start).min()?;
        let end = clips.map(|c| c.start + c.duration).max()?;
        Some((start, end))
    }
}

/// The top-level objects of a project, by id.
struct Objects<'a, 'i> {
    by_id: HashMap<&'a str, Node<'a, 'i>>,
    by_uid: HashMap<&'a str, Node<'a, 'i>>,
}

impl<'a, 'i> Objects<'a, 'i> {
    fn new(doc: &'a Document<'i>) -> Self {
        let mut by_id = HashMap::new();
        let mut by_uid = HashMap::new();
        for n in doc.root_element().children().filter(|n| n.is_element()) {
            if let Some(id) = n.attribute("ObjectID") {
                by_id.insert(id, n);
            }
            if let Some(uid) = n.attribute("ObjectUID") {
                by_uid.insert(uid, n);
            }
        }
        Objects { by_id, by_uid }
    }

    /// The object an element refers to.
    fn target(&self, n: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
        if let Some(r) = n.attribute("ObjectRef") {
            return self.by_id.get(r).copied();
        }
        n.attribute("ObjectURef").and_then(|r| self.by_uid.get(r).copied())
    }

    /// The object referred to by the first descendant of `n` named `name`.
    fn follow(&self, n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
        n.descendants()
            .find(|d| d.is_element() && d.tag_name().name() == name && (d.has_attribute("ObjectRef") || d.has_attribute("ObjectURef")))
            .and_then(|d| self.target(d))
    }
}

fn parse(xml: &str) -> Result<Document<'_>> {
    Document::parse(xml).map_err(|e| parse_error(e.to_string()))
}

fn sequences<'a, 'i>(doc: &'a Document<'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    doc.root_element().children().filter(|n| n.is_element() && n.tag_name().name() == "Sequence")
}

/// The names of the project's sequences (a preset pack's presets), in file order.
pub fn sequence_names(xml: &str) -> Result<Vec<String>> {
    let doc = parse(xml)?;
    Ok(sequences(&doc).filter_map(|s| child_text(s, "Name")).map(str::to_string).collect())
}

fn first_text<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.descendants().find(|d| d.is_element() && d.tag_name().name() == name).and_then(|d| d.text()).map(str::trim)
}

fn ticks(s: Option<&str>) -> Option<i64> {
    let s = s?;
    s.parse::<i64>().ok().or_else(|| s.parse::<f64>().ok().filter(|f| f.is_finite() && f.abs() < 9.0e18).map(|f| f.round() as i64))
}

/// Read sequence `name` of the project as a preset. Points come out in pixels of `frame`.
pub fn read_preset(xml: &str, name: &str, frame: (u32, u32), report: &mut Report) -> Result<Preset> {
    let doc = parse(xml)?;
    let objs = Objects::new(&doc);
    let seq = sequences(&doc).find(|s| child_text(*s, "Name") == Some(name)).ok_or_else(|| Error::Other(format!("no preset named {name:?} in the project")))?;
    let mut preset = Preset { name: name.to_string(), anchor: anchor(&objs, seq), tracks: Vec::new() };
    let mut clips = 0usize;
    for (kind, group_tag, track_tag) in [(TrackKind::Video, "VideoTrackGroup", "VideoClipTrack"), (TrackKind::Audio, "AudioTrackGroup", "AudioClipTrack")] {
        let mut groups = seq
            .descendants()
            .filter(|d| d.is_element() && d.tag_name().name() == "TrackGroup")
            .filter_map(|g| child(g, "Second"))
            .filter_map(|s| objs.target(s));
        let Some(group) = groups.find(|g| g.tag_name().name() == group_tag) else { continue };
        let Some(tracks) = group.descendants().find(|d| d.is_element() && d.tag_name().name() == "Tracks") else { continue };
        let refs = tracks.children().filter(|t| t.is_element() && t.tag_name().name() == "Track").filter_map(|t| objs.target(t));
        for (index, track) in refs.filter(|t| t.tag_name().name() == track_tag).enumerate() {
            let mut out = PresetTrack { kind, index, clips: Vec::new() };
            // clips (not transitions): ClipTrack/ClipItems/TrackItems
            let clip_items = track.descendants().find(|d| d.is_element() && d.tag_name().name() == "ClipItems");
            let Some(items) = clip_items.and_then(|c| child(c, "TrackItems")) else { continue };
            for it in items.children().filter(|t| t.is_element() && t.tag_name().name() == "TrackItem").filter_map(|t| objs.target(t)) {
                if clips >= MAX_CLIPS {
                    report.warn(format!("the preset has more than {MAX_CLIPS} clips; the rest were skipped"));
                    break;
                }
                if let Some(c) = read_clip(&objs, it, kind, frame, report) {
                    out.clips.push(c);
                    clips += 1;
                }
            }
            if !out.clips.is_empty() {
                out.clips.sort_by_key(|c| c.start);
                preset.tracks.push(out);
            }
        }
    }
    if preset.tracks.is_empty() {
        return Err(Error::Empty);
    }
    Ok(preset)
}

/// The earliest marker of the sequence.
fn anchor<'a, 'i>(objs: &Objects<'a, 'i>, seq: Node<'a, 'i>) -> Option<Tick> {
    let markers = objs.follow(seq, "Markers")?;
    markers
        .descendants()
        .filter(|d| d.is_element() && d.tag_name().name() == "Second")
        .filter_map(|s| objs.target(s))
        .filter_map(|m| child_text(m, "DVAMarker"))
        .filter_map(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .filter_map(|v| v.pointer("/DVAMarker/mStartTime/ticks").and_then(|t| t.as_i64().or_else(|| t.as_str().and_then(|s| s.parse().ok()))))
        .min()
        .map(Tick)
}

fn read_clip<'a, 'i>(objs: &Objects<'a, 'i>, it: Node<'a, 'i>, kind: TrackKind, frame: (u32, u32), report: &mut Report) -> Option<PresetClip> {
    let item = child(it, "ClipTrackItem")?;
    let ti = child(item, "TrackItem")?;
    let start = ticks(child_text(ti, "Start")).unwrap_or(0);
    let end = ticks(child_text(ti, "End"))?;
    if end <= start {
        return None;
    }
    let sub = objs.target(child(item, "SubClip")?)?;
    let name = child_text(sub, "Name").unwrap_or("").to_string();
    let clip = objs.target(child(sub, "Clip")?)?;
    let inner = child(clip, "Clip")?;
    let in_point = ticks(child_text(inner, "InPoint")).unwrap_or(0);
    let adjustment = child_text(clip, "AdjustmentLayer").is_some_and(|t| t.eq_ignore_ascii_case("true"));
    let source = if adjustment {
        PresetSource::AdjustmentLayer
    } else {
        let src = objs.target(child(inner, "Source")?)?;
        let media = objs.follow(src, "Media")?;
        let path = child_text(media, "ActualMediaFilePath").or_else(|| child_text(media, "FilePath")).unwrap_or("");
        // synthetic media (Black Video, bars) has a number for a path
        if path.is_empty() || path.bytes().all(|b| b.is_ascii_digit()) {
            report.info(format!("synthetic clip \"{name}\" was skipped"));
            return None;
        }
        let frame_ticks = child(media, "VideoStream").and_then(|v| objs.target(v)).and_then(|v| ticks(first_text(v, "FrameRate"))).filter(|t| *t > 0);
        PresetSource::File { path: path.to_string(), frame_ticks }
    };
    // keyframe times are media times: re-base them on the clip's FilmCraft source in
    let source_in = if adjustment { 0 } else { in_point };
    let shift = source_in - in_point;
    let mut effects = Vec::new();
    if let Some(chain) = objs.follow(item, "Components") {
        let comps =
            chain.descendants().filter(|d| d.is_element() && d.tag_name().name() == "Component" && d.has_attribute("Index")).filter_map(|c| objs.target(c));
        for comp in comps {
            if let Some(e) = read_effect(objs, comp, kind, frame, shift, report) {
                effects.push(e);
            }
        }
    }
    Some(PresetClip { name, start: Tick(start), duration: Tick(end - start), source_in: Tick(source_in), source, effects, enabled: true })
}

/// FilmCraft's effect for a Premiere component (match name, then display name).
fn effect_for(match_name: &str, display: &str) -> Option<&'static EffectDef> {
    let id = match match_name {
        "AE.ADBE Opacity" => "opacity",
        "AE.ADBE Motion" => "motion",
        "AE.ADBE Geometry" | "AE.ADBE Geometry2" => "transform",
        "AE.ADBE Gaussian Blur 2" | "AE.ADBE Gaussian Blur" => "gaussian_blur",
        "AE.ADBE Motion Blur" => "directional_blur",
        "AE.ADBE Brightness & Contrast 2" => "brightness_contrast",
        "AE.ADBE AECrop" => "crop",
        "AE.ADBE ProcAmp" => "proc_amp",
        "AE.ADBE Noise2" | "AE.ADBE Noise" => "noise",
        "AE.ADBE Lumetri" => "lumetri",
        "PR.ADBE Lens Distortion" => "lens_distortion",
        "PR.ADBE Levels" => "levels",
        _ => "",
    };
    find_effect(id).or_else(|| find_effect_by_name(display.trim()))
}

/// Unnamed parameters, by (effect, Premiere parameter id): the FilmCraft parameter and whether the
/// value is inverted.
fn unnamed(effect: &str, pid: &str) -> Option<(&'static str, bool)> {
    match (effect, pid) {
        ("transform", "11") => Some(("uniform_scale", false)),
        // "Use Composition's Shutter Angle" is the opposite of FilmCraft's override switch
        ("transform", "9") => Some(("shutter_override", true)),
        ("motion", "4") => Some(("uniform_scale", false)),
        ("gaussian_blur", "3") => Some(("repeat_edge", false)),
        ("crop", "5") => Some(("zoom", false)),
        _ => None,
    }
}

fn read_effect<'a, 'i>(
    objs: &Objects<'a, 'i>,
    comp: Node<'a, 'i>,
    kind: TrackKind,
    frame: (u32, u32),
    shift: i64,
    report: &mut Report,
) -> Option<EffectInstance> {
    let match_name = child_text(comp, "MatchName").unwrap_or("");
    let display = first_text(comp, "DisplayName").unwrap_or(match_name);
    let bypass = first_text(comp, "Bypass").is_some_and(|b| b.eq_ignore_ascii_case("true"));
    let params: Vec<Node> =
        comp.descendants().filter(|d| d.is_element() && d.tag_name().name() == "Param" && d.has_attribute("Index")).filter_map(|p| objs.target(p)).collect();
    if kind == TrackKind::Audio {
        return read_audio_effect(display, &params, shift);
    }
    if match_name == "AE.ADBE AEMask" {
        report.warn("masks in presets are not imported yet");
        return None;
    }
    let def = effect_for(match_name, display).filter(|d| d.kind == EffectKind::Video);
    let Some(def) = def else {
        report.warn(format!("effect \"{display}\" ({match_name}) is not available in FilmCraft and was skipped"));
        return None;
    };
    let mut inst = def.instance();
    inst.enabled = !bypass;
    // Motion and Opacity are already on every clip: carry only what the preset sets
    if def.intrinsic {
        inst.params.clear();
    }
    let mut used: HashSet<&'static str> = HashSet::new();
    for p in params {
        let pname = child_text(p, "Name").map(str::trim).unwrap_or("");
        let pid = child_text(p, "ParameterID").unwrap_or("");
        // Motion and Opacity also store unbounded legacy duplicates (a Scale of 1080); skip them
        if def.intrinsic && huge_bounds(p) {
            continue;
        }
        // Opacity carries its blend mode twice: the menu (control type 7) is the one to read
        if def.id == "opacity" && pname == "Blend Mode" && child_text(p, "ParameterControlType") != Some("7") {
            continue;
        }
        let (target, invert) = match unnamed(def.id, pid).filter(|_| pname.is_empty()) {
            Some(t) => t,
            None => {
                let found = def.params.iter().find(|d| !used.contains(d.id) && !pname.is_empty() && d.label.eq_ignore_ascii_case(pname));
                match found {
                    Some(d) => (d.id, false),
                    None => continue,
                }
            }
        };
        let Some(pdef) = def.param(target) else { continue };
        used.insert(pdef.id);
        let conv = Conv { def: def.id, kind: &pdef.kind, default: &pdef.default, frame, invert, unit_fraction: unit_fraction(p) };
        if let Some(param) = read_param(p, &conv, shift) {
            inst.params.insert(pdef.id.to_string(), param);
        }
    }
    Some(inst)
}

/// A Volume component: its Level, a linear gain, as decibels.
fn read_audio_effect(display: &str, params: &[Node], shift: i64) -> Option<EffectInstance> {
    if !display.eq_ignore_ascii_case("Volume") {
        return None;
    }
    let level = params.iter().find(|p| child_text(**p, "Name") == Some("Level"))?;
    let def = find_effect("volume")?;
    let mut inst = def.instance();
    let mut param = read_values(*level, shift, |s| s.parse::<f64>().ok().map(|g| ParamValue::Float(gain_db(g))))?;
    if param.keyframes.len() > 1 || param.value != ParamValue::Float(0.0) {
        param.keyframes.dedup_by_key(|k| k.time);
        inst.params.insert("level".into(), param);
        return Some(inst);
    }
    None
}

fn gain_db(g: f64) -> f64 {
    if g > 1e-9 { (20.0 * g.log10()).clamp(-96.0, 24.0) } else { -96.0 }
}

fn huge_bounds(p: Node) -> bool {
    child_text(p, "UpperBound").and_then(|v| v.parse::<f64>().ok()).is_some_and(|v| v > 1e30)
}

/// A parameter stored as 0–1 where FilmCraft uses a percentage.
fn unit_fraction(p: Node) -> bool {
    child_text(p, "LowerBound").and_then(|v| v.parse::<f64>().ok()) == Some(0.0) && child_text(p, "UpperBound").and_then(|v| v.parse::<f64>().ok()) == Some(1.0)
}

struct Conv<'a> {
    def: &'static str,
    kind: &'a ParamKind,
    default: &'a ParamValue,
    frame: (u32, u32),
    invert: bool,
    unit_fraction: bool,
}

/// The separators of Premiere's blend mode menu (they take an index each).
const BLEND_SEPARATORS: [u32; 5] = [2, 8, 14, 22, 27];

fn convert(raw: &str, c: &Conv) -> Option<ParamValue> {
    let raw = raw.trim();
    Some(match c.default {
        ParamValue::Float(_) => {
            let mut v = raw.parse::<f64>().ok().filter(|v| v.is_finite())?;
            if c.unit_fraction && matches!(c.kind, ParamKind::Float { max, .. } if *max >= 99.0) {
                v *= 100.0;
            }
            if let ParamKind::Float { min, max, .. } = c.kind
                && min <= max
            {
                v = v.clamp(*min, *max);
            }
            ParamValue::Float(v)
        }
        ParamValue::Vec2(_) => {
            let (x, y) = raw.split_once(':')?;
            let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            ParamValue::Vec2(Vec2::new(x * f64::from(c.frame.0), y * f64::from(c.frame.1)))
        }
        ParamValue::Bool(_) => {
            let b = raw.eq_ignore_ascii_case("true") || raw.parse::<f64>().is_ok_and(|v| v != 0.0);
            ParamValue::Bool(b != c.invert)
        }
        ParamValue::Choice(_) => {
            let mut v = raw.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)? as u32;
            if c.def == "opacity" {
                if BLEND_SEPARATORS.contains(&v) {
                    return None;
                }
                v -= BLEND_SEPARATORS.iter().filter(|s| **s < v).count() as u32;
            }
            let n = match c.kind {
                ParamKind::Choice(options) => options.len() as u32,
                _ => 1,
            };
            if v >= n {
                return None;
            }
            ParamValue::Choice(v)
        }
        ParamValue::Color(_) => {
            // 16 bits per channel, ARGB
            let v = raw.parse::<u64>().ok()?;
            let ch = |shift: u32| ((v >> shift) & 0xffff) as f32 / 65535.0;
            ParamValue::Color([ch(32), ch(16), ch(0), ch(48)])
        }
        _ => return None,
    })
}

fn read_param(p: Node, c: &Conv, shift: i64) -> Option<Param> {
    read_values(p, shift, |raw| convert(raw, c))
}

/// A parameter's static value and keyframes, each value converted by `value`.
fn read_values(p: Node, shift: i64, value: impl Fn(&str) -> Option<ParamValue>) -> Option<Param> {
    let animated = child_text(p, "IsTimeVarying").is_none_or(|t| !t.eq_ignore_ascii_case("false"));
    let keyframes = child_text(p, "Keyframes").filter(|k| animated && !k.is_empty());
    if let Some(list) = keyframes {
        let mut out: Vec<Keyframe> = Vec::new();
        for k in list.split(';').filter(|k| !k.trim().is_empty()).take(MAX_KEYFRAMES) {
            let f: Vec<&str> = k.split(',').collect();
            let (Some(t), Some(raw)) = (f.first().and_then(|t| ticks(Some(t.trim()))), f.get(1)) else { continue };
            let Some(v) = value(raw.trim()) else { continue };
            let mut kf = Keyframe::new(Tick(t.saturating_add(shift)), v);
            kf.interp = interpolation(f.get(2).copied(), f.get(3).copied());
            if let Some(i) = f.get(5).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()) {
                kf.in_influence = i.clamp(0.01, 1.0);
            }
            if let Some(o) = f.get(7).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()) {
                kf.out_influence = o.clamp(0.01, 1.0);
            }
            out.push(kf);
        }
        out.sort_by_key(|k| k.time);
        out.dedup_by_key(|k| k.time);
        if let Some(first) = out.first() {
            let mut param = Param::new(first.value.clone());
            if out.len() > 1 {
                param.keyframes = out;
            }
            return Some(param);
        }
    }
    let raw =
        child_text(p, "CurrentValue").map(str::to_string).or_else(|| child_text(p, "StartKeyframe").and_then(|s| s.split(',').nth(1)).map(str::to_string))?;
    value(&raw).map(Param::new)
}

/// Premiere's keyframe interpolation: 0 linear, 4 hold, 5 Bézier (sub-kind 2 auto, 4 continuous).
fn interpolation(mode: Option<&str>, sub: Option<&str>) -> Interpolation {
    match (mode.map(str::trim), sub.map(str::trim)) {
        (Some("4"), _) => Interpolation::Hold,
        (Some("5"), Some("2")) => Interpolation::AutoBezier,
        (Some("5"), Some("4")) => Interpolation::ContinuousBezier,
        (Some("5"), _) => Interpolation::Bezier,
        _ => Interpolation::Linear,
    }
}

#[cfg(test)]
#[path = "prproj_tests.rs"]
mod tests;
