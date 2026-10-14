# Sandii — more love

## Why this exists

Sandii already has something real: a **falling-sand sim** with many materials, rigid bodies, a desktop sandbox, and a **web/WASM** build. It deserves playtime, polish, and a clear direction — not to sit quiet in Misc.

## Mission

Make Sandii **delightful to open and play with** — the kind of sandbox you launch “for five minutes” and stay for thirty. Keep the core feel (paint matter, watch chaos, discover reactions) while improving discoverability, fun loops, and ship surfaces (desktop + web).

## North star

> Someone who isn’t you can open Sandii, understand what to do in 10 seconds, and find at least one “whoa” reaction without reading the README.

## Goal state (near-term)

### Play & feel
- [x] First-run guidance in-app (materials, paint, erase, clear) — desktop and/or web *(web: dismissible tip + recipe how-to; desktop: panel hints)*
- [x] Material picker is fun and scannable (swatches, names, short blurbs — shared `materials::blurbs`; desktop multi-row named chips + selected blurb; web chips + hover titles)
- [x] At least **3 “wow recipes”** documented and easy to try (e.g. lava+water, fire chain, acid, C4) — in UI or a tiny recipe strip *(shared `recipes` module; desktop strip + web chips; click cycles ingredients; metal→water charge)*
- [x] Performance feels smooth on a typical laptop (`sandbox_window` release + web tab) *(active dense grid ≤700×400; headless storm p95 under 16 ms sim step — see README Known limits; 2026-07-22)*

### Systems (P0 packet)
- [x] Multithreading work units **16:9** (`CHUNK_W×CHUNK_H` = 64×36); checkerboard still per-chunk
- [x] **Metal + electricity** through water (`conductive`, `ELECTRIFIED`, paintable + blurbs)
- [x] Rigid boundary pipeline (MS → closed RDP → triangulation) + dedicated geometry tests
- [x] **ASLEEP** settled sand → static colliders; wake on neighbor edit / support loss

### Surfaces
- [x] Desktop: `cargo run --release --example sandbox_window` is the happy path; controls match README
- [ ] Web: `crates/sandii_web` trunk serve works; UI remains light (no heavy framework required)
- [x] README / GOAL stay accurate when controls change

### Quality
- [ ] Tester has a short play matrix (materials, brush, rigid, clear, web vs desktop)
- [x] No crashers on common paint storms; known limits documented *(README Known limits + `paint_storm_desktop_scale_stays_stable` / stress_scene tests)*
- [ ] Producer tracks a living **love list** (ideas) vs **ship list** (this week)

### Optional stretch (only if core is fun)
- [ ] Shareable web deploy (static host) once polish is there
- [ ] Godot adapter exploration only if it multiplies fun — not a rewrite for its own sake

## Non-goals (for now)

- Competing with Noita feature-for-feature  
- Full multiplayer  
- Mobile store ship (unless it falls out of web naturally later)

## Team

| Agent | Role |
| --- | --- |
| **sandii-producer** | PoC — vision, priorities, “more love” roadmap |
| **sandii** | Engineer — sim, desktop, web, materials |
| **sandii-tester** | Play as a curious user; find fun + bugs |

**Project:** Sandii  
**Code:** `/Users/sterlinglong/NonCloud/PAZA/Projects/sandii`

## How we work

1. Play first, then fix what broke the joy  
2. Small vertical slices > big engine rewrites  
3. Document recipes and controls when you change them  
4. Producer: light heartbeat to keep momentum without over-managing  

## First week vibe

Open the sandbox. Fall in love again. Pick one friction (onboarding, picker, a broken reaction, web UX). Ship that. Repeat.
