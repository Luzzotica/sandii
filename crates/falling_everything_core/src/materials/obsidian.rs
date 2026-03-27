use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::OBSIDIAN,
    name: "Obsidian",
    props: MaterialProps {
        density: 260,
        motion: MaterialMotion::InertSolid,
        thermal_conductivity: 80,
        volumetric_heat_capacity: 200,
        melt_temperature: 1273,
        melt_into: material::LAVA,
        durability: 250,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF1A1A2E,
};
