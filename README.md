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

| Input           | Action                                                                             |
| --------------- | ---------------------------------------------------------------------------------- |
| **0–7**         | Select material (empty, sand, water, gas, static, rigid, light water, heavy water) |
| **[** / **]**   | Smaller / larger brush (hold to repeat)                                            |
| **Left click**  | Paint with the selected material                                                   |
| **Right click** | Erase (empty)                                                                      |
| **R**           | Spawn a rigid rectangle at the cursor                                              |
| **C**           | Clear a large area around the center                                               |
| **P**           | Toggle parallel simulation                                                         |
| **Esc**         | Quit                                                                               |

The window title bar repeats the material and shortcut hints.

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

**Alternative (wasm-pack + static server):** from the same directory, `wasm-pack build --target web --out-dir pkg`, then serve this folder with a server that sets `Content-Type: application/wasm` for `.wasm` files (many dev servers do). Use [`index_wasm_pack.html`](crates/sandii_web/index_wasm_pack.html) as `index.html` next to `pkg/`, or open it after adjusting paths.

## Other examples

Headless performance benchmark (prints timings to the terminal, no window):

```bash
cargo run --example headless_bench
```

Release mode:

```bash
cargo run --release --example headless_bench
```

## Project layout

- **`crates/falling_everything_core`** — simulation engine
- **`crates/falling_everything_godot`** — adapter types for future Godot integration (not used by `sandbox_window`)
- **`crates/sandii_web`** — wasm-bindgen canvas shell for the browser (`trunk serve`)
- **`examples/sandbox_window.rs`** — minifb window and input; this is the interactive “game” entry point
