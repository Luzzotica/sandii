use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{Cell, Vec2i, World};

use crate::sim::{MoveIntent, SimGrids};

pub(crate) fn step_smoke(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let life = cell.lifetime();

    if life == 0 {
        sg.set_cell(p, Cell::new());
        return;
    }
    sg.set_lifetime(p, life - 1);

    let props = world.material_props(cell.material());
    let new_vel = ((cell.velocity_x() as i16) + props.acceleration() as i16)
        .min(props.max_speed() as i16) as i8;
    sg.set_velocity(p, new_vel);

    let steps = (new_vel as i32).max(1);
    let mut current = p;

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
