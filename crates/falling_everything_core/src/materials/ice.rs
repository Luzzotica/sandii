use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::ICE,
    name: "Ice",
    props: MaterialProps {
        density: 100,
        motion: MaterialMotion::InertSolid,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 40,
        },
        base_temperature: 220,
        thermal_conductivity: 130,
        volumetric_heat_capacity: 210,
        melt_temperature: 274,
        melt_into: material::LIQUID,
        durability: 55,
        structure_integrity: 0.45,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF9EC5E8,
};
