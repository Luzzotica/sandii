use super::shared_rules::NS_FIRE_SMOLDER_WAX;
use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::WAX,
    name: "Wax",
    props: MaterialProps {
        density: 160,
        motion: MaterialMotion::InertSolid,
        ignitability: 48,
        consumption_rate: 30,
        neighbor_spawns: NS_FIRE_SMOLDER_WAX,
        fuel_mass: 220,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 67,
        },
        structure_integrity: 0.45,
        autoignition_temperature: 310,
        thermal_conductivity: 10,
        volumetric_heat_capacity: 160,
        heat_generation_rate: 1,
        durability: 52,
        melt_temperature: 320,
        melt_into: material::MELTED_WAX,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFFF5E6CA,
};
