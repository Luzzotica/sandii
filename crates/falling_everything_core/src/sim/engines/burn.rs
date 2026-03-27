use rand::rngs::SmallRng;
use rand::Rng;

use crate::cell64::MAX_TEMPERATURE;
use crate::sim::thermal::thermal_props_for;
use crate::sim::SimGrids;
use crate::world::{
    cell_flags, material, AdjacentInfluenceRule, Cell, MaterialId, MaterialProps, Phase, Vec2i,
    World,
};

use super::influence::{try_adjacent_influence_on_neighbor, water_to_steam_cell};

const IGNITION_PULSE_SCALE: i32 = 256;

/// When a cell first becomes a burning fuel, push heat into cardinal neighbors (or cool if `ignition_thermal_pulse` < 0).
pub(crate) fn apply_ignition_neighbor_pulse(
    sg: &SimGrids,
    world: &World,
    center: Vec2i,
    props: &MaterialProps,
) {
    let pulse: i32 = if props.ignition_thermal_pulse != 0 {
        props.ignition_thermal_pulse as i32
    } else {
        (props.heat_generation_rate as i32 * 6).max(8)
    };
    for &(dx, dy) in &[(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
        let np = Vec2i::new(center.x + dx, center.y + dy);
        let nc = sg.get(np);
        let Some(np_props) = thermal_props_for(nc, world) else {
            continue;
        };
        let vhc = np_props.volumetric_heat_capacity.max(1) as i32;
        let dt = (pulse * IGNITION_PULSE_SCALE) / vhc;
        let nt = (nc.temperature() as i32 + dt).clamp(0, MAX_TEMPERATURE as i32) as u16;
        sg.set_temperature(np, nt);
    }
}

#[inline]
fn smolder_replacement_material(props: &MaterialProps) -> MaterialId {
    if props.smolder_extinguish_material == material::EMPTY {
        material::SMOKE
    } else {
        props.smolder_extinguish_material
    }
}

#[inline]
/// Lifetime for the cell that replaces this material when fire/ember-style death runs out of `lifetime`
/// and becomes `on_death_become` (or implicit smoke).
pub(crate) fn on_death_replacement_lifetime(props: &MaterialProps, rng: &mut SmallRng) -> u8 {
    let lo = props.on_death_lifetime_lo;
    let hi = props.on_death_lifetime_hi;
    if hi > lo {
        rng.gen_range(lo..hi)
    } else {
        rng.gen_range(20..60)
    }
}

fn smolder_replacement_lifetime_range(props: &MaterialProps) -> (u8, u8) {
    let lo = props.smolder_extinguish_lifetime_lo;
    let hi = props.smolder_extinguish_lifetime_hi;
    if hi > lo {
        (lo, hi)
    } else {
        (24, 64)
    }
}

#[inline]
fn smolder_burnout_lifetime_range(props: &MaterialProps) -> (u8, u8) {
    let lo = props.smolder_burnout_lifetime_lo;
    let hi = props.smolder_burnout_lifetime_hi;
    if hi > lo {
        (lo, hi)
    } else {
        (20, 56)
    }
}

/// Instant heat replacement when `fuel_mass == 0` (fire spread / flash ignite). Same `smolder_burnout_*` fields as smolder burnout; capped by `child_life_cap`.
pub(crate) fn instant_heat_ignition_cell(
    nprops: &MaterialProps,
    child_life_cap: u8,
    rng: &mut SmallRng,
) -> (MaterialId, u8) {
    let mat = if nprops.smolder_burnout_become != material::EMPTY {
        nprops.smolder_burnout_become
    } else {
        material::FIRE
    };
    let raw_life = if nprops.smolder_burnout_lifetime_hi > nprops.smolder_burnout_lifetime_lo {
        rng.gen_range(nprops.smolder_burnout_lifetime_lo..nprops.smolder_burnout_lifetime_hi)
    } else if nprops.smolder_burnout_become != material::EMPTY {
        let (lo, hi) = smolder_burnout_lifetime_range(nprops);
        rng.gen_range(lo..hi)
    } else {
        child_life_cap.saturating_sub(rng.gen_range(10..30))
    };
    (mat, raw_life.min(child_life_cap))
}

/// Replacement material + half-open lifetime range after smolder ends.
fn smolder_elimination_replace(props: &MaterialProps, is_burnout: bool) -> (MaterialId, u8, u8) {
    if is_burnout && props.smolder_burnout_become != material::EMPTY {
        let (lo, hi) = smolder_burnout_lifetime_range(props);
        (props.smolder_burnout_become, lo, hi)
    } else {
        let mat = smolder_replacement_material(props);
        let (lo, hi) = smolder_replacement_lifetime_range(props);
        (mat, lo, hi)
    }
}

const FLASH_NEIGHBORS8: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// When a **plant** smolder cell burns out, try to ignite each neighbor like fire spread (`ignitability` rolls).
/// Water becomes steam only; the dying plant cell is not quenched by this pass.
pub(crate) fn flash_ignite_adjacent_from_burnout(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    const HEAT: f32 = 1.0;
    const CHILD_CAP: u8 = 72;
    let source_material = sg.get(p).material();
    for &(dx, dy) in &FLASH_NEIGHBORS8 {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        try_flash_ignite_neighbor(
            sg,
            world,
            p,
            source_material,
            np,
            dx,
            dy,
            HEAT,
            CHILD_CAP,
            rng,
            explosions,
        );
    }
}

fn try_flash_ignite_neighbor(
    sg: &SimGrids,
    world: &World,
    source_pos: Vec2i,
    source_material: MaterialId,
    np: Vec2i,
    dx: i32,
    dy: i32,
    heat_factor: f32,
    child_life_cap: u8,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    let ncell = sg.get(np);
    if ncell.material() == material::EMPTY {
        return;
    }
    let nprops = world.material_props(ncell.material());

    if nprops.extinguishes_fire() {
        sg.set_cell(np, water_to_steam_cell(rng));
        return;
    }

    if nprops.on_death_become != material::EMPTY && ncell.material() == nprops.on_death_become {
        return;
    }

    let src_props = world.material_props(source_material);
    let rules: &[AdjacentInfluenceRule] = if src_props.has_adjacent_influence() {
        &src_props.adjacent_influence
    } else if src_props.smolder_burnout_ignites_neighbors {
        &world.material_props(material::FIRE).adjacent_influence
    } else {
        &[]
    };
    if try_adjacent_influence_on_neighbor(
        sg,
        world,
        source_pos,
        source_material,
        np,
        dx,
        dy,
        rules,
        heat_factor,
        child_life_cap,
        rng,
    ) {
        return;
    }

    if nprops.ignitability == 0 {
        return;
    }

    let always_ignite = nprops.ignitability == 255;
    if !always_ignite {
        let ignite_chance = (nprops.ignitability as f32 / 255.0) * heat_factor;
        if rng.gen::<f32>() >= ignite_chance {
            return;
        }
    }

    if nprops.explosion_radius > 0 {
        explosions.push((np, nprops.explosion_radius as i32));
        sg.set_cell(np, Cell::new());
        return;
    }

    if nprops.on_heat_become != material::EMPTY {
        let become_props = world.material_props(nprops.on_heat_become);
        sg.set_cell(
            np,
            Cell::new()
                .with_material(nprops.on_heat_become)
                .with_lifetime(ncell.lifetime())
                .with_temperature(become_props.base_temperature)
                .with_variant(ncell.variant()),
        );
        return;
    }

    if nprops.fuel_mass > 0 {
        // Already burning: do not re-apply ignition (would reset lifetime to `fuel_mass` every tick from neighbors).
        if ncell.has_flag(cell_flags::ON_FIRE) {
            return;
        }
        let ignite_temp = ncell.temperature().max(nprops.autoignition_temperature);
        sg.set_cell(
            np,
            Cell::new()
                .with_material(ncell.material())
                .with_flags(ncell.flags() | cell_flags::ON_FIRE)
                .with_lifetime(nprops.fuel_mass)
                .with_temperature(ignite_temp)
                .with_variant(ncell.variant()),
        );
        apply_ignition_neighbor_pulse(sg, world, np, &nprops);
        return;
    }

    // Same rule as [`spread_burn_to_neighbors`]: no instant gas FIRE for solids (see comment there).
    if nprops.phase() == Phase::Solid {
        return;
    }

    let (ignite_mat, ignite_life) = instant_heat_ignition_cell(&nprops, child_life_cap, rng);
    let ignite_props = world.material_props(ignite_mat);
    sg.set_cell(
        np,
        Cell::new()
            .with_material(ignite_mat)
            .with_lifetime(ignite_life)
            .with_temperature(ignite_props.base_temperature)
            .with_variant(if ignite_mat == material::FIRE {
                0
            } else {
                ncell.variant()
            }),
    );
}

/// Smolder ends (water fully quenched or fuel burnout). Burnout-only: neighbor flash-ignite, optional center explosion.
pub(crate) fn eliminate_smoldering_fuel_at(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    props: &MaterialProps,
    dead_cell: &Cell,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
    is_burnout: bool,
) {
    if is_burnout {
        if props.smolder_burnout_ignites_neighbors {
            flash_ignite_adjacent_from_burnout(sg, world, p, rng, explosions);
        }
        let r = props.smolder_burnout_explosion_radius;
        if r > 0 {
            explosions.push((p, r as i32));
            return;
        }
    }
    let (mat, lo, hi) = smolder_elimination_replace(props, is_burnout);
    let life = rng.gen_range(lo..hi);
    sg.set_cell(
        p,
        Cell::new()
            .with_material(mat)
            .with_lifetime(life)
            .with_variant(dead_cell.variant()),
    );
}

const SPAWN_NEIGHBORS8: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// [`MaterialProps::neighbor_spawns`]: probabilistic placement into random empty 8-neighbors.
pub(crate) fn try_neighbor_spawns(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    props: &MaterialProps,
    rng: &mut SmallRng,
) {
    if !props.neighbor_spawns.iter().any(|r| r.is_active()) {
        return;
    }
    sg.wake_at(p);
    for rule in props.neighbor_spawns {
        if !rule.is_active() {
            continue;
        }
        if !rng.gen_ratio(rule.chance as u32, 256) {
            continue;
        }
        let start = rng.gen_range(0..8);
        for i in 0..8 {
            let (dx, dy) = SPAWN_NEIGHBORS8[(start + i) % 8];
            let np = Vec2i::new(p.x + dx, p.y + dy);
            if sg.get(np).material() != material::EMPTY {
                continue;
            }
            let spawn_props = world.material_props(rule.spawn_material);
            let lifetime = if rule.lifetime_hi > rule.lifetime_lo {
                rng.gen_range(rule.lifetime_lo..rule.lifetime_hi)
            } else {
                World::initial_lifetime_for(rule.spawn_material, &spawn_props)
            };
            sg.set_cell(
                np,
                Cell::new()
                    .with_material(rule.spawn_material)
                    .with_flags(rule.spawn_flags)
                    .with_lifetime(lifetime)
                    .with_temperature(spawn_props.base_temperature)
                    .with_variant(rng.gen_range(0..16)),
            );
            break;
        }
    }
}

/// Returns `true` if the source at `p` was extinguished (replaced with smoke).
pub(crate) fn spread_burn_to_neighbors(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    source_material: MaterialId,
    heat_factor: f32,
    child_life_cap: u8,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) -> bool {
    const NEIGHBORS: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];

    let src_props = world.material_props(source_material);

    for &(dx, dy) in &NEIGHBORS {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        let ncell = sg.get(np);
        if ncell.material() == material::EMPTY || ncell.material() == source_material {
            continue;
        }

        let nprops = world.material_props(ncell.material());

        // Water does not ignite; quenching is via thermal conduction (no lifetime drain here).
        if nprops.extinguishes_fire() {
            continue;
        }

        if nprops.on_death_become != material::EMPTY && ncell.material() == nprops.on_death_become {
            continue;
        }

        if src_props.has_adjacent_influence()
            && try_adjacent_influence_on_neighbor(
                sg,
                world,
                p,
                source_material,
                np,
                dx,
                dy,
                &src_props.adjacent_influence,
                heat_factor,
                child_life_cap,
                rng,
            )
        {
            continue;
        }

        if nprops.ignitability == 0 {
            continue;
        }

        let always_ignite = nprops.ignitability == 255;
        if !always_ignite {
            let ignite_chance = (nprops.ignitability as f32 / 255.0) * heat_factor;
            if rng.gen::<f32>() >= ignite_chance {
                continue;
            }
        }

        if nprops.explosion_radius > 0 {
            explosions.push((np, nprops.explosion_radius as i32));
            sg.set_cell(np, Cell::new());
            continue;
        }

        if nprops.on_heat_become != material::EMPTY {
            let become_props = world.material_props(nprops.on_heat_become);
            sg.set_cell(
                np,
                Cell::new()
                    .with_material(nprops.on_heat_become)
                    .with_lifetime(ncell.lifetime())
                    .with_temperature(become_props.base_temperature)
                    .with_variant(ncell.variant()),
            );
            continue;
        }

        if nprops.fuel_mass > 0 {
            if ncell.has_flag(cell_flags::ON_FIRE) {
                continue;
            }
            let ignite_temp = ncell.temperature().max(nprops.autoignition_temperature);
            sg.set_cell(
                np,
                Cell::new()
                    .with_material(ncell.material())
                    .with_flags(ncell.flags() | cell_flags::ON_FIRE)
                    .with_lifetime(nprops.fuel_mass)
                    .with_temperature(ignite_temp)
                    .with_variant(ncell.variant()),
            );
            apply_ignition_neighbor_pulse(sg, world, np, &nprops);
            continue;
        }

        // Never replace a solid with a gas FIRE cell via instant heat. Smolder fuels must use the
        // `fuel_mass` branch (same material + ON_FIRE). The old guard only skipped solids when
        // `autoignition_temperature > 0`; `fuel_mass == 0` + `autoignition_temperature == 0` (mis-tuned
        // props) fell through to `instant_heat_ignition_cell`, which looks like wood "vanishing".
        if nprops.phase() == Phase::Solid {
            continue;
        }

        let (ignite_mat, ignite_life) = instant_heat_ignition_cell(&nprops, child_life_cap, rng);
        let ignite_props = world.material_props(ignite_mat);
        sg.set_cell(
            np,
            Cell::new()
                .with_material(ignite_mat)
                .with_lifetime(ignite_life)
                .with_temperature(ignite_props.base_temperature)
                .with_variant(if ignite_mat == material::FIRE {
                    0
                } else {
                    ncell.variant()
                }),
        );
    }
    false
}
