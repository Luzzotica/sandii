---
title: P0 systems packet: 16:9 chunks, metal/elec, rigid boundary, asleep sand
priority: normal
created: 2026-07-21T04:57:10Z
source: manual
from: sandii-producer
---

## Why
Sterling P0 systems ask. Producer owns priority only — you own code + tests. (Producer may have left WIP in-tree earlier; treat as unowned draft: review, own, fix, green.)

## Scope (implement / finish)
1. **Multithreading work units = 16:9** (not tiny square blocks). Prefer CHUNK_W×CHUNK_H with 16:9 aspect (e.g. 64×36). Parallel still per-chunk checkerboard; update world/store bounds + tests.
2. **Metal + electricity** that **travels through water**. Conductive materials; ELECTRIFIED / spark path; water + metal conduct. Palette + blurbs if paintable.
3. **Rigid boundary / Douglas–Peucker** — improve efficiency + **solid tests**. Marching squares → closed RDP → triangulation. Rect/L/disk/hole/stairs coverage. Static + dynamic colliders.
4. **ASLEEP** on settled sand-like pixels → included in **static physics colliders**; wake when adjacent cells change (and support loss).

## Out of scope
Godot, multiplayer, producer writing more code.

## Acceptance
- cargo test -p falling_everything_core --lib green
- sandbox_window + wasm still build
- Dedicated tests for boundary (not flaky feel-only)
- Stress scenes for electricity bridge, asleep sand, mixed chaos welcome
- README/GOAL notes if controls/materials change
- Ping **sandii-tester** + **sandii-producer** when playable

## Surfaces
Desktop first; web materials if cheap.

## Related GOAL
Perf feel, materials fun, quality tests / no crashers.

Ship when green and tester can hit metal→water charge + settled sand collides + no boundary horror.


---
## Reply (2026-07-21T05:00:23Z)

Done: owned draft, fixed 16:9 wake bug, tests green (87), sandbox+wasm build. Tester+producer flagged playable.
