use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

use super::shared_rules::{ADJ_PLANT_GROWTH, NS_FIRE_SMOLDER_1};

pub const DEF: MaterialDef = MaterialDef {
    id: material::WOOD,
    name: "Wood",
    props: MaterialProps {
        density: 210,
        motion: MaterialMotion::InertSolid,
        ignitability: 240,
        consumption_rate: 50,
        neighbor_spawns: NS_FIRE_SMOLDER_1,
        fuel_mass: 30,
        smolder_burnout_ignites_neighbors: true,
        smolder_extinguish_material: material::EMBER,
        smolder_extinguish_lifetime_lo: 120,
        smolder_extinguish_lifetime_hi: 200,
        adjacent_transforms: ADJ_PLANT_GROWTH,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 53,
        },
        autoignition_temperature: 490,
        thermal_conductivity: 62,
        volumetric_heat_capacity: 520,
        heat_generation_rate: 25,
        durability: 75,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF5C3D2E,
};
