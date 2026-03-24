use crate::world::{
    cell_flags, material, AcidCorrosionSource, AcidVulnerability, AdjacentInfluenceRule, AdjacentTransformRule,
    InfluenceSourceEffect, InfluenceVictimLifetime, MaterialId, MaterialMotion, MaterialProps, MaterialRule,
    NeighborSpawnRule, ADJ_ACTOR_WATER_QUENCH_LIFETIME, MAX_ADJACENT_INFLUENCE_RULES,
    MAX_ADJACENT_TRANSFORM_RULES, MAX_NEIGHBOR_SPAWN_RULES,
};

/// Plant and wood: cardinal water may become plant (80% per neighbor per tick).
const ADJ_PLANT_GROWTH: [AdjacentTransformRule; MAX_ADJACENT_TRANSFORM_RULES] = [
    AdjacentTransformRule {
        from: material::LIQUID,
        to: material::PLANT,
        chance_percent: 20,
        cardinal_neighbors_only: true,
        actor_lifetime_delta: 0,
    },
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
];

/// Fire / lava: adjacent water becomes steam and weakens the hot cell (lower chance = steam less often).
const ADJ_HOT_VAPORIZE_WATER: [AdjacentTransformRule; MAX_ADJACENT_TRANSFORM_RULES] = [
    AdjacentTransformRule {
        from: material::LIQUID,
        to: material::STEAM,
        chance_percent: 1,
        cardinal_neighbors_only: false,
        actor_lifetime_delta: ADJ_ACTOR_WATER_QUENCH_LIFETIME,
    },
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
];

/// Water: high chance to erase adjacent `FIRE` / `EMBER` (neighbor becomes empty).
const ADJ_LIQUID_EXTINGUISH: [AdjacentTransformRule; MAX_ADJACENT_TRANSFORM_RULES] = [
    AdjacentTransformRule {
        from: material::FIRE,
        to: material::EMPTY,
        chance_percent: 98,
        cardinal_neighbors_only: false,
        actor_lifetime_delta: 0,
    },
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
];

/// Fire / lava / ember: dry wet sand in place + optional steam in a random empty neighbor; ignite smolder fuels.
const ADJ_HOT_INFLUENCE: [AdjacentInfluenceRule; MAX_ADJACENT_INFLUENCE_RULES] = [
    AdjacentInfluenceRule {
        victim: material::SAND,
        chance_percent: 100,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: false,
        require_victim_flags_any: cell_flags::WET,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: cell_flags::WET,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::STEAM,
        empty_neighbor_spawn_lifetime_lo: 36,
        empty_neighbor_spawn_lifetime_hi: 72,
    },
    AdjacentInfluenceRule {
        victim: material::PLANT,
        chance_percent: 100,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: true,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::UseVictimFuelMass,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::WOOD,
        chance_percent: 100,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: true,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::UseVictimFuelMass,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::WAX,
        chance_percent: 100,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: true,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::UseVictimFuelMass,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::OIL,
        chance_percent: 100,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: true,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::UseVictimFuelMass,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
];

/// Water influence: strip `ON_FIRE` from smoldering neighbors, wet dry sand. (`FIRE`/`EMBER` use [`ADJ_LIQUID_EXTINGUISH`].)
const ADJ_WATER_INFLUENCE: [AdjacentInfluenceRule; MAX_ADJACENT_INFLUENCE_RULES] = [
    AdjacentInfluenceRule {
        victim: material::LAVA,
        chance_percent: 88,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: false,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::PLANT,
        chance_percent: 88,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: false,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::WOOD,
        chance_percent: 88,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: false,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::WAX,
        chance_percent: 88,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: false,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::OIL,
        chance_percent: 88,
        cardinal_neighbors_only: false,
        requires_victim_ignitability: false,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: 0,
        flags_or: 0,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::None,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule {
        victim: material::SAND,
        chance_percent: 45,
        cardinal_neighbors_only: true,
        requires_victim_ignitability: false,
        require_victim_flags_any: 0,
        exclude_victim_flags_any: cell_flags::WET,
        flags_or: cell_flags::WET,
        flags_clear: 0,
        victim_lifetime: InfluenceVictimLifetime::Unchanged,
        source_effect: InfluenceSourceEffect::ClearSourceCell,
        if_cleared_mask: 0,
        spawn_on_clear_material: material::EMPTY,
        if_set_mask: 0,
        spawn_on_set_material: material::EMPTY,
        spawn_lifetime_lo: 0,
        spawn_lifetime_hi: 0,
        empty_neighbor_spawn: material::EMPTY,
        empty_neighbor_spawn_lifetime_lo: 0,
        empty_neighbor_spawn_lifetime_hi: 0,
    },
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
];

const NS_FIRE_SMOLDER_22: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
    NeighborSpawnRule {
        spawn_material: material::FIRE,
        chance: 22,
        lifetime_lo: 14,
        lifetime_hi: 42,
        spawn_flags: 0,
    },
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
];

const NS_FIRE_SMOLDER_1: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
    NeighborSpawnRule {
        spawn_material: material::FIRE,
        chance: 1,
        lifetime_lo: 0,
        lifetime_hi: 15,
        spawn_flags: 0,
    },
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
];

const NS_EMBER_SPARKS: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
    NeighborSpawnRule {
        spawn_material: material::FIRE,
        chance: 42,
        lifetime_lo: 12,
        lifetime_hi: 38,
        spawn_flags: 0,
    },
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
];

const NS_TORCH_FIRE: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
    NeighborSpawnRule {
        spawn_material: material::FIRE,
        chance: 24,
        lifetime_lo: 18,
        lifetime_hi: 48,
        spawn_flags: 0,
    },
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
];

const NS_WELL_WATER: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
    NeighborSpawnRule {
        spawn_material: material::LIQUID,
        chance: 14,
        lifetime_lo: 0,
        lifetime_hi: 0,
        spawn_flags: 0,
    },
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
];

/// One-stop definition for a built-in material. To add a new material:
/// 1. Add an ID constant in `world::material`
/// 2. Add an entry to `BUILTINS` below -- that's it for the core crate. Use struct update syntax:
///    set the fields you need, then `..MaterialProps::default_const()` (same values as `Default`).
/// 3. Burning: `smolder_burnout_become` + `smolder_burnout_lifetime_*` drive both smolder **burnout** and
///    **instant** heat (`fuel_mass == 0`). Set `smolder_extinguish_*` when it smolders (`fuel_mass` > 0),
///    plus optional `smolder_burnout_ignites_neighbors` / explosion radius on burnout.
///    `FIRE` uses `on_death_become` + `on_death_lifetime_*` for smoke when its tick lifetime expires;
///    `LAVA` uses [`MaterialMotion::Liquid`] with high viscosity; painted `ON_FIRE` with `lifetime` = `fuel_mass`;
///    each tick runs smolder then `step_liquid` so it flows slowly but stays cohesive (`viscosity` 255).
/// 4. Optional neighbor reactions: `world::MaterialProps::adjacent_transforms` (up to four `from` → `to` rules at `chance_percent`, optional `actor_lifetime_delta` on the stepped cell; `to: EMPTY` clears the neighbor). Water uses this to erase adjacent `FIRE` / `EMBER` (`ADJ_LIQUID_EXTINGUISH`).
/// 5. Optional `neighbor_spawns`: up to four rules (material, `/256` chance, lifetime range) for smolder, ember, and inert props (torch, well, spout).
/// 6. Acid: `acid_corrosion` on the corrosive liquid; victims use `acid_vulnerability` and optional `corrosion_max_hp`. [`MaterialProps::default_const`] enables acid at 50% for new materials; builtins opt out only `EMPTY` and `ACID`, and tune sand/static/wax/plant/wood.
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

pub const BUILTINS: &[MaterialDef] = &[
    MaterialDef {
        id: material::EMPTY,
        name: "Empty",
        props: MaterialProps {
            density: i16::MIN,
            motion: MaterialMotion::Gas {
                viscosity: 0,
                max_speed: 0,
                acceleration: 0,
            },
            durability: 0,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 1, miscible: true },
        color_argb: 0xFF000000,
    },
    MaterialDef {
        id: material::SAND,
        name: "Sand",
        props: MaterialProps {
            density: 180,
            motion: MaterialMotion::SolidGranular {
                viscosity: 100,
                max_speed: 6,
                acceleration: 2,
                inertial_resistance: 80,
            },
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 71,
            },
            structure_integrity: 0.55,
            durability: 72,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 1, miscible: false },
        color_argb: 0xFFFFC369,
    },
    MaterialDef {
        id: material::LIQUID,
        name: "Water",
        props: MaterialProps {
            density: 100,
            motion: MaterialMotion::Liquid {
                viscosity: 5,
                max_speed: 4,
                acceleration: 1,
                extinguishes_fire: true,
            },
            base_temperature: 20,
            thermal_conductivity: 200,
            specific_heat: 255,
            melt_temperature: 373,
            melt_into: material::STEAM,
            adjacent_transforms: ADJ_LIQUID_EXTINGUISH,
            adjacent_influence: ADJ_WATER_INFLUENCE,
            durability: 58,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 4, miscible: true },
        color_argb: 0xFF3264D2,
    },
    MaterialDef {
        id: material::GAS,
        name: "Gas",
        props: MaterialProps {
            density: 10,
            motion: MaterialMotion::Gas {
                viscosity: 1,
                max_speed: 3,
                acceleration: 1,
            },
            durability: 0,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 4, miscible: true },
        color_argb: 0xFFBBBBBB,
    },
    MaterialDef {
        id: material::STATIC,
        name: "Static",
        props: MaterialProps {
            density: i16::MAX,
            motion: MaterialMotion::InertSolid,
            // Rigid bodies use STATIC as a sim-step placeholder; keep acid able to eat through
            // at a reasonable rate without making world walls trivial.
            corrosion_max_hp: 96,
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 28,
            },
            durability: 248,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF707070,
    },
    MaterialDef {
        id: material::RIGID,
        name: "Rigid",
        props: MaterialProps {
            density: 220,
            motion: MaterialMotion::SolidGranular {
                viscosity: 255,
                max_speed: 0,
                acceleration: 0,
                inertial_resistance: 255,
            },
            structure_integrity: 1.2,
            durability: 85,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 1, miscible: false },
        color_argb: 0xFF8B7355,
    },
    MaterialDef {
        id: material::OIL,
        name: "Oil",
        props: MaterialProps {
            density: 130,
            motion: MaterialMotion::Liquid {
                viscosity: 40,
                max_speed: 4,
                acceleration: 1,
                extinguishes_fire: false,
            },
            ignitability: 255,
            consumption_rate: 255,
            fuel_mass: 6,
            smolder_burnout_become: material::FIRE,
            smolder_burnout_lifetime_lo: 40,
            smolder_burnout_lifetime_hi: 160,
            ignition_temperature: 200,
            thermal_conductivity: 60,
            specific_heat: 100,
            heat_output: 50,
            durability: 42,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 3, miscible: true },
        color_argb: 0xFF3D2B1F,
    },
    MaterialDef {
        id: material::FIRE,
        name: "Fire",
        props: MaterialProps {
            density: 5,
            motion: MaterialMotion::Gas {
                viscosity: 0,
                max_speed: 2,
                acceleration: 1,
            },
            on_death_become: material::SMOKE,
            on_death_lifetime_lo: 10,
            on_death_lifetime_hi: 96,
            base_temperature: 800,
            thermal_conductivity: 220,
            specific_heat: 80,
            heat_output: 60,
            adjacent_transforms: ADJ_HOT_VAPORIZE_WATER,
            adjacent_influence: ADJ_HOT_INFLUENCE,
            durability: 0,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 2, miscible: true },
        color_argb: 0xFFFF6600,
    },
    MaterialDef {
        id: material::WAX,
        name: "Wax",
        props: MaterialProps {
            density: 160,
            motion: MaterialMotion::SolidGranular {
                viscosity: 220,
                max_speed: 5,
                acceleration: 2,
                inertial_resistance: 120,
            },
            ignitability: 48,
            consumption_rate: 9,
            neighbor_spawns: NS_FIRE_SMOLDER_22,
            fuel_mass: 220,
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 67,
            },
            structure_integrity: 0.45,
            ignition_temperature: 280,
            thermal_conductivity: 40,
            specific_heat: 160,
            heat_output: 30,
            durability: 52,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 1, miscible: false },
        color_argb: 0xFFF5E6CA,
    },
    MaterialDef {
        id: material::C4,
        name: "C4",
        props: MaterialProps {
            density: 200,
            motion: MaterialMotion::SolidGranular {
                viscosity: 255,
                max_speed: 5,
                acceleration: 2,
                inertial_resistance: 100,
            },
            ignitability: 255,
            explosion_radius: 20,
            ignition_temperature: 180,
            durability: 28,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 1, miscible: false },
        color_argb: 0xFFC8D8C0,
    },
    MaterialDef {
        id: material::SMOKE,
        name: "Smoke",
        props: MaterialProps {
            density: 3,
            motion: MaterialMotion::Gas {
                viscosity: 0,
                max_speed: 2,
                acceleration: 1,
            },
            durability: 0,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 3, miscible: true },
        color_argb: 0xAA888888,
    },
    MaterialDef {
        id: material::PLANT,
        name: "Plant",
        props: MaterialProps {
            density: 210,
            motion: MaterialMotion::InertSolid,
            ignitability: 200,
            consumption_rate: 230,
            fuel_mass: 3,
            smolder_extinguish_material: material::SMOKE,
            smolder_burnout_ignites_neighbors: true,
            smolder_burnout_become: material::FIRE,
            smolder_burnout_lifetime_lo: 22,
            smolder_burnout_lifetime_hi: 52,
            adjacent_transforms: ADJ_PLANT_GROWTH,
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 61,
            },
            ignition_temperature: 250,
            thermal_conductivity: 25,
            specific_heat: 150,
            heat_output: 35,
            durability: 48,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF2D8A3E,
    },
    MaterialDef {
        id: material::ACID,
        name: "Acid",
        props: MaterialProps {
            density: 115,
            motion: MaterialMotion::Liquid {
                viscosity: 28,
                max_speed: 3,
                acceleration: 1,
                extinguishes_fire: false,
            },
            acid_corrosion: AcidCorrosionSource {
                neighbor_damage: 200,
                self_lifetime_cost: 200,
            },
            acid_vulnerability: AcidVulnerability::inactive(),
            durability: 50,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 3, miscible: true },
        color_argb: 0xFF6BCC3A,
    },
    MaterialDef {
        id: material::EMBER,
        name: "Ember",
        props: MaterialProps {
            density: 225,
            motion: MaterialMotion::InertSolid,
            ignitability: 165,
            consumption_rate: 6,
            neighbor_spawns: NS_EMBER_SPARKS,
            adjacent_influence: ADJ_HOT_INFLUENCE,
            base_temperature: 600,
            thermal_conductivity: 100,
            specific_heat: 60,
            heat_output: 15,
            durability: 38,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF7A2E0A,
    },
    MaterialDef {
        id: material::WOOD,
        name: "Wood",
        props: MaterialProps {
            density: 210,
            motion: MaterialMotion::InertSolid,
            ignitability: 240,
            consumption_rate: 50,
            neighbor_spawns: NS_FIRE_SMOLDER_1,
            fuel_mass: 20,
            smolder_extinguish_material: material::EMBER,
            smolder_extinguish_lifetime_lo: 120,
            smolder_extinguish_lifetime_hi: 200,
            adjacent_transforms: ADJ_PLANT_GROWTH,
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 53,
            },
            ignition_temperature: 300,
            thermal_conductivity: 30,
            specific_heat: 180,
            heat_output: 25,
            durability: 75,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF5C3D2E,
    },
    MaterialDef {
        id: material::STEAM,
        name: "Steam",
        props: MaterialProps {
            density: 2,
            motion: MaterialMotion::Gas {
                viscosity: 0,
                max_speed: 3,
                acceleration: 1,
            },
            base_temperature: 120,
            thermal_conductivity: 30,
            specific_heat: 40,
            freeze_temperature: 99,
            freeze_into: material::LIQUID,
            durability: 0,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 3, miscible: true },
        color_argb: 0xCCDDEEFF,
    },
    MaterialDef {
        id: material::LAVA,
        name: "Lava",
        props: MaterialProps {
            density: 240,
            motion: MaterialMotion::Liquid {
                viscosity: 255,
                max_speed: 1,
                acceleration: 1,
                extinguishes_fire: false,
            },
            consumption_rate: 50,
            neighbor_spawns: NS_FIRE_SMOLDER_1,
            fuel_mass: 255,
            smolder_extinguish_material: material::SAND,
            smolder_extinguish_lifetime_lo: 0,
            smolder_extinguish_lifetime_hi: 1,
            smolder_burnout_become: material::SAND,
            smolder_burnout_lifetime_lo: 0,
            smolder_burnout_lifetime_hi: 1,
            adjacent_transforms: ADJ_HOT_VAPORIZE_WATER,
            adjacent_influence: ADJ_HOT_INFLUENCE,
            structure_integrity: 2.75,
            base_temperature: 1200,
            thermal_conductivity: 180,
            specific_heat: 200,
            heat_output: 30,
            freeze_temperature: 400,
            freeze_into: material::OBSIDIAN,
            durability: 95,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule {
            lateral_spread: 1,
            miscible: true,
        },
        color_argb: 0xFFFF3300,
    },
    MaterialDef {
        id: material::TORCH,
        name: "Torch",
        props: MaterialProps {
            density: 120,
            motion: MaterialMotion::InertSolid,
            neighbor_spawns: NS_TORCH_FIRE,
            base_temperature: 500,
            thermal_conductivity: 80,
            specific_heat: 100,
            durability: 55,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF8B4513,
    },
    MaterialDef {
        id: material::WELL,
        name: "Well",
        props: MaterialProps {
            density: 200,
            motion: MaterialMotion::InertSolid,
            neighbor_spawns: NS_WELL_WATER,
            durability: 70,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF4A6B8A,
    },
    MaterialDef {
        id: material::DIRT,
        name: "Dirt",
        props: MaterialProps {
            density: i16::MAX,
            motion: MaterialMotion::InertSolid,
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 55,
            },
            structure_integrity: 0.38,
            durability: 80,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF6B4226,
    },
    MaterialDef {
        id: material::GRASS,
        name: "Grass",
        props: MaterialProps {
            density: i16::MAX,
            motion: MaterialMotion::InertSolid,
            durability: 38,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF3A7D32,
    },
    MaterialDef {
        id: material::OBSIDIAN,
        name: "Obsidian",
        props: MaterialProps {
            density: 260,
            motion: MaterialMotion::InertSolid,
            thermal_conductivity: 80,
            specific_heat: 200,
            melt_temperature: 1200,
            melt_into: material::LAVA,
            adjacent_transforms: ADJ_HOT_VAPORIZE_WATER,
            durability: 250,
            ..MaterialProps::default_const()
        },
        rule: MaterialRule { lateral_spread: 0, miscible: false },
        color_argb: 0xFF1A1A2E,
    },
];

pub fn builtin_props() -> [MaterialProps; material::MAX_MATERIALS] {
    let mut props = [MaterialProps::default(); material::MAX_MATERIALS];
    for def in BUILTINS {
        props[def.id as usize] = def.props;
    }
    props
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
