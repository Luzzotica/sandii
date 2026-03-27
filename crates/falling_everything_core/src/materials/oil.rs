use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

pub const DEF: MaterialDef = MaterialDef {
    id: material::OIL,
    name: "Oil",
    props: MaterialProps {
        density: 130,
        motion: MaterialMotion::Liquid {
            viscosity: 40,
            max_speed: 4,
            acceleration: 1,
            extinguishes_fire: false,
        },
        ignitability: 255,
        consumption_rate: 255,
        fuel_mass: 6,
        smolder_burnout_become: material::FIRE,
        smolder_burnout_lifetime_lo: 40,
        smolder_burnout_lifetime_hi: 160,
        autoignition_temperature: 473,
        thermal_conductivity: 60,
        volumetric_heat_capacity: 100,
        heat_generation_rate: 9,
        durability: 42,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 3,
        miscible: true,
    },
    color_argb: 0xFF3D2B1F,
};
