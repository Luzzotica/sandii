use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
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
    rule: MaterialRule {
        lateral_spread: 3,
        miscible: true,
    },
    color_argb: 0xAA888888,
};
