use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::METAL,
    name: "Metal",
    props: MaterialProps {
        density: i16::MAX,
        motion: MaterialMotion::InertSolid,
        conductive: true,
        durability: 220,
        structure_integrity: 1.4,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 18,
        },
        corrosion_max_hp: 120,
        thermal_conductivity: 200,
        volumetric_heat_capacity: 900,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFFB0B8C0,
};
