use rand::rngs::SmallRng;
use rand::seq::SliceRandom;
use rand::Rng;
use rand::SeedableRng;
use rayon::prelude::*;

use crate::bresenham::{bresenham_line, isqrt_i32};
use crate::world::{
    cell_flags, material, AdjacentInfluenceRule, Cell, ChunkCoord, InfluenceSourceEffect, InfluenceVictimLifetime,
    MaterialId, MaterialProps, MaterialRule, Phase, ReactionOutcome, RectI, Vec2i, World,
    ADJ_ACTOR_WATER_QUENCH_LIFETIME,
};
use crate::SimulationEvent;

/// 8-neighbor offsets (same winding as burn spread and adjacent transforms).
const ADJ_TRANSFORM_NEIGHBORS8: [(i32, i32); 8] = [
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
        if sg.get(np).material != material::EMPTY {
            continue;
        }
        sg.set_cell(
            np,
            Cell {
                material: sm,
                flags: 0,
                velocity: 0,
                lifetime: life,
                variant: rng.gen_range(0..16),
                scorch: 0,
            },
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
            sg.set_cell(source_pos, Cell::default());
        }
        InfluenceSourceEffect::AddSourceLifetime(n) => {
            let c = sg.get(source_pos);
            if c.material == source_material {
                sg.set_lifetime(source_pos, c.lifetime.saturating_add(n));
            }
        }
        InfluenceSourceEffect::SubtractSourceLifetime(n) => {
            let c = sg.get(source_pos);
            if c.material == source_material {
                sg.set_lifetime(source_pos, c.lifetime.saturating_sub(n));
            }
        }
    }
}

/// First matching active rule wins. Returns `true` if a rule applied (caller skips legacy burn for this neighbor).
fn try_adjacent_influence_on_neighbor(
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
    if sg.get(source_pos).material != source_material {
        return false;
    }
    let ncell = sg.get(neighbor_pos);
    if ncell.material == material::EMPTY || ncell.material == source_material {
        return false;
    }
    let nprops = world.material_props(ncell.material);

    for rule in rules {
        if !rule.is_active() {
            continue;
        }
        if ncell.material != rule.victim {
            continue;
        }
        if rule.cardinal_neighbors_only && !(dx == 0 || dy == 0) {
            continue;
        }
        if rule.require_victim_flags_any != 0 && (ncell.flags & rule.require_victim_flags_any) == 0 {
            continue;
        }
        if rule.exclude_victim_flags_any != 0 && (ncell.flags & rule.exclude_victim_flags_any) != 0 {
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

        let old_flags = ncell.flags;
        let new_flags = (old_flags | rule.flags_or) & !rule.flags_clear;

        if rule.if_cleared_mask != 0 && rule.spawn_on_clear_material != material::EMPTY {
            let m = rule.if_cleared_mask;
            if (old_flags & m) != 0 && (new_flags & m) == 0 {
                let sm = rule.spawn_on_clear_material;
                let sprops = world.material_props(sm);
                let life = influence_transition_spawn_lifetime(rule, sm, &sprops, rng);
                sg.set_cell(
                    neighbor_pos,
                    Cell {
                        material: sm,
                        flags: 0,
                        velocity: 0,
                        lifetime: life,
                        variant: 0,
                        scorch: 0,
                    },
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
                    Cell {
                        material: sm,
                        flags: 0,
                        velocity: 0,
                        lifetime: life,
                        variant: 0,
                        scorch: 0,
                    },
                );
                apply_influence_source_effect(sg, source_pos, source_material, rule.source_effect);
                try_place_influence_empty_neighbor_spawn(sg, world, neighbor_pos, rule, rng);
                return true;
            }
        }

        let new_lifetime = match rule.victim_lifetime {
            InfluenceVictimLifetime::Unchanged => ncell.lifetime,
            InfluenceVictimLifetime::Set(v) => v,
            InfluenceVictimLifetime::UseVictimFuelMass => {
                if nprops.fuel_mass == 0 {
                    continue;
                }
                nprops.fuel_mass
            }
        };

        sg.set_cell(
            neighbor_pos,
            Cell {
                material: ncell.material,
                flags: new_flags,
                velocity: 0,
                lifetime: new_lifetime,
                variant: ncell.variant,
                scorch: ncell.scorch,
            },
        );
        apply_influence_source_effect(sg, source_pos, source_material, rule.source_effect);
        try_place_influence_empty_neighbor_spawn(sg, world, neighbor_pos, rule, rng);
        return true;
    }
    false
}

/// Water / other sources: run all eight offsets; stop early if the source cell is cleared.
fn liquid_adjacent_influence_pass(
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
        if sg.get(p).material != source_material {
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
        ) && (sg.get(p).material == material::EMPTY || sg.get(p).material != source_material)
        {
            return true;
        }
    }
    false
}

fn water_to_steam_cell(rng: &mut SmallRng) -> Cell {
    Cell {
        material: material::STEAM,
        flags: 0,
        velocity: 0,
        lifetime: rng.gen_range(36..72),
        variant: 0,
        scorch: 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    SingleThreadSeeded,
    ThreadPool,
}

struct ChunkStepState {
    pass_order: [usize; 4],
    pass_idx: usize,
    coords: Vec<ChunkCoord>,
    coord_idx: usize,
    pending_explosions: Vec<(Vec2i, i32)>,
}

pub struct Scheduler {
    mode: SchedulerMode,
    seed: u64,
    tick: u64,
    /// When true, skips the 4-pass checkerboard chunk schedule: single-threaded full-grid scan once
    /// per tick (no `rayon`). Ignores `ThreadPool` for stepping. Reseeds RNG like `SingleThreadSeeded`.
    debug_full_world_single_pass: bool,
    /// One checkerboard chunk per [`Scheduler::step_world`] call; uses [`World::set_debug_chunk_highlight`].
    debug_chunk_step: bool,
    chunk_step: Option<ChunkStepState>,
}

impl Scheduler {
    pub fn new(mode: SchedulerMode, seed: u64) -> Self {
        Self {
            mode,
            seed,
            tick: 0,
            debug_full_world_single_pass: false,
            debug_chunk_step: false,
            chunk_step: None,
        }
    }

    pub fn mode(&self) -> SchedulerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: SchedulerMode) {
        self.mode = mode;
    }

    pub fn debug_full_world_single_pass(&self) -> bool {
        self.debug_full_world_single_pass
    }

    pub fn set_debug_full_world_single_pass(&mut self, enabled: bool) {
        self.debug_full_world_single_pass = enabled;
    }

    pub fn debug_chunk_step(&self) -> bool {
        self.debug_chunk_step
    }

    pub fn set_debug_chunk_step(&mut self, enabled: bool) {
        self.debug_chunk_step = enabled;
    }

    /// Finish a partial chunk-step tick without stepping further (e.g. when turning debug off).
    pub fn abort_debug_chunk_step(&mut self, world: &mut World) {
        if let Some(state) = self.chunk_step.take() {
            world.set_debug_chunk_highlight(None);
            world.finish_sim();
            for (center, radius) in state.pending_explosions {
                process_explosion(world, center, radius);
            }
        }
    }

    pub fn step_world(&mut self, world: &mut World, rng: &mut SmallRng) {
        if self.debug_full_world_single_pass {
            self.tick = self.tick.saturating_add(1);
            let reseed_tick_rng =
                matches!(self.mode, SchedulerMode::SingleThreadSeeded) || self.debug_full_world_single_pass;
            if reseed_tick_rng {
                *rng = SmallRng::seed_from_u64(self.seed ^ self.tick);
            }
            world.advance_awake_flags();
            world.prepare_sim();
            let sg = SimGrids::from_world(world);
            let all_explosions = process_world_full_pass(&sg, world, rng);
            world.finish_sim();
            for (center, radius) in all_explosions {
                process_explosion(world, center, radius);
            }
            return;
        }

        if self.debug_chunk_step {
            self.step_world_one_chunk(world, rng);
            return;
        }

        self.tick = self.tick.saturating_add(1);
        let reseed_tick_rng = matches!(self.mode, SchedulerMode::SingleThreadSeeded);
        if reseed_tick_rng {
            *rng = SmallRng::seed_from_u64(self.seed ^ self.tick);
        }

        world.advance_awake_flags();
        world.prepare_sim();

        let mut all_explosions: Vec<(Vec2i, i32)> = Vec::new();

        let pass_order: [usize; 4] = if (self.tick & 1) == 0 {
            [0, 1, 2, 3]
        } else {
            [2, 3, 0, 1]
        };
        for pass in pass_order {
            let coords: Vec<ChunkCoord> = world.active_chunk_coords_for_pass(pass).into_iter().collect();
            match self.mode {
                SchedulerMode::SingleThreadSeeded => {
                    for coord in coords {
                        let sg = SimGrids::from_world(world);
                        let explosions = process_chunk(&sg, world, coord, pass as u8, rng);
                        all_explosions.extend(explosions);
                    }
                }
                SchedulerMode::ThreadPool => {
                    // Parallelism is **per chunk** within this pass: each `ChunkCoord` is one rayon task.
                    // A single 64×64 chunk is still stepped on one thread (cells are not split across threads).
                    let sg = SimGrids::from_world(world);
                    let tick = self.tick;
                    let seed = self.seed;
                    let pass_u8 = pass as u8;
                    let results: Vec<Vec<(Vec2i, i32)>> = coords
                        .par_iter()
                        .map(|coord| {
                            let mut local_rng =
                                SmallRng::seed_from_u64(seed ^ tick ^ ((coord.x as u64) << 32) ^ coord.y as u64);
                            process_chunk(&sg, world, *coord, pass_u8, &mut local_rng)
                        })
                        .collect();
                    for explosions in results {
                        all_explosions.extend(explosions);
                    }
                }
            }
        }

        world.finish_sim();

        for (center, radius) in all_explosions {
            process_explosion(world, center, radius);
        }
    }

    fn step_world_one_chunk(&mut self, world: &mut World, rng: &mut SmallRng) {
        if self.chunk_step.is_none() {
            self.tick = self.tick.saturating_add(1);
            let reseed_tick_rng = matches!(self.mode, SchedulerMode::SingleThreadSeeded) || self.debug_chunk_step;
            if reseed_tick_rng {
                *rng = SmallRng::seed_from_u64(self.seed ^ self.tick);
            }
            world.advance_awake_flags();
            world.prepare_sim();
            let pass_order: [usize; 4] = if (self.tick & 1) == 0 {
                [0, 1, 2, 3]
            } else {
                [2, 3, 0, 1]
            };
            let mut pass_idx = 0usize;
            let mut coords = world.active_chunk_coords_for_pass(pass_order[0]);
            while coords.is_empty() && pass_idx < 3 {
                pass_idx += 1;
                coords = world.active_chunk_coords_for_pass(pass_order[pass_idx]);
            }
            if coords.is_empty() {
                world.finish_sim();
                world.set_debug_chunk_highlight(None);
                return;
            }
            self.chunk_step = Some(ChunkStepState {
                pass_order,
                pass_idx,
                coords,
                coord_idx: 0,
                pending_explosions: Vec::new(),
            });
        }

        let state = self.chunk_step.as_mut().expect("chunk_step");
        let pass = state.pass_order[state.pass_idx] as u8;
        let coord = state.coords[state.coord_idx];
        world.set_debug_chunk_highlight(Some((coord, pass)));

        let sg = SimGrids::from_world(world);
        let explosions = match self.mode {
            SchedulerMode::SingleThreadSeeded => process_chunk(&sg, world, coord, pass, rng),
            SchedulerMode::ThreadPool => {
                // Chunk-step mode runs one chunk per `step_world`; there is nothing for rayon to fan out.
                let mut local_rng =
                    SmallRng::seed_from_u64(self.seed ^ self.tick ^ ((coord.x as u64) << 32) ^ coord.y as u64);
                process_chunk(&sg, world, coord, pass, &mut local_rng)
            }
        };
        state.pending_explosions.extend(explosions);
        state.coord_idx += 1;

        if state.coord_idx >= state.coords.len() {
            state.coord_idx = 0;
            state.pass_idx += 1;
            while state.pass_idx < 4 {
                state.coords = world.active_chunk_coords_for_pass(state.pass_order[state.pass_idx]);
                if !state.coords.is_empty() {
                    break;
                }
                state.pass_idx += 1;
            }
        }

        if state.pass_idx >= 4 {
            let explosions = std::mem::take(&mut state.pending_explosions);
            self.chunk_step = None;
            world.set_debug_chunk_highlight(None);
            world.finish_sim();
            for (center, radius) in explosions {
                process_explosion(world, center, radius);
            }
        } else {
            // `World::get_cell` reads the read buffer; sim writes the write buffer until `finish_sim`
            // swaps. Without committing after each chunk, rendering (and any read-buffer logic) stays
            // stuck on the pre-tick read grid for the whole multi-chunk tick — looks like the sim froze.
            world.finish_sim();
            world.prepare_sim();
        }
    }
}

fn process_chunk(sg: &SimGrids, world: &World, coord: ChunkCoord, pass: u8, rng: &mut SmallRng) -> Vec<(Vec2i, i32)> {
    let bounds = world.bounds_for_chunk(coord);

    let mut explosions = Vec::new();
    for y in (bounds.min.y..=bounds.max.y).rev() {
        let reverse_x = rng.gen_bool(0.5);
        if reverse_x {
            for x in (bounds.min.x..=bounds.max.x).rev() {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, pass);
                step_pixel(sg, world, p, rng, &mut explosions);
            }
        } else {
            for x in bounds.min.x..=bounds.max.x {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, pass);
                step_pixel(sg, world, p, rng, &mut explosions);
            }
        }
    }
    explosions
}

/// Single-threaded: every cell in the loaded grid once, bottom-to-top (same row order as `process_chunk`).
/// Pass id `4` distinguishes debug overlay from checkerboard passes 0–3.
fn process_world_full_pass(sg: &SimGrids, world: &World, rng: &mut SmallRng) -> Vec<(Vec2i, i32)> {
    let Some(bounds) = world.dirty_world_bounds() else {
        return Vec::new();
    };
    const PASS: u8 = 4;
    let mut explosions = Vec::new();
    for y in (bounds.min.y..=bounds.max.y).rev() {
        let reverse_x = rng.gen_bool(0.5);
        if reverse_x {
            for x in (bounds.min.x..=bounds.max.x).rev() {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, PASS);
                step_pixel(sg, world, p, rng, &mut explosions);
            }
        } else {
            for x in bounds.min.x..=bounds.max.x {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, PASS);
                step_pixel(sg, world, p, rng, &mut explosions);
            }
        }
    }
    explosions
}

fn process_explosion(world: &mut World, center: Vec2i, radius: i32) {
    let r2 = radius * radius;
    let perimeter_inner = (radius - 2) * (radius - 2);
    let cx = center.x;
    let cy = center.y;
    // Integer-only disk fill: each scanline is a Bresenham horizontal segment (chord half-width from isqrt).
    for y in (cy - radius)..=(cy + radius) {
        let dy = y - cy;
        let dy2 = dy.saturating_mul(dy);
        if dy2 > r2 {
            continue;
        }
        let w = isqrt_i32(r2 - dy2);
        let a = Vec2i::new(cx - w, y);
        let b = Vec2i::new(cx + w, y);
        for p in bresenham_line(a, b) {
            let dx = p.x - cx;
            let d2 = dx * dx + dy2;
            let existing = world.get_cell(p);
            if existing.material == material::STATIC {
                continue;
            }
            if d2 >= perimeter_inner {
                world.set_cell(p, Cell {
                    material: material::FIRE,
                    flags: 0,
                    velocity: 0,
                    lifetime: 40,
                    variant: 0,
                    scorch: 0,
                });
            } else {
                world.set_cell(p, Cell::default());
            }
        }
    }
}

/// Provides read/write access to the double-buffered world grids.
/// Read grid is immutable (frame snapshot). Write grid is the working copy.
/// Safety: the checkerboard pass ordering guarantees that concurrent threads
/// write to non-overlapping regions of the write grid.
pub(crate) struct SimGrids {
    read: *const Cell,
    write: *mut Cell,
    width: i32,
    height: i32,
    origin: Vec2i,
    awake_next: *mut bool,
    chunks_x: i32,
    chunks_y: i32,
    chunk_size: i32,
    debug_pass: *mut u8,
    debug_pass_enabled: bool,
}

unsafe impl Send for SimGrids {}
unsafe impl Sync for SimGrids {}

impl SimGrids {
    pub fn from_world(world: &mut World) -> Self {
        let debug_pass_enabled = world.debug_pass_enabled();
        let debug_pass = if debug_pass_enabled { world.debug_pass_ptr() } else { std::ptr::null_mut() };
        Self {
            read: world.read_cells().as_ptr(),
            write: world.write_cells_ptr(),
            width: world.grid_width(),
            height: world.grid_height(),
            origin: world.grid_origin(),
            awake_next: world.chunk_awake_next_ptr(),
            chunks_x: world.chunks_x(),
            chunks_y: world.chunks_y(),
            chunk_size: world.chunk_size(),
            debug_pass,
            debug_pass_enabled,
        }
    }

    fn stamp_debug_pass(&self, p: Vec2i, pass: u8) {
        if self.debug_pass_enabled {
            if let Some(i) = self.index(p) {
                unsafe { *self.debug_pass.add(i) = pass; }
            }
        }
    }

    pub(crate) fn index(&self, p: Vec2i) -> Option<usize> {
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y;
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            return None;
        }
        Some((y * self.width + x) as usize)
    }

    fn get(&self, p: Vec2i) -> Cell {
        self.index(p)
            .map(|i| unsafe { *self.write.add(i) })
            .unwrap_or(Cell {
                material: material::STATIC,
                flags: 0,
                velocity: 0,
                lifetime: 0,
                variant: 0,
                scorch: 0,
            })
    }

    #[inline]
    fn was_moved(&self, p: Vec2i) -> bool {
        self.index(p)
            .map(|i| {
                unsafe { *self.read.add(i) != *self.write.add(i) }
            })
            .unwrap_or(false)
    }

    fn set_velocity(&self, p: Vec2i, vel: i8) {
        if let Some(i) = self.index(p) {
            unsafe {
                (*self.write.add(i)).velocity = vel;
            }
            self.wake_at(p);
        }
    }

    fn set_cell(&self, p: Vec2i, cell: Cell) {
        if let Some(i) = self.index(p) {
            unsafe {
                *self.write.add(i) = cell;
            }
            self.wake_at(p);
        }
    }

    fn set_lifetime(&self, p: Vec2i, lifetime: u8) {
        if let Some(i) = self.index(p) {
            unsafe {
                (*self.write.add(i)).lifetime = lifetime;
            }
            self.wake_at(p);
        }
    }

    fn set_flag(&self, p: Vec2i, flag: u16) {
        if let Some(i) = self.index(p) {
            unsafe { (*self.write.add(i)).flags |= flag; }
            self.wake_at(p);
        }
    }

    fn clear_flag(&self, p: Vec2i, flag: u16) {
        if let Some(i) = self.index(p) {
            unsafe { (*self.write.add(i)).flags &= !flag; }
            self.wake_at(p);
        }
    }

    fn wake_at(&self, p: Vec2i) {
        let lx = p.x - self.origin.x;
        let ly = p.y - self.origin.y;
        let cx = crate::world::div_floor(lx, self.chunk_size);
        let cy = crate::world::div_floor(ly, self.chunk_size);
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let ncx = cx + dx;
                let ncy = cy + dy;
                if ncx >= 0 && ncx < self.chunks_x && ncy >= 0 && ncy < self.chunks_y {
                    let idx = (ncy * self.chunks_x + ncx) as usize;
                    unsafe { *self.awake_next.add(idx) = true; }
                }
            }
        }
    }

    fn try_displace(&self, world: &World, from: Vec2i, to: Vec2i, intent: MoveIntent, rng: &mut SmallRng) -> bool {
        // Conflict policy: checkerboard phases prevent concurrent adjacent chunk stepping, so writes to
        // the same target cell in a phase are not expected. If two candidates contend across serial order,
        // the first processed source in scan order wins and tags frame bits; later attempts observe the new state.
        let Some(from_idx) = self.index(from) else {
            return false;
        };
        let Some(to_idx) = self.index(to) else {
            return false;
        };
        let from_cell = unsafe { *self.write.add(from_idx) };
        let to_cell = unsafe { *self.write.add(to_idx) };
        if from_cell.material == material::EMPTY {
            return false;
        }

        let from_props = world.material_props(from_cell.material);
        let to_props = world.material_props(to_cell.material);
        let from_rule = world.material_rule(from_cell.material);
        let to_rule = world.material_rule(to_cell.material);
        if from_props.inert() || to_props.inert() {
            return false;
        }

        let reaction = world.reaction(from_cell.material, to_cell.material);
        if let ReactionOutcome::Transform(from_to, to_to) = reaction {
            unsafe {
                (*self.write.add(to_idx)).material = from_to;
                (*self.write.add(from_idx)).material = to_to;
            }
            self.wake_at(from);
            self.wake_at(to);
            return true;
        }
        if reaction == ReactionOutcome::Swap {
            unsafe {
                *self.write.add(to_idx) = from_cell;
                *self.write.add(from_idx) = to_cell;
            }
            self.wake_at(from);
            self.wake_at(to);
            return true;
        }

        if !can_displace(from_props, from_rule, to_cell, to_props, to_rule, intent, rng) {
            return false;
        }

        unsafe {
            *self.write.add(to_idx) = from_cell;
            *self.write.add(from_idx) = if to_cell.material == material::EMPTY {
                Cell::default()
            } else {
                to_cell
            };
        }
        self.wake_at(from);
        self.wake_at(to);
        true
    }
}

#[derive(Clone, Copy)]
enum MoveIntent {
    VerticalDown,
    VerticalUp,
    Lateral,
}

/// Density-driven vertical **swap** is for liquids mixing/stacking and for solids sinking through
/// liquid or gas. It is not used for liquid-into-solid (e.g. lava should ride on sand, not push it up).
#[inline]
fn vertical_down_allows_density_swap(from: Phase, to: Phase) -> bool {
    match (from, to) {
        (Phase::Liquid, Phase::Liquid) => true,
        (Phase::Solid, Phase::Liquid) => true,
        (Phase::Solid, Phase::Gas) | (Phase::Liquid, Phase::Gas) => true,
        (Phase::Gas, Phase::Gas) => true,
        _ => false,
    }
}

fn can_displace(
    from_props: MaterialProps,
    _from_rule: MaterialRule,
    to_cell: Cell,
    to_props: MaterialProps,
    _to_rule: MaterialRule,
    intent: MoveIntent,
    rng: &mut SmallRng,
) -> bool {
    if to_cell.material == material::EMPTY {
        return true;
    }

    match intent {
        MoveIntent::VerticalDown => {
            if !vertical_down_allows_density_swap(from_props.phase(), to_props.phase()) {
                return false;
            }
            if from_props.density <= to_props.density {
                return false;
            }
            let diff = (from_props.density - to_props.density) as f32 / 256.0;
            rng.gen::<f32>() < diff.clamp(0.05, 0.6)
        }
        MoveIntent::VerticalUp => {
            if from_props.density >= to_props.density {
                return false;
            }
            let diff = (to_props.density - from_props.density) as f32 / 256.0;
            rng.gen::<f32>() < diff.clamp(0.05, 0.6)
        }
        MoveIntent::Lateral => match (from_props.phase(), to_props.phase()) {
            (Phase::Gas, Phase::Gas) => true,
            // Liquids do not swap sideways with other liquids (avoids pool shimmer / edge ping-pong).
            (Phase::Liquid, Phase::Liquid) => false,
            (Phase::Liquid, Phase::Gas) => true,
            _ => false,
        },
    }
}

fn step_pixel(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng, explosions: &mut Vec<(Vec2i, i32)>) {
    if sg.was_moved(p) {
        return;
    }
    let cell = sg.get(p);
    if cell.material == material::EMPTY {
        return;
    }
    // Rigid-body pixels are STATIC + RIGID_BODY_SIM for the sim step; still vaporize adjacent water like lava.
    if cell.material == material::STATIC
        && (cell.flags & cell_flags::RIGID_BODY_SIM) != 0
        && (cell.flags & cell_flags::ON_FIRE) != 0
        && cell.lifetime > 0
    {
        let lava_props = world.material_props(material::LAVA);
        if lava_props.has_adjacent_transforms() {
            step_adjacent_transform_neighbors(sg, world, p, &lava_props, rng);
        }
    }
    if cell.material == material::EMBER {
        step_ember(sg, world, p, rng, explosions);
        return;
    }
    let props = world.material_props(cell.material);
    let lava_adj_early = cell.material == material::LAVA && props.has_adjacent_transforms();
    if lava_adj_early {
        step_adjacent_transform_neighbors(sg, world, p, &props, rng);
    }
    if cell.flags & cell_flags::ON_FIRE != 0
        && cell.material != material::FIRE
    {
        step_smoldering_fuel(sg, world, p, rng, explosions);
        if sg.get(p).material == material::LAVA {
            step_liquid(sg, world, p, rng);
        }
        return;
    }
    if props.has_adjacent_transforms() && !lava_adj_early {
        step_adjacent_transform_neighbors(sg, world, p, &props, rng);
    }
    // Fire/smolder sparks (wood, wax, lava) must run only from `step_smoldering_fuel` while `ON_FIRE`.
    // Cold wood/wax was incorrectly rolling `neighbor_spawns` here and spawning adjacent fire every tick.
    if props.has_neighbor_spawns() {
        let on_fire = cell.flags & cell_flags::ON_FIRE != 0;
        let cold_spawner = matches!(
            cell.material,
            material::TORCH | material::WELL | material::SPOUT
        );
        if on_fire || cold_spawner {
            try_neighbor_spawns(sg, world, p, &props, rng);
        }
    }
    if props.inert() {
        return;
    }
    match props.phase() {
        Phase::Solid => step_sand(sg, world, p, rng),
        Phase::Liquid => step_liquid(sg, world, p, rng),
        Phase::Gas => {
            // Smoke must always use step_smoke: with lifetime==0 it used to fall through to step_gas,
            // which never decays — smoke persisted forever. Chunk sleep needs wake_at in set_lifetime
            // so stationary decaying smoke keeps its chunk awake.
            if cell.material == material::SMOKE {
                step_smoke(sg, world, p, rng);
            } else if cell.material == material::STEAM {
                step_steam(sg, world, p, rng);
            } else if props.on_death_become != material::EMPTY {
                step_fire(sg, world, p, rng, explosions);
            } else if cell.lifetime > 0 {
                step_smoke(sg, world, p, rng);
            } else {
                step_gas(sg, world, p, rng);
            }
        }
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
fn on_death_replacement_lifetime(props: &MaterialProps, rng: &mut SmallRng) -> u8 {
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
fn instant_heat_ignition_cell(
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

/// Smolder ends (water fully quenched or fuel burnout). Burnout-only: neighbor flash-ignite, optional center explosion.
fn eliminate_smoldering_fuel_at(
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
        Cell {
            material: mat,
            flags: 0,
            velocity: 0,
            lifetime: life,
            variant: dead_cell.variant,
            scorch: dead_cell.scorch,
        },
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
fn try_neighbor_spawns(
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
            if sg.get(np).material != material::EMPTY {
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
                Cell {
                    material: rule.spawn_material,
                    flags: rule.spawn_flags,
                    velocity: 0,
                    lifetime,
                    variant: rng.gen_range(0..16),
                    scorch: 0,
                },
            );
            break;
        }
    }
}

/// Smoldering fuel (`ON_FIRE`, non-`FIRE` material). Keeps the chunk awake every tick so low
/// `consumption_rate` materials (e.g. lava) are not skipped after chunk sleep.
fn step_smoldering_fuel(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    sg.wake_at(p);
    let cell = sg.get(p);
    let rigid_heat_proxy = cell.material == material::STATIC
        && (cell.flags & cell_flags::RIGID_BODY_SIM) != 0
        && (cell.flags & cell_flags::ON_FIRE) != 0;
    let props = if rigid_heat_proxy {
        world.material_props(material::LAVA)
    } else {
        world.material_props(cell.material)
    };

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
        let np = Vec2i::new(p.x + dx, p.y + dy);
        let nprops = world.material_props(sg.get(np).material);
        if !nprops.extinguishes_fire() {
            continue;
        }
        sg.set_cell(np, water_to_steam_cell(rng));
        let src = sg.get(p);
        if src.material != cell.material || (src.flags & cell_flags::ON_FIRE == 0) {
            return;
        }
        let nl = src.lifetime.saturating_sub(ADJ_ACTOR_WATER_QUENCH_LIFETIME);
        if nl == 0 {
            let out_props = world.material_props(if (src.flags & cell_flags::RIGID_BODY_SIM) != 0 {
                material::LAVA
            } else {
                src.material
            });
            eliminate_smoldering_fuel_at(sg, world, p, &out_props, &src, rng, explosions, false);
            return;
        }
        sg.set_cell(
            p,
            Cell {
                material: cell.material,
                flags: cell.flags | cell_flags::ON_FIRE,
                velocity: cell.velocity,
                lifetime: nl,
                variant: cell.variant,
                scorch: cell.scorch,
            },
        );
    }

    let cell = sg.get(p);
    let life = cell.lifetime;
    let rate = props.consumption_rate.max(1) as u32;
    if rng.gen_ratio(rate, 256) {
        if life <= 1 {
            let burn_props = world.material_props(if (cell.flags & cell_flags::RIGID_BODY_SIM) != 0 {
                material::LAVA
            } else {
                cell.material
            });
            eliminate_smoldering_fuel_at(sg, world, p, &burn_props, &cell, rng, explosions, true);
            return;
        }
        sg.set_lifetime(p, life - 1);
    }

    try_neighbor_spawns(sg, world, p, &props, rng);

    let c = sg.get(p);
    let rigid_lava_proxy = c.material == material::STATIC
        && (c.flags & cell_flags::RIGID_BODY_SIM) != 0
        && c.lifetime > 0
        && (c.flags & cell_flags::ON_FIRE != 0);
    let molten_lava = c.material == material::LAVA
        && c.lifetime > 0
        && (c.flags & cell_flags::ON_FIRE != 0);

    if rigid_lava_proxy || molten_lava {
        let life = c.lifetime;
        let heat_factor = life as f32 / 255.0;
        let _ = spread_burn_to_neighbors(
            sg,
            world,
            p,
            material::LAVA,
            heat_factor,
            life,
            rng,
            explosions,
        );
    }
}

/// [`MaterialProps::adjacent_transforms`]: probabilistic `from` → `to` on neighbors each tick.
fn step_adjacent_transform_neighbors(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    props: &MaterialProps,
    rng: &mut SmallRng,
) {
    for &(dx, dy) in &ADJ_TRANSFORM_NEIGHBORS8 {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        let neighbor_mat = sg.get(np).material;
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
                sg.set_cell(np, Cell::default());
            } else {
                let to_props = world.material_props(rule.to);
                let new_lifetime = if rule.to == material::STEAM {
                    rng.gen_range(36..72)
                } else {
                    World::initial_lifetime_for(rule.to, &to_props)
                };
                sg.set_cell(
                    np,
                    Cell {
                        material: rule.to,
                        flags: 0,
                        velocity: 0,
                        lifetime: new_lifetime,
                        variant: rng.gen_range(0..16),
                        scorch: 0,
                    },
                );
            }
            if rule.actor_lifetime_delta > 0 {
                let actor = sg.get(p);
                let nl = actor
                    .lifetime
                    .saturating_sub(rule.actor_lifetime_delta);
                sg.set_lifetime(p, nl);
            }
            break;
        }
    }
}

fn step_sand(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let props = world.material_props(cell.material);
    let free_falling = cell.flags & cell_flags::IS_FREE_FALLING != 0;

    if !free_falling {
        let below = Vec2i::new(p.x, p.y + 1);
        let below_cell = sg.get(below);
        let below_props = world.material_props(below_cell.material);
        if below_cell.material == material::EMPTY
            || (!below_props.inert()
                && vertical_down_allows_density_swap(props.phase(), below_props.phase())
                && props.density > below_props.density)
        {
            sg.set_flag(p, cell_flags::IS_FREE_FALLING);
        } else {
            if below_props.phase() == Phase::Liquid && rng.gen_ratio(1, 5) {
                let dir = if rng.gen_bool(0.5) { -1 } else { 1 };
                let side = Vec2i::new(p.x + dir, p.y);
                if sg.get(side).material == material::EMPTY {
                    let _ = sg.try_displace(world, p, side, MoveIntent::Lateral, rng);
                }
            }
            return;
        }
    }

    let new_vel = ((cell.velocity as i16) + props.acceleration() as i16).min(props.max_speed() as i16) as i8;
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
            if ncell.material != material::EMPTY
                && ncell.flags & cell_flags::IS_FREE_FALLING == 0
            {
                let nprops = world.material_props(ncell.material);
                if nprops.phase() == Phase::Solid && !nprops.inert() && nprops.inertial_resistance() < 255 {
                    let dislodge_chance = (255 - nprops.inertial_resistance()) as u32;
                    if rng.gen_ratio(dislodge_chance.max(1), 256) {
                        sg.set_flag(neighbor, cell_flags::IS_FREE_FALLING);
                    }
                }
            }
        }
    }
}

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

fn step_liquid(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let props = world.material_props(cell.material);
    if props.has_acid_corrosion() {
        acid_corrode_neighbors(sg, world, p, &props, rng);
    }
    if props.has_adjacent_influence() {
        let src_mat = cell.material;
        if liquid_adjacent_influence_pass(sg, world, p, src_mat, &props.adjacent_influence, rng) {
            return;
        }
    }

    let cell = sg.get(p);
    let props = world.material_props(cell.material);

    let below = Vec2i::new(p.x, p.y + 1);
    let below_cell = sg.get(below);
    let below_props = world.material_props(below_cell.material);
    let can_fall = below_cell.material == material::EMPTY
        || (!below_props.inert()
            && vertical_down_allows_density_swap(props.phase(), below_props.phase())
            && props.density > below_props.density);

    if !can_fall {
        if cell.velocity == 0 {
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

        let v = sg.get(p).velocity;
        let friction = liquid_supported_friction(&props);
        sg.set_velocity(p, (v - friction).max(0));
        return;
    }

    let new_vel = ((cell.velocity as i16) + props.acceleration() as i16).min(props.max_speed() as i16) as i8;
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

        let v = sg.get(current).velocity;
        let friction = liquid_supported_friction(&props);
        sg.set_velocity(current, (v - friction).max(0));
        return;
    }

    let below_current = Vec2i::new(current.x, current.y + 1);
    let below_cell = sg.get(below_current);
    let below_props = world.material_props(below_cell.material);
    if below_cell.material != material::EMPTY
        && (below_props.inert()
            || !vertical_down_allows_density_swap(props.phase(), below_props.phase())
            || props.density <= below_props.density)
    {
        if !step_liquid_supported_slide(sg, world, current, rng) {
            let v = sg.get(current).velocity;
            let friction = liquid_supported_friction(&props);
            sg.set_velocity(current, (v - friction).max(0));
        }
    }
}

/// While supported, slide toward the side whose column has void (empty or hole-below) closest
/// below this row. Tie-break: shallower `liquid_column_void_depth` then [`liquid_prefer_left_first`].
fn step_liquid_supported_slide(sg: &SimGrids, world: &World, from: Vec2i, rng: &mut SmallRng) -> bool {
    let mat = sg.get(from).material;
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
        if cell.material == material::EMPTY {
            return Some(dy);
        }
        let below = Vec2i::new(col_x, surface_y + dy + 1);
        if sg.index(below).is_none() {
            continue;
        }
        if sg.get(below).material == material::EMPTY {
            return Some(dy);
        }
    }
    None
}

fn acid_corrode_neighbors(sg: &SimGrids, world: &World, p: Vec2i, acid_props: &MaterialProps, rng: &mut SmallRng) {
    const CARDINAL: [(i32, i32); 4] = [(0, -1), (-1, 0), (1, 0), (0, 1)];
    let src = acid_props.acid_corrosion;
    if !src.is_active() {
        return;
    }
    let mut acid = sg.get(p);
    let corrosive_mat = acid.material;
    if corrosive_mat == material::EMPTY || acid.lifetime == 0 {
        return;
    }
    for &(dx, dy) in &CARDINAL {
        acid = sg.get(p);
        if acid.material != corrosive_mat || acid.lifetime == 0 {
            return;
        }
        let np = Vec2i::new(p.x + dx, p.y + dy);
        if sg.index(np).is_none() {
            continue;
        }
        let ncell = sg.get(np);
        if ncell.material == material::EMPTY {
            continue;
        }
        let nprops = world.material_props(ncell.material);
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
            sg.set_cell(np, Cell::default());
        } else {
            let cur_hp = if ncell.lifetime == 0 {
                nprops.corrosion_max_hp
            } else {
                ncell.lifetime
            };
            let new_hp = cur_hp.saturating_sub(src.neighbor_damage);
            if new_hp == 0 {
                sg.set_cell(np, Cell::default());
            } else {
                let mut c = ncell;
                c.lifetime = new_hp;
                sg.set_cell(np, c);
            }
        }

        let next_acid = acid.lifetime.saturating_sub(src.self_lifetime_cost);
        if next_acid == 0 {
            sg.set_cell(p, Cell::default());
            return;
        }
        sg.set_lifetime(p, next_acid);
    }
}

fn scan_lateral_target(sg: &SimGrids, from: Vec2i, dir: i32, max_dist: i32) -> Option<i32> {
    for i in 1..=max_dist {
        let p = Vec2i::new(from.x + dir * i, from.y);
        let cell = sg.get(p);
        if cell.material == material::EMPTY {
            return Some(i);
        }
        let below = Vec2i::new(p.x, p.y + 1);
        let below_cell = sg.get(below);
        if below_cell.material == material::EMPTY {
            return Some(i);
        }
    }
    None
}

fn step_gas(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let props = world.material_props(cell.material);
    let new_vel = ((cell.velocity as i16) + props.acceleration() as i16).min(props.max_speed() as i16) as i8;
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

fn step_ember(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng, explosions: &mut Vec<(Vec2i, i32)>) {
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

    let cell = sg.get(p);
    let ember_props = world.material_props(material::EMBER);
    let mut life = cell.lifetime;

    if life == 0 {
        sg.set_cell(
            p,
            Cell {
                material: material::SMOKE,
                flags: 0,
                velocity: 0,
                lifetime: rng.gen_range(18..52),
                variant: cell.variant,
                scorch: cell.scorch,
            },
        );
        return;
    }

    for &(dx, dy) in &NEIGHBORS8 {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        let nprops = world.material_props(sg.get(np).material);
        if !nprops.extinguishes_fire() {
            continue;
        }
        sg.set_cell(np, water_to_steam_cell(rng));
        let src = sg.get(p);
        if src.material != material::EMBER {
            return;
        }
        let nl = src.lifetime.saturating_sub(ADJ_ACTOR_WATER_QUENCH_LIFETIME);
        if nl == 0 {
            sg.set_cell(
                p,
                Cell {
                    material: material::SMOKE,
                    flags: 0,
                    velocity: 0,
                    lifetime: rng.gen_range(18..52),
                    variant: cell.variant,
                    scorch: cell.scorch,
                },
            );
            return;
        }
        sg.set_lifetime(p, nl);
    }

    life = sg.get(p).lifetime;

    let heat_factor = ember_props.ignitability as f32 / 255.0;
    if spread_burn_to_neighbors(
        sg,
        world,
        p,
        material::EMBER,
        heat_factor,
        life,
        rng,
        explosions,
    ) {
        return;
    }

    life = sg.get(p).lifetime;
    let rate = ember_props.consumption_rate.max(1) as u32;
    if rng.gen_ratio(rate, 256) {
        if life <= 1 {
            sg.set_cell(
                p,
                Cell {
                    material: material::SMOKE,
                    flags: 0,
                    velocity: 0,
                    lifetime: rng.gen_range(18..52),
                    variant: cell.variant,
                    scorch: cell.scorch,
                },
            );
            return;
        }
        sg.set_lifetime(p, life - 1);
    }

    try_neighbor_spawns(sg, world, p, &ember_props, rng);
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
fn flash_ignite_adjacent_from_burnout(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    explosions: &mut Vec<(Vec2i, i32)>,
) {
    const HEAT: f32 = 1.0;
    const CHILD_CAP: u8 = 72;
    let source_material = sg.get(p).material;
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
    if ncell.material == material::EMPTY {
        return;
    }
    let nprops = world.material_props(ncell.material);

    if nprops.extinguishes_fire() {
        sg.set_cell(np, water_to_steam_cell(rng));
        return;
    }

    if nprops.on_death_become != material::EMPTY && ncell.material == nprops.on_death_become {
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
        sg.set_cell(np, Cell::default());
        return;
    }

    if nprops.on_heat_become != material::EMPTY {
        sg.set_cell(
            np,
            Cell {
                material: nprops.on_heat_become,
                flags: 0,
                velocity: 0,
                lifetime: ncell.lifetime,
                variant: ncell.variant,
                scorch: ncell.scorch,
            },
        );
        return;
    }

    if nprops.fuel_mass > 0 {
        sg.set_cell(
            np,
            Cell {
                material: ncell.material,
                flags: ncell.flags | cell_flags::ON_FIRE,
                velocity: 0,
                lifetime: nprops.fuel_mass,
                variant: ncell.variant,
                scorch: ncell.scorch,
            },
        );
        return;
    }

    let (ignite_mat, ignite_life) = instant_heat_ignition_cell(&nprops, child_life_cap, rng);
    sg.set_cell(np, Cell {
        material: ignite_mat,
        flags: 0,
        velocity: 0,
        lifetime: ignite_life,
        variant: if ignite_mat == material::FIRE {
            0
        } else {
            ncell.variant
        },
        scorch: 0,
    });
}

/// Returns `true` if the source at `p` was extinguished (replaced with smoke).
fn spread_burn_to_neighbors(
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
        if ncell.material == material::EMPTY || ncell.material == source_material {
            continue;
        }

        let nprops = world.material_props(ncell.material);

        if nprops.extinguishes_fire() {
            if source_material == material::FIRE || source_material == material::LAVA {
                continue;
            }
            sg.set_cell(np, water_to_steam_cell(rng));
            let src = sg.get(p);
            if src.material != source_material {
                continue;
            }
            let nl = src.lifetime.saturating_sub(ADJ_ACTOR_WATER_QUENCH_LIFETIME);
            if nl == 0 {
                sg.set_cell(p, Cell {
                    material: material::SMOKE,
                    flags: 0,
                    velocity: 0,
                    lifetime: rng.gen_range(12..44),
                    variant: 0,
                    scorch: 0,
                });
                return true;
            }
            sg.set_lifetime(p, nl);
            continue;
        }

        if nprops.on_death_become != material::EMPTY && ncell.material == nprops.on_death_become {
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
            sg.set_cell(np, Cell::default());
            continue;
        }

        if nprops.on_heat_become != material::EMPTY {
            sg.set_cell(np, Cell {
                material: nprops.on_heat_become,
                flags: 0,
                velocity: 0,
                lifetime: ncell.lifetime,
                variant: ncell.variant,
                scorch: ncell.scorch,
            });
            continue;
        }

        if nprops.fuel_mass > 0 {
            sg.set_cell(
                np,
                Cell {
                    material: ncell.material,
                    flags: ncell.flags | cell_flags::ON_FIRE,
                    velocity: 0,
                    lifetime: nprops.fuel_mass,
                    variant: ncell.variant,
                    scorch: ncell.scorch,
                },
            );
            continue;
        }

        let (ignite_mat, ignite_life) = instant_heat_ignition_cell(&nprops, child_life_cap, rng);
        sg.set_cell(np, Cell {
            material: ignite_mat,
            flags: 0,
            velocity: 0,
            lifetime: ignite_life,
            variant: if ignite_mat == material::FIRE {
                0
            } else {
                ncell.variant
            },
            scorch: 0,
        });
    }
    false
}

fn step_fire(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng, explosions: &mut Vec<(Vec2i, i32)>) {
    let cell = sg.get(p);
    let fire_props = world.material_props(cell.material);
    let life = cell.lifetime;

    if life == 0 {
        let death_mat = if fire_props.on_death_become != material::EMPTY {
            fire_props.on_death_become
        } else {
            material::SMOKE
        };
        if death_mat == material::EMPTY {
            sg.set_cell(p, Cell::default());
        } else {
            sg.set_cell(p, Cell {
                material: death_mat,
                flags: 0,
                velocity: 0,
                lifetime: on_death_replacement_lifetime(&fire_props, rng),
                variant: 0,
                scorch: 0,
            });
        }
        return;
    }

    sg.set_lifetime(p, life - 1);

    let heat_factor = life as f32 / 255.0;
    let source_mat = cell.material;
    if spread_burn_to_neighbors(
        sg,
        world,
        p,
        source_mat,
        heat_factor,
        life,
        rng,
        explosions,
    ) {
        return;
    }

    let current = p;
    let up = Vec2i::new(current.x, current.y - 1);
    if sg.get(up).material == material::EMPTY {
        let up_cell = Cell {
            material: cell.material,
            flags: 0,
            velocity: 0,
            lifetime: life.saturating_sub(1),
            variant: 0,
            scorch: 0,
        };
        sg.set_cell(up, up_cell);
        sg.set_cell(p, Cell::default());
        return;
    }

    let drift = if rng.gen_bool(0.5) { -1 } else { 1 };
    let side_up = Vec2i::new(current.x + drift, current.y - 1);
    if sg.get(side_up).material == material::EMPTY {
        let moved_cell = Cell {
            material: cell.material,
            flags: 0,
            velocity: 0,
            lifetime: life.saturating_sub(1),
            variant: 0,
            scorch: 0,
        };
        sg.set_cell(side_up, moved_cell);
        sg.set_cell(p, Cell::default());
        return;
    }

    let side = Vec2i::new(current.x + drift, current.y);
    if sg.get(side).material == material::EMPTY {
        let moved_cell = Cell {
            material: cell.material,
            flags: 0,
            velocity: 0,
            lifetime: life.saturating_sub(1),
            variant: 0,
            scorch: 0,
        };
        sg.set_cell(side, moved_cell);
        sg.set_cell(p, Cell::default());
    }
}

fn step_smoke(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let life = cell.lifetime;

    if life == 0 {
        sg.set_cell(p, Cell::default());
        return;
    }
    sg.set_lifetime(p, life - 1);

    let props = world.material_props(cell.material);
    let new_vel = ((cell.velocity as i16) + props.acceleration() as i16).min(props.max_speed() as i16) as i8;
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

fn step_steam(sg: &SimGrids, world: &World, p: Vec2i, rng: &mut SmallRng) {
    let cell = sg.get(p);
    let life = cell.lifetime;

    if life == 0 {
        sg.set_cell(
            p,
            Cell {
                material: material::LIQUID,
                flags: 0,
                velocity: 0,
                lifetime: 0,
                variant: cell.variant,
                scorch: 0,
            },
        );
        return;
    }
    sg.set_lifetime(p, life - 1);

    let props = world.material_props(material::STEAM);
    let new_vel = ((cell.velocity as i16) + props.acceleration() as i16).min(props.max_speed() as i16) as i8;
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

#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub pos: (f32, f32),
    pub vel: (f32, f32),
    pub cell: Cell,
    pub lifetime: f32,
}

pub struct ParticleSim {
    particles: Vec<Particle>,
    gravity: f32,
}

impl ParticleSim {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            gravity: 50.0,
        }
    }

    pub fn spawn(&mut self, particle: Particle) {
        self.particles.push(particle);
    }

    pub fn step(&mut self, dt: f32, world: &mut World, events: &mut Vec<SimulationEvent>) {
        let mut survivors = Vec::with_capacity(self.particles.len());
        for mut particle in self.particles.drain(..) {
            particle.vel.1 += self.gravity * dt;
            particle.pos.0 += particle.vel.0 * dt;
            particle.pos.1 += particle.vel.1 * dt;
            particle.lifetime -= dt;

            let cell_pos = Vec2i::new(particle.pos.0.round() as i32, particle.pos.1.round() as i32);
            if particle.lifetime <= 0.0 || world.get_cell(cell_pos).material == material::EMPTY {
                if world.get_cell(cell_pos).material == material::EMPTY {
                    world.set_cell(cell_pos, particle.cell);
                } else {
                    survivors.push(particle);
                }
            } else {
                survivors.push(particle);
            }
            events.push(SimulationEvent::PixelEjectedToParticle { at: cell_pos });
        }
        self.particles = survivors;
    }

    pub fn cull_outside(&mut self, bounds: RectI) {
        self.particles.retain(|particle| {
            let p = Vec2i::new(particle.pos.0.round() as i32, particle.pos.1.round() as i32);
            bounds.contains(p)
        });
    }
}

pub fn deterministic_hash(world: &World) -> Option<u64> {
    let bounds: RectI = world.dirty_world_bounds()?;
    let mut hash: u64 = 1469598103934665603;
    for y in bounds.min.y..=bounds.max.y {
        for x in bounds.min.x..=bounds.max.x {
            let c = world.get_cell(Vec2i::new(x, y));
            hash ^= c.material as u64;
            hash = hash.wrapping_mul(1099511628211);
            hash ^= c.flags as u64;
            hash = hash.wrapping_mul(1099511628211);
            hash ^= c.velocity as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
    }
    Some(hash)
}
