use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

use super::shared_rules::ADJ_PLANT_GROWTH;

pub const DEF: MaterialDef = MaterialDef {
    id: material::PLANT,
    name: "Plant",
    props: MaterialProps {
        density: 210,
        motion: MaterialMotion::InertSolid,
        ignitability: 200,
        consumption_rate: 230,
        fuel_mass: 3,
        smolder_extinguish_material: material::SMOKE,
        smolder_burnout_ignites_neighbors: true,
        smolder_burnout_become: material::FIRE,
        smolder_burnout_lifetime_lo: 6,
        smolder_burnout_lifetime_hi: 14,
        adjacent_transforms: ADJ_PLANT_GROWTH,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 61,
        },
        autoignition_temperature: 523,
        thermal_conductivity: 25,
        volumetric_heat_capacity: 150,
        heat_generation_rate: 21,
        durability: 48,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF2D8A3E,
};
