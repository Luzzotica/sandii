use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::DIRT,
    name: "Dirt",
    props: MaterialProps {
        density: i16::MAX,
        motion: MaterialMotion::InertSolid,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 55,
        },
        structure_integrity: 0.38,
        durability: 80,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF6B4226,
};
