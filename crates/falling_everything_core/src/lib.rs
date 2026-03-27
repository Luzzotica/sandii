pub mod bresenham;
pub mod cell64;
pub mod explosion;
pub mod materials;

pub use explosion::{base_strength_for_radius, ExplosionParams, ExplosionSpawn};
pub use sim::FlameSpawn;
pub mod render;
pub mod rigid;
pub mod sim;
pub mod world;
pub mod worldgen;

pub use rigid::STRUCTURAL_STRESS_SCALE;
pub use world::MaterialMotion;

use rand::rngs::SmallRng;
use rand::Rng;
use rand::SeedableRng;
use web_time::Instant;

use crate::cell64::MAX_TEMPERATURE;
use render::{DirtyChunkView, PixelRegion};
use rigid::{RigidBodySpec, RigidBridge};
use sim::{ParticleSim, Scheduler, SchedulerMode};
use world::{
    material, Cell, MaterialId, MaterialProps, MaterialRule, ReactionOutcome, RectI, Vec2i, World,
    CHUNK_SIZE,
};

#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub chunk_size: i32,
    pub region_size: i32,
    pub seed: u64,
    pub deterministic: bool,
    /// Single-threaded full-grid one pass per tick (no checkerboard, no rayon). For debugging vs multithreaded seams.
    pub debug_full_world_single_pass: bool,
    /// Ambient temperature for thermal relaxation (Kelvin).
    pub ambient_temperature_k: u16,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            chunk_size: CHUNK_SIZE,
            region_size: 512,
            seed: 1,
            deterministic: true,
            debug_full_world_single_pass: false,
            ambient_temperature_k: 293,
        }
    }
}

#[derive(Debug)]
pub enum SimulationEvent {
    PixelEjectedToParticle { at: Vec2i },
    RegionLoaded { region: Vec2i },
    RegionSaved { region: Vec2i },
}

pub struct Simulation {
    world: World,
    particles: ParticleSim,
    rigid: RigidBridge,
    scheduler: Scheduler,
    rng: SmallRng,
    events: Vec<SimulationEvent>,
    fixed_dt: f32,
    accumulator: f32,
    max_substeps: u32,
    /// With chunk-step on: run at most one grid substep every this many frames (1 = every frame).
    chunk_step_stride_frames: u32,
    chunk_step_stride_counter: u32,
    /// Master toggle for pass-pixel tint + pass-batch grid (sandbox **D**).
    debug_views_enabled: bool,
    last_stats: SimulationStats,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SimulationStats {
    pub substeps: u32,
    pub sim_ms: f32,
    pub active_chunks: usize,
    pub sleeping_chunks: usize,
}

impl Simulation {
    pub fn new(config: SimulationConfig) -> Self {
        assert_eq!(
            config.chunk_size, CHUNK_SIZE,
            "Simulation chunk size is fixed at {}; got {}",
            CHUNK_SIZE, config.chunk_size
        );
        let scheduler_mode = if config.deterministic {
            SchedulerMode::SingleThreadSeeded
        } else {
            SchedulerMode::ThreadPool
        };

        let mut scheduler = Scheduler::new(scheduler_mode, config.seed);
        scheduler.set_debug_full_world_single_pass(config.debug_full_world_single_pass);

        let mut world = World::new(config.chunk_size, config.region_size);
        world.set_ambient_temperature_k(config.ambient_temperature_k);
        Self {
            world,
            particles: ParticleSim::new(),
            rigid: RigidBridge::new(),
            scheduler,
            rng: SmallRng::seed_from_u64(config.seed),
            events: Vec::new(),
            fixed_dt: 1.0 / 60.0,
            accumulator: 0.0,
            max_substeps: 4,
            chunk_step_stride_frames: 8,
            chunk_step_stride_counter: 0,
            debug_views_enabled: false,
            last_stats: SimulationStats::default(),
        }
    }

    pub fn set_storage_dir<P: Into<std::path::PathBuf>>(&mut self, dir: P) {
        self.world.set_storage_dir(dir.into());
    }

    pub fn set_focus(&mut self, focus: Vec2i) {
        let stream_events = self.world.set_focus(focus);
        self.events
            .extend(stream_events.into_iter().map(|event| match event {
                world::WorldEvent::RegionLoaded(region) => SimulationEvent::RegionLoaded { region },
                world::WorldEvent::RegionSaved(region) => SimulationEvent::RegionSaved { region },
            }));
    }

    fn collect_rigid_hits_paint_disk(
        world: &World,
        center: Vec2i,
        radius: i32,
    ) -> Vec<(Vec2i, u32)> {
        let mut out = Vec::new();
        let r2 = radius * radius;
        for y in (center.y - radius)..=(center.y + radius) {
            for x in (center.x - radius)..=(center.x + radius) {
                let dx = x - center.x;
                let dy = y - center.y;
                if dx * dx + dy * dy <= r2 {
                    let p = Vec2i::new(x, y);
                    if let Some(id) = world.get_rigid_id(p) {
                        out.push((p, id));
                    }
                }
            }
        }
        out
    }

    fn collect_rigid_hits_paint_rect(world: &World, min: Vec2i, max: Vec2i) -> Vec<(Vec2i, u32)> {
        let mut out = Vec::new();
        let x0 = min.x.min(max.x);
        let x1 = min.x.max(max.x);
        let y0 = min.y.min(max.y);
        let y1 = min.y.max(max.y);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Vec2i::new(x, y);
                if let Some(id) = world.get_rigid_id(p) {
                    out.push((p, id));
                }
            }
        }
        out
    }

    pub fn paint_circle(&mut self, center: Vec2i, radius: i32, material: MaterialId) {
        // Erasing with EMPTY skips immediate carve so `check_splits` can see gaps and split bodies.
        let hits = if material != world::material::EMPTY {
            Self::collect_rigid_hits_paint_disk(&self.world, center, radius)
        } else {
            Vec::new()
        };
        self.world.paint_circle(center, radius, material);
        if !hits.is_empty() {
            self.rigid
                .carve_dynamic_bodies_at_world_cells(&mut self.world, &hits);
            self.rigid.rebuild_dirty_dynamic_colliders();
        }
    }

    pub fn paint_rect_filled(&mut self, min: Vec2i, max: Vec2i, material: MaterialId) {
        let hits = if material != world::material::EMPTY {
            Self::collect_rigid_hits_paint_rect(&self.world, min, max)
        } else {
            Vec::new()
        };
        self.world.paint_rect_filled(min, max, material);
        if !hits.is_empty() {
            self.rigid
                .carve_dynamic_bodies_at_world_cells(&mut self.world, &hits);
            self.rigid.rebuild_dirty_dynamic_colliders();
        }
    }

    /// Add `delta` to cell temperatures in a disk (Kelvin). Does not clear rigid-body IDs.
    pub fn adjust_temperature_disk(&mut self, center: Vec2i, radius: i32, delta: i32) {
        self.world.adjust_temperature_disk(center, radius, delta);
    }

    /// Add `delta` to cell temperatures in a filled axis-aligned rectangle (Kelvin).
    pub fn adjust_temperature_rect_filled(&mut self, min: Vec2i, max: Vec2i, delta: i32) {
        self.world.adjust_temperature_rect_filled(min, max, delta);
    }

    #[inline]
    pub fn ambient_temperature_k(&self) -> u16 {
        self.world.ambient_temperature_k()
    }

    pub fn set_ambient_temperature_k(&mut self, k: u16) {
        self.world.set_ambient_temperature_k(k);
    }

    pub fn cell(&self, p: Vec2i) -> Cell {
        self.world.get_cell(p)
    }

    pub fn paint_cell(&mut self, p: Vec2i, cell: Cell) {
        let hit = if cell.material() != world::material::EMPTY {
            self.world.get_rigid_id(p).map(|id| (p, id))
        } else {
            None
        };
        self.world.set_cell(p, cell);
        if let Some((wp, id)) = hit {
            self.rigid
                .carve_dynamic_bodies_at_world_cells(&mut self.world, &[(wp, id)]);
            self.rigid.rebuild_dirty_dynamic_colliders();
        }
    }

    /// Bresenham chord explosion (shuffled rays, center-out). Carves dynamic rigid voxels in the blast disk
    /// like [`Self::paint_circle`] with [`world::material::EMPTY`], then refreshes colliders.
    pub fn apply_explosion(&mut self, params: crate::explosion::ExplosionParams) {
        let center = params.center;
        let radius = params.radius;
        let hits = Self::collect_rigid_hits_paint_disk(&self.world, center, radius);
        crate::explosion::apply(&mut self.world, &mut self.rng, params);
        if !hits.is_empty() {
            self.rigid
                .carve_dynamic_bodies_at_world_cells(&mut self.world, &hits);
            self.rigid.rebuild_dirty_dynamic_colliders();
        }
        self.mark_rigid_colliders_stale();
    }

    /// Paints a thick brush along the integer Bresenham line from `a` to `b` (inclusive).
    /// Use when the pointer jumps between frames so no gaps appear in the stroke.
    pub fn paint_line_brush(
        &mut self,
        a: Vec2i,
        b: Vec2i,
        brush_radius: i32,
        material: MaterialId,
    ) {
        for p in crate::bresenham::bresenham_line(a, b) {
            self.paint_circle(p, brush_radius, material);
        }
    }

    pub fn spawn_rigid_body_rect(&mut self, min: Vec2i, max: Vec2i, material: MaterialId) -> u32 {
        self.rigid
            .spawn_from_world_rect(&mut self.world, RigidBodySpec { min, max, material })
    }

    pub fn spawn_rigid_body_from_pixels(
        &mut self,
        positions: &[Vec2i],
        material: MaterialId,
    ) -> u32 {
        self.rigid
            .spawn_from_pixels(&mut self.world, positions, material)
    }

    pub fn spawn_rigid_body_circle(
        &mut self,
        center: Vec2i,
        radius: i32,
        material: MaterialId,
    ) -> u32 {
        self.spawn_rigid_body_circle_with_temp(center, radius, material, None)
    }

    pub fn spawn_rigid_body_circle_with_temp(
        &mut self,
        center: Vec2i,
        radius: i32,
        material: MaterialId,
        temp_override: Option<u16>,
    ) -> u32 {
        let r2 = radius * radius;
        let positions: Vec<Vec2i> = (center.y - radius..=center.y + radius)
            .flat_map(|y| {
                (center.x - radius..=center.x + radius).filter_map(move |x| {
                    let dx = x - center.x;
                    let dy = y - center.y;
                    if dx * dx + dy * dy <= r2 {
                        Some(Vec2i::new(x, y))
                    } else {
                        None
                    }
                })
            })
            .collect();
        self.rigid.spawn_from_circle_with_temp(
            &mut self.world,
            &positions,
            material,
            radius as f32,
            temp_override,
        )
    }

    /// Rebuild dynamic rigid colliders on the next [`Self::step`] for all bodies (e.g. after upgrading meshing).
    pub fn mark_rigid_colliders_stale(&mut self) {
        self.rigid.mark_all_dynamic_colliders_stale();
    }

    fn flush_pending_grid_explosions(&mut self) {
        for (center, radius) in self.scheduler.take_pending_grid_explosions() {
            self.apply_explosion(ExplosionParams::gameplay_incendiary(center, radius));
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.rigid.rebuild_dirty_dynamic_colliders();
        self.rigid.record_positions(&mut self.world);
        self.scheduler.step_world(&mut self.world, &mut self.rng);
        self.flush_pending_grid_explosions();
        for spawn in self.scheduler.take_flame_spawns() {
            let fp = self.world.material_props(material::FIRE);
            let lo = fp.on_death_lifetime_lo.max(1);
            let hi = fp.on_death_lifetime_hi.max(lo.saturating_add(1));
            let life = self.rng.gen_range(lo..hi).min(255);
            let fire_cell = Cell::new()
                .with_material(material::FIRE)
                .with_temperature(spawn.temperature_k.min(MAX_TEMPERATURE))
                .with_lifetime(life);
            // Place fire only in the chosen empty neighbor (see [`FlameSpawn`]). Do not spawn a flying
            // particle from the fuel cell — that put fire deep inside solids or far from the wood after
            // gravity/velocity, and even neighbor-spawned particles fell out of the adjacent cell same frame.
            let p = spawn.pos;
            if self.world.get_cell(p).material() == material::EMPTY {
                self.world.set_cell(p, fire_cell);
            }
        }
        self.world.prune_rigid_ids_for_empty_cells();
        let loose_hits = self.world.collect_liquid_gas_rigid_hits();
        if !loose_hits.is_empty() {
            self.rigid
                .carve_dynamic_bodies_at_world_cells(&mut self.world, &loose_hits);
            for (p, _) in &loose_hits {
                self.world.clear_rigid_id(*p);
            }
            self.rigid.rebuild_dirty_dynamic_colliders();
        }
        self.rigid.check_splits(&mut self.world);
        let physics_dirty = self.world.take_physics_dirty_chunks();
        self.rigid
            .rebuild_static_colliders(&self.world, &physics_dirty);
        self.rigid.step(dt);
        self.rigid.sync_pixels_to_physics(&mut self.world);
        self.particles.step(dt, &mut self.world, &mut self.events);
        self.world.finish_frame();
    }

    pub fn despawn_outside(&mut self, bounds: RectI) {
        self.rigid.cull_outside(&mut self.world, bounds);
        self.particles.cull_outside(bounds);
        self.world.clear_outside_rect(bounds);
    }

    pub fn set_solid_bounds(&mut self, bounds: RectI) {
        self.rigid.clear_all_static_colliders();
        self.rigid.remove_world_border_colliders();
        self.world.set_solid_bounds(bounds);
        self.world.mark_all_chunks_physics_dirty();
        self.rigid.set_world_border_colliders(bounds);
    }

    /// Re-center the simulation grid around `camera_center` if the camera has drifted
    /// far enough from the current grid center. The grid is sized at 3x the screen
    /// dimensions to provide a one-screen buffer in every direction.
    pub fn relocate_if_needed(&mut self, camera_center: Vec2i, screen_w: i32, screen_h: i32) {
        let origin = self.world.grid_origin();
        let gw = self.world.grid_width();
        let gh = self.world.grid_height();
        let grid_center = Vec2i::new(origin.x + gw / 2, origin.y + gh / 2);

        let dx = (camera_center.x - grid_center.x).abs();
        let dy = (camera_center.y - grid_center.y).abs();

        if dx > screen_w / 2 || dy > screen_h / 2 {
            let half_w = screen_w * 3 / 2;
            let half_h = screen_h * 3 / 2;
            if self.world.relocate_around(camera_center, half_w, half_h) {
                self.rigid.clear_all_static_colliders();
            }
        }
    }

    pub fn grid_origin(&self) -> Vec2i {
        self.world.grid_origin()
    }

    pub fn grid_width(&self) -> i32 {
        self.world.grid_width()
    }

    pub fn grid_height(&self) -> i32 {
        self.world.grid_height()
    }

    pub fn set_material_props(&mut self, id: MaterialId, props: MaterialProps) {
        self.world.set_material_props(id, props);
    }

    pub fn set_material_rule(&mut self, id: MaterialId, rule: MaterialRule) {
        self.world.set_material_rule(id, rule);
    }

    pub fn set_reaction(&mut self, from: MaterialId, to: MaterialId, reaction: ReactionOutcome) {
        self.world.set_reaction(from, to, reaction);
    }

    pub fn get_dirty_chunks(&self) -> Vec<DirtyChunkView> {
        render::get_dirty_chunks(&self.world)
    }

    pub fn copy_rgba_for_region(&self, rect: RectI) -> PixelRegion {
        render::copy_rgba_for_region(&self.world, Some(&self.rigid), rect)
    }

    pub fn copy_argb32_for_region(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region(&self.world, Some(&self.rigid), rect)
    }

    pub fn copy_argb32_for_region_chunk_step_viz(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region_chunk_step_viz(&self.world, Some(&self.rigid), rect)
    }

    pub fn copy_argb32_for_region_pass_batch_viz(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region_pass_batch_viz(&self.world, Some(&self.rigid), rect)
    }

    pub fn copy_debug_argb32_for_region(&self, rect: RectI) -> Vec<u32> {
        render::copy_debug_argb32_for_region(&self.world, rect)
    }

    pub fn copy_argb32_for_region_all_debug_views(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region_all_debug_views(&self.world, Some(&self.rigid), rect)
    }

    pub fn debug_collider_lines(&self) -> Vec<[(f32, f32); 2]> {
        self.rigid.debug_collider_lines()
    }

    /// Enables per-pixel pass tint and pass-batch chunk grid together (sandbox **D**).
    pub fn set_debug_views_enabled(&mut self, enabled: bool) {
        self.debug_views_enabled = enabled;
        self.world.set_debug_pass_enabled(enabled);
        self.world.set_debug_pass_batch_outlines(enabled);
    }

    pub fn debug_views_enabled(&self) -> bool {
        self.debug_views_enabled
    }

    pub fn set_debug_pass_enabled(&mut self, enabled: bool) {
        self.world.set_debug_pass_enabled(enabled);
    }

    pub fn debug_pass_enabled(&self) -> bool {
        self.world.debug_pass_enabled()
    }

    pub fn set_debug_pass_batch_outlines(&mut self, enabled: bool) {
        self.world.set_debug_pass_batch_outlines(enabled);
    }

    pub fn debug_pass_batch_outlines(&self) -> bool {
        self.world.debug_pass_batch_outlines()
    }

    pub fn copy_palette_indices_for_region(&self, rect: RectI) -> Vec<u16> {
        render::copy_palette_indices_for_region(&self.world, rect)
    }

    pub fn drain_events(&mut self) -> Vec<SimulationEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn set_parallel(&mut self, enabled: bool) {
        let mode = if enabled {
            SchedulerMode::ThreadPool
        } else {
            SchedulerMode::SingleThreadSeeded
        };
        self.scheduler.set_mode(mode);
    }

    pub fn is_parallel(&self) -> bool {
        self.scheduler.mode() == SchedulerMode::ThreadPool
    }

    /// When enabled, the grid is stepped in **one** single-threaded full-world pass (no 4-pass checkerboard, no `rayon`).
    /// `ThreadPool` mode is ignored for stepping until this is turned off.
    pub fn set_debug_full_world_single_pass(&mut self, enabled: bool) {
        if enabled {
            self.scheduler.abort_debug_chunk_step(&mut self.world);
            self.flush_pending_grid_explosions();
            self.scheduler.set_debug_chunk_step(false);
        }
        self.scheduler.set_debug_full_world_single_pass(enabled);
    }

    pub fn debug_full_world_single_pass(&self) -> bool {
        self.scheduler.debug_full_world_single_pass()
    }

    /// One checkerboard chunk per simulation step; [`Self::copy_argb32_for_region_chunk_step_viz`] outlines the active chunk.
    /// [`Self::advance_frame`] runs at most one fixed substep per frame while this is on.
    pub fn set_debug_chunk_step(&mut self, enabled: bool) {
        if !enabled {
            self.scheduler.abort_debug_chunk_step(&mut self.world);
            self.flush_pending_grid_explosions();
            self.scheduler.set_debug_chunk_step(false);
            self.chunk_step_stride_counter = 0;
            return;
        }
        self.scheduler.abort_debug_chunk_step(&mut self.world);
        self.flush_pending_grid_explosions();
        self.scheduler.set_debug_full_world_single_pass(false);
        self.scheduler.set_debug_chunk_step(true);
        self.chunk_step_stride_counter = 0;
        self.chunk_step_stride_frames = 1;
    }

    pub fn debug_chunk_step(&self) -> bool {
        self.scheduler.debug_chunk_step()
    }

    /// Chunk-step mode runs a grid substep at most once per `stride` frames (larger = slower).
    pub fn set_debug_chunk_step_stride_frames(&mut self, stride: u32) {
        self.chunk_step_stride_frames = stride.clamp(1, 128);
        self.chunk_step_stride_counter = 0;
    }

    pub fn debug_chunk_step_stride_frames(&self) -> u32 {
        self.chunk_step_stride_frames
    }

    pub fn set_fixed_timestep(&mut self, dt: f32, max_substeps: u32) {
        self.fixed_dt = dt.max(1.0 / 240.0);
        self.max_substeps = max_substeps.max(1);
    }

    pub fn advance_frame(&mut self, frame_dt: f32) -> SimulationStats {
        self.accumulator =
            (self.accumulator + frame_dt).min(self.fixed_dt * self.max_substeps as f32);
        let start = Instant::now();
        let mut substeps = 0u32;
        let max_sub = if self.scheduler.debug_chunk_step() {
            self.chunk_step_stride_counter = self.chunk_step_stride_counter.saturating_add(1);
            if self.chunk_step_stride_counter < self.chunk_step_stride_frames {
                0
            } else {
                self.chunk_step_stride_counter = 0;
                1
            }
        } else {
            self.chunk_step_stride_counter = 0;
            self.max_substeps
        };
        while self.accumulator >= self.fixed_dt && substeps < max_sub {
            self.step(self.fixed_dt);
            self.accumulator -= self.fixed_dt;
            substeps += 1;
        }
        self.last_stats = SimulationStats {
            substeps,
            sim_ms: start.elapsed().as_secs_f32() * 1000.0,
            active_chunks: self.world.active_chunk_count(),
            sleeping_chunks: self.world.sleeping_chunk_count(),
        };
        self.last_stats
    }

    pub fn last_stats(&self) -> SimulationStats {
        self.last_stats
    }
}

#[cfg(test)]
impl Simulation {
    pub(crate) fn test_world(&self) -> &World {
        &self.world
    }

    pub(crate) fn test_world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub(crate) fn test_rigid(&self) -> &RigidBridge {
        &self.rigid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rigid::RigidBridge;
    use crate::sim::deterministic_hash;
    use crate::world::{
        material, AdjacentTransformRule, Cell, ChunkCoord, MaterialId, MaterialMotion, Vec2i,
        World, CHUNK_SIZE,
    };

    fn count_material(sim: &Simulation, rect: RectI, id: u16) -> usize {
        sim.copy_palette_indices_for_region(rect)
            .into_iter()
            .filter(|m| *m == id)
            .count()
    }

    fn average_y_for_material(sim: &Simulation, rect: RectI, id: u16) -> Option<f32> {
        let palette = sim.copy_palette_indices_for_region(rect);
        let mut total_y = 0usize;
        let mut total = 0usize;
        let mut i = 0usize;
        for y in rect.min.y..=rect.max.y {
            for _x in rect.min.x..=rect.max.x {
                if palette[i] == id {
                    total_y += y as usize;
                    total += 1;
                }
                i += 1;
            }
        }
        (total > 0).then_some(total_y as f32 / total as f32)
    }

    #[test]
    fn sand_falls_downward() {
        let mut sim = Simulation::new(SimulationConfig::default());
        sim.paint_circle(Vec2i::new(10, 10), 1, material::SAND);
        for _ in 0..3 {
            sim.step(1.0 / 60.0);
        }
        let lower =
            sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(6, 12), Vec2i::new(14, 40)));
        assert!(lower.iter().any(|m| *m == material::SAND));
    }

    #[test]
    fn deterministic_seeded_mode_is_repeatable() {
        let mut a = Simulation::new(SimulationConfig::default());
        let mut b = Simulation::new(SimulationConfig::default());
        a.paint_circle(Vec2i::new(20, 20), 4, material::LIQUID);
        b.paint_circle(Vec2i::new(20, 20), 4, material::LIQUID);
        for _ in 0..30 {
            a.step(1.0 / 60.0);
            b.step(1.0 / 60.0);
        }
        let ha = deterministic_hash(&a.world);
        let hb = deterministic_hash(&b.world);
        assert_eq!(ha, hb);
    }

    #[test]
    fn pixels_do_not_disappear_near_bounds() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);

        // Put pixels close to edges where scratch-grid border logic matters.
        sim.paint_circle(Vec2i::new(2, 2), 2, material::SAND);
        let initial = sim
            .copy_palette_indices_for_region(bounds)
            .into_iter()
            .filter(|m| *m == material::SAND)
            .count();

        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }

        let final_count = sim
            .copy_palette_indices_for_region(bounds)
            .into_iter()
            .filter(|m| *m == material::SAND)
            .count();
        assert_eq!(initial, final_count);
    }

    #[test]
    fn sand_sinks_through_liquid() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(47, 47));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(24, 30), 10, material::LIQUID);
        sim.paint_circle(Vec2i::new(24, 10), 4, material::SAND);
        let sand_before = count_material(&sim, bounds, material::SAND);

        for _ in 0..200 {
            sim.step(1.0 / 60.0);
        }

        let sand_below = count_material(
            &sim,
            RectI::new(Vec2i::new(0, 26), Vec2i::new(47, 47)),
            material::SAND,
        );
        assert_eq!(sand_before, count_material(&sim, bounds, material::SAND));
        assert!(sand_below > sand_before / 2);
    }

    #[test]
    fn gas_rises_through_liquid() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(47, 47));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(24, 22), 12, material::LIQUID);
        sim.paint_circle(Vec2i::new(24, 34), 4, material::GAS);
        let initial = count_material(&sim, bounds, material::GAS);

        for _ in 0..240 {
            sim.step(1.0 / 60.0);
        }

        let upper_gas = count_material(
            &sim,
            RectI::new(Vec2i::new(0, 0), Vec2i::new(47, 18)),
            material::GAS,
        );
        assert_eq!(initial, count_material(&sim, bounds, material::GAS));
        assert!(upper_gas > 0);
    }

    #[test]
    fn lighter_liquid_floats_on_heavier_liquid() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(47, 47));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(24, 30), 10, material::HEAVY_LIQUID);
        sim.paint_circle(Vec2i::new(24, 12), 7, material::LIGHT_LIQUID);

        for _ in 0..260 {
            sim.step(1.0 / 60.0);
        }

        let light_y = average_y_for_material(&sim, bounds, material::LIGHT_LIQUID).unwrap();
        let heavy_y = average_y_for_material(&sim, bounds, material::HEAVY_LIQUID).unwrap();
        assert!(light_y < heavy_y);
    }

    #[test]
    fn static_blocks_all_displacement() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(16, 16), 4, material::STONE);
        sim.paint_circle(Vec2i::new(16, 2), 3, material::SAND);
        let stone_before = count_material(&sim, bounds, material::STONE);

        for _ in 0..180 {
            sim.step(1.0 / 60.0);
        }

        assert_eq!(
            stone_before,
            count_material(&sim, bounds, material::STONE)
        );
    }

    #[test]
    fn material_count_conserved_in_closed_box() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(63, 63));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(20, 20), 6, material::SAND);
        sim.paint_circle(Vec2i::new(40, 25), 7, material::LIQUID);
        sim.paint_circle(Vec2i::new(30, 50), 5, material::GAS);

        let before_sand = count_material(&sim, bounds, material::SAND);
        let before_liquid = count_material(&sim, bounds, material::LIQUID);
        let before_gas = count_material(&sim, bounds, material::GAS);

        for _ in 0..300 {
            sim.step(1.0 / 60.0);
        }

        assert_eq!(before_sand, count_material(&sim, bounds, material::SAND));
        // `LIQUID` can be cleared when it wets adjacent sand (`adjacent_influence` + `ClearSourceCell`).
        assert!(count_material(&sim, bounds, material::LIQUID) <= before_liquid);
        assert_eq!(before_gas, count_material(&sim, bounds, material::GAS));
    }

    #[test]
    fn material_props_can_be_overridden() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        sim.set_material_props(
            material::SAND,
            crate::world::MaterialProps {
                density: 5,
                motion: MaterialMotion::Gas {
                    viscosity: 0,
                    max_speed: 4,
                    acceleration: 1,
                },
                ..crate::world::MaterialProps::default_const()
            },
        );
        sim.paint_circle(Vec2i::new(12, 12), 2, material::SAND);
        for _ in 0..30 {
            sim.step(1.0 / 60.0);
        }
        let upper = count_material(
            &sim,
            RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 10)),
            material::SAND,
        );
        assert!(upper > 0);
    }

    fn paint_cell(sim: &mut Simulation, p: Vec2i, mat: MaterialId) {
        sim.paint_circle(p, 0, mat);
    }

    #[test]
    fn adjacent_liquid_wets_dry_sand() {
        use crate::world::cell_flags;
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for &(x, y) in &[
            (9, 9),
            (10, 9),
            (11, 9),
            (9, 10),
            (11, 10),
            (9, 11),
            (11, 11),
            (9, 12),
            (10, 12),
            (11, 12),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::LIQUID);
        paint_cell(&mut sim, Vec2i::new(10, 11), material::SAND);

        for _ in 0..240 {
            sim.step(1.0 / 60.0);
            let c = sim.cell(Vec2i::new(10, 11));
            if c.material() == material::SAND && (c.flags() & cell_flags::WET) != 0 {
                return;
            }
        }
        panic!("expected WET on sand from adjacent water influence");
    }

    #[test]
    fn plant_converts_adjacent_water() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        // Chamber: plant (10,10), water (11,10), static perimeter (incl. corners) so water cannot escape diagonally.
        for &(x, y) in &[
            (9, 9),
            (10, 9),
            (11, 9),
            (12, 9),
            (9, 10),
            (12, 10),
            (9, 11),
            (10, 11),
            (11, 11),
            (12, 11),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::PLANT);
        paint_cell(&mut sim, Vec2i::new(11, 10), material::LIQUID);

        let probe = RectI::new(Vec2i::new(9, 9), Vec2i::new(12, 11));
        assert_eq!(count_material(&sim, probe, material::PLANT), 1);
        assert_eq!(count_material(&sim, probe, material::LIQUID), 1);

        for _ in 0..64 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::LIQUID) == 0 {
                break;
            }
        }

        assert_eq!(count_material(&sim, probe, material::LIQUID), 0);
        assert_eq!(count_material(&sim, probe, material::PLANT), 2);
    }

    #[test]
    fn wood_converts_adjacent_water_to_plant() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for &(x, y) in &[
            (9, 9),
            (10, 9),
            (11, 9),
            (12, 9),
            (9, 10),
            (12, 10),
            (9, 11),
            (10, 11),
            (11, 11),
            (12, 11),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::WOOD);
        paint_cell(&mut sim, Vec2i::new(11, 10), material::LIQUID);

        let probe = RectI::new(Vec2i::new(9, 9), Vec2i::new(12, 11));
        assert_eq!(count_material(&sim, probe, material::WOOD), 1);
        assert_eq!(count_material(&sim, probe, material::LIQUID), 1);
        assert_eq!(count_material(&sim, probe, material::PLANT), 0);

        for _ in 0..64 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::LIQUID) == 0 {
                break;
            }
        }

        assert_eq!(count_material(&sim, probe, material::LIQUID), 0);
        assert_eq!(count_material(&sim, probe, material::PLANT), 1);
        assert_eq!(count_material(&sim, probe, material::WOOD), 1);
    }

    /// Rigid WOOD still runs `adjacent_transforms`, but new PLANT stays **off** the rigid bridge (no `rigid_id`),
    /// so colliders/anchors do not grow with each conversion.
    #[test]
    fn rigid_wood_converts_adjacent_water_to_loose_plant_not_body_owned() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for &(x, y) in &[
            (9, 9),
            (10, 9),
            (11, 9),
            (12, 9),
            (9, 10),
            (12, 10),
            (9, 11),
            (10, 11),
            (11, 11),
            (12, 11),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        let wood_p = Vec2i::new(10, 10);
        let water_p = Vec2i::new(11, 10);
        let body_id = sim.spawn_rigid_body_from_pixels(&[wood_p], material::WOOD);
        assert!(body_id != 0);
        paint_cell(&mut sim, water_p, material::LIQUID);

        let probe = RectI::new(Vec2i::new(9, 9), Vec2i::new(12, 11));
        assert_eq!(count_material(&sim, probe, material::WOOD), 1);
        assert_eq!(count_material(&sim, probe, material::LIQUID), 1);
        assert_eq!(count_material(&sim, probe, material::PLANT), 0);

        for _ in 0..256 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::LIQUID) == 0 {
                break;
            }
        }

        assert_eq!(count_material(&sim, probe, material::LIQUID), 0);
        assert_eq!(count_material(&sim, probe, material::PLANT), 1);
        assert_eq!(count_material(&sim, probe, material::WOOD), 1);
        assert_eq!(sim.test_world().get_rigid_id(wood_p), Some(body_id));
        assert_eq!(sim.test_world().get_rigid_id(water_p), None);
    }

    /// [`check_phase_transition`] must strip [`cell_flags::RIGID_PIXEL`] when a rigid-tagged solid melts.
    #[test]
    fn phase_transition_clears_rigid_flags_on_melt() {
        use crate::world::cell_flags;

        let mut cfg = SimulationConfig::default();
        cfg.debug_full_world_single_pass = true;
        let mut sim = Simulation::new(cfg);
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        let p = Vec2i::new(10, 10);
        sim.paint_cell(
            p,
            Cell::new()
                .with_material(material::ICE)
                .with_temperature(280)
                .with_flags(cell_flags::RIGID_PIXEL),
        );
        sim.step(1.0 / 60.0);
        let probe = RectI::new(Vec2i::new(8, 8), Vec2i::new(12, 12));
        let mut found = false;
        'probe: for y in probe.min.y..=probe.max.y {
            for x in probe.min.x..=probe.max.x {
                let c = sim.test_world().get_cell(Vec2i::new(x, y));
                if c.material() == material::LIQUID && !c.has_flag(cell_flags::RIGID_PIXEL) {
                    assert_eq!(c.rigid_source_material(), 0);
                    found = true;
                    break 'probe;
                }
            }
        }
        assert!(
            found,
            "expected melted ice (LIQUID) without RIGID_PIXEL in probe"
        );
    }

    #[test]
    fn adjacent_transform_uses_material_props_rules() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        let mut plant_props = crate::materials::BUILTINS
            .iter()
            .find(|d| d.id == material::PLANT)
            .unwrap()
            .props;
        plant_props.adjacent_transforms = [
            crate::world::AdjacentTransformRule {
                from: material::SAND,
                to: material::LIQUID,
                chance_percent: 100,
                cardinal_neighbors_only: true,
                actor_lifetime_delta: 0,
            },
            crate::world::AdjacentTransformRule::inactive(),
            crate::world::AdjacentTransformRule::inactive(),
            crate::world::AdjacentTransformRule::inactive(),
        ];
        sim.set_material_props(material::PLANT, plant_props);

        for &(x, y) in &[
            (9, 9),
            (10, 9),
            (11, 9),
            (12, 9),
            (9, 10),
            (12, 10),
            (9, 11),
            (10, 11),
            (11, 11),
            (12, 11),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::SAND);
        paint_cell(&mut sim, Vec2i::new(11, 10), material::PLANT);

        let probe = RectI::new(Vec2i::new(9, 9), Vec2i::new(12, 11));
        assert_eq!(count_material(&sim, probe, material::SAND), 1);
        assert_eq!(count_material(&sim, probe, material::LIQUID), 0);

        for _ in 0..30 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::SAND) == 0 {
                break;
            }
        }

        assert_eq!(count_material(&sim, probe, material::SAND), 0);
        assert!(count_material(&sim, probe, material::LIQUID) >= 1);
        assert_eq!(count_material(&sim, probe, material::PLANT), 1);
    }

    #[test]
    fn smoldering_wood_burns_away_eventually() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        let mut fast_burn = crate::materials::BUILTINS
            .iter()
            .find(|def| def.id == material::WOOD)
            .unwrap()
            .props;
        fast_burn.fuel_mass = 24;
        fast_burn.consumption_rate = 200;
        fast_burn.neighbor_spawns = [
            crate::world::NeighborSpawnRule::inactive(),
            crate::world::NeighborSpawnRule::inactive(),
            crate::world::NeighborSpawnRule::inactive(),
            crate::world::NeighborSpawnRule::inactive(),
        ];
        sim.set_material_props(material::WOOD, fast_burn);
        // Pocket: plant | fire, walls so fire cannot drift away before igniting.
        for &(x, y) in &[
            (9, 20),
            (12, 20),
            (9, 19),
            (10, 19),
            (11, 19),
            (12, 19),
            (9, 21),
            (10, 21),
            (11, 21),
            (12, 21),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(10, 20), material::WOOD);
        paint_cell(&mut sim, Vec2i::new(11, 20), material::FIRE);
        let start_wood = count_material(&sim, bounds, material::WOOD);
        assert!(start_wood > 0);
        for _ in 0..400 {
            sim.step(1.0 / 60.0);
        }
        let end_wood = count_material(&sim, bounds, material::WOOD);
        assert!(end_wood < start_wood);
    }

    /// Neighbor spread / flash ignite must not replace smolder solids with a `FIRE` gas cell. That was
    /// `instant_heat_ignition_cell` when `fuel_mass == 0`; wood must smolder as same material + `ON_FIRE`.
    #[test]
    fn fire_spread_does_not_replace_wood_with_fire_gas() {
        let mut cfg = SimulationConfig::default();
        cfg.debug_full_world_single_pass = true;
        let mut sim = Simulation::new(cfg);
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        let mut wp = sim.test_world().material_props(material::WOOD);
        wp.fuel_mass = 0;
        wp.ignitability = 255;
        sim.set_material_props(material::WOOD, wp);
        paint_cell(&mut sim, Vec2i::new(10, 10), material::WOOD);
        paint_cell(&mut sim, Vec2i::new(10, 11), material::FIRE);
        for _ in 0..300 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(
            sim.test_world().get_cell(Vec2i::new(10, 10)).material(),
            material::WOOD
        );
    }

    /// Mis-tuned wood (`fuel_mass == 0` and `autoignition_temperature == 0`) used to fall through to
    /// instant heat and become a `FIRE` gas cell in one tick — same as "wood deleted" without smolder.
    #[test]
    fn fire_spread_does_not_instant_gas_when_wood_autoignition_cleared() {
        let mut cfg = SimulationConfig::default();
        cfg.debug_full_world_single_pass = true;
        let mut sim = Simulation::new(cfg);
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        let mut wp = sim.test_world().material_props(material::WOOD);
        wp.fuel_mass = 0;
        wp.autoignition_temperature = 0;
        wp.ignitability = 255;
        sim.set_material_props(material::WOOD, wp);
        paint_cell(&mut sim, Vec2i::new(10, 10), material::WOOD);
        paint_cell(&mut sim, Vec2i::new(10, 11), material::FIRE);
        sim.step(1.0 / 60.0);
        assert_eq!(
            sim.test_world().get_cell(Vec2i::new(10, 10)).material(),
            material::WOOD
        );
    }

    /// Regression: neighbor fire spread must not re-ignite cells that are already `ON_FIRE`, or it resets
    /// `lifetime` to `fuel_mass` every tick and wood never burns out.
    #[test]
    fn four_chunk_wood_grid_burns_out_from_bottom_fire() {
        let mut cfg = SimulationConfig::default();
        cfg.debug_full_world_single_pass = true;
        let mut sim = Simulation::new(cfg);
        let w = 2 * CHUNK_SIZE;
        let h = 2 * CHUNK_SIZE;
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);

        let mut wood_props = crate::materials::BUILTINS
            .iter()
            .find(|def| def.id == material::WOOD)
            .unwrap()
            .props;
        // Keep test runtime reasonable; physics is the same with default props after the spread fix.
        wood_props.consumption_rate = 180;
        wood_props.neighbor_spawns = [
            crate::world::NeighborSpawnRule::inactive(),
            crate::world::NeighborSpawnRule::inactive(),
            crate::world::NeighborSpawnRule::inactive(),
            crate::world::NeighborSpawnRule::inactive(),
        ];
        sim.set_material_props(material::WOOD, wood_props);

        sim.paint_rect_filled(Vec2i::new(0, 0), Vec2i::new(w - 1, h - 1), material::WOOD);
        let fire_y = h;
        for x in 0..w {
            paint_cell(&mut sim, Vec2i::new(x, fire_y), material::FIRE);
        }

        let probe = RectI::new(Vec2i::new(0, 0), Vec2i::new(w - 1, fire_y));
        let start_wood = count_material(&sim, probe, material::WOOD);
        assert_eq!(start_wood, (w * h) as usize);

        const MAX_STEPS: usize = 500_000;
        for step in 0..MAX_STEPS {
            if step % 90 == 0 {
                for x in 0..w {
                    let p = Vec2i::new(x, fire_y);
                    if sim.test_world().get_cell(p).material() == material::EMPTY {
                        paint_cell(&mut sim, p, material::FIRE);
                    }
                }
            }
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::WOOD) == 0 {
                return;
            }
        }
        panic!(
            "expected all wood to burn (neighbor spread must not refill lifetime); wood left {}",
            count_material(&sim, probe, material::WOOD)
        );
    }

    #[test]
    fn acid_surrounded_wax_eventually_dissolves() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(16, 16), 7, material::ACID);
        sim.paint_circle(Vec2i::new(16, 16), 0, material::WAX);
        assert_eq!(count_material(&sim, bounds, material::WAX), 1);
        for _ in 0..8000 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(count_material(&sim, bounds, material::WAX), 0);
    }

    #[test]
    fn acid_surrounded_sand_eventually_dissolves() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(16, 16), 7, material::ACID);
        sim.paint_circle(Vec2i::new(16, 16), 0, material::SAND);
        assert_eq!(count_material(&sim, bounds, material::SAND), 1);
        for _ in 0..8000 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(count_material(&sim, bounds, material::SAND), 0);
    }

    #[test]
    fn acid_adjacent_static_wall_eventually_breaches() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        sim.paint_circle(Vec2i::new(16, 16), 7, material::ACID);
        sim.paint_circle(Vec2i::new(16, 16), 0, material::STONE);
        assert_eq!(count_material(&sim, bounds, material::STONE), 1);
        for _ in 0..80_000 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(count_material(&sim, bounds, material::STONE), 0);
    }

    #[test]
    fn plant_does_not_fall_when_suspended() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        // Single plant with empty cells below (no static floor under it).
        paint_cell(&mut sim, Vec2i::new(16, 8), material::PLANT);

        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }

        assert_eq!(count_material(&sim, bounds, material::PLANT), 1);
        assert_eq!(
            sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(16, 8), Vec2i::new(16, 8)))
                [0],
            material::PLANT
        );
    }

    #[test]
    fn fire_adjacent_transform_vaporizes_water_or_extinguishes_fire() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        // 5x5 static shell with fire surrounding water on all cardinal sides.
        for y in 8..=12 {
            for x in 8..=12 {
                paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
            }
        }
        // Hollow out the 3x3 interior and place fire + water.
        for y in 9..=11 {
            for x in 9..=11 {
                paint_cell(&mut sim, Vec2i::new(x, y), material::FIRE);
            }
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::LIQUID);

        let probe = RectI::new(Vec2i::new(8, 8), Vec2i::new(12, 12));
        assert_eq!(count_material(&sim, probe, material::LIQUID), 1);
        assert!(count_material(&sim, probe, material::FIRE) >= 1);

        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }
        let liquid = count_material(&sim, probe, material::LIQUID);
        let steam = count_material(&sim, probe, material::STEAM);
        let fire_left = count_material(&sim, probe, material::FIRE);
        // Fire-water adjacency should produce at least one outcome:
        // water vaporized to steam, or fire extinguished by water.
        assert!(
            liquid == 0 || fire_left == 0,
            "fire/water adjacency should vaporize water or extinguish fire \
             (liquid={liquid}, fire={fire_left}, steam={steam})"
        );
    }

    #[test]
    fn steam_condenses_to_water_when_lifetime_ends() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                paint_cell(&mut sim, Vec2i::new(16 + dx, 16 + dy), material::STONE);
            }
        }
        paint_cell(&mut sim, Vec2i::new(16, 16), material::STEAM);

        for _ in 0..200 {
            sim.step(1.0 / 60.0);
        }

        assert_eq!(
            sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(16, 16), Vec2i::new(16, 16)))
                [0],
            material::LIQUID
        );
        assert_eq!(count_material(&sim, bounds, material::STEAM), 0);
    }

    #[test]
    fn lava_trapped_cools_to_obsidian() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                paint_cell(&mut sim, Vec2i::new(16 + dx, 16 + dy), material::STONE);
            }
        }
        paint_cell(&mut sim, Vec2i::new(16, 16), material::LAVA);

        for _ in 0..60_000 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, bounds, material::LAVA) == 0 {
                break;
            }
        }

        let final_mat = sim
            .copy_palette_indices_for_region(RectI::new(Vec2i::new(16, 16), Vec2i::new(16, 16)))[0];
        assert!(
            final_mat == material::OBSIDIAN || final_mat == material::SAND,
            "lava should cool to obsidian (or sand via burnout), got material {}",
            final_mat
        );
        assert_eq!(count_material(&sim, bounds, material::LAVA), 0);
    }

    #[test]
    fn torch_neighbor_spawns_fire() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        paint_cell(&mut sim, Vec2i::new(15, 15), material::TORCH);
        let probe = RectI::new(Vec2i::new(14, 14), Vec2i::new(16, 16));
        assert_eq!(count_material(&sim, probe, material::FIRE), 0);

        for _ in 0..4000 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, bounds, material::FIRE) > 0 {
                break;
            }
        }

        assert!(count_material(&sim, bounds, material::FIRE) >= 1);
    }

    #[test]
    fn lava_adjacent_water_produces_steam_and_obsidian() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for &(x, y) in &[
            (9, 9),
            (10, 9),
            (11, 9),
            (12, 9),
            (9, 10),
            (12, 10),
            (9, 11),
            (10, 11),
            (11, 11),
            (12, 11),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::LAVA);
        paint_cell(&mut sim, Vec2i::new(11, 10), material::LIQUID);

        let probe = RectI::new(Vec2i::new(9, 9), Vec2i::new(12, 11));
        let mut saw_steam = false;
        for _ in 0..500 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::STEAM) >= 1 {
                saw_steam = true;
                break;
            }
        }
        assert!(saw_steam, "lava should eventually vaporize adjacent water via heat conduction or adjacent transform");
        assert_eq!(count_material(&sim, probe, material::LIQUID), 0);
    }

    fn test_dirt_cell() -> Cell {
        Cell::new().with_material(material::DIRT)
    }

    /// Misaligned grid origin: `bounds_for_chunk` must still cover every dense cell for its chunk key.
    #[test]
    fn static_physics_bounds_cover_cells_when_origin_not_chunk_aligned() {
        let mut world = World::new(CHUNK_SIZE, 256);
        let min = Vec2i::new(-1001, 14);
        let max = Vec2i::new(-1001 + 255, 14 + 127);
        world.set_solid_bounds(RectI::new(min, max));
        let origin = world.grid_origin();
        assert_ne!(
            origin.x.rem_euclid(CHUNK_SIZE),
            0,
            "test requires misaligned origin.x (got {})",
            origin.x
        );

        let y = origin.y + 3;
        let x1 = origin.x + CHUNK_SIZE - 1;
        let x2 = origin.x + CHUNK_SIZE;
        let x3 = origin.x + 2 * CHUNK_SIZE;

        for &x in &[x1, x2, x3] {
            let p = Vec2i::new(x, y);
            let coord = world.dense_chunk_coord_for_cell(p).expect("in dense grid");
            let b = world.bounds_for_chunk(coord);
            assert!(
                b.contains(p),
                "p={p:?} coord={coord:?} bounds={b:?} origin={origin:?}"
            );
        }
    }

    /// Aligned origin: bounds still match legacy world-chunk grid.
    #[test]
    fn static_physics_bounds_aligned_origin_matches_world_grid() {
        let mut world = World::new(CHUNK_SIZE, 128);
        world.set_solid_bounds(RectI::new(Vec2i::new(-960, 0), Vec2i::new(-960 + 127, 95)));
        let origin = world.grid_origin();
        assert_eq!(origin.x.rem_euclid(CHUNK_SIZE), 0);

        let p = Vec2i::new(origin.x + CHUNK_SIZE + 3, origin.y + 5);
        let coord = world.dense_chunk_coord_for_cell(p).unwrap();
        assert_eq!(coord.x * CHUNK_SIZE, origin.x + CHUNK_SIZE);
        let b = world.bounds_for_chunk(coord);
        assert!(b.contains(p));
    }

    /// After toggling static contribution, drained physics-dirty chunk coords must cover the cell.
    #[test]
    fn physics_dirty_drained_chunks_cover_modified_cells() {
        let mut world = World::new(CHUNK_SIZE, 256);
        world.set_solid_bounds(RectI::new(
            Vec2i::new(-1003, 8),
            Vec2i::new(-1003 + 255, 8 + 191),
        ));
        let _ = world.take_physics_dirty_chunks();

        let probes = [
            Vec2i::new(-1000, 20),
            Vec2i::new(-1000 + CHUNK_SIZE, 20),
            Vec2i::new(-1000 + 2 * CHUNK_SIZE, 20),
        ];
        for p in probes {
            world.set_cell(p, test_dirt_cell());
            let dirty = world.take_physics_dirty_chunks();
            assert!(
                dirty.iter().any(|c| world.bounds_for_chunk(*c).contains(p)),
                "p={p:?} dirty={dirty:?}"
            );
        }
    }

    /// One-cell-at-a-time paint across chunk columns; Rapier static handles appear for each chunk touched.
    #[test]
    fn static_colliders_follow_incremental_paint_across_chunks() {
        let mut world = World::new(CHUNK_SIZE, 256);
        world.set_solid_bounds(RectI::new(
            Vec2i::new(-1005, 11),
            Vec2i::new(-1005 + 255, 11 + 160),
        ));
        let _ = world.take_physics_dirty_chunks();
        let mut rigid = RigidBridge::new();
        let origin = world.grid_origin();
        let y = origin.y + 7;
        let x0 = origin.x + 1;
        for dx in 0..(3 * CHUNK_SIZE + 5) {
            let p = Vec2i::new(x0 + dx, y);
            if world.get_cell(p).material() == material::EMPTY {
                world.set_cell(p, test_dirt_cell());
            }
            let dirty = world.take_physics_dirty_chunks();
            rigid.rebuild_static_colliders(&world, &dirty);
            let coord = world.dense_chunk_coord_for_cell(p).expect("p in grid");
            assert!(
                rigid.static_collider_count_for_chunk(coord) > 0,
                "no static colliders for chunk {coord:?} after painting {p:?}"
            );
        }
    }

    /// Clearing static cells one-by-one removes static colliders when the chunk has no inert solids left.
    #[test]
    fn static_colliders_follow_incremental_clear_across_chunks() {
        let mut world = World::new(CHUNK_SIZE, 256);
        world.set_solid_bounds(RectI::new(
            Vec2i::new(-1007, 9),
            Vec2i::new(-1007 + 255, 9 + 160),
        ));
        let _ = world.take_physics_dirty_chunks();
        let mut rigid = RigidBridge::new();
        let origin = world.grid_origin();
        let y = origin.y + 11;
        let x0 = origin.x + 2;
        let span = 3 * CHUNK_SIZE + 3;
        let mut coords_order: Vec<ChunkCoord> = Vec::new();
        for dx in 0..span {
            let p = Vec2i::new(x0 + dx, y);
            world.set_cell(p, test_dirt_cell());
            let dirty = world.take_physics_dirty_chunks();
            rigid.rebuild_static_colliders(&world, &dirty);
            let c = world.dense_chunk_coord_for_cell(p).unwrap();
            if !coords_order.contains(&c) {
                coords_order.push(c);
            }
        }
        assert!(
            coords_order.len() >= 2,
            "expected multiple chunks, got {:?}",
            coords_order
        );

        for dx in 0..span {
            let p = Vec2i::new(x0 + dx, y);
            world.set_cell(p, Cell::new());
            let dirty = world.take_physics_dirty_chunks();
            rigid.rebuild_static_colliders(&world, &dirty);
        }
        for c in &coords_order {
            assert_eq!(
                rigid.static_collider_count_for_chunk(*c),
                0,
                "chunk {:?} should have no static colliders after clear",
                c
            );
        }
    }

    /// Vertical strip across chunk rows.
    #[test]
    fn static_colliders_follow_incremental_paint_vertical_across_chunks() {
        let mut world = World::new(CHUNK_SIZE, 256);
        world.set_solid_bounds(RectI::new(
            Vec2i::new(-1002, 20),
            Vec2i::new(-1002 + 200, 20 + 255),
        ));
        let _ = world.take_physics_dirty_chunks();
        let mut rigid = RigidBridge::new();
        let origin = world.grid_origin();
        let x = origin.x + 5;
        let y0 = origin.y + 2;
        for dy in 0..(2 * CHUNK_SIZE + 4) {
            let p = Vec2i::new(x, y0 + dy);
            if world.get_cell(p).material() == material::EMPTY {
                world.set_cell(p, test_dirt_cell());
            }
            let dirty = world.take_physics_dirty_chunks();
            rigid.rebuild_static_colliders(&world, &dirty);
            let coord = world.dense_chunk_coord_for_cell(p).expect("p in grid");
            assert!(
                rigid.static_collider_count_for_chunk(coord) > 0,
                "vertical: no colliders for {coord:?} at {p:?}"
            );
        }
    }

    /// Full `Simulation::step` still refreshes static colliders after paint (integration).
    #[test]
    fn simulation_step_rebuilds_static_colliders_after_paint_misaligned() {
        let mut sim = Simulation::new(SimulationConfig::default());
        sim.set_solid_bounds(RectI::new(
            Vec2i::new(-1004, 16),
            Vec2i::new(-1004 + 255, 16 + 180),
        ));
        for _ in 0..3 {
            sim.step(1.0 / 60.0);
        }
        let p = Vec2i::new(-990, 40);
        sim.paint_cell(p, test_dirt_cell());
        sim.step(1.0 / 60.0);
        let coord = sim
            .test_world()
            .dense_chunk_coord_for_cell(p)
            .expect("probe in world");
        assert!(
            sim.test_rigid().static_collider_count_for_chunk(coord) > 0,
            "expected static colliders in chunk {:?}",
            coord
        );
    }

    #[test]
    fn paint_over_rigid_body_pixel_clears_rigid_id_and_cell_persists() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);
        let min = Vec2i::new(50, 50);
        let max = Vec2i::new(52, 52);
        let body_id = sim.spawn_rigid_body_rect(min, max, material::RIGID);
        let p = Vec2i::new(51, 51);
        assert_eq!(sim.test_world().get_rigid_id(p), Some(body_id));

        // Non-inert paint: `check_splits` + paint-time carve drop anchors.
        sim.paint_circle(p, 0, material::SAND);
        assert_eq!(sim.test_world().get_rigid_id(p), None);
        assert_eq!(sim.test_world().get_cell(p).material(), material::SAND);

        for _ in 0..8 {
            sim.step(1.0 / 60.0);
        }
        assert_ne!(sim.test_world().get_rigid_id(p), Some(body_id));
    }

    #[test]
    fn paint_inert_static_over_rigid_body_pixel_carves_anchor_and_clears_ownership() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);
        let min = Vec2i::new(50, 50);
        let max = Vec2i::new(52, 52);
        let body_id = sim.spawn_rigid_body_rect(min, max, material::RIGID);
        let p = Vec2i::new(51, 51);
        assert_eq!(sim.test_world().get_rigid_id(p), Some(body_id));

        sim.paint_circle(p, 0, material::STONE);
        assert_eq!(sim.test_world().get_rigid_id(p), None);
        assert_eq!(sim.test_world().get_cell(p).material(), material::STONE);

        for _ in 0..8 {
            sim.step(1.0 / 60.0);
        }
        assert_ne!(sim.test_world().get_rigid_id(p), Some(body_id));
        assert_eq!(sim.test_world().get_cell(p).material(), material::STONE);
    }

    /// Regression: inert terrain must not strip anchors via `sync_pixels_to_physics` when the body rests on it.
    #[test]
    fn rigid_body_resting_on_static_terrain_not_carved_by_inert_sync() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);
        for x in 10..118 {
            sim.paint_circle(Vec2i::new(x, 90), 0, material::STONE);
        }
        sim.spawn_rigid_body_rect(Vec2i::new(55, 70), Vec2i::new(57, 72), material::RIGID);
        assert_eq!(sim.test_rigid().dynamic_body_count(), 1);
        for _ in 0..180 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(
            sim.test_rigid().dynamic_body_count(),
            1,
            "body should survive resting on static terrain"
        );
    }

    /// Carve a full vertical column through a 5×3 rigid slab so the remaining anchors form two
    /// 4-connected components in local space; `check_splits` should spawn two dynamic bodies.
    #[test]
    fn chop_rigid_rect_in_half_yields_two_dynamic_bodies() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);

        let min = Vec2i::new(48, 50);
        let max = Vec2i::new(52, 52);
        sim.spawn_rigid_body_rect(min, max, material::RIGID);
        assert_eq!(
            sim.test_rigid().dynamic_body_count(),
            1,
            "expected one rigid before carve"
        );

        // Middle column x = 50 separates x = 48..49 from x = 51..52.
        sim.paint_rect_filled(Vec2i::new(50, 50), Vec2i::new(50, 52), material::EMPTY);

        sim.step(1.0 / 60.0);

        assert_eq!(
            sim.test_rigid().dynamic_body_count(),
            2,
            "expected two rigids after splitting with a through-column carve"
        );

        let alive = sim.test_rigid().all_body_ids();
        assert_eq!(
            sim.test_world().stale_rigid_reference_count(&alive),
            0,
            "morph-close fill must not leave orphan rigid_id cells after a split"
        );
    }

    #[test]
    fn purge_rigid_body_ownership_clears_orphan_placeholder_pixels() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);
        let p = Vec2i::new(70, 70);
        let w = sim.test_world_mut();
        w.set_cell(p, Cell::new().with_material(material::STONE));
        w.set_rigid_id(p, 9_001);
        w.purge_rigid_body_ownership(9_001);
        assert_eq!(w.get_rigid_id(p), None);
        assert_eq!(w.get_cell(p).material(), material::EMPTY);
    }

    #[test]
    fn removing_last_rigid_body_leaves_no_stale_rigid_references() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
        sim.set_solid_bounds(bounds);
        let min = Vec2i::new(40, 40);
        let max = Vec2i::new(42, 42);
        sim.spawn_rigid_body_rect(min, max, material::RIGID);
        sim.step(1.0 / 60.0);
        sim.paint_rect_filled(min, max, material::EMPTY);
        sim.step(1.0 / 60.0);
        assert_eq!(sim.test_rigid().dynamic_body_count(), 0);
        let alive = sim.test_rigid().all_body_ids();
        assert_eq!(
            sim.test_world().stale_rigid_reference_count(&alive),
            0,
            "destroyed body should not leave rigid_id map entries"
        );
    }

    #[test]
    fn paint_rect_filled_spans_chunk_boundary() {
        let mut sim = Simulation::new(SimulationConfig::default());
        sim.set_solid_bounds(RectI::new(
            Vec2i::new(-1005, 11),
            Vec2i::new(-1005 + 255, 11 + 160),
        ));
        let origin = sim.test_world().grid_origin();
        let y = origin.y + 8;
        let x_mid = origin.x + CHUNK_SIZE;
        sim.paint_rect_filled(
            Vec2i::new(x_mid - 2, y),
            Vec2i::new(x_mid + 2, y),
            material::SAND,
        );
        assert_eq!(
            sim.test_world()
                .get_cell(Vec2i::new(x_mid - 2, y))
                .material(),
            material::SAND
        );
        assert_eq!(
            sim.test_world()
                .get_cell(Vec2i::new(x_mid + 2, y))
                .material(),
            material::SAND
        );
    }

    #[test]
    fn temperature_conduction_hot_to_cold() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for &(x, y) in &[
            (14, 14),
            (15, 14),
            (16, 14),
            (17, 14),
            (14, 15),
            (17, 15),
            (14, 16),
            (15, 16),
            (16, 16),
            (17, 16),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(15, 15), material::LAVA);
        paint_cell(&mut sim, Vec2i::new(16, 15), material::SAND);
        let sand_temp_before = sim.test_world().get_cell(Vec2i::new(16, 15)).temperature();
        for _ in 0..20 {
            sim.step(1.0 / 60.0);
        }
        let sand_temp_after = sim.test_world().get_cell(Vec2i::new(16, 15)).temperature();
        assert!(
            sand_temp_after > sand_temp_before,
            "sand should heat up from adjacent lava: before={}, after={}",
            sand_temp_before,
            sand_temp_after
        );
    }

    #[test]
    fn heat_conducts_through_empty_air_gap() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        // Single-cell air gap: lava — EMPTY — sand (each chunk runs one checkerboard pass per `step`).
        for &(x, y) in &[
            (12, 14),
            (13, 14),
            (14, 14),
            (15, 14),
            (16, 14),
            (17, 14),
            (12, 15),
            (17, 15),
            (12, 16),
            (13, 16),
            (14, 16),
            (15, 16),
            (16, 16),
            (17, 16),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(13, 15), material::LAVA);
        paint_cell(&mut sim, Vec2i::new(15, 15), material::SAND);
        let sand_p = Vec2i::new(15, 15);
        let sand_temp_before = sim.test_world().get_cell(sand_p).temperature();
        for _ in 0..200 {
            sim.step(1.0 / 60.0);
        }
        let sand_temp_after = sim.test_world().get_cell(sand_p).temperature();
        assert!(
            sand_temp_after > sand_temp_before,
            "sand should heat across one EMPTY (air) cell: before={}, after={}",
            sand_temp_before,
            sand_temp_after
        );
    }

    #[test]
    fn lava_freezes_into_obsidian() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                paint_cell(&mut sim, Vec2i::new(16 + dx, 16 + dy), material::LIQUID);
            }
        }
        paint_cell(&mut sim, Vec2i::new(16, 16), material::LAVA);

        let mut saw_obsidian = false;
        for _ in 0..10_000 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, bounds, material::OBSIDIAN) > 0 {
                saw_obsidian = true;
                break;
            }
        }
        assert!(
            saw_obsidian,
            "lava surrounded by water should cool and freeze into obsidian"
        );
    }

    #[test]
    fn water_boils_to_steam_from_heat() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for &(x, y) in &[
            (14, 14),
            (15, 14),
            (16, 14),
            (17, 14),
            (14, 15),
            (17, 15),
            (14, 16),
            (15, 16),
            (16, 16),
            (17, 16),
        ] {
            paint_cell(&mut sim, Vec2i::new(x, y), material::STONE);
        }
        paint_cell(&mut sim, Vec2i::new(15, 15), material::LAVA);
        paint_cell(&mut sim, Vec2i::new(16, 15), material::LIQUID);

        let mut saw_steam = false;
        for _ in 0..500 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, bounds, material::STEAM) > 0 {
                saw_steam = true;
                break;
            }
        }
        assert!(
            saw_steam,
            "water next to lava should boil into steam via temperature phase transition"
        );
    }

    #[test]
    fn smolder_extinguishes_when_cooled_below_autoignition() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        let mut wood_props = sim.test_world().material_props(material::WOOD);
        wood_props.adjacent_transforms = [
            AdjacentTransformRule::inactive(),
            AdjacentTransformRule::inactive(),
            AdjacentTransformRule::inactive(),
            AdjacentTransformRule::inactive(),
        ];
        sim.set_material_props(material::WOOD, wood_props);
        let w = Vec2i::new(16, 16);
        let fuel = sim.test_world().material_props(material::WOOD).fuel_mass;
        sim.test_world_mut().set_cell(
            w,
            Cell::new()
                .with_material(material::WOOD)
                .with_lifetime(fuel)
                .with_temperature(650),
        );
        let cold = 293u16;
        sim.test_world_mut().set_cell(
            Vec2i::new(15, 16),
            Cell::new()
                .with_material(material::LIQUID)
                .with_temperature(cold),
        );
        sim.test_world_mut().set_cell(
            Vec2i::new(17, 16),
            Cell::new()
                .with_material(material::LIQUID)
                .with_temperature(cold),
        );
        let mut saw_change = false;
        for _ in 0..4000 {
            sim.step(1.0 / 60.0);
            if sim.test_world().get_cell(w).material() != material::WOOD {
                saw_change = true;
                break;
            }
        }
        assert!(
            saw_change,
            "wood should stop smoldering when cooled below autoignition via conduction"
        );
    }
}
