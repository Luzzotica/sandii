use super::MaterialDef;
use crate::world::{
    cell_flags, material, AcidVulnerability, AdjacentInfluenceRule, AdjacentTransformRule,
    InfluenceSourceEffect, InfluenceVictimLifetime, MaterialMotion, MaterialProps, MaterialRule,
    MAX_ADJACENT_INFLUENCE_RULES, MAX_ADJACENT_TRANSFORM_RULES,
};

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

/// Wet dry sand only; smolder quenching is thermal (conduction), not influence rolls on wood/lava.
const ADJ_WATER_INFLUENCE: [AdjacentInfluenceRule; MAX_ADJACENT_INFLUENCE_RULES] = [
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
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
    AdjacentInfluenceRule::inactive(),
];

pub const DEF: MaterialDef = MaterialDef {
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
        base_temperature: 293,
        thermal_conductivity: 255,
        // High capacity but not extreme — pairs with LIQUID_EDGE_THERMAL_BONUS in thermal.rs so water
        // still pulls heat from lava aggressively without the old u8 cap.
        volumetric_heat_capacity: 2200,
        freeze_temperature: 273,
        freeze_into: material::ICE,
        melt_temperature: 373,
        melt_into: material::STEAM,
        adjacent_transforms: ADJ_LIQUID_EXTINGUISH,
        adjacent_influence: ADJ_WATER_INFLUENCE,
        durability: 58,
        // Water is inert to acid (no neutralization sim yet).
        acid_vulnerability: AcidVulnerability::inactive(),
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 4,
        miscible: true,
    },
    color_argb: 0xFF3264D2,
};
