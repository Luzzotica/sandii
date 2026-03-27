use rand::rngs::SmallRng;
use rand::Rng;
use rand::SeedableRng;
#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::bresenham::bresenham_line;
use crate::world::{
    cell_flags, material, Cell, ChunkCoord, MaterialProps, MaterialRule, Phase, ReactionOutcome,
    RectI, Vec2i, World,
};
use crate::SimulationEvent;

mod engines;
mod step_pixel;
mod steps;
mod thermal;

pub(crate) use thermal::{check_phase_transition, step_temperature};

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

/// Empty cell adjacent to burning fuel that should receive [`material::FIRE`] after the grid step
/// ([`crate::Simulation::step`] writes the cell directly so fire never spawns inside fuel or beyond neighbors).
#[derive(Debug, Clone)]
pub struct FlameSpawn {
    pub pos: Vec2i,
    pub temperature_k: u16,
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
    /// Collected during the last [`Self::step_world`]; drain with [`Self::take_flame_spawns`].
    pending_flame_spawns: Vec<FlameSpawn>,
    /// Detonations from the grid step; [`crate::Simulation::step`] applies via [`crate::Simulation::apply_explosion`].
    pending_grid_explosions: Vec<(Vec2i, i32)>,
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
            pending_flame_spawns: Vec::new(),
            pending_grid_explosions: Vec::new(),
        }
    }

    pub fn take_flame_spawns(&mut self) -> Vec<FlameSpawn> {
        std::mem::take(&mut self.pending_flame_spawns)
    }

    pub fn take_pending_grid_explosions(&mut self) -> Vec<(Vec2i, i32)> {
        std::mem::take(&mut self.pending_grid_explosions)
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
            self.pending_grid_explosions.extend(state.pending_explosions);
        }
    }

    pub fn step_world(&mut self, world: &mut World, rng: &mut SmallRng) {
        if self.debug_full_world_single_pass {
            self.tick = self.tick.saturating_add(1);
            let reseed_tick_rng = matches!(self.mode, SchedulerMode::SingleThreadSeeded)
                || self.debug_full_world_single_pass;
            if reseed_tick_rng {
                *rng = SmallRng::seed_from_u64(self.seed ^ self.tick);
            }
            world.advance_awake_flags();
            world.prepare_sim();
            let sg = SimGrids::from_world(world);
            self.pending_flame_spawns.clear();
            let (all_explosions, flames) = process_world_full_pass(&sg, world, rng, self.tick);
            self.pending_flame_spawns = flames;
            world.finish_sim();
            self.pending_grid_explosions.extend(all_explosions);
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

        self.pending_flame_spawns.clear();
        let mut all_explosions: Vec<(Vec2i, i32)> = Vec::new();

        let pass_order: [usize; 4] = if (self.tick & 1) == 0 {
            [0, 1, 2, 3]
        } else {
            [2, 3, 0, 1]
        };
        for pass in pass_order {
            let coords: Vec<ChunkCoord> = world
                .active_chunk_coords_for_pass(pass)
                .into_iter()
                .collect();
            match self.mode {
                SchedulerMode::SingleThreadSeeded => {
                    for coord in coords {
                        let sg = SimGrids::from_world(world);
                        let (explosions, flames) =
                            process_chunk(&sg, world, coord, pass as u8, rng, self.tick);
                        all_explosions.extend(explosions);
                        self.pending_flame_spawns.extend(flames);
                    }
                }
                SchedulerMode::ThreadPool => {
                    // Parallelism is **per chunk** within this pass: each `ChunkCoord` is one rayon task.
                    // A single 64×64 chunk is still stepped on one thread (cells are not split across threads).
                    let sg = SimGrids::from_world(world);
                    let tick = self.tick;
                    let seed = self.seed;
                    let pass_u8 = pass as u8;
                    #[cfg(feature = "parallel")]
                    {
                        let results: Vec<(Vec<(Vec2i, i32)>, Vec<FlameSpawn>)> = coords
                            .par_iter()
                            .map(|coord| {
                                let mut local_rng = SmallRng::seed_from_u64(
                                    seed ^ tick ^ ((coord.x as u64) << 32) ^ coord.y as u64,
                                );
                                process_chunk(&sg, world, *coord, pass_u8, &mut local_rng, tick)
                            })
                            .collect();
                        for (explosions, flames) in results {
                            all_explosions.extend(explosions);
                            self.pending_flame_spawns.extend(flames);
                        }
                    }
                    #[cfg(not(feature = "parallel"))]
                    {
                        for coord in coords {
                            let mut local_rng = SmallRng::seed_from_u64(
                                seed ^ tick ^ ((coord.x as u64) << 32) ^ coord.y as u64,
                            );
                            let (explosions, flames) =
                                process_chunk(&sg, world, coord, pass_u8, &mut local_rng, tick);
                            all_explosions.extend(explosions);
                            self.pending_flame_spawns.extend(flames);
                        }
                    }
                }
            }
        }

        world.finish_sim();

        self.pending_grid_explosions.extend(all_explosions);
    }

    fn step_world_one_chunk(&mut self, world: &mut World, rng: &mut SmallRng) {
        if self.chunk_step.is_none() {
            self.tick = self.tick.saturating_add(1);
            self.pending_flame_spawns.clear();
            let reseed_tick_rng =
                matches!(self.mode, SchedulerMode::SingleThreadSeeded) || self.debug_chunk_step;
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
        let (explosions, flames) = match self.mode {
            SchedulerMode::SingleThreadSeeded => {
                process_chunk(&sg, world, coord, pass, rng, self.tick)
            }
            SchedulerMode::ThreadPool => {
                // Chunk-step mode runs one chunk per `step_world`; there is nothing for rayon to fan out.
                let mut local_rng = SmallRng::seed_from_u64(
                    self.seed ^ self.tick ^ ((coord.x as u64) << 32) ^ coord.y as u64,
                );
                process_chunk(&sg, world, coord, pass, &mut local_rng, self.tick)
            }
        };
        state.pending_explosions.extend(explosions);
        self.pending_flame_spawns.extend(flames);
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
            self.pending_grid_explosions.extend(explosions);
        } else {
            // `World::get_cell` reads the read buffer; sim writes the write buffer until `finish_sim`
            // swaps. Without committing after each chunk, rendering (and any read-buffer logic) stays
            // stuck on the pre-tick read grid for the whole multi-chunk tick — looks like the sim froze.
            world.finish_sim();
            world.prepare_sim();
        }
    }
}

fn process_chunk(
    sg: &SimGrids,
    world: &World,
    coord: ChunkCoord,
    pass: u8,
    rng: &mut SmallRng,
    sim_tick: u64,
) -> (Vec<(Vec2i, i32)>, Vec<FlameSpawn>) {
    let bounds = world.bounds_for_chunk(coord);

    let mut explosions = Vec::new();
    let mut flame_spawns = Vec::new();
    for y in (bounds.min.y..=bounds.max.y).rev() {
        let reverse_x = rng.gen_bool(0.5);
        if reverse_x {
            for x in (bounds.min.x..=bounds.max.x).rev() {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, pass);
                step_pixel::step_pixel(
                    sg,
                    world,
                    p,
                    rng,
                    &mut explosions,
                    &mut flame_spawns,
                    sim_tick,
                );
            }
        } else {
            for x in bounds.min.x..=bounds.max.x {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, pass);
                step_pixel::step_pixel(
                    sg,
                    world,
                    p,
                    rng,
                    &mut explosions,
                    &mut flame_spawns,
                    sim_tick,
                );
            }
        }
    }
    (explosions, flame_spawns)
}

/// Single-threaded: every cell in the loaded grid once, bottom-to-top (same row order as `process_chunk`).
/// Pass id `4` distinguishes debug overlay from checkerboard passes 0–3.
fn process_world_full_pass(
    sg: &SimGrids,
    world: &World,
    rng: &mut SmallRng,
    sim_tick: u64,
) -> (Vec<(Vec2i, i32)>, Vec<FlameSpawn>) {
    let Some(bounds) = world.dirty_world_bounds() else {
        return (Vec::new(), Vec::new());
    };
    const PASS: u8 = 4;
    let mut explosions = Vec::new();
    let mut flame_spawns = Vec::new();
    for y in (bounds.min.y..=bounds.max.y).rev() {
        let reverse_x = rng.gen_bool(0.5);
        if reverse_x {
            for x in (bounds.min.x..=bounds.max.x).rev() {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, PASS);
                step_pixel::step_pixel(
                    sg,
                    world,
                    p,
                    rng,
                    &mut explosions,
                    &mut flame_spawns,
                    sim_tick,
                );
            }
        } else {
            for x in bounds.min.x..=bounds.max.x {
                let p = Vec2i::new(x, y);
                sg.stamp_debug_pass(p, PASS);
                step_pixel::step_pixel(
                    sg,
                    world,
                    p,
                    rng,
                    &mut explosions,
                    &mut flame_spawns,
                    sim_tick,
                );
            }
        }
    }
    (explosions, flame_spawns)
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
        let debug_pass = if debug_pass_enabled {
            world.debug_pass_ptr()
        } else {
            std::ptr::null_mut()
        };
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
                unsafe {
                    *self.debug_pass.add(i) = pass;
                }
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
            .unwrap_or(Cell::new().with_material(material::STONE))
    }

    #[inline]
    fn was_moved(&self, p: Vec2i) -> bool {
        self.index(p)
            .map(|i| unsafe { *self.read.add(i) != *self.write.add(i) })
            .unwrap_or(false)
    }

    fn set_velocity(&self, p: Vec2i, vel: i8) {
        if let Some(i) = self.index(p) {
            unsafe {
                (*self.write.add(i)).set_velocity_x(vel);
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
                (*self.write.add(i)).set_lifetime(lifetime);
            }
            self.wake_at(p);
        }
    }

    fn set_flag(&self, p: Vec2i, flag: u8) {
        if let Some(i) = self.index(p) {
            unsafe {
                (*self.write.add(i)).or_flags(flag);
            }
            self.wake_at(p);
        }
    }

    fn clear_flag(&self, p: Vec2i, flag: u8) {
        if let Some(i) = self.index(p) {
            unsafe {
                (*self.write.add(i)).clear_flag_bits(flag);
            }
            self.wake_at(p);
        }
    }

    fn set_temperature(&self, p: Vec2i, temp: u16) {
        if let Some(i) = self.index(p) {
            unsafe {
                (*self.write.add(i)).set_temperature(temp);
            }
            self.wake_at(p);
        }
    }

    pub(crate) fn wake_at(&self, p: Vec2i) {
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
                    unsafe {
                        *self.awake_next.add(idx) = true;
                    }
                }
            }
        }
    }

    pub(crate) fn try_displace(
        &self,
        world: &World,
        from: Vec2i,
        to: Vec2i,
        intent: MoveIntent,
        rng: &mut SmallRng,
    ) -> bool {
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
        if from_cell.material() == material::EMPTY {
            return false;
        }
        if from_cell.has_flag(cell_flags::RIGID_PIXEL) || to_cell.has_flag(cell_flags::RIGID_PIXEL)
        {
            return false;
        }

        let from_props = world.material_props(from_cell.material());
        let to_props = world.material_props(to_cell.material());
        let from_rule = world.material_rule(from_cell.material());
        let to_rule = world.material_rule(to_cell.material());
        if from_props.inert() || to_props.inert() {
            return false;
        }

        let reaction = world.reaction(from_cell.material(), to_cell.material());
        if let ReactionOutcome::Transform(from_to, to_to) = reaction {
            unsafe {
                (*self.write.add(to_idx)).set_material(from_to);
                (*self.write.add(from_idx)).set_material(to_to);
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

        if !can_displace(
            from_props, from_rule, to_cell, to_props, to_rule, intent, rng,
        ) {
            return false;
        }

        unsafe {
            *self.write.add(to_idx) = from_cell;
            *self.write.add(from_idx) = if to_cell.material() == material::EMPTY {
                Cell::new()
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
pub(crate) enum MoveIntent {
    VerticalDown,
    VerticalUp,
    Lateral,
}

/// Density-driven vertical **swap** is for liquids mixing/stacking and for solids sinking through
/// liquid or gas. It is not used for liquid-into-solid (e.g. lava should ride on sand, not push it up).
#[inline]
pub(crate) fn vertical_down_allows_density_swap(from: Phase, to: Phase) -> bool {
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
    if to_cell.material() == material::EMPTY {
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
            // Gas below, gas above: lighter always trades places with heavier (buoyancy). Using the same
            // density-diff probability as liquids would clamp tiny diffs to ~5% (see `diff.clamp` below),
            // so smoke/fire barely swapped in practice.
            if from_props.phase() == Phase::Gas && to_props.phase() == Phase::Gas {
                return true;
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

#[inline]
pub(crate) fn is_burning(cell: Cell, props: &MaterialProps) -> bool {
    // Hot gas / flame particles are not "on fire" (no [`cell_flags::ON_FIRE`] semantics).
    if cell.material() == material::FIRE {
        return false;
    }
    if props.fuel_mass == 0 || cell.lifetime() == 0 {
        return false;
    }
    // Lava-style molten fuel: no autoignition threshold — always "burning" while fuel remains.
    if props.autoignition_temperature == 0 {
        return true;
    }
    cell.has_flag(cell_flags::ON_FIRE)
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

#[inline]
fn particle_cell_coord(pos: (f32, f32)) -> Vec2i {
    Vec2i::new(pos.0.round() as i32, pos.1.round() as i32)
}

/// When the rounded end cell is empty, the segment may still pass through solid cells (tunneling).
/// Returns the last empty cell along the Bresenham line before the first solid encountered after
/// entering empty space (or the end cell if the path is clear). Leading solids (e.g. spawn inside
/// fuel) are skipped until the path first reaches empty air.
fn resolve_particle_deposit_cell(world: &World, start: Vec2i, end: Vec2i) -> Option<Vec2i> {
    let mut emerged = false;
    let mut candidate: Option<Vec2i> = None;
    for p in bresenham_line(start, end) {
        let m = world.get_cell(p).material();
        if m == material::EMPTY {
            candidate = Some(p);
            emerged = true;
        } else if emerged {
            return candidate;
        }
    }
    candidate
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
            let prev_pos = particle.pos;
            particle.vel.1 += self.gravity * dt;
            particle.pos.0 += particle.vel.0 * dt;
            particle.pos.1 += particle.vel.1 * dt;
            particle.lifetime -= dt;

            let start_cell = particle_cell_coord(prev_pos);
            let end_cell = particle_cell_coord(particle.pos);

            // Same rule as before: only deposit when the rounded end cell is empty (lifetime is
            // irrelevant for that — matches previous `inner` branch).
            if world.get_cell(end_cell).material() == material::EMPTY {
                if let Some(p) = resolve_particle_deposit_cell(world, start_cell, end_cell) {
                    debug_assert_eq!(world.get_cell(p).material(), material::EMPTY);
                    world.set_cell(p, particle.cell);
                } else {
                    survivors.push(particle);
                }
            } else {
                survivors.push(particle);
            }
            events.push(SimulationEvent::PixelEjectedToParticle { at: end_cell });
        }
        self.particles = survivors;
    }

    #[cfg(test)]
    pub(crate) fn set_gravity_for_test(&mut self, g: f32) {
        self.gravity = g;
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
            hash ^= c.material() as u64;
            hash = hash.wrapping_mul(1099511628211);
            hash ^= c.flags() as u64;
            hash = hash.wrapping_mul(1099511628211);
            hash ^= c.velocity_x() as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
    }
    Some(hash)
}

#[cfg(test)]
mod particle_step_tests {
    use super::*;
    use crate::world::{material, Cell, World, CHUNK_SIZE};

    #[test]
    fn particle_deposit_clamped_before_wall_not_past_it() {
        let mut world = World::new(CHUNK_SIZE, 512);
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        world.set_solid_bounds(bounds);
        world.set_cell(
            Vec2i::new(10, 10),
            Cell::new().with_material(material::STONE),
        );

        let mut particles = ParticleSim::new();
        particles.set_gravity_for_test(0.0);
        let fire_cell = Cell::new()
            .with_material(material::FIRE)
            .with_temperature(800)
            .with_lifetime(72);
        particles.spawn(Particle {
            pos: (10.4, 11.4),
            vel: (0.0, -180.0),
            cell: fire_cell,
            lifetime: 1.0,
        });
        let mut events = Vec::new();
        particles.step(1.0 / 60.0, &mut world, &mut events);

        assert_ne!(world.get_cell(Vec2i::new(10, 8)).material(), material::FIRE);
        assert_eq!(
            world.get_cell(Vec2i::new(10, 11)).material(),
            material::FIRE
        );
        assert_eq!(
            world.get_cell(Vec2i::new(10, 10)).material(),
            material::STONE
        );
    }
}
