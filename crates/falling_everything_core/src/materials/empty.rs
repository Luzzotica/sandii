use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
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
        thermal_conductivity: 52,
        volumetric_heat_capacity: 95,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 1,
        miscible: true,
    },
    color_argb: 0xFF000000,
};
