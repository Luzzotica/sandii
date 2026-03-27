use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
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
    rule: MaterialRule {
        lateral_spread: 1,
        miscible: false,
    },
    color_argb: 0xFF8B7355,
};
