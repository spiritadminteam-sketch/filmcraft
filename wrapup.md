---
description: Close out the current task - save a short handoff note, update annotations and the code map, then tell me to /clear
---

Wrap up this session so the next one starts small and cheap. Do these steps in order and keep output short.

1. **Annotations:** for every file changed this session, make sure each `@@` annotation matches the code under it and new sections have one. Fix any that are stale.
2. **Code map:** update `CODEMAP.md` for any files added, moved or deleted. Create it if missing.
3. **Handoff note:** overwrite `SESSION_NOTES.md` (replace it, never append) with at most 20 lines:
   - Current state: what works now, in 1-3 bullets
   - Open items: unfinished work or known bugs
   - Decisions: choices made this session that the next session must respect
   - Files touched: path - one-line reason
   Leave out finished videos/shorts, exports, logs and step-by-step history.
4. **Don't export or render anything.**
5. Reply with exactly: "Wrapped up. Notes saved to SESSION_NOTES.md - run /clear to start fresh." plus any open item I should know about, in one line.

$ARGUMENTS
