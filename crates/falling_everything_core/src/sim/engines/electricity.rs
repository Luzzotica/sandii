//! Electricity as a **jagged visitor**: free sparks live only in empty cells; charge
//! travels through metal/water via [`cell_flags::ELECTRIFIED`] without replacing the medium.
//!
//! Flow: empty spark → enters conductor (flag only) → zigzag hops on conductive graph →
//! pops out into empty as a free spark again. Conductors stay metal/water after the visit.

use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::Rng;

use crate::world::{cell_flags, material, Cell, Vec2i, World};

use crate::sim::SimGrids;

const CARDINALS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
/// Lifetime ticks for free sparks in empty space.
const SPARK_LIFE: u8 = 18;
/// Hops along conductors per free-spark entry (jagged path length).
const CONDUCTOR_WALK_MIN: usize = 4;
const CONDUCTOR_WALK_MAX: usize = 14;
/// Per-tick chance (/256) an electrified conductor loses its glow (keep high so charge dies out).
const CONDUCTOR_DECAY: u32 = 96;
/// Chance (/256) a charged conductor edge spawns a free spark into empty (pop-out / branch).
const POP_OUT_CHANCE: u32 = 36;
/// Chance (/256) free spark branches an extra air hop after moving.
const AIR_BRANCH_CHANCE: u32 = 40;

#[inline]
fn is_conductive(world: &World, mat: u16) -> bool {
    mat != material::EMPTY
        && mat != material::ELECTRICITY
        && world.material_props(mat).conductive
}

#[inline]
fn free_spark_cell(temp: u16, life: u8) -> Cell {
    Cell::new()
        .with_material(material::ELECTRICITY)
        .with_lifetime(life.max(1))
        .with_temperature(temp)
        .with_flags(cell_flags::ELECTRIFIED)
}

/// Shuffle cardinals into `out` (fixed array, no alloc).
fn shuffled_cardinals(rng: &mut SmallRng, out: &mut [(i32, i32); 4]) {
    *out = CARDINALS;
    out.shuffle(rng);
}

/// Mark conductive cell electrified; never changes material id.
fn charge_conductor(sg: &SimGrids, p: Vec2i) {
    let c = sg.get(p);
    if c.material() == material::EMPTY || c.material() == material::ELECTRICITY {
        return;
    }
    sg.set_flag(p, cell_flags::ELECTRIFIED);
}

/// Jagged random walk on the conductive graph. Marks ELECTRIFIED along the path.
/// Returns the last conductive cell visited.
fn jagged_walk_conductors(
    sg: &SimGrids,
    world: &World,
    start: Vec2i,
    rng: &mut SmallRng,
    max_hops: usize,
) -> Vec2i {
    let mut cur = start;
    charge_conductor(sg, cur);
    let mut dirs = CARDINALS;
    for _ in 0..max_hops {
        dirs.shuffle(rng);
        let mut stepped = false;
        // Prefer a fresh direction; allow any conductive neighbor (including reverse sometimes).
        for &(dx, dy) in &dirs {
            let np = Vec2i::new(cur.x + dx, cur.y + dy);
            let n = sg.get(np);
            if is_conductive(world, n.material()) {
                charge_conductor(sg, np);
                cur = np;
                stepped = true;
                // Occasional branch: charge a side neighbor without moving the head.
                if rng.gen_ratio(50, 256) {
                    let side = dirs[rng.gen_range(0..4)];
                    let sp = Vec2i::new(cur.x + side.0, cur.y + side.1);
                    if is_conductive(world, sg.get(sp).material()) {
                        charge_conductor(sg, sp);
                    }
                }
                break;
            }
        }
        if !stepped {
            break;
        }
    }
    cur
}

/// Clear a free spark cell only if it is still ELECTRICITY (never erase media).
fn clear_free_spark(sg: &SimGrids, world: &World, p: Vec2i) {
    if sg.get(p).material() == material::ELECTRICITY {
        sg.set_cell(
            p,
            Cell::new().with_temperature(world.ambient_temperature_k()),
        );
    }
}

/// Try to place a free spark in an empty neighbor of `from` (pop-out arc).
fn try_pop_out(sg: &SimGrids, from: Vec2i, rng: &mut SmallRng, life: u8) -> bool {
    let mut dirs = CARDINALS;
    dirs.shuffle(rng);
    let temp = sg.get(from).temperature();
    for &(dx, dy) in &dirs {
        let np = Vec2i::new(from.x + dx, from.y + dy);
        // Strict empty only — never write sparks onto conductors or other matter.
        if sg.get(np).material() == material::EMPTY {
            sg.set_cell(np, free_spark_cell(temp, life));
            return true;
        }
    }
    false
}

/// Step a free electricity spark in empty space: jagged air hop or enter conductors.
pub(crate) fn step_electricity(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    if cell.material() != material::ELECTRICITY {
        return;
    }

    let life = if cell.lifetime() == 0 {
        SPARK_LIFE
    } else {
        cell.lifetime()
    };
    let temp = cell.temperature();

    // If a free spark somehow sits on a conductor material cell, convert to charge visit.
    // (Normal paint path also avoids this — see World::set_cell.)
    // Here material is ELECTRICITY so we only handle empty hops / enter neighbors.

    let mut dirs = [(0i32, 0i32); 4];
    shuffled_cardinals(rng, &mut dirs);

    // Prefer entering a conductor when adjacent (travel through metal/water).
    let mut entered = false;
    for &(dx, dy) in &dirs {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if is_conductive(world, sg.get(np).material()) {
            // Consume free spark only — medium is never replaced; we only flag conductors.
            clear_free_spark(sg, world, p);
            let hops = rng.gen_range(CONDUCTOR_WALK_MIN..=CONDUCTOR_WALK_MAX);
            let end = jagged_walk_conductors(sg, world, np, rng, hops);
            // Pop out far side when possible (visible spark again).
            let out_life = (life / 2).max(4);
            let _ = try_pop_out(sg, end, rng, out_life);
            // Extra branch pop sometimes for jagged feel.
            if rng.gen_ratio(80, 256) {
                let _ = try_pop_out(sg, end, rng, out_life.saturating_sub(2).max(2));
            }
            entered = true;
            break;
        }
    }
    if entered {
        return;
    }

    // Air zigzag: hop to a random empty neighbor (not buoyancy-biased).
    let new_life = life.saturating_sub(1);
    if new_life == 0 {
        clear_free_spark(sg, world, p);
        return;
    }

    let mut hopped = false;
    for &(dx, dy) in &dirs {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if sg.get(np).material() == material::EMPTY {
            clear_free_spark(sg, world, p);
            // Re-check empty after clear (another spark may have claimed the cell).
            if sg.get(np).material() == material::EMPTY {
                sg.set_cell(np, free_spark_cell(temp, new_life));
                hopped = true;
                // Rare branch spark
                if rng.gen_ratio(AIR_BRANCH_CHANCE, 256) {
                    let mut d2 = CARDINALS;
                    d2.shuffle(rng);
                    for &(bx, by) in &d2 {
                        let bp = Vec2i::new(np.x + bx, np.y + by);
                        if sg.get(bp).material() == material::EMPTY {
                            sg.set_cell(bp, free_spark_cell(temp, (new_life / 2).max(2)));
                            break;
                        }
                    }
                }
            }
            break;
        }
    }
    if !hopped {
        // Stuck: still decay in place if we are still a free spark.
        if sg.get(p).material() == material::ELECTRICITY {
            sg.set_lifetime(p, new_life);
            sg.set_flag(p, cell_flags::ELECTRIFIED);
        }
    }
}

/// Electrified metal/water: keep zigzagging charge and occasionally pop out; never replace medium.
pub(crate) fn step_electrified_conductor(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
) {
    let cell = sg.get(p);
    if cell.material() == material::ELECTRICITY || cell.material() == material::EMPTY {
        return;
    }
    if !cell.has_flag(cell_flags::ELECTRIFIED) {
        return;
    }
    if !world.material_props(cell.material()).conductive {
        sg.clear_flag(p, cell_flags::ELECTRIFIED);
        return;
    }

    // One jagged hop only — long walks happen on free-spark *entry*, not every conductor tick
    // (otherwise charge re-infects the network forever and never dies out).
    let mut dirs = CARDINALS;
    dirs.shuffle(rng);
    let mut end = p;
    for &(dx, dy) in &dirs {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if is_conductive(world, sg.get(np).material()) {
            charge_conductor(sg, np);
            end = np;
            break;
        }
    }

    if rng.gen_ratio(POP_OUT_CHANCE, 256) {
        let _ = try_pop_out(sg, end, rng, SPARK_LIFE / 3);
    }

    if rng.gen_ratio(CONDUCTOR_DECAY, 256) {
        sg.clear_flag(p, cell_flags::ELECTRIFIED);
    }
}
