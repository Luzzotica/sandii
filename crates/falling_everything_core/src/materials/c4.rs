use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::C4,
    name: "C4",
    props: MaterialProps {
        density: i16::MAX,
        motion: MaterialMotion::InertSolid,
        ignitability: 255,
        explosion_radius: 40,
        autoignition_temperature: 453,
        durability: 28,
        corrosion_max_hp: 48,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 28,
        },
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFFC8D8C0,
};
