use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::STEAM,
    name: "Steam",
    props: MaterialProps {
        density: 2,
        motion: MaterialMotion::Gas {
            viscosity: 0,
            max_speed: 3,
            acceleration: 1,
        },
        base_temperature: 393,
        thermal_conductivity: 30,
        volumetric_heat_capacity: 40,
        freeze_temperature: 372,
        freeze_into: material::LIQUID,
        durability: 0,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 3,
        miscible: true,
    },
    color_argb: 0xCCDDEEFF,
};
