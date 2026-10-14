# Sandii

Falling-sand style simulation with materials, rigid bodies, and a small interactive sandbox.

## Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain with `cargo`)

No extra assets or servers are required for the default windowed demo.

## Launch the game

From the repository root:

```bash
cargo run --example sandbox_window
```

For a faster, optimized build:

```bash
cargo run --release --example sandbox_window
```

The first run compiles dependencies; later runs start quickly.

### Close the game

Press **Esc**, or close the window.

## Controls

| Input | Action |
| --- | --- |
| **Click material chip** | Select brush material (named color chips + selected blurb in the left panel) |
| **Click wow recipe** | Load a reaction recipe; click again to cycle ingredients (lava+water, fire, acid, C4, metal→water charge) |
| **Left click** | Paint / mode action (draw, rigid stroke, explode, heat, cool) |
| **Right click** | Erase (empty) |
| **[** / **]** | Smaller / larger brush (hold to repeat); explosion radius in Blast mode |
| **Tab** | Cycle modes: Draw → Rigid → Blast → Heat → Cool |
| **Shift+drag** | Rectangle paint / erase (or rigid rect in Rigid mode) |
| **C** | Clear a large area around the view center |
| **P** | Toggle parallel simulation |
| **Esc** | Quit |

Number keys and letter shortcuts also jump to palette slots when available. The panel repeats mode and recipe hints.

## Web (WASM)

The crate [`crates/sandii_web`](crates/sandii_web) runs the same simulation in the browser (canvas + HTML controls). The view is **640×360** pixels (same as the world width/height at 1× scale) to keep the wasm build light. The page uses a **material swatch grid** and **mode toggle buttons** (not dropdowns); Explosion and Heat/Cool hide the material row; Rigid grays out materials that cannot form rigid bodies. The engine is built **without** the `parallel` feature on wasm; simulation uses **at most one fixed substep per animation frame** to keep the tab responsive. Use **Trunk** for a local server that serves `.wasm` with the correct MIME type.

Install the wasm target and Trunk once:

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
```

From [`crates/sandii_web`](crates/sandii_web):

```bash
cd crates/sandii_web
env -u NO_COLOR trunk serve
```

If `trunk` errors about `--no-color`, your environment may set `NO_COLOR` to a non-boolean value; unset it as above or run `NO_COLOR=0 trunk serve`.

Open the URL Trunk prints (usually `http://127.0.0.1:8080`). The page loads the wasm module and starts the simulation via `#[wasm_bindgen(start)]`.

**Wow recipes:** open **Materials & modes** and use the recipe chips (lava + water, fire chain, acid bite, C4 boom, metal → water charge). Tap a chip to load a material; tap again to cycle the next ingredient. A short first-run tip also points at paint / erase / recipes.

### Systems notes (playable)

- **Active sim size:** desktop viewport **640×360**; dense grid is viewport + thin margin, capped at **≤700×400** cells (not a 3× screen buffer). Web viewport is **448×252** with tight bounds.
- **Chunks:** parallel work units are **64×36** (16:9), not square 32×32.
- **Metal + electricity:** jagged visitor — free sparks in empty; charge zigzags through metal/water via `ELECTRIFIED` glow **without replacing** the medium, then pops out. Recipe: *Metal → water*.
- **Asleep sand:** settled sand grains sleep into the **static physics** mesh (HUD may show `SLEEP`); dig or remove support to wake them.
- **Rigid boundaries:** pixel bodies use marching-squares → Douglas–Peucker → ear-clip colliders (covered by dedicated tests).

### Known limits (perf & storms)

Honest laptop budget after the active-grid dial-back (see `Simulation::ACTIVE_SIM_MAX_*`):

| Surface | Size | What to expect |
| --- | --- | --- |
| **Desktop** (`sandbox_window` **release**) | Viewport **640×360**; active dense grid **≤700×400** | Idle HUD ~50–60 fps. Dense paint storms (sand/water/recipes) stay playable — not a slideshow. Occasional step spikes above one 16 ms frame are possible under max chaos. |
| **Web** (`sandii_web`) | Viewport **448×252**; **one** fixed substep/frame; no `parallel` | Lighter than desktop; still smooth for paint play. Heavier than one substep (or huge brushes) can drop frame rate in the tab. |
| **Debug / unoptimized** | Same sizes | Expect much lower FPS; always prefer `--release` for feel. |

**What still costs FPS**

- **Active cell count** — the old ~3× screen buffer (~1920×1080) is intentionally gone; do not re-expand past the cap without re-measuring.
- **Dense falling matter** — full-view sand + liquid + oil/lava reactions dominate step time more than idle terrain.
- **Rigid bodies + static mesh rebuilds** — asleep sand and big rigid strokes add physics cost.
- **Render** — minifb blit + UI panel are extra on top of sim step (headless numbers are sim-only).

**Crash / stress:** common paint storms should not crash. Core smoke: `cargo test -p falling_everything_core paint_storm stress_scene`. Measure sim step yourself:

```bash
cargo run --release --example headless_bench
```

Look for `desktop_idle`, `desktop_paint_storm`, and `desktop_recipe_storm` (p95 under ~16 ms is the 60 Hz budget for sim alone). Evidence snapshot (Apple Silicon laptop, 2026-07-22): paint-storm p50 ≈ 6.4 ms, p95 ≈ 14.9 ms; recipe-storm p95 ≈ 9.6 ms; active grid 700×400.

**Alternative (wasm-pack + static server):** from the same directory, `wasm-pack build --target web --out-dir pkg`, then serve this folder with a server that sets `Content-Type: application/wasm` for `.wasm` files (many dev servers do). Use [`index_wasm_pack.html`](crates/sandii_web/index_wasm_pack.html) as `index.html` next to `pkg/`, or open it after adjusting paths.

## Other examples

Headless performance benchmark (small + desktop-scale idle/storm tiers; prints timings, no window):

```bash
cargo run --example headless_bench
```

Release mode (use this for honest numbers):

```bash
cargo run --release --example headless_bench
```

## Project layout

- **`crates/falling_everything_core`** — simulation engine
- **`crates/falling_everything_godot`** — adapter types for future Godot integration (not used by `sandbox_window`)
- **`crates/sandii_web`** — wasm-bindgen canvas shell for the browser (`trunk serve`)
- **`examples/sandbox_window.rs`** — minifb window and input; this is the interactive “game” entry point
