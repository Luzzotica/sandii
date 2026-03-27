use super::shared_rules::NS_FIRE_SMOLDER_WAX;
use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::MELTED_WAX,
    name: "Melted wax",
    props: MaterialProps {
        density: 155,
        motion: MaterialMotion::Liquid {
            viscosity: 180,
            max_speed: 1,
            acceleration: 1,
            extinguishes_fire: false,
        },
        ignitability: 48,
        consumption_rate: 30,
        neighbor_spawns: NS_FIRE_SMOLDER_WAX,
        fuel_mass: 220,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 67,
        },
        structure_integrity: 0.35,
        autoignition_temperature: 310,
        thermal_conductivity: 38,
        volumetric_heat_capacity: 155,
        heat_generation_rate: 1,
        durability: 52,
        base_temperature: 336,
        freeze_temperature: 300,
        freeze_into: material::WAX,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 2,
        miscible: false,
    },
    color_argb: 0xFFE2C9A4,
};
