use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{cell_flags, material, Phase, Vec2i, World};

use crate::sim::{vertical_down_allows_density_swap, MoveIntent, SimGrids};

pub(crate) fn step_sand(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let props = world.material_props(cell.material());
    let free_falling = cell.has_flag(cell_flags::IS_FREE_FALLING);

    if !free_falling {
        let below = Vec2i::new(p.x, p.y + 1);
        let below_cell = sg.get(below);
        let below_props = world.material_props(below_cell.material());
        if below_cell.material() == material::EMPTY
            || (!below_props.inert()
                && vertical_down_allows_density_swap(props.phase(), below_props.phase())
                && props.density > below_props.density)
        {
            sg.set_flag(p, cell_flags::IS_FREE_FALLING);
        } else {
            if below_props.phase() == Phase::Liquid && rng.gen_ratio(1, 5) {
                let dir = if rng.gen_bool(0.5) { -1 } else { 1 };
                let side = Vec2i::new(p.x + dir, p.y);
                if sg.get(side).material() == material::EMPTY {
                    let _ = sg.try_displace(world, p, side, MoveIntent::Lateral, rng);
                }
            }
            return;
        }
    }

    let new_vel = ((cell.velocity_x() as i16) + props.acceleration() as i16)
        .min(props.max_speed() as i16) as i8;
    sg.set_velocity(p, new_vel);

    let steps = (new_vel as i32).max(1);
    let mut current = p;
    let mut moved = false;

    for _ in 0..steps {
        let down = Vec2i::new(current.x, current.y + 1);
        if sg.try_displace(world, current, down, MoveIntent::VerticalDown, rng) {
            current = down;
            moved = true;
            continue;
        }
        let left_first = rng.gen_bool(0.5);
        let dl = Vec2i::new(current.x - 1, current.y + 1);
        let dr = Vec2i::new(current.x + 1, current.y + 1);
        let (first, second) = if left_first { (dl, dr) } else { (dr, dl) };
        if sg.try_displace(world, current, first, MoveIntent::VerticalDown, rng) {
            current = first;
            moved = true;
        } else if sg.try_displace(world, current, second, MoveIntent::VerticalDown, rng) {
            current = second;
            moved = true;
        } else {
            sg.set_velocity(current, 0);
            sg.clear_flag(current, cell_flags::IS_FREE_FALLING);
            break;
        }
    }

    if moved {
        for dx in [-1i32, 1] {
            let neighbor = Vec2i::new(current.x + dx, current.y);
            let ncell = sg.get(neighbor);
            if ncell.material() != material::EMPTY && !ncell.has_flag(cell_flags::IS_FREE_FALLING) {
                let nprops = world.material_props(ncell.material());
                if nprops.phase() == Phase::Solid
                    && !nprops.inert()
                    && nprops.inertial_resistance() < 255
                {
                    let dislodge_chance = (255 - nprops.inertial_resistance()) as u32;
                    if rng.gen_ratio(dislodge_chance.max(1), 256) {
                        sg.set_flag(neighbor, cell_flags::IS_FREE_FALLING);
                    }
                }
            }
        }
    }
}
