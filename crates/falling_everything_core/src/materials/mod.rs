//! Built-in [`MaterialDef`] entries — one submodule per material ID.
//!
//! ## Phases and temperatures (Kelvin)
//!
//! | Material | Engine phase @ 293 K | Notes |
//! |----------|------------------------|-------|
//! | EMPTY, STONE | — | — |
//! | SAND, DIRT, GRASS, C4, OBSIDIAN, PLANT, WOOD | Solid | Room ~293 K |
//! | RIGID | Solid | Engine-only placeholder for dynamic bodies; not in [`paintable_builtin_count`] |
//! | WAX | Inert solid | Melts ≥334 K → melted wax |
//! | MELTED_WAX | Liquid | Freezes ≤333 K → wax (solid melts ≥334 K) |
//! | ICE | Inert solid | Melt ≥274 K → water; sub-freezing |
//! | LIQUID (water) | Liquid | Freeze ≤273 K → ice; boil ≥373 K → steam |
//! | OIL, ACID, LIGHT/HEAVY liquid (defaults) | Liquid | Room |
//! | LAVA | Liquid | ~1473 K; freeze ≤673 K → obsidian |
//! | GAS, SMOKE | Gas | Room |
//! | STEAM | Gas | Condense ≤372 K → water |
//! | FIRE, EMBER | Gas | Hot combustion |
//! | TORCH, WELL, SPOUT | Inert solid | Props vary |

use crate::world::{material, MaterialId, MaterialProps, MaterialRule};

/// One-stop definition for a built-in material. To add a new material:
/// 1. Add an ID constant in `world::material`
/// 2. Add `materials/<name>.rs` with `pub const DEF` and list it in [`BUILTINS`] below — that's it for the core crate. Use struct update syntax:
///    set the fields you need, then `..MaterialProps::default_const()` (same values as `Default`).
/// 3. Burning: fuels use [`cell_flags::ON_FIRE`] once ignited (temperature crosses [`MaterialProps::autoignition_temperature`]
///    or neighbor ignition). `smolder_burnout_become` + `smolder_burnout_lifetime_*` drive both fuel **burnout** and
///    **instant** heat (`fuel_mass == 0`). Set `smolder_extinguish_*` when it burns (`fuel_mass` > 0),
///    plus optional `smolder_burnout_ignites_neighbors` / explosion radius on burnout.
///    `FIRE` uses `on_death_become` + `on_death_lifetime_*` for smoke when its tick lifetime expires;
///    `LAVA` uses [`MaterialMotion::Liquid`] with high viscosity; painted `ON_FIRE` with `lifetime` = `fuel_mass`;
///    each tick runs smolder then `step_liquid` so it flows slowly but stays cohesive (`viscosity` 255).
/// 4. Optional neighbor reactions: `world::MaterialProps::adjacent_transforms` (up to four `from` → `to` rules at `chance_percent`, optional `actor_lifetime_delta` on the stepped cell; `to: EMPTY` clears the neighbor). Water uses this to erase adjacent `FIRE` / `EMBER` (`ADJ_LIQUID_EXTINGUISH` in `liquid.rs`).
/// 5. Optional `neighbor_spawns`: up to four rules (material, `/256` chance, lifetime range) for smolder, ember, and inert props (torch, well, spout).
/// 6. Acid: `acid_corrosion` on the corrosive liquid; victims use `acid_vulnerability` and optional `corrosion_max_hp`. [`MaterialProps::default_const`] enables acid at 50% for new materials; builtins opt out only `EMPTY` and `ACID`, and tune sand/stone/wax/plant/wood.
/// 7. `adjacent_influence` on fire/lava/ember/water: neighbor flags, lifetime, optional transition spawns (see `world::AdjacentInfluenceRule`).
/// 8. Optionally add a keyboard shortcut in the sandbox example.
/// 9. Rigid bodies: `structure_integrity` (see [`MaterialProps::structure_integrity`]) — low for crumbly
///    materials (e.g. dirt), high for tough ones (e.g. lava); scales collision damage vs inert terrain.
/// 10. Explosions: `durability` — ray blast absorption; `0` = no strength loss through that cell. Solids/liquids
///     without `fuel_mass` also seed structural [`Cell::lifetime`] from durability when painted (see [`World::initial_lifetime_for`]).
///     Custom debris: [`crate::explosion::ExplosionParams`] + [`crate::explosion::ExplosionSpawn`] (`fill_on_destroy`, `edge_on_destroy`, `obliterate_disk`).
#[derive(Debug, Clone, Copy)]
pub struct MaterialDef {
    pub id: MaterialId,
    pub name: &'static str,
    pub props: MaterialProps,
    pub rule: MaterialRule,
    pub color_argb: u32,
}

mod acid;
mod c4;
mod dirt;
mod ember;
mod empty;
mod fire;
mod gas;
mod grass;
mod ice;
mod lava;
mod liquid;
mod melted_wax;
mod obsidian;
mod oil;
mod plant;
mod rigid;
mod sand;
mod smoke;
mod stone;
mod steam;
mod torch;
mod wax;
mod well;
mod wood;

pub mod shared_rules;

pub const BUILTINS: &[MaterialDef] = &[
    empty::DEF,
    sand::DEF,
    liquid::DEF,
    gas::DEF,
    stone::DEF,
    rigid::DEF,
    oil::DEF,
    fire::DEF,
    wax::DEF,
    melted_wax::DEF,
    c4::DEF,
    smoke::DEF,
    plant::DEF,
    acid::DEF,
    ember::DEF,
    wood::DEF,
    steam::DEF,
    lava::DEF,
    torch::DEF,
    well::DEF,
    dirt::DEF,
    grass::DEF,
    obsidian::DEF,
    ice::DEF,
];

/// Materials offered in player brush palettes. Excludes [`material::RIGID`], which is only used as an
/// engine placeholder when spawning dynamic rigid bodies (circles, shortcuts), not as painted terrain.
#[inline]
pub fn paintable_builtin_count() -> usize {
    BUILTINS.iter().filter(|d| d.id != material::RIGID).count()
}

#[inline]
pub fn paintable_builtin_at(index: usize) -> Option<&'static MaterialDef> {
    BUILTINS.iter().filter(|d| d.id != material::RIGID).nth(index)
}

pub fn builtin_props() -> [MaterialProps; material::MAX_MATERIALS] {
    // Heap: `[MaterialProps; MAX_MATERIALS]` on the stack overflows typical wasm stacks (~1 MiB).
    let mut props = vec![MaterialProps::default(); material::MAX_MATERIALS];
    for def in BUILTINS {
        props[def.id as usize] = def.props;
    }
    <[MaterialProps; material::MAX_MATERIALS]>::try_from(props).unwrap_or_else(|v| {
        panic!(
            "builtin_props: expected {} elements, got {}",
            material::MAX_MATERIALS,
            v.len()
        )
    })
}

pub fn builtin_rules() -> [MaterialRule; material::MAX_MATERIALS] {
    let mut rules = [MaterialRule::default(); material::MAX_MATERIALS];
    for def in BUILTINS {
        rules[def.id as usize] = def.rule;
    }
    rules
}

pub fn builtin_palette_argb() -> [u32; material::MAX_MATERIALS] {
    let mut palette = [0xFF202020u32; material::MAX_MATERIALS];
    for def in BUILTINS {
        palette[def.id as usize] = def.color_argb;
    }
    palette
}

pub fn builtin_palette_rgba() -> [[u8; 4]; material::MAX_MATERIALS] {
    let mut palette = [[255u8, 0, 255, 255]; material::MAX_MATERIALS];
    for def in BUILTINS {
        let a = def.color_argb;
        palette[def.id as usize] = [
            ((a >> 16) & 0xFF) as u8,
            ((a >> 8) & 0xFF) as u8,
            (a & 0xFF) as u8,
            ((a >> 24) & 0xFF) as u8,
        ];
    }
    palette
}
