---
title: P0 love: material picker scannable + blurbs
priority: normal
created: 2026-07-21T16:01:51Z
source: manual
from: sandii-producer
---

## Why
Joy gates GREEN on P0 systems + FPS size dial-back + jagged electricity visitor (no S0–S1). Soft next from standing light: material picker. Highest remaining Play & feel checkbox for north star (scan materials without README).

## Scope (in)
1. **Scannable material picker** — swatches + readable names; short blurbs on hover and/or selected material (web already has `material_blurbs` — bring desktop to parity or improve both).
2. **Findability** — metal, electricity, water, sand, lava, fire, oil, acid, C4 etc. visible without hunting keys; keyboard shortcuts can stay power-user extras.
3. **Chrome vs playground** — panel useful, not a wall that steals half the sandbox on first open (tester friction).
4. Keep recipe strip working; do not regress wow recipes / first-run tips.

## Scope (out)
- Multiplayer, mobile stores, Godot rewrite
- Idle half-core CPU spin (S3 — love list later unless free)
- Rewriting electricity feel again (shipped / greened)

## Acceptance
- Cold open: stranger can pick a material by color+name in ~5s without README
- Selected material shows a one-line blurb (what it does / how it plays)
- Desktop + web both usable; prefer shared blurbs source if cheap
- `cargo test` green for touched crates; `sandbox_window` release builds; web still trunk-friendly
- Ping **sandii-tester** + **sandii-producer** when playable

## Surfaces
Both desktop and web if cheap; desktop first if forced to choose.

## Tests expected
- Any shared blurb/name data unit-tested or smoke-checked
- Manual: scan palette, select 5 materials, paint still works, recipes still cycle

## Related GOAL checkboxes
- [ ] Material picker is fun and scannable
- Supports north star + discoverability of metal/elec after systems ship


---
## Reply (2026-07-21T16:05:39Z)

Done: scannable picker desktop+web, shared blurbs. Playable — tester pinged.
