# ADR 0002: VST3 audio plug-ins, hosted in a separate process

- **Status:** accepted in the `spiritadminteam-sketch/filmcraft` fork (2026-10-15, approved by
  the fork's owner); proposed upstream for [G11](../gaps.md#g11-no-plugin-hosting)
- **Builds on:** [ADR 0001](0001-platform-ffi.md) (`unsafe` only in `crates/platform`)

## Context

Editors coming from Premiere expect their third-party audio plug-ins (VST3 EQs, compressors,
de-essers, reverbs). A VST3 plug-in is native code in a shared library, driven through COM-style
C++ interfaces: hosting one needs FFI (`unsafe`), and the plug-in's own code can crash, hang or
corrupt memory. FilmCraft's first rule is that it never crashes and never loses work; nothing
inside the editor process can contain a segfault in someone else's library.

Steinberg's VST3 SDK headers are MIT-licensed (SDK 3.8). The `vst3` crate (MIT OR Apache-2.0)
provides Rust bindings generated from them.

## Decision

1. **Plug-ins run in a helper process, never in the editor.** `apps/filmcraft-plugin-host`
   (`filmcraft-plugin-host`, shipped next to `filmcraft`) loads plug-ins and processes audio
   for FilmCraft over its stdin / stdout pipes. The protocol (`filmcraft_plugins::proto`) is
   length-prefixed frames: JSON for control (scan, create, destroy) and raw `f32` for audio
   blocks; every length and count is capped before allocating.
2. **Failure means pass-through.** The client (`filmcraft_plugins::client`) waits with time-outs
   (3 s per block, 30 s to create an instance, 60 s to scan a module). A host that crashes, hangs
   or answers nonsense is killed; the affected effects pass their audio through unchanged and log
   why, and the next new instance starts a fresh host. Scans use their own host process, so a
   slow or crashing module never blocks playback; a module that crashes the scan host is reported
   and the scan goes on.
3. **The FFI lives in `crates/platform`** (module `vst3host`, feature `vst3`, enabled only by
   the helper), under ADR 0001's rules: `#[allow(unsafe_code)]` on that module only, a
   `// SAFETY:` comment on every `unsafe` block, a safe public API (`Module`, `Instance`).
   In the helper every request also runs under `catch_unwind`.
4. **Plug-ins are effects like any other.** A scan registers each audio effect class as an
   `EffectDef` (`vst3:<class id>`, Effects ▸ Audio Effects ▸ VST3) through
   `filmcraft_project::register_plugin_effects`; every parameter becomes a keyframeable 0–100 %
   slider in Effect Controls (VST3's normalised value × 100). The audio engine creates the
   processor through a factory hook (`filmcraft_render::audio_fx::set_external_factory`), set by
   the engine; with no hook or an unknown plug-in the effect passes audio through. Clip effects
   and track inserts both work, in playback and in export.
5. **Scope of the first version:** Windows and Linux; audio effects (instruments are skipped);
   stereo in and out (a mono plug-in gets the mix); no plug-in editor windows yet, and plug-in
   state beyond its parameters is not saved with the project.

## Consequences

- A plug-in can at worst stall one audio block for 3 s before it is cut off; it cannot take
  the editor down.
- One pipe round trip per block per plug-in costs some CPU; fine for a handful of plug-ins.
- New crates: `filmcraft-plugins` (L2: protocol, client, scan cache, effect wrapper) and the
  `filmcraft-plugin-host` app (L6).
- CI (`.github/workflows/plugins.yml`) builds the `vst3` crate's example gain plug-in as a real
  bundle on Linux and Windows and runs the helper against it end to end.
