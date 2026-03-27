use super::MaterialDef;
use crate::world::{
    material, MaterialMotion, MaterialProps, MaterialRule, NeighborSpawnRule,
    MAX_NEIGHBOR_SPAWN_RULES,
};

use super::shared_rules::ADJ_HOT_INFLUENCE;

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

pub const DEF: MaterialDef = MaterialDef {
    id: material::EMBER,
    name: "Ember",
    props: MaterialProps {
        density: 225,
        motion: MaterialMotion::InertSolid,
        ignitability: 165,
        fuel_mass: 1,
        consumption_rate: 6,
        neighbor_spawns: NS_EMBER_SPARKS,
        adjacent_influence: ADJ_HOT_INFLUENCE,
        base_temperature: 900,
        autoignition_temperature: 520,
        thermal_conductivity: 100,
        volumetric_heat_capacity: 60,
        heat_generation_rate: 4,
        durability: 38,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF7A2E0A,
};
