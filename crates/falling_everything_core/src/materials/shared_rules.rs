//! Rule tables referenced by more than one builtin [`super::MaterialDef`].
use crate::world::{
    cell_flags, material, AdjacentInfluenceRule, AdjacentTransformRule, InfluenceSourceEffect,
    InfluenceVictimLifetime, NeighborSpawnRule, MAX_ADJACENT_INFLUENCE_RULES,
    MAX_ADJACENT_TRANSFORM_RULES, MAX_NEIGHBOR_SPAWN_RULES,
};

/// Plant and wood: cardinal water may become plant (80% per neighbor per tick).
pub const ADJ_PLANT_GROWTH: [AdjacentTransformRule; MAX_ADJACENT_TRANSFORM_RULES] = [
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

/// Fire / lava: rare adjacent water → steam (gameplay exception); primary boil is `melt_temperature` on water cells.
pub const ADJ_HOT_VAPORIZE_WATER: [AdjacentTransformRule; MAX_ADJACENT_TRANSFORM_RULES] = [
    AdjacentTransformRule {
        from: material::LIQUID,
        to: material::STEAM,
        chance_percent: 1,
        cardinal_neighbors_only: false,
        actor_lifetime_delta: 0,
    },
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
    AdjacentTransformRule::inactive(),
];

/// Fire / lava / ember: dry wet sand in place + optional steam in a random empty neighbor; ignite smolder fuels.
pub const ADJ_HOT_INFLUENCE: [AdjacentInfluenceRule; MAX_ADJACENT_INFLUENCE_RULES] = [
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
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
];

/// Solid and molten wax: smolder may throw short-lived `FIRE` sparks.
pub const NS_FIRE_SMOLDER_WAX: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
    NeighborSpawnRule {
        spawn_material: material::FIRE,
        chance: 1,
        lifetime_lo: 2,
        lifetime_hi: 5,
        spawn_flags: 0,
    },
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
    NeighborSpawnRule::inactive(),
];

/// Lava smolder + wood: low-rate fire neighbor spawns.
pub const NS_FIRE_SMOLDER_1: [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES] = [
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
