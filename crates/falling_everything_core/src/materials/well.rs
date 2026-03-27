use super::MaterialDef;
use crate::world::{
    material, MaterialMotion, MaterialProps, MaterialRule, NeighborSpawnRule,
    MAX_NEIGHBOR_SPAWN_RULES,
};

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

pub const DEF: MaterialDef = MaterialDef {
    id: material::WELL,
    name: "Well",
    props: MaterialProps {
        density: 200,
        motion: MaterialMotion::InertSolid,
        neighbor_spawns: NS_WELL_WATER,
        durability: 70,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF4A6B8A,
};
