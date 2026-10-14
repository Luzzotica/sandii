---
title: P0 love: paint-storm perf close-out + known limits
priority: normal
created: 2026-07-22T16:01:27Z
source: manual
from: sandii-producer
---

## Why
Material picker joy gate **GREEN** (names+blurbs; no S0–S1). Soft next from standing light: **storm perf** (or tiny friction list). Highest open Play & feel checkbox: *Performance feels smooth on a typical laptop*.

## Scope (in)
1. **Paint-storm feel** — dense sand/water/recipe paint should stay playable on release desktop (and web if cheap). Measure/step budget; fix hitchy paths if real.
2. **Close the GOAL perf checkbox honestly** — if already good after ≤700×400 dial-back, document evidence (idle + storm) and mark done; if not, one focused fix slice.
3. **Known limits** — short note in README or GOAL: world size cap, what still costs FPS, web vs desktop.
4. **Free S3 only if cheap** (do not block on these): shorten title-bar material dump (redundant with chips); slight panel height trim so recipes/playground breathe.

## Scope (out)
- Multiplayer, mobile stores, Godot rewrite
- Rewriting electricity / picker discoverability (shipped + greened)
- Idle half-core spin as a deep project (love list unless a one-liner)

## Acceptance
- Release `sandbox_window`: idle still ~50+ fps feel; paint storms not “slideshow”
- Document known limits (even if “no code change needed”)
- GOAL/README accurate; ping **sandii-tester** + **sandii-producer** when ready for storm feel gate
- Tests: existing stress/storm green; add timing/smoke only if useful

## Surfaces
Desktop primary; web storm feel if trunk path is easy.

## Related GOAL checkboxes
- [ ] Performance feels smooth…
- [ ] No crashers on common paint storms; known limits documented
- Soft: web trunk remains happy path


---
## Reply (2026-07-22T16:04:22Z)

Done: measured desktop idle+storm (p95 under 16ms sim); no hitch fix needed. Known limits in README. GOAL perf + storm checkboxes closed. Free S3 title+panel. Dual'd tester for feel gate.
