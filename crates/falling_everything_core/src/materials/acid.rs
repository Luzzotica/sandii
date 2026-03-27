use super::MaterialDef;
use crate::world::{
    material, AcidCorrosionSource, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule,
};

pub const DEF: MaterialDef = MaterialDef {
    id: material::ACID,
    name: "Acid",
    props: MaterialProps {
        density: 115,
        motion: MaterialMotion::Liquid {
            viscosity: 28,
            max_speed: 3,
            acceleration: 1,
            extinguishes_fire: false,
        },
        acid_corrosion: AcidCorrosionSource {
            neighbor_damage: 200,
            self_lifetime_cost: 200,
        },
        acid_vulnerability: AcidVulnerability::inactive(),
        durability: 50,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 3,
        miscible: true,
    },
    color_argb: 0xFF6BCC3A,
};
