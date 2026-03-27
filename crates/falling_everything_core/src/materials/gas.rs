use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::GAS,
    name: "Gas",
    props: MaterialProps {
        density: 10,
        motion: MaterialMotion::Gas {
            viscosity: 1,
            max_speed: 3,
            acceleration: 1,
        },
        durability: 0,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 4,
        miscible: true,
    },
    color_argb: 0xFFBBBBBB,
};
