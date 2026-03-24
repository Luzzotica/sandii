//! Bresenham chord explosions: rays from center to targets on/near the blast circle (shuffled),
//! center → out, live `World` updates so overlapping chords concentrate damage near the core.
//!
//! Chord endpoints combine the integer circle outline with **evenly spaced angles** so vertical
//! and diagonal directions get as many rays as horizontal (fixes “pinched” poles).

use std::collections::HashSet;

use rand::seq::SliceRandom;
use rand::rngs::SmallRng;

use crate::bresenham::{bresenham_line, isqrt_i32};
use crate::world::{material, Cell, MaterialId, Vec2i, World};

/// Material + lifetime to place when a cell is destroyed by the blast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplosionSpawn {
    pub material: MaterialId,
    /// `None` uses [`World::initial_lifetime_for`] for `material`.
    pub lifetime: Option<u8>,
    /// `None` uses [`MaterialProps::base_temperature`](crate::world::MaterialProps::base_temperature).
    pub temperature: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplosionParams {
    pub center: Vec2i,
    pub radius: i32,
    /// Per-chord starting energy (ignored when [`Self::obliterate_disk`] is true).
    pub base_strength: u32,
    /// Clears every non-empty cell in the filled disk at once (no ray attenuation).
    pub obliterate_disk: bool,
    /// Inner destroyed cells: `None` → empty.
    pub fill_on_destroy: Option<ExplosionSpawn>,
    /// Outer annulus (see [`Self::edge_band_inward`]): `None` → use fill / empty everywhere.
    pub edge_on_destroy: Option<ExplosionSpawn>,
    /// Edge band is pixels with `dist_sq >= (radius.saturating_sub(this))^2` (legacy incendiary uses `2`).
    pub edge_band_inward: i32,
}

impl ExplosionParams {
    /// C4 / smolder-style blast: empty interior, fire ring on the outer band.
    pub fn gameplay_incendiary(center: Vec2i, radius: i32) -> Self {
        Self {
            center,
            radius,
            base_strength: base_strength_for_radius(radius),
            obliterate_disk: false,
            fill_on_destroy: None,
            edge_on_destroy: Some(ExplosionSpawn {
                material: material::FIRE,
                lifetime: Some(40),
                temperature: Some(800),
            }),
            edge_band_inward: 2,
        }
    }
}

/// Suggested strength for gameplay explosions from a radius (C4, smolder burnout, etc.).
#[inline]
pub fn base_strength_for_radius(radius: i32) -> u32 {
    (radius as u32).saturating_mul(36).max(96)
}

/// Integer circle outline: left/right intersections per scanline.
fn scanline_circle_outline(center: Vec2i, r: i32, set: &mut HashSet<Vec2i>) {
    let r2 = r * r;
    let cx = center.x;
    let cy = center.y;
    for y in (cy - r)..=(cy + r) {
        let dy = y - cy;
        let dy2 = dy.saturating_mul(dy);
        if dy2 > r2 {
            continue;
        }
        let w = isqrt_i32(r2 - dy2);
        set.insert(Vec2i::new(cx - w, y));
        if w > 0 {
            set.insert(Vec2i::new(cx + w, y));
        }
    }
}

/// All chord endpoints: scanline outline ∪ many evenly spaced angles (integer-rounded), deduped.
fn chord_endpoints(center: Vec2i, r: i32) -> Vec<Vec2i> {
    let mut set: HashSet<Vec2i> = HashSet::new();
    if r <= 0 {
        set.insert(center);
        return set.into_iter().collect();
    }
    scanline_circle_outline(center, r, &mut set);
    let n = (8 * r).max(32);
    let cx = center.x as f64;
    let cy = center.y as f64;
    let rf = r as f64;
    for i in 0..n {
        let t = std::f64::consts::TAU * (i as f64) / (n as f64);
        let bx = (cx + t.cos() * rf).round() as i32;
        let by = (cy + t.sin() * rf).round() as i32;
        set.insert(Vec2i::new(bx, by));
    }
    set.into_iter().collect()
}

#[inline]
fn build_spawn_cell(world: &World, s: &ExplosionSpawn) -> Cell {
    let props = world.material_props(s.material);
    let life = s
        .lifetime
        .unwrap_or_else(|| World::initial_lifetime_for(s.material, &props));
    let temp = s.temperature.unwrap_or(props.base_temperature);
    Cell::new()
        .with_material(s.material)
        .with_lifetime(life)
        .with_temperature(temp)
}

fn replacement_after_destroy(world: &World, params: &ExplosionParams, d2: i32) -> Cell {
    let r = params.radius;
    let inner_r = r.saturating_sub(params.edge_band_inward);
    let inner_r2 = inner_r.saturating_mul(inner_r);
    let use_edge = params.edge_on_destroy.is_some() && r > 0 && d2 >= inner_r2;
    if use_edge {
        if let Some(ref e) = params.edge_on_destroy {
            return build_spawn_cell(world, e);
        }
    }
    if let Some(ref f) = params.fill_on_destroy {
        return build_spawn_cell(world, f);
    }
    Cell::new()
}

fn obliterate_disk(world: &mut World, params: &ExplosionParams) {
    let cx = params.center.x;
    let cy = params.center.y;
    let r = params.radius;
    if r < 0 {
        return;
    }
    let r2 = r * r;
    for y in (cy - r)..=(cy + r) {
        let dy = y - cy;
        for x in (cx - r)..=(cx + r) {
            let dx = x - cx;
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let p = Vec2i::new(x, y);
            let cell = world.get_cell(p);
            if cell.material() == material::EMPTY {
                continue;
            }
            let d2 = dx * dx + dy * dy;
            world.set_cell(p, replacement_after_destroy(world, params, d2));
        }
    }
}

#[inline]
fn damage_to_lifetime(incoming: u32) -> u8 {
    ((incoming / 10).max(1).min(255)) as u8
}

/// Apply an explosion: optional full-disk erase, else shuffled chords from center outward.
pub fn apply(world: &mut World, rng: &mut SmallRng, params: ExplosionParams) {
    if params.radius < 0 {
        return;
    }
    if params.obliterate_disk {
        obliterate_disk(world, &params);
        return;
    }

    let mut targets = chord_endpoints(params.center, params.radius);
    if targets.is_empty() {
        return;
    }
    targets.shuffle(rng);

    let center = params.center;

    for b in targets {
        let mut strength = params.base_strength;
        for p in bresenham_line(center, b) {
            if strength == 0 {
                break;
            }
            let cell = world.get_cell(p);
            let mat = cell.material();
            if mat == material::EMPTY {
                continue;
            }

            let props = world.material_props(mat);
            let d = props.durability;
            let life = cell.lifetime();
            let incoming = strength;

            if d == 0 && life == 0 {
                let dx = p.x - center.x;
                let dy = p.y - center.y;
                let d2 = dx * dx + dy * dy;
                world.set_cell(p, replacement_after_destroy(world, &params, d2));
                continue;
            }

            let dmg = damage_to_lifetime(incoming);
            let nl = life.saturating_sub(dmg);

            strength = incoming.saturating_sub(d as u32 + life as u32);

            if nl == 0 {
                let dx = p.x - center.x;
                let dy = p.y - center.y;
                let d2 = dx * dx + dy * dy;
                world.set_cell(p, replacement_after_destroy(world, &params, d2));
            } else {
                world.set_cell(p, cell.with_lifetime(nl));
            }
        }
    }
}
