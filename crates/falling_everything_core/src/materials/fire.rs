use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

use super::shared_rules::ADJ_HOT_INFLUENCE;

pub const DEF: MaterialDef = MaterialDef {
    id: material::FIRE,
    name: "Fire",
    props: MaterialProps {
        density: 1,
        motion: MaterialMotion::Gas {
            viscosity: 0,
            max_speed: 2,
            acceleration: 1,
        },
        on_death_become: material::SMOKE,
        on_death_lifetime_lo: 10,
        on_death_lifetime_hi: 96,
        base_temperature: 1200,
        thermal_conductivity: 220,
        volumetric_heat_capacity: 80,
        heat_generation_rate: 19,
        adjacent_influence: ADJ_HOT_INFLUENCE,
        durability: 0,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 2,
        miscible: true,
    },
    color_argb: 0xFFFF6600,
};
