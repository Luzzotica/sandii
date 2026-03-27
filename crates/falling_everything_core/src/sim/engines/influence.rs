use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::Rng;

use crate::world::{
    material, AdjacentInfluenceRule, Cell, InfluenceSourceEffect, InfluenceVictimLifetime,
    MaterialId, MaterialProps, Vec2i, World,
};

use crate::sim::SimGrids;

/// 8-neighbor offsets (same winding as burn spread and adjacent transforms).
pub(crate) const ADJ_TRANSFORM_NEIGHBORS8: [(i32, i32); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Heat from stepped cells combines with per-material `victim_chance` as documented on [`AdjacentInfluenceRule`].
fn influence_transition_spawn_lifetime(
    rule: &AdjacentInfluenceRule,
    spawn_mat: MaterialId,
    spawn_props: &MaterialProps,
    rng: &mut SmallRng,
) -> u8 {
    if rule.spawn_lifetime_hi > rule.spawn_lifetime_lo {
        rng.gen_range(rule.spawn_lifetime_lo..rule.spawn_lifetime_hi)
    } else {
        World::initial_lifetime_for(spawn_mat, spawn_props)
    }
}

fn influence_empty_neighbor_spawn_lifetime(
    rule: &AdjacentInfluenceRule,
    spawn_mat: MaterialId,
    spawn_props: &MaterialProps,
    rng: &mut SmallRng,
) -> u8 {
    if rule.empty_neighbor_spawn_lifetime_hi > rule.empty_neighbor_spawn_lifetime_lo {
        rng.gen_range(rule.empty_neighbor_spawn_lifetime_lo..rule.empty_neighbor_spawn_lifetime_hi)
    } else {
        World::initial_lifetime_for(spawn_mat, spawn_props)
    }
}

/// Tries a random permutation of 8-neighbors around `victim_pos`.
fn try_place_influence_empty_neighbor_spawn(
    sg: &SimGrids,
    world: &World,
    victim_pos: Vec2i,
    rule: &AdjacentInfluenceRule,
    rng: &mut SmallRng,
) {
    let sm = rule.empty_neighbor_spawn;
    if sm == material::EMPTY {
        return;
    }
    let sprops = world.material_props(sm);
    let life = influence_empty_neighbor_spawn_lifetime(rule, sm, &sprops, rng);
    let mut offs = ADJ_TRANSFORM_NEIGHBORS8;
    offs.shuffle(rng);
    for &(dx, dy) in &offs {
        let np = Vec2i::new(victim_pos.x + dx, victim_pos.y + dy);
        if sg.get(np).material() != material::EMPTY {
            continue;
        }
        sg.set_cell(
            np,
            Cell::new()
                .with_material(sm)
                .with_lifetime(life)
                .with_temperature(sprops.base_temperature)
                .with_variant(rng.gen_range(0..16)),
        );
        return;
    }
}

fn apply_influence_source_effect(
    sg: &SimGrids,
    source_pos: Vec2i,
    source_material: MaterialId,
    effect: InfluenceSourceEffect,
) {
    match effect {
        InfluenceSourceEffect::None => {}
        InfluenceSourceEffect::ClearSourceCell => {
            sg.set_cell(source_pos, Cell::new());
        }
        InfluenceSourceEffect::AddSourceLifetime(n) => {
            let c = sg.get(source_pos);
            if c.material() == source_material {
                sg.set_lifetime(source_pos, c.lifetime().saturating_add(n));
            }
        }
        InfluenceSourceEffect::SubtractSourceLifetime(n) => {
            let c = sg.get(source_pos);
            if c.material() == source_material {
                sg.set_lifetime(source_pos, c.lifetime().saturating_sub(n));
            }
        }
    }
}

/// First matching active rule wins. Returns `true` if a rule applied (caller skips legacy burn for this neighbor).
pub(crate) fn try_adjacent_influence_on_neighbor(
    sg: &SimGrids,
    world: &World,
    source_pos: Vec2i,
    source_material: MaterialId,
    neighbor_pos: Vec2i,
    dx: i32,
    dy: i32,
    rules: &[AdjacentInfluenceRule],
    heat_factor: f32,
    _child_life_cap: u8,
    rng: &mut SmallRng,
) -> bool {
    let hf = heat_factor.clamp(0.0, 1.0);
    if sg.get(source_pos).material() != source_material {
        return false;
    }
    let ncell = sg.get(neighbor_pos);
    if ncell.material() == material::EMPTY || ncell.material() == source_material {
        return false;
    }
    let nprops = world.material_props(ncell.material());

    for rule in rules {
        if !rule.is_active() {
            continue;
        }
        if ncell.material() != rule.victim {
            continue;
        }
        if rule.cardinal_neighbors_only && !(dx == 0 || dy == 0) {
            continue;
        }
        if rule.require_victim_flags_any != 0
            && (ncell.flags() & rule.require_victim_flags_any) == 0
        {
            continue;
        }
        if rule.exclude_victim_flags_any != 0
            && (ncell.flags() & rule.exclude_victim_flags_any) != 0
        {
            continue;
        }

        let base = rule.chance_percent as f32 / 100.0;
        let p = if rule.requires_victim_ignitability {
            let ign = nprops.ignitability as u32;
            if ign == 0 {
                continue;
            }
            if ign >= 255 {
                1.0
            } else {
                base * hf * (ign as f32 / 255.0)
            }
        } else {
            base * hf
        };

        if p < 1.0 && rng.gen::<f32>() >= p {
            continue;
        }

        let old_flags = ncell.flags();
        let new_flags = (old_flags | rule.flags_or) & !rule.flags_clear;

        if rule.if_cleared_mask != 0 && rule.spawn_on_clear_material != material::EMPTY {
            let m = rule.if_cleared_mask;
            if (old_flags & m) != 0 && (new_flags & m) == 0 {
                let sm = rule.spawn_on_clear_material;
                let sprops = world.material_props(sm);
                let life = influence_transition_spawn_lifetime(rule, sm, &sprops, rng);
                sg.set_cell(
                    neighbor_pos,
                    Cell::new()
                        .with_material(sm)
                        .with_lifetime(life)
                        .with_temperature(sprops.base_temperature),
                );
                apply_influence_source_effect(sg, source_pos, source_material, rule.source_effect);
                try_place_influence_empty_neighbor_spawn(sg, world, neighbor_pos, rule, rng);
                return true;
            }
        }

        if rule.if_set_mask != 0 && rule.spawn_on_set_material != material::EMPTY {
            let m = rule.if_set_mask;
            if (old_flags & m) == 0 && (new_flags & m) != 0 {
                let sm = rule.spawn_on_set_material;
                let sprops = world.material_props(sm);
                let life = influence_transition_spawn_lifetime(rule, sm, &sprops, rng);
                sg.set_cell(
                    neighbor_pos,
                    Cell::new()
                        .with_material(sm)
                        .with_lifetime(life)
                        .with_temperature(sprops.base_temperature),
                );
                apply_influence_source_effect(sg, source_pos, source_material, rule.source_effect);
                try_place_influence_empty_neighbor_spawn(sg, world, neighbor_pos, rule, rng);
                return true;
            }
        }

        let new_lifetime = match rule.victim_lifetime {
            InfluenceVictimLifetime::Unchanged => ncell.lifetime(),
            InfluenceVictimLifetime::Set(v) => v,
            InfluenceVictimLifetime::UseVictimFuelMass => {
                if nprops.fuel_mass == 0 {
                    continue;
                }
                nprops.fuel_mass
            }
        };

        let mut updated = sg.get(neighbor_pos);
        updated.set_flags((updated.flags() | rule.flags_or) & !rule.flags_clear);
        updated.set_lifetime(new_lifetime);
        sg.set_cell(neighbor_pos, updated);
        apply_influence_source_effect(sg, source_pos, source_material, rule.source_effect);
        try_place_influence_empty_neighbor_spawn(sg, world, neighbor_pos, rule, rng);
        return true;
    }
    false
}

/// Water / other sources: run all eight offsets; stop early if the source cell is cleared.
pub(crate) fn liquid_adjacent_influence_pass(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    source_material: MaterialId,
    rules: &[AdjacentInfluenceRule],
    rng: &mut SmallRng,
) -> bool {
    const NEIGHBORS8: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];
    for &(dx, dy) in &NEIGHBORS8 {
        if sg.get(p).material() != source_material {
            return true;
        }
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if try_adjacent_influence_on_neighbor(
            sg,
            world,
            p,
            source_material,
            np,
            dx,
            dy,
            rules,
            1.0,
            255,
            rng,
        ) && (sg.get(p).material() == material::EMPTY || sg.get(p).material() != source_material)
        {
            return true;
        }
    }
    false
}

pub(crate) fn water_to_steam_cell(rng: &mut SmallRng) -> Cell {
    Cell::new()
        .with_material(material::STEAM)
        .with_lifetime(rng.gen_range(36..72))
        .with_temperature(393)
}
