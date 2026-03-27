use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::STONE,
    name: "Stone",
    props: MaterialProps {
        density: i16::MAX,
        motion: MaterialMotion::InertSolid,
        // Rigid bodies use STONE as a sim-step placeholder; keep acid able to eat through
        // at a reasonable rate without making world walls trivial.
        corrosion_max_hp: 96,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 28,
        },
        durability: 248,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF707070,
};
