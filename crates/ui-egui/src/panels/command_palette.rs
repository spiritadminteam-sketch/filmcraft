//! Command Palette (Window ▸ Command Palette…): a small search box over everything FilmCraft can
//! do, in the spirit of Premiere's Excalibur extension. Type part of a name and press Enter:
//! - any menu or keyboard command (it runs as from its menu);
//! - an effect ("Effect: Gaussian Blur"), applied to every selected clip in one undo step;
//! - a text fill colour ("Fill: Yellow"), set on every selected graphic in one undo step;
//! - a speaker marker at the playhead ("Marker: Random");
//! - values, typed: `+5` / `-12` move the playhead by frames (`1.` one second, `2..` two
//!   minutes, as Excalibur writes them); `scale 120`, `opacity 50`, `volume -3` set that on
//!   every selected clip.
//!
//! First letters work too: `g b` finds Gaussian Blur, `f r` Fill: Red.
//!
//! Up / Down pick, Enter runs, Esc closes. What was run last comes first next time.
//! Automation ids: `palette.input`, `palette.item.<n>` (n from 0, top first).

use egui::{Align2, Color32, Rect, Sense, pos2, vec2};
use filmcraft_project::EffectKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FilmcraftApp;
use crate::theme::Tokens;

/// Most recent actions kept for ranking.
const RECENT_MAX: usize = 30;
/// Rows shown.
const SHOWN: usize = 12;

/// The palette while it is open.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaletteState {
    pub query: String,
    /// The highlighted row.
    pub selected: usize,
    /// The input still has to take the keyboard focus.
    #[serde(skip)]
    pub focus: bool,
}

/// One thing the palette can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Command(String),
    Effect(String),
    Fill(&'static str),
    /// Move the playhead by this many frames.
    Step(i64),
    /// Set (effect, param) to a value on every selected clip.
    Set(&'static str, &'static str, f64),
    /// A marker at the playhead: (name, colour label).
    Marker(&'static str, &'static str),
}

impl Action {
    /// A stable key, for the recent list.
    pub fn key(&self) -> String {
        match self {
            Action::Command(id) => format!("cmd:{id}"),
            Action::Effect(id) => format!("fx:{id}"),
            Action::Fill(hex) => format!("fill:{hex}"),
            Action::Step(n) => format!("step:{n}"),
            Action::Set(e, p, v) => format!("set:{e}.{p}={v}"),
            Action::Marker(n, _) => format!("marker:{n}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    /// Where it comes from (a menu path, "Effect", "Fill"), shown on the right and searched too.
    pub detail: String,
    pub shortcut: Option<String>,
    pub action: Action,
}

/// Text fill colours: his caption speaker colours first, then the rest of his saved styles.
pub const FILLS: &[(&str, &str)] = &[
    ("Green (Zach)", "#26FF0F"),
    ("Yellow (Random)", "#FCFF0F"),
    ("Red (Random 2)", "#FF0F0F"),
    ("Orange", "#FF9C0F"),
    ("Purple", "#910FFF"),
    ("Pink", "#FF00F0"),
    ("Blue", "#0F58FF"),
    ("White", "#FFFFFF"),
    ("Black", "#000000"),
];

fn hex_rgb(hex: &str) -> Color32 {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Everything the palette offers, before filtering.
pub fn entries(app: &FilmcraftApp) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for c in filmcraft_engine::command_specs() {
        // menu commands only: the rest need parameters a palette can't ask for
        if c.menu.is_empty() || !seen.insert(c.id) {
            continue;
        }
        out.push(Entry {
            label: c.label.trim_end_matches('…').to_string(),
            detail: c.menu.join(" ▸ "),
            shortcut: app.session.shortcuts.primary(c.id),
            action: Action::Command(c.id.to_string()),
        });
    }
    for c in crate::menus::UI_COMMANDS.iter().chain(crate::panels::keyboard::COMMANDS) {
        if c.id == "window.commandPalette" || !seen.insert(c.id) {
            continue;
        }
        out.push(Entry {
            label: c.label.trim_end_matches('…').to_string(),
            detail: if c.menu.is_empty() { "Keyboard".into() } else { c.menu.join(" ▸ ") },
            shortcut: app.session.shortcuts.primary(c.id),
            action: Action::Command(c.id.to_string()),
        });
    }
    for e in filmcraft_project::effect_defs() {
        if e.intrinsic || !matches!(e.kind, EffectKind::Video | EffectKind::Audio) {
            continue;
        }
        let kind = if e.kind == EffectKind::Video { "Video Effect" } else { "Audio Effect" };
        out.push(Entry { label: format!("Effect: {}", e.name), detail: kind.into(), shortcut: None, action: Action::Effect(e.id.to_string()) });
    }
    for (name, hex) in FILLS {
        out.push(Entry { label: format!("Fill: {name}"), detail: format!("Text colour {hex}"), shortcut: None, action: Action::Fill(hex) });
    }
    for (name, color) in SPEAKERS {
        out.push(Entry {
            label: format!("Marker: {name}"),
            detail: format!("Speaker marker ({color}) at the playhead"),
            shortcut: None,
            action: Action::Marker(name, color),
        });
    }
    out
}

/// His speakers and their marker colours.
pub const SPEAKERS: &[(&str, &str)] = &[("Zach", "Forest"), ("Random", "Yellow"), ("Random 2", "Rose")];

/// Clip values a typed number sets: (word, effect, param, shown name, unit).
const VALUES: &[(&str, &str, &str, &str, &str)] = &[
    ("scale", "motion", "scale", "Scale", "%"),
    ("opacity", "opacity", "opacity", "Opacity", "%"),
    ("volume", "volume", "level", "Volume", " dB"),
    ("rotation", "motion", "rotation", "Rotation", "°"),
];

/// Frames in an Excalibur-style time: `5` frames, `1.` seconds, `2..` minutes, `.3` frames.
fn frames_of(s: &str, fps: f64) -> Option<i64> {
    let dots = s.chars().rev().take_while(|c| *c == '.').count();
    let body = s.trim_end_matches('.');
    if let Some(f) = body.strip_prefix('.') {
        return f.parse::<i64>().ok();
    }
    let n: f64 = body.parse().ok()?;
    let f = match dots {
        0 => n,
        1 => n * fps,
        2 => n * fps * 60.0,
        _ => return None,
    };
    f.is_finite().then_some(f.round() as i64).filter(|f| f.abs() <= 1_000_000_000)
}

/// Entries made from what was typed: a playhead move or a value for the selected clips.
pub fn typed(app: &FilmcraftApp, query: &str) -> Vec<Entry> {
    let q = query.trim();
    let mut out = Vec::new();
    let fps = app.session.sequence_rate().as_f64().max(1.0);
    if let Some(sign @ ('+' | '-')) = q.chars().next()
        && let Some(n) = frames_of(q[1..].trim(), fps)
        && n != 0
    {
        let n = if sign == '-' { -n } else { n };
        out.push(Entry { label: format!("Move playhead {n:+} frames"), detail: "Timecode".into(), shortcut: None, action: Action::Step(n) });
    }
    let mut words = q.split_whitespace();
    if let (Some(w), Some(v), None) = (words.next(), words.next(), words.next())
        && let Ok(v) = v.parse::<f64>()
        && v.is_finite()
    {
        let w = w.to_lowercase();
        for (word, effect, param, name, unit) in VALUES {
            if word.starts_with(&w) {
                out.push(Entry {
                    label: format!("Set {name} to {v}{unit}"),
                    detail: "Selected clips".into(),
                    shortcut: None,
                    action: Action::Set(effect, param, v),
                });
            }
        }
    }
    out
}

/// How well `e` matches `query` (None: not at all). Every word of the query must appear in the
/// label or its detail; a label that starts with the query, then a word that does, ranks first.
pub fn score(e: &Entry, query: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let label = e.label.to_lowercase();
    let hay = format!("{label} {}", e.detail.to_lowercase());
    let words: Vec<&str> = q.split_whitespace().collect();
    // first letters, as Excalibur searches: `g b` = Gaussian Blur (each word starts a word of the
    // label, in order)
    let initials = words.len() > 1 && words.iter().all(|w| w.len() <= 2) && {
        let lw: Vec<&str> = label.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        let bare_words: Vec<&str> = match label.split_once(": ") {
            Some((_, b)) => b.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect(),
            None => lw.clone(),
        };
        let seq = |lw: &[&str]| {
            let mut it = lw.iter();
            words.iter().all(|q| it.any(|w| w.starts_with(q)))
        };
        seq(&bare_words) || seq(&lw)
    };
    if initials {
        return Some(260 - (label.len() as i32).min(80));
    }
    if !words.iter().all(|w| hay.contains(w)) {
        return None;
    }
    // "Effect: Gaussian Blur" / "Fill: Red" also count as starting with what follows the colon
    let bare = label.split_once(": ").map_or(label.as_str(), |(_, b)| b);
    let mut s = 0;
    if label.starts_with(&q) || bare.starts_with(&q) {
        s += 300;
    } else if label.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(words.first().copied().unwrap_or(""))) {
        s += 150;
    } else if label.contains(&q) {
        s += 80;
    }
    // shorter labels first among equals
    s -= (label.len() as i32).min(80);
    Some(s)
}

/// The matches for `query`, best first, recent ones lifted.
pub fn matches(app: &FilmcraftApp, query: &str) -> Vec<Entry> {
    let recent = &app.ui.palette_recent;
    let typed = typed(app, query);
    let mut v: Vec<(i32, Entry)> = typed.into_iter().map(|e| (10_000, e)).collect();
    v.extend(entries(app).into_iter().filter_map(|e| {
        let s = score(&e, query)?;
        let r = recent.iter().position(|k| *k == e.action.key()).map_or(0, |i| 400 - (i as i32) * 10);
        Some((s + r, e))
    }));
    v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    v.into_iter().map(|(_, e)| e).collect()
}

/// Run `cmds` as one undo step named `label`.
fn as_one_step(app: &mut FilmcraftApp, label: &str, cmds: Vec<(&str, Value)>) -> Result<(), String> {
    let before = app.session.history.undo.len();
    let mut err = None;
    for (c, p) in cmds {
        if let Err(e) = app.session.execute(c, p) {
            err.get_or_insert(e.to_string());
        }
    }
    let h = &mut app.session.history;
    if h.undo.len() > before + 1 {
        h.undo.truncate(before + 1);
        if let Some(u) = h.undo.get_mut(before) {
            u.0 = label.to_string();
        }
    }
    err.map_or(Ok(()), Err)
}

/// The selected clips that are graphics (text).
fn selected_graphics(app: &FilmcraftApp) -> Vec<u64> {
    let Some(seq) = app.session.active_sequence() else { return Vec::new() };
    app.session
        .state
        .selection
        .iter()
        .filter(|c| seq.find_item(**c).is_some_and(|(_, it)| it.effects.iter().any(|e| e.effect == "graphic_text")))
        .map(|c| c.0)
        .collect()
}

/// Do `a`.
pub fn run(app: &mut FilmcraftApp, ctx: &egui::Context, a: &Action) -> Result<(), String> {
    let r = match a {
        Action::Command(id) => crate::menus::invoke(app, ctx, id, json!({})).map(|_| ()),
        Action::Effect(id) => {
            let clips: Vec<u64> = app.session.state.selection.iter().map(|c| c.0).collect();
            if clips.is_empty() {
                Err("select the clips to apply the effect to".into())
            } else {
                app.session.execute("effects.apply", json!({"clips": clips, "effect": id})).map(|_| ()).map_err(|e| e.to_string())
            }
        }
        Action::Step(n) => app.session.execute("playhead.step", json!({"frames": n})).map(|_| ()).map_err(|e| e.to_string()),
        Action::Set(effect, param, v) => {
            let clips: Vec<u64> = app.session.state.selection.iter().map(|c| c.0).collect();
            if clips.is_empty() {
                Err("select the clips to change".into())
            } else {
                let cmds = clips.iter().map(|c| ("effects.setParam", json!({"clip": c, "effect": effect, "param": param, "value": v}))).collect();
                as_one_step(app, "Set Value", cmds)
            }
        }
        Action::Marker(name, color) => app.session.execute("markers.add", json!({"name": name, "color": color})).map(|_| ()).map_err(|e| e.to_string()),
        Action::Fill(hex) => {
            let clips = selected_graphics(app);
            if clips.is_empty() {
                Err("select the text clips to colour".into())
            } else {
                let cmds = clips.iter().map(|c| ("graphics.set", json!({"clip": c, "props": {"fill_color": hex, "fill": true}}))).collect();
                as_one_step(app, "Fill Color", cmds)
            }
        }
    };
    if r.is_ok() {
        let k = a.key();
        app.ui.palette_recent.retain(|x| *x != k);
        app.ui.palette_recent.insert(0, k);
        app.ui.palette_recent.truncate(RECENT_MAX);
    }
    r
}

/// Open the palette (or close it when it is open).
pub fn toggle(app: &mut FilmcraftApp) {
    app.ui.palette = match app.ui.palette {
        Some(_) => None,
        None => Some(PaletteState { focus: true, ..Default::default() }),
    };
}

/// The palette window, while open.
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut st) = app.ui.palette.clone() else { return };
    let t: Tokens = app.tokens;
    let found = matches(app, &st.query);
    let n = found.len().min(SHOWN);
    let (up, down, enter, esc) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    if down && n > 0 {
        st.selected = (st.selected + 1) % n;
    }
    if up && n > 0 {
        st.selected = (st.selected + n - 1) % n;
    }
    st.selected = st.selected.min(n.saturating_sub(1));
    let mut chosen: Option<Action> = if enter { found.get(st.selected).map(|e| e.action.clone()) } else { None };
    let mut elems: Vec<(String, Rect, String)> = Vec::new();
    let screen = ctx.content_rect();
    let width = 520.0_f32.min(screen.width() - 32.0).max(200.0);
    let before = st.query.clone();
    egui::Area::new(egui::Id::new("command-palette")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(width);
            let r = ui.add(
                egui::TextEdit::singleline(&mut st.query).hint_text(tl!("Type a command, effect or colour…")).desired_width(width).font(egui::TextStyle::Body),
            );
            if st.focus {
                r.request_focus();
                st.focus = false;
            }
            elems.push(("palette.input".into(), r.rect, "Command Palette".into()));
            ui.add_space(4.0);
            for (i, e) in found.iter().take(SHOWN).enumerate() {
                let (row, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
                if i == st.selected {
                    ui.painter().rect_filled(row, 3.0, t.accent.gamma_multiply(0.35));
                } else if resp.hovered() {
                    ui.painter().rect_filled(row, 3.0, t.hover);
                }
                let mut x = row.min.x + 8.0;
                if let Action::Fill(hex) = &e.action {
                    ui.painter().rect_filled(Rect::from_center_size(pos2(x + 6.0, row.center().y), vec2(12.0, 12.0)), 2.0, hex_rgb(hex));
                    x += 18.0;
                }
                ui.painter().text(pos2(x, row.center().y), Align2::LEFT_CENTER, &e.label, Tokens::ui(13.0), t.text);
                let right = match &e.shortcut {
                    Some(s) => format!("{}   {s}", e.detail),
                    None => e.detail.clone(),
                };
                ui.painter().text(pos2(row.max.x - 8.0, row.center().y), Align2::RIGHT_CENTER, right, Tokens::ui(11.0), t.text_faint);
                elems.push((format!("palette.item.{i}"), row, e.label.clone()));
                if resp.clicked() {
                    chosen = Some(e.action.clone());
                }
            }
            if found.is_empty() {
                ui.label(egui::RichText::new(tl!("Nothing matches")).color(t.text_faint));
            }
        });
    });
    if st.query != before {
        st.selected = 0;
    }
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if esc {
        app.ui.palette = None;
        return;
    }
    if let Some(a) = chosen {
        app.ui.palette = None;
        if let Err(e) = run(app, ctx, &a) {
            app.ui.status = e;
        }
        return;
    }
    app.ui.palette = Some(st);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(label: &str, detail: &str) -> Entry {
        Entry { label: label.into(), detail: detail.into(), shortcut: None, action: Action::Command(label.into()) }
    }

    #[test]
    fn every_word_must_match_and_prefixes_rank_first() {
        assert!(score(&e("Effect: Gaussian Blur", "Video Effect"), "gauss blur").is_some());
        assert!(score(&e("Effect: Gaussian Blur", "Video Effect"), "gauss sharpen").is_none());
        let pre = score(&e("Effect: Gaussian Blur", "Video Effect"), "gaus").unwrap_or(i32::MIN);
        let mid = score(&e("Effect: Directional Blur", "Video Effect"), "blur").unwrap_or(i32::MIN);
        let inside = score(&e("Effect: Camera Blur", "Video Effect"), "amera").unwrap_or(i32::MIN);
        assert!(pre > inside && mid > inside, "{pre} {mid} {inside}");
        assert!(score(&e("Fill: Red", "Text colour #FF0F0F"), "red").unwrap_or(0) >= 200);
        // the detail (a menu path) is searched too
        assert!(score(&e("Ripple Delete", "Edit"), "edit ripple").is_some());
        assert_eq!(score(&e("Anything", ""), "  "), Some(0));
    }

    #[test]
    fn first_letters_find_words_in_order() {
        assert!(score(&e("Effect: Gaussian Blur", "Video Effect"), "g b").unwrap_or(0) >= 200);
        assert!(score(&e("Fill: Red (Random 2)", ""), "f r").unwrap_or(0) >= 200);
        assert!(score(&e("Add Marker to Selection", ""), "a m t s").unwrap_or(0) >= 200);
        assert!(score(&e("Effect: Gaussian Blur", "Video Effect"), "b g").is_none(), "in order");
    }

    #[test]
    fn excalibur_times() {
        assert_eq!(frames_of("5", 60.0), Some(5));
        assert_eq!(frames_of("1.", 60.0), Some(60));
        assert_eq!(frames_of("2..", 60.0), Some(7200));
        assert_eq!(frames_of(".3", 60.0), Some(3));
        assert_eq!(frames_of("x", 60.0), None);
        assert_eq!(frames_of("1...", 60.0), None);
        assert_eq!(frames_of("1e300", 60.0), None, "hostile numbers don't overflow");
    }

    #[test]
    fn fills_parse() {
        for (_, hex) in FILLS {
            assert_eq!(hex.len(), 7);
            assert!(u32::from_str_radix(&hex[1..], 16).is_ok(), "{hex}");
        }
        assert_eq!(hex_rgb("#FF0F0F"), Color32::from_rgb(255, 15, 15));
    }
}
