use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{material, Cell, MaterialProps, Vec2i, World};

use crate::sim::SimGrids;

use super::influence::ADJ_TRANSFORM_NEIGHBORS8;

/// [`MaterialProps::adjacent_transforms`]: probabilistic `from` → `to` on neighbors each tick.
pub(crate) fn step_adjacent_transform_neighbors(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    props: &MaterialProps,
    rng: &mut SmallRng,
) {
    for &(dx, dy) in &ADJ_TRANSFORM_NEIGHBORS8 {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        let neighbor_mat = sg.get(np).material();
        for rule in props.adjacent_transforms {
            if !rule.is_active() {
                continue;
            }
            let chance = rule.chance_percent.min(100);
            if rule.cardinal_neighbors_only && dx != 0 && dy != 0 {
                continue;
            }
            if neighbor_mat != rule.from {
                continue;
            }
            if rng.gen_range(0u8..100) >= chance {
                break;
            }
            if rule.to == material::EMPTY {
                sg.set_cell(np, Cell::new());
            } else {
                let to_props = world.material_props(rule.to);
                let new_lifetime = if rule.to == material::STEAM {
                    rng.gen_range(36..72)
                } else {
                    World::initial_lifetime_for(rule.to, &to_props)
                };
                sg.set_cell(
                    np,
                    Cell::new()
                        .with_material(rule.to)
                        .with_lifetime(new_lifetime)
                        .with_temperature(to_props.base_temperature)
                        .with_variant(rng.gen_range(0..16)),
                );
            }
            if rule.actor_lifetime_delta > 0 {
                let actor = sg.get(p);
                let nl = actor.lifetime().saturating_sub(rule.actor_lifetime_delta);
                sg.set_lifetime(p, nl);
            }
            break;
        }
    }
}
