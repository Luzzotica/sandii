use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::SAND,
    name: "Sand",
    props: MaterialProps {
        density: 180,
        motion: MaterialMotion::SolidGranular {
            viscosity: 100,
            max_speed: 4,
            acceleration: 2,
            inertial_resistance: 80,
        },
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 71,
        },
        structure_integrity: 0.55,
        durability: 72,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 1,
        miscible: false,
    },
    color_argb: 0xFFFFC369,
};
