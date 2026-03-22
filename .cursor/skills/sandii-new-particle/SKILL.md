---
name: sandii-new-particle
description: Add or rename a grid material (particle type) in the sandii / falling_everything_core Rust simulation. Use when introducing new materials, palette entries, or custom step behavior.
---

# Sandii — adding a new particle (material)

## When to use

You are working in this repo’s falling-sand engine (`crates/falling_everything_core`) and need a new material ID, physics behavior, color, or sandbox palette entry.

## Checklist

1. **ID** — In [`crates/falling_everything_core/src/world/mod.rs`](crates/falling_everything_core/src/world/mod.rs), add a new `pub const` in `world::material`. Keep IDs contiguous; do not reuse numbers (saved regions store numeric material IDs).

2. **Builtin definition** — In [`crates/falling_everything_core/src/materials.rs`](crates/falling_everything_core/src/materials.rs), append a `MaterialDef` to `BUILTINS` with `props`, `rule`, and `color_argb`. The file header lists the minimal steps; palette and props arrays are filled from `BUILTINS` automatically.

3. **Custom behavior** — If `Phase` + `inert` + existing `step_sand` / `step_liquid` / `step_gas` / `step_fire` is not enough:
   - Branch in [`step_pixel`](crates/falling_everything_core/src/sim/mod.rs) (often **before** `if props.inert`, if the material must stay `inert: true` for `try_displace` but still run logic each tick — see **Plant** below).
   - Adjust [`step_fire`](crates/falling_everything_core/src/sim/mod.rs) only if fire interaction needs special cases.
   - Use [`World::set_reaction`](crates/falling_everything_core/src/world/mod.rs) / `ReactionOutcome` only when displacement-style reactions fit (see `try_displace`).

4. **Default lifetime** — If the material needs a non-zero `lifetime` when painted, extend [`World::initial_lifetime_for`](crates/falling_everything_core/src/world/mod.rs) (used by `paint_circle`).

5. **Rendering** — Most materials use the static palette from `BUILTINS`. For animated or special shading (e.g. fire/smoke), extend [`cell_to_rgba`](crates/falling_everything_core/src/render/mod.rs).

6. **Sandbox** — [`examples/sandbox_window.rs`](examples/sandbox_window.rs) builds the palette from `materials::BUILTINS` and maps number/letter keys by index; new builtins appear automatically.

7. **Verify** — Run `cargo test -p falling_everything_core` and `cargo build`.

## Example: inert solid + custom step (`PLANT`)

`PLANT` is `inert: true` (so liquids/solids do not displace it) but must still run each frame to convert adjacent water. That is implemented by handling `material::PLANT` in `step_pixel` **before** the `if props.inert { return; }` guard, calling a dedicated `step_plant` that only mutates neighbors.

Burning uses `ignitability`, `consumption_rate`, `neighbor_spawns` (e.g. fire sparks while smoldering), `fuel_mass` (smolder + `ON_FIRE`), and instant `FIRE` from heat; see `step_fire` / `step_smoldering_fuel` / `try_neighbor_spawns`. **Adjacent influence** — `MaterialProps::adjacent_influence` (up to eight `world::AdjacentInfluenceRule` entries) drives probabilistic neighbor flag/lifetime changes from a stepped cell: fire/lava/ember spread (`ON_FIRE` + fuel), water wetting sand (`cell_flags::WET`), optional transition spawns when flags change (e.g. custom rules), etc. Implemented in `sim/mod.rs` (`try_adjacent_influence_on_neighbor`, `spread_burn_to_neighbors`, `step_liquid`). Rules can set/clear flags, set neighbor lifetime, replace the neighbor when a flag transition matches (`if_cleared_mask` / `if_set_mask` + spawn material), place an extra material in a random **empty** 8-neighbor of the victim (`empty_neighbor_spawn` + lifetime range, e.g. steam when drying wet sand), and optionally clear or adjust the source cell (`InfluenceSourceEffect`). Corrosive liquids use `MaterialProps::corrosion_adjacent` (victim material, `chance_percent`, `neighbor_damage`, `self_lifetime_cost`); victims use `corrosion_max_hp` for multi-hit melt (`step_liquid` / `acid_corrode_neighbors`).

## Notes

- **`LIQUID`** is the main “water” ID. Other water-like liquids (`LIGHT_LIQUID`, `HEAVY_LIQUID`) are separate; growth or reactions that should affect only true water should match `material::LIQUID` explicitly unless you intentionally include them. True water removes adjacent `FIRE` / `EMBER` via `adjacent_transforms` (`to: EMPTY`), and wets sand / strips `ON_FIRE` from smoldering neighbors via `adjacent_influence`.
- Do not store Cursor skills under `~/.cursor/skills-cursor/` (reserved for Cursor internals).
