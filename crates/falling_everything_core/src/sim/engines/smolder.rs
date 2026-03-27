use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{material, Phase, Vec2i, World};

use crate::sim::{is_burning, SimGrids};

use super::burn::{eliminate_smoldering_fuel_at, spread_burn_to_neighbors, try_neighbor_spawns};

/// Burning fuel (temperature >= autoignition_temperature, non-`FIRE` material). Keeps the chunk
/// awake every tick so low `consumption_rate` materials (e.g. lava) are not skipped after chunk sleep.
pub(crate) fn step_smoldering_fuel(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    sg.wake_at(p);
    let cell = sg.get(p);
    let props = world.material_props(cell.material());

    let life = cell.lifetime();
    let rate = props.consumption_rate.max(1) as u32;
    if rng.gen_ratio(rate, 256) {
        if life <= 1 {
            eliminate_smoldering_fuel_at(sg, world, p, &props, &cell, rng, explosions, true);
            return;
        }
        sg.set_lifetime(p, life - 1);
    }

    try_neighbor_spawns(sg, world, p, &props, rng);

    let c = sg.get(p);
    let molten_lava = c.material() == material::LAVA && c.lifetime() > 0 && is_burning(c, &props);

    if molten_lava {
        let life = c.lifetime();
        let heat_factor = life as f32 / 255.0;
        let _ = spread_burn_to_neighbors(
            sg,
            world,
            p,
            material::LAVA,
            heat_factor,
            life,
            rng,
            explosions,
        );
    } else if is_burning(c, &props) && props.phase() == Phase::Solid {
        let life = c.lifetime();
        let fm = props.fuel_mass.max(1) as f32;
        let heat_factor = (life as f32 / fm).min(1.0);
        let _ = spread_burn_to_neighbors(
            sg,
            world,
            p,
            c.material(),
            heat_factor,
            life,
            rng,
            explosions,
        );
    }
}
