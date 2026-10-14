use super::MaterialDef;
use crate::world::{material, AcidVulnerability, MaterialMotion, MaterialProps, MaterialRule};

/// Short-lived charge: spreads into adjacent conductive cells (metal / water), then dies.
pub const DEF: MaterialDef = MaterialDef {
    id: material::ELECTRICITY,
    name: "Electricity",
    props: MaterialProps {
        density: 1,
        motion: MaterialMotion::Gas {
            viscosity: 1,
            max_speed: 6,
            acceleration: 0,
        },
        conductive: true,
        base_temperature: 293,
        thermal_conductivity: 0,
        volumetric_heat_capacity: 1,
        // Lifetime seeds from on_death range when painted via initial_lifetime — use fuel_mass as HP ticks.
        fuel_mass: 0,
        on_death_become: material::EMPTY,
        on_death_lifetime_lo: 1,
        on_death_lifetime_hi: 2,
        acid_vulnerability: AcidVulnerability::inactive(),
        durability: 0,
        ..MaterialProps::default_const()
    },
    rule: MaterialRule {
        lateral_spread: 4,
        miscible: false,
    },
    color_argb: 0xFF7DF9FF,
};
