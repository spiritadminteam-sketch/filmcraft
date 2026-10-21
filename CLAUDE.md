# Working rules for this repo

Goal: keep credit/token usage low on a large codebase. Read less, read targeted, and never sacrifice correctness to save tokens.

## 1. Section annotations (the code map)

- Every ~50 lines of code, add a one-line section comment using the marker `@@`:
  - JS/TS/C/C++/Swift: `// @@ Caption track: re-times clips to word timestamps`
  - Python/shell: `# @@ Export: builds ffmpeg args from sequence settings`
- One line, plain English, says *what the section does* (not how). Max ~15 words.
- Start every file with a header annotation: `@@ FILE: <purpose of this file in one line>`.
- Put an annotation at natural boundaries (function/class/block), not mechanically at line 50.
- When you change a section, update its `@@` line in the same edit. A stale annotation is a bug.
- When you add new code, add its annotation. When you touch an un-annotated file, annotate the parts you work on.

## 2. How to find code (cheap first)

Follow this order. Stop as soon as you have what you need.

1. Check `CODEMAP.md` (if present) to pick the right file.
2. Search annotations, not code: `grep -rn "@@" <dir>` or `grep -n "@@" <file>`.
3. Read **only** the section you need, using line ranges (offset/limit). Do not open whole large files.
4. Only search raw code (function names, symbols) if annotations don't point you to it.

## 3. How to edit (correctness rules)

- Annotations are for **locating** code. Always read the actual lines before editing them. Never write an edit based on the annotation alone.
- If an annotation doesn't match the code under it, trust the code, fix the annotation, and mention it in one line.
- Read enough surrounding context to keep the edit consistent (imports, types, callers you're changing).

## 4. Token-saving habits

- Don't re-read a file you just read or just edited; use what you already have.
- Don't paste large files, logs or build output into the conversation. Pipe through `tail -n 50`, `grep`, or `head`.
- Run tests/builds with quiet flags and only show failures.
- Keep plans and status updates short: a few bullets, no recap of finished steps.
- Don't explore the whole repo "to understand it". Use the code map and annotations.
- Prefer small, focused edits over rewriting whole files.
- One task per session where possible (e.g. building a feature vs. editing a video vs. adding a plugin). Start a fresh session when switching tasks instead of carrying a huge context.

## 5. CODEMAP.md

- Maintain `CODEMAP.md` at the repo root: one line per important file or folder, `path — what it does`.
- Update it when you add, move or delete a file. Keep it short; it's an index, not documentation.
- If it doesn't exist yet, create it the first time you need to orient yourself, from the `@@ FILE:` headers.

## 6. Exports and renders

- Never export, render or encode final videos (shorts or long-form) unless I explicitly ask in that message. Exports burn a lot of credits; I export manually.
- When an edit is ready, stop and say "Ready to export: <sequence name>" in one line.
- For checking your work, use a single-frame render or a short preview, never a full export.

## 7. Finishing a task

- When a task is done (a video is finished, a feature works), say so in one line and suggest running `/wrapup`.
- Don't keep finished work (old edits, old shorts, old export tests) in play. Treat finished projects as closed unless I reopen them.
- At the start of a session, read `SESSION_NOTES.md` if it exists, then continue from it. Don't re-explore what it already covers.

## Compact instructions

When compacting, keep: the current task, open bugs, decisions made, files changed and why. Drop: finished videos and shorts, export/render output, logs, tool output, and anything already recorded in `SESSION_NOTES.md`.

## 8. Keep this file lean

This file is loaded into every session, so every line here costs tokens every time. Add rules only when they prevent a repeated mistake. Put long docs elsewhere and link to them.
