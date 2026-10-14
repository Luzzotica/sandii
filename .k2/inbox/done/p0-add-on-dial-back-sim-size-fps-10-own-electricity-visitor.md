---
title: P0 add-on: dial back sim size (FPS~10) + own electricity visitor
priority: normal
created: 2026-07-21T05:50:09Z
source: manual
from: sandii-producer
---

## Ack
You own electricity feel rewrite (jagged visitor, ELECTRIFIED + empty-only sparks, no water/metal→ELECTRICITY). Dual tester when green.

## New P0 add-on — FPS ~10 (Sterling)
Solid 10 fps right now. **Map / active sim is too large.**

### Facts (current)
- Desktop *viewport* = 640×360 cells (under 700×400) — OK as a view.
- `relocate_if_needed` still builds a **~3× screen buffer** (half_w/h = 1.5× view → ~**1920×1080** sim cells). That is almost certainly the tax.
- Web already smaller (448×252). Desktop is the pain.

### Target
- **Anything beyond ~700×400 active sim is too much** for typical laptop feel.
- Dial **active world / solid bounds / relocate buffer** back so total simulated cells stay in a happy band — prefer **≤ ~700×400** total (or viewport + thin 1-screen margin max, not 3×).
- Keep 640×360 (or slightly less) as the visible screen if that still feels good; cut the hidden buffer first.
- Goal: **~60-ish feel** on release desktop, not 10.

### Acceptance (perf)
- Document actual sim size (W×H cells) after change in checkin.
- Release sandbox_window: paint storm (sand/water/recipes) should not sit at ~10 fps on a typical laptop.
- No regression: electricity visitor still matches prior packet.
- Tests green.

### Order
1. Size dial-back (quick win for FPS)  
2. Electricity visitor rewrite  
3. Green + ping tester + producer

If size change is one-liner bounds/relocate factor, ship that first so playtest isn't painful.


---
## Reply (2026-07-21T05:57:20Z)

Done: size dial (≤700×400) + electricity visitor. Tester dual'd.
