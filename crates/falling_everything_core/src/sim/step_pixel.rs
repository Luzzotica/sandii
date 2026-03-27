use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{cell_flags, material, Cell, Phase, Vec2i, World};

use crate::cell64::MAX_TEMPERATURE;
use crate::sim::engines::burn::{
    apply_ignition_neighbor_pulse, eliminate_smoldering_fuel_at, try_neighbor_spawns,
};
use crate::sim::engines::{
    adjacent::step_adjacent_transform_neighbors, gas::step_gas, granular::step_sand,
    liquid::step_liquid, smolder::step_smoldering_fuel,
};
use crate::sim::steps::{ember::step_ember, fire::step_fire, smoke::step_smoke, steam::step_steam};
use crate::sim::{check_phase_transition, is_burning, step_temperature, FlameSpawn, SimGrids};

/// Heat from chemistry: `ΔT ≈ heat_generation_rate * 256 / volumetric_heat_capacity` when a chemistry step runs (abstract units).
const HEAT_GEN_SCALE: u32 = 256;
/// Apply burning chemistry heat every N sim ticks; conduction still runs every tick via [`step_temperature`].
const CHEMISTRY_HEAT_PERIOD: u64 = 5;

/// Base range for `1/N` flame spawn probability; actual N is randomized so burning fuel is not a particle hose.
const FLAME_PARTICLE_ODDS_LO: u32 = 48;
const FLAME_PARTICLE_ODDS_HI: u32 = 140;

fn maybe_spawn_flame_particle(
    sg: &SimGrids,
    p: Vec2i,
    rng: &mut SmallRng,
    flame_spawns: &mut Vec<FlameSpawn>,
) {
    let cell = sg.get(p);
    if cell.material() == material::FIRE || !cell.has_flag(cell_flags::ON_FIRE) {
        return;
    }
    let n = rng.gen_range(FLAME_PARTICLE_ODDS_LO..=FLAME_PARTICLE_ODDS_HI);
    if !rng.gen_ratio(1, n) {
        return;
    }
    const NEI: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
    let start = rng.gen_range(0..8);
    for i in 0..8 {
        let (dx, dy) = NEI[(start + i) % 8];
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if sg.get(np).material() == material::EMPTY {
            flame_spawns.push(FlameSpawn {
                pos: np,
                temperature_k: cell.temperature(),
            });
            return;
        }
    }
}

pub(crate) fn step_pixel(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
    flame_spawns: &mut Vec<FlameSpawn>,
    sim_tick: u64,
) {
    if sg.was_moved(p) {
        return;
    }
    let cell = sg.get(p);
    if cell.material() == material::EMPTY {
        step_temperature(sg, world, p, rng, sim_tick);
        return;
    }

    if cell.material() == material::FIRE && cell.has_flag(cell_flags::ON_FIRE) {
        sg.clear_flag(p, cell_flags::ON_FIRE);
    }

    step_temperature(sg, world, p, rng, sim_tick);
    {
        let c = sg.get(p);
        let pr = world.material_props(c.material());
        // Thermal detonation (e.g. C4 heated by adjacent lava): no `fuel_mass` / fire-spread path.
        if pr.explosion_radius > 0
            && pr.autoignition_temperature > 0
            && c.temperature() >= pr.autoignition_temperature
        {
            explosions.push((p, pr.explosion_radius as i32));
            sg.set_cell(p, Cell::new());
            return;
        }
    }
    {
        let c = sg.get(p);
        let pr = world.material_props(c.material());
        // Painted fuels use `lifetime` 0 from [`World::initial_lifetime_for`] even when `fuel_mass` > 0;
        // seed HP from `fuel_mass` the first time temperature crosses autoignition.
        if pr.fuel_mass > 0
            && pr.autoignition_temperature > 0
            && c.temperature() >= pr.autoignition_temperature
            && !c.has_flag(cell_flags::ON_FIRE)
        {
            if c.lifetime() == 0 {
                sg.set_lifetime(p, pr.fuel_mass);
            }
            sg.set_flag(p, cell_flags::ON_FIRE);
            apply_ignition_neighbor_pulse(sg, world, p, &pr);
        }
    }
    {
        let c = sg.get(p);
        let pr = world.material_props(c.material());
        let chemistry = if pr.fuel_mass == 0 {
            true
        } else if pr.autoignition_temperature == 0 {
            c.lifetime() > 0
        } else {
            is_burning(c, &pr)
        };
        if pr.heat_generation_rate > 0
            && c.lifetime() > 0
            && chemistry
            && sim_tick % CHEMISTRY_HEAT_PERIOD == 0
        {
            let vhc = pr.volumetric_heat_capacity as u32;
            let add = (pr.heat_generation_rate as u32 * HEAT_GEN_SCALE / vhc.max(1))
                .min(MAX_TEMPERATURE as u32) as u32;
            let new_temp = (c.temperature() as u32 + add).min(MAX_TEMPERATURE as u32) as u16;
            sg.set_temperature(p, new_temp);
        }
    }
    maybe_spawn_flame_particle(sg, p, rng, flame_spawns);
    check_phase_transition(sg, world, p);
    let cell = sg.get(p);
    if cell.material() == material::EMPTY {
        return;
    }
    let props_quench = world.material_props(cell.material());
    if props_quench.fuel_mass > 0
        && props_quench.autoignition_temperature > 0
        && cell.lifetime() > 0
        && cell.temperature() < props_quench.autoignition_temperature
    {
        eliminate_smoldering_fuel_at(sg, world, p, &props_quench, &cell, rng, explosions, false);
        return;
    }

    if cell.material() == material::EMBER {
        step_ember(sg, world, p, rng, explosions);
        return;
    }
    let props = world.material_props(cell.material());
    let lava_adj_early = cell.material() == material::LAVA && props.has_adjacent_transforms();
    if lava_adj_early {
        step_adjacent_transform_neighbors(sg, world, p, &props, rng);
    }
    if is_burning(cell, &props) && cell.material() != material::FIRE {
        step_smoldering_fuel(sg, world, p, rng, explosions);
        // Molten fuels must still run liquid physics after smolder (lava was the only case before).
        let after = sg.get(p);
        if after.material() != material::EMPTY {
            let after_props = world.material_props(after.material());
            if after_props.phase() == Phase::Liquid {
                step_liquid(sg, world, p, rng);
            }
        }
        return;
    }
    if props.has_adjacent_transforms() && !lava_adj_early {
        step_adjacent_transform_neighbors(sg, world, p, &props, rng);
    }
    if props.has_neighbor_spawns() {
        let burning = is_burning(cell, &props);
        let cold_spawner = matches!(
            cell.material(),
            material::TORCH | material::WELL | material::SPOUT
        );
        if burning || cold_spawner {
            try_neighbor_spawns(sg, world, p, &props, rng);
        }
    }
    if props.inert() {
        return;
    }
    let cell = sg.get(p);
    if cell.has_flag(cell_flags::RIGID_PIXEL) {
        return;
    }
    match props.phase() {
        Phase::Solid => step_sand(sg, world, p, rng),
        Phase::Liquid => step_liquid(sg, world, p, rng),
        Phase::Gas => {
            if cell.material() == material::SMOKE {
                step_smoke(sg, world, p, rng);
            } else if cell.material() == material::STEAM {
                step_steam(sg, world, p, rng);
            } else if props.on_death_become != material::EMPTY {
                step_fire(sg, world, p, rng, explosions);
            } else if cell.lifetime() > 0 {
                step_smoke(sg, world, p, rng);
            } else {
                step_gas(sg, world, p, rng);
            }
        }
    }
}
