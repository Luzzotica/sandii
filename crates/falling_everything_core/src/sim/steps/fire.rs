use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{material, Cell, Vec2i, World};

use crate::sim::{MoveIntent, SimGrids};

use crate::sim::engines::burn::{on_death_replacement_lifetime, spread_burn_to_neighbors};

pub(crate) fn step_fire(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    let cell = sg.get(p);
    let fire_props = world.material_props(cell.material());
    let life = cell.lifetime();

    if life == 0 {
        let death_mat = if fire_props.on_death_become != material::EMPTY {
            fire_props.on_death_become
        } else {
            material::SMOKE
        };
        if death_mat == material::EMPTY {
            sg.set_cell(p, Cell::new());
        } else {
            sg.set_cell(
                p,
                Cell::new()
                    .with_material(death_mat)
                    .with_lifetime(on_death_replacement_lifetime(&fire_props, rng)),
            );
        }
        return;
    }

    sg.set_lifetime(p, life - 1);

    let heat_factor = life as f32 / 255.0;
    let source_mat = cell.material();
    if spread_burn_to_neighbors(sg, world, p, source_mat, heat_factor, life, rng, explosions) {
        return;
    }

    let current = p;
    let up = Vec2i::new(current.x, current.y - 1);
    if sg.try_displace(world, current, up, MoveIntent::VerticalUp, rng) {
        return;
    }

    let drift = if rng.gen_bool(0.5) { -1 } else { 1 };
    let side_up = Vec2i::new(current.x + drift, current.y - 1);
    if sg.try_displace(world, current, side_up, MoveIntent::VerticalUp, rng) {
        return;
    }

    let side = Vec2i::new(current.x + drift, current.y);
    let _ = sg.try_displace(world, current, side, MoveIntent::Lateral, rng);
}
