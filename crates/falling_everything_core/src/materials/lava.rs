use super::MaterialDef;
use crate::world::{material, MaterialMotion, MaterialProps, MaterialRule};

use super::shared_rules::{ADJ_HOT_INFLUENCE, ADJ_HOT_VAPORIZE_WATER, NS_FIRE_SMOLDER_1};

pub const DEF: MaterialDef = MaterialDef {
    id: material::LAVA,
    name: "Lava",
    props: MaterialProps {
        density: 240,
        motion: MaterialMotion::Liquid {
            viscosity: 255,
            max_speed: 1,
            acceleration: 1,
            extinguishes_fire: false,
        },
        consumption_rate: 50,
        neighbor_spawns: NS_FIRE_SMOLDER_1,
        fuel_mass: 255,
        smolder_extinguish_material: material::SAND,
        smolder_extinguish_lifetime_lo: 0,
        smolder_extinguish_lifetime_hi: 1,
        smolder_burnout_become: material::SAND,
        smolder_burnout_lifetime_lo: 0,
        smolder_burnout_lifetime_hi: 1,
        adjacent_transforms: ADJ_HOT_VAPORIZE_WATER,
        adjacent_influence: ADJ_HOT_INFLUENCE,
        structure_integrity: 2.75,
        base_temperature: 1473,
        thermal_conductivity: 120,
        volumetric_heat_capacity: 2500,
        heat_generation_rate: 24,
        // Below 673 K lava stays molten longer before crossing into solid obsidian.
        freeze_temperature: 548,
        freeze_into: material::OBSIDIAN,
        durability: 95,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 1,
        miscible: true,
    },
    color_argb: 0xFFFF3300,
};
