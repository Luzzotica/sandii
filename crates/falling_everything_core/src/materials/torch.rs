use super::MaterialDef;
use crate::world::{
    material, MaterialMotion, MaterialProps, MaterialRule, NeighborSpawnRule,
    MAX_NEIGHBOR_SPAWN_RULES,
};

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

pub const DEF: MaterialDef = MaterialDef {
    id: material::TORCH,
    name: "Torch",
    props: MaterialProps {
        density: 120,
        motion: MaterialMotion::InertSolid,
        neighbor_spawns: NS_TORCH_FIRE,
        base_temperature: 773,
        thermal_conductivity: 80,
        volumetric_heat_capacity: 100,
        durability: 55,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF8B4513,
};
