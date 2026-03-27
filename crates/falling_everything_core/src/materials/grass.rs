use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::GRASS,
    name: "Grass",
    props: MaterialProps {
        density: i16::MAX,
        motion: MaterialMotion::InertSolid,
        ignitability: 210,
        consumption_rate: 200,
        fuel_mass: 5,
        smolder_extinguish_material: material::SMOKE,
        smolder_extinguish_lifetime_lo: 12,
        smolder_extinguish_lifetime_hi: 48,
        smolder_burnout_ignites_neighbors: true,
        acid_vulnerability: AcidVulnerability {
            affected: true,
            chance_percent: 55,
        },
        autoignition_temperature: 500,
        thermal_conductivity: 30,
        volumetric_heat_capacity: 180,
        heat_generation_rate: 20,
        durability: 24,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 0,
        miscible: false,
    },
    color_argb: 0xFF3A7D32,
};
