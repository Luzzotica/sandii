use rand::rngs::SmallRng;

use crate::world::{material, MaterialProps, Vec2i, World};

use crate::sim::{vertical_down_allows_density_swap, MoveIntent, SimGrids};

use super::acid::acid_corrode_neighbors;
use super::influence::liquid_adjacent_influence_pass;

/// Diagonal / lateral tie-break: **do not** use vertical `velocity` (it is almost always > 0 after
/// gravity accel and wrongly biases flow to the right).
#[inline]
fn liquid_prefer_left_first(p: Vec2i) -> bool {
    (p.x ^ p.y) & 1 == 0
}

/// Supported liquid loses this much vertical speed per tick when it cannot slide (viscous drag).
#[inline]
fn liquid_supported_friction(props: &MaterialProps) -> i8 {
    (1 + (props.viscosity() as i16 / 32).min(3)) as i8
}

pub(crate) fn step_liquid(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let props = world.material_props(cell.material());
    if props.has_acid_corrosion() {
        acid_corrode_neighbors(sg, world, p, &props, rng);
    }
    if props.has_adjacent_influence() {
        let src_mat = cell.material();
        if liquid_adjacent_influence_pass(sg, world, p, src_mat, &props.adjacent_influence, rng) {
            return;
        }
    }

    let cell = sg.get(p);
    let props = world.material_props(cell.material());

    let below = Vec2i::new(p.x, p.y + 1);
    let below_cell = sg.get(below);
    let below_props = world.material_props(below_cell.material());
    let can_fall = below_cell.material() == material::EMPTY
        || (!below_props.inert()
            && vertical_down_allows_density_swap(props.phase(), below_props.phase())
            && props.density > below_props.density);

    if !can_fall {
        if cell.velocity_x() == 0 {
            // One slip attempt without requiring fall speed (opens v=0 puddles toward holes only).
            if step_liquid_supported_slide(sg, world, p, rng) {
                return;
            }
            sg.set_velocity(p, 0);
            return;
        }

        if step_liquid_supported_slide(sg, world, p, rng) {
            return;
        }

        let v = sg.get(p).velocity_x();
        let friction = liquid_supported_friction(&props);
        sg.set_velocity(p, (v - friction).max(0));
        return;
    }

    let new_vel = ((cell.velocity_x() as i16) + props.acceleration() as i16)
        .min(props.max_speed() as i16) as i8;
    sg.set_velocity(p, new_vel);

    let steps = (new_vel as i32).max(1);
    let mut current = p;

    for _ in 0..steps {
        let down = Vec2i::new(current.x, current.y + 1);
        if sg.try_displace(world, current, down, MoveIntent::VerticalDown, rng) {
            current = down;
            continue;
        }

        let dl = Vec2i::new(current.x - 1, current.y + 1);
        let dr = Vec2i::new(current.x + 1, current.y + 1);
        let prefer_left_first = liquid_prefer_left_first(current);

        let moved_diag = if prefer_left_first {
            if sg.try_displace(world, current, dl, MoveIntent::VerticalDown, rng) {
                current = dl;
                true
            } else if sg.try_displace(world, current, dr, MoveIntent::VerticalDown, rng) {
                current = dr;
                true
            } else {
                false
            }
        } else if sg.try_displace(world, current, dr, MoveIntent::VerticalDown, rng) {
            current = dr;
            true
        } else if sg.try_displace(world, current, dl, MoveIntent::VerticalDown, rng) {
            current = dl;
            true
        } else {
            false
        };

        if moved_diag {
            continue;
        }

        if step_liquid_supported_slide(sg, world, current, rng) {
            return;
        }

        let v = sg.get(current).velocity_x();
        let friction = liquid_supported_friction(&props);
        sg.set_velocity(current, (v - friction).max(0));
        return;
    }

    let below_current = Vec2i::new(current.x, current.y + 1);
    let below_cell = sg.get(below_current);
    let below_props = world.material_props(below_cell.material());
    if below_cell.material() != material::EMPTY
        && (below_props.inert()
            || !vertical_down_allows_density_swap(props.phase(), below_props.phase())
            || props.density <= below_props.density)
    {
        if !step_liquid_supported_slide(sg, world, current, rng) {
            let v = sg.get(current).velocity_x();
            let friction = liquid_supported_friction(&props);
            sg.set_velocity(current, (v - friction).max(0));
        }
    }
}

/// While supported, slide toward the side whose column has void (empty or hole-below) closest
/// below this row. Tie-break: shallower `liquid_column_void_depth` then [`liquid_prefer_left_first`].
fn step_liquid_supported_slide(
    sg: &SimGrids,
    world: &World,
    from: Vec2i,
    rng: &mut SmallRng,
) -> bool {
    let mat = sg.get(from).material();
    let rule = world.material_rule(mat);
    let spread = rule.lateral_spread.max(1) as i32;

    let left_target = scan_lateral_target(sg, from, -1, spread);
    let right_target = scan_lateral_target(sg, from, 1, spread);

    let dir: i32 = match (left_target, right_target) {
        (Some(l), Some(r)) => {
            if l < r {
                -1
            } else if r < l {
                1
            } else {
                let dl = liquid_column_void_depth(sg, from.x - 1, from.y, spread);
                let dr = liquid_column_void_depth(sg, from.x + 1, from.y, spread);
                match (dl, dr) {
                    (Some(a), Some(b)) if a < b => -1,
                    (Some(a), Some(b)) if b < a => 1,
                    _ => {
                        if liquid_prefer_left_first(from) {
                            -1
                        } else {
                            1
                        }
                    }
                }
            }
        }
        (Some(_), None) => -1,
        (None, Some(_)) => 1,
        // No reachable void within lateral_spread — do not lateral nudge (keeps flat pools at rest).
        (None, None) => return false,
    };

    let max_move = spread.min(2);
    let mut cur = from;
    let mut moved_any = false;
    for i in 1..=max_move {
        let side = Vec2i::new(cur.x + dir * i, cur.y);
        if sg.try_displace(world, cur, side, MoveIntent::Lateral, rng) {
            cur = side;
            moved_any = true;
        } else {
            break;
        }
    }
    moved_any
}

/// Shortest vertical offset `dy >= 1` such that `(col_x, surface_y + dy)` is empty or has empty
/// directly below (same rule as [`scan_lateral_target`]). Scans up to `max_dy` rows.
fn liquid_column_void_depth(sg: &SimGrids, col_x: i32, surface_y: i32, max_dy: i32) -> Option<i32> {
    for dy in 1..=max_dy {
        let p = Vec2i::new(col_x, surface_y + dy);
        if sg.index(p).is_none() {
            break;
        }
        let cell = sg.get(p);
        if cell.material() == material::EMPTY {
            return Some(dy);
        }
        let below = Vec2i::new(col_x, surface_y + dy + 1);
        if sg.index(below).is_none() {
            continue;
        }
        if sg.get(below).material() == material::EMPTY {
            return Some(dy);
        }
    }
    None
}

fn scan_lateral_target(sg: &SimGrids, from: Vec2i, dir: i32, max_dist: i32) -> Option<i32> {
    for i in 1..=max_dist {
        let p = Vec2i::new(from.x + dir * i, from.y);
        let cell = sg.get(p);
        if cell.material() == material::EMPTY {
            return Some(i);
        }
        let below = Vec2i::new(p.x, p.y + 1);
        let below_cell = sg.get(below);
        if below_cell.material() == material::EMPTY {
            return Some(i);
        }
    }
    None
}
