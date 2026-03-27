use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{material, Cell, MaterialProps, Vec2i, World};

use crate::sim::SimGrids;

pub(crate) fn acid_corrode_neighbors(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    acid_props: &MaterialProps,
    rng: &mut SmallRng,
) {
    const CARDINAL: [(i32, i32); 4] = [(0, -1), (-1, 0), (1, 0), (0, 1)];
    let src = acid_props.acid_corrosion;
    if !src.is_active() {
        return;
    }
    let mut acid = sg.get(p);
    let corrosive_mat = acid.material();
    if corrosive_mat == material::EMPTY || acid.lifetime() == 0 {
        return;
    }
    for &(dx, dy) in &CARDINAL {
        acid = sg.get(p);
        if acid.material() != corrosive_mat || acid.lifetime() == 0 {
            return;
        }
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if sg.index(np).is_none() {
            continue;
        }
        let ncell = sg.get(np);
        if ncell.material() == material::EMPTY {
            continue;
        }
        let nprops = world.material_props(ncell.material());
        if !nprops.acid_vulnerability.affected {
            continue;
        }
        let chance = nprops.acid_vulnerability.chance_percent.min(100);
        if chance == 0 || rng.gen_range(0u8..100) >= chance {
            continue;
        }
        if src.neighbor_damage == 0 && src.self_lifetime_cost == 0 {
            continue;
        }

        if nprops.corrosion_max_hp == 0 {
            sg.set_cell(np, Cell::new());
        } else {
            let cur_hp = if ncell.lifetime() == 0 {
                nprops.corrosion_max_hp
            } else {
                ncell.lifetime()
            };
            let new_hp = cur_hp.saturating_sub(src.neighbor_damage);
            if new_hp == 0 {
                sg.set_cell(np, Cell::new());
            } else {
                let mut c = ncell;
                c.set_lifetime(new_hp);
                sg.set_cell(np, c);
            }
        }

        let next_acid = acid.lifetime().saturating_sub(src.self_lifetime_cost);
        if next_acid == 0 {
            sg.set_cell(p, Cell::new());
            return;
        }
        sg.set_lifetime(p, next_acid);
    }
}
