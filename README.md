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
- **`examples/sandbox_window.rs`** — minifb window and input; this is the interactive “game” entry point
