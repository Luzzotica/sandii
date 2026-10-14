---
title: P0 electricity feel: jagged visitor, not replace medium
priority: normal
created: 2026-07-21T05:48:50Z
source: manual
from: sandii-producer
---

## Why
Sterling play feel: electricity should *travel through* conductors as a jagged charge, not permanently replace metal/water.

## Desired behavior
1. **Motion / look:** Electricity is **jagged zigzag** from its current cell — hops in **seemingly random** directions (not a calm straight BFS flood only). Think sparks branching / flying off.
2. **Through metal:** Enters metal, travels along the metal body, **pops out the other side** (into empty or next conductor) as visible electricity again.
3. **Through water:** Same — travels through water volume, exits far side.
4. **Restore medium:** While charge passes, metal stays metal and water stays water (or only a brief ELECTRIFIED flag / overlay). **After the spark leaves, cells are metal/water again** — never leave a permanent trail of ELECTRICITY material replacing the conductor.

## Implementation hints (your call)
- Prefer ELECTRIFIED + short-lived ELECTRICITY only in empty (or as a particle/overlay) for the “pop out” arcs; conductors keep material id.
- Randomize hop direction each step (biased along connected conductive graph, with noise / branch chance).
- Avoid set_cell(water → electricity) as the main travel mechanic if it destroys the pool.

## Acceptance
- Paint metal bar + water bridge + spark: charge zigzags through, exits other end; bar/pool still metal/water.
- No permanent conversion of conductors to ELECTRICITY.
- Unit test(s): after charge dies, conductor cells at sample positions are still METAL/LIQUID (not ELECTRICITY).
- cargo test core green; desktop playable.

## Surfaces
Desktop first; web if materials already shared.

Ping sandii-tester + sandii-producer when playable.


---
## Reply (2026-07-21T05:57:20Z)

Done: jagged visitor, restore medium, tests. Playable.
