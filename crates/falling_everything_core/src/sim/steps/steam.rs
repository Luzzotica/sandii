use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{material, Cell, Vec2i, World};

use crate::sim::{MoveIntent, SimGrids};

pub(crate) fn step_steam(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let life = cell.lifetime();

    if life == 0 {
        let water_props = world.material_props(material::LIQUID);
        sg.set_cell(
            p,
            Cell::new()
                .with_material(material::LIQUID)
                .with_temperature(water_props.base_temperature)
                .with_variant(cell.variant()),
        );
        return;
    }
    sg.set_lifetime(p, life - 1);

    let props = world.material_props(material::STEAM);
    let new_vel = ((cell.velocity_x() as i16) + props.acceleration() as i16)
        .min(props.max_speed() as i16) as i8;
    sg.set_velocity(p, new_vel);

    let mut current = p;
    let dx = if rng.gen_bool(0.67) {
        if rng.gen_bool(0.5) {
            -1
        } else {
            1
        }
    } else {
        0
    };
    if dx != 0 {
        let side = Vec2i::new(current.x + dx, current.y);
        if sg.try_displace(world, current, side, MoveIntent::Lateral, rng) {
            current = side;
        }
    }

    let steps = (new_vel as i32).max(1);
    for _ in 0..steps {
        let up = Vec2i::new(current.x, current.y - 1);
        if sg.try_displace(world, current, up, MoveIntent::VerticalUp, rng) {
            current = up;
            continue;
        }
        let left_first = rng.gen_bool(0.5);
        let ul = Vec2i::new(current.x - 1, current.y - 1);
        let ur = Vec2i::new(current.x + 1, current.y - 1);
        let (first, second) = if left_first { (ul, ur) } else { (ur, ul) };
        if sg.try_displace(world, current, first, MoveIntent::VerticalUp, rng) {
            current = first;
        } else if sg.try_displace(world, current, second, MoveIntent::VerticalUp, rng) {
            current = second;
        } else {
            sg.set_velocity(current, 0);
            break;
        }
    }
}
