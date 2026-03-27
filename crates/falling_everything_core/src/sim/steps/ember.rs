use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{material, Cell, Vec2i, World};

use crate::sim::SimGrids;

use crate::sim::engines::burn::{spread_burn_to_neighbors, try_neighbor_spawns};

pub(crate) fn step_ember(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    let cell = sg.get(p);
    let ember_props = world.material_props(material::EMBER);
    let mut life = cell.lifetime();

    if life == 0 {
        sg.set_cell(
            p,
            Cell::new()
                .with_material(material::SMOKE)
                .with_lifetime(rng.gen_range(18..52))
                .with_variant(cell.variant()),
        );
        return;
    }

    let heat_factor = ember_props.ignitability as f32 / 255.0;
    if spread_burn_to_neighbors(
        sg,
        world,
        p,
        material::EMBER,
        heat_factor,
        life,
        rng,
        explosions,
    ) {
        return;
    }

    life = sg.get(p).lifetime();
    let rate = ember_props.consumption_rate.max(1) as u32;
    if rng.gen_ratio(rate, 256) {
        if life <= 1 {
            sg.set_cell(
                p,
                Cell::new()
                    .with_material(material::SMOKE)
                    .with_lifetime(rng.gen_range(18..52))
                    .with_variant(cell.variant()),
            );
            return;
        }
        sg.set_lifetime(p, life - 1);
    }

    try_neighbor_spawns(sg, world, p, &ember_props, rng);
}
