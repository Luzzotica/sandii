pub mod bresenham;
pub mod cell64;
pub mod materials;
pub mod render;
pub mod rigid;
pub mod sim;
pub mod world;
pub mod worldgen;

pub use world::MaterialMotion;

use rand::rngs::SmallRng;
use rand::SeedableRng;
use std::time::Instant;

use render::{DirtyChunkView, PixelRegion};
use rigid::{RigidBodySpec, RigidBridge};
use sim::{ParticleSim, Scheduler, SchedulerMode};
use world::{Cell, MaterialId, MaterialProps, MaterialRule, ReactionOutcome, RectI, Vec2i, World, CHUNK_SIZE};

#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub chunk_size: i32,
    pub region_size: i32,
    pub seed: u64,
    pub deterministic: bool,
    /// Single-threaded full-grid one pass per tick (no checkerboard, no rayon). For debugging vs multithreaded seams.
    pub debug_full_world_single_pass: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            chunk_size: CHUNK_SIZE,
            region_size: 512,
            seed: 1,
            deterministic: true,
            debug_full_world_single_pass: false,
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

        Self {
            world: World::new(config.chunk_size, config.region_size),
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
        self.events.extend(stream_events.into_iter().map(|event| match event {
            world::WorldEvent::RegionLoaded(region) => SimulationEvent::RegionLoaded { region },
            world::WorldEvent::RegionSaved(region) => SimulationEvent::RegionSaved { region },
        }));
    }

    pub fn paint_circle(&mut self, center: Vec2i, radius: i32, material: MaterialId) {
        self.world.paint_circle(center, radius, material);
    }

    pub fn cell(&self, p: Vec2i) -> Cell {
        self.world.get_cell(p)
    }

    pub fn paint_cell(&mut self, p: Vec2i, cell: Cell) {
        self.world.set_cell(p, cell);
    }

    /// Paints a thick brush along the integer Bresenham line from `a` to `b` (inclusive).
    /// Use when the pointer jumps between frames so no gaps appear in the stroke.
    pub fn paint_line_brush(&mut self, a: Vec2i, b: Vec2i, brush_radius: i32, material: MaterialId) {
        for p in crate::bresenham::bresenham_line(a, b) {
            self.world.paint_circle(p, brush_radius, material);
        }
    }

    pub fn spawn_rigid_body_rect(&mut self, min: Vec2i, max: Vec2i, material: MaterialId) -> u32 {
        self.rigid
            .spawn_from_world_rect(&mut self.world, RigidBodySpec { min, max, material })
    }

    pub fn spawn_rigid_body_from_pixels(&mut self, positions: &[Vec2i], material: MaterialId) -> u32 {
        self.rigid
            .spawn_from_pixels(&mut self.world, positions, material)
    }

    pub fn spawn_rigid_body_circle(&mut self, center: Vec2i, radius: i32, material: MaterialId) -> u32 {
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
        self.rigid
            .spawn_from_circle(&mut self.world, &positions, material, radius as f32)
    }

    pub fn step(&mut self, dt: f32) {
        self.rigid.rebuild_dirty_dynamic_colliders();
        self.rigid.record_positions(&mut self.world);
        self.scheduler.step_world(&mut self.world, &mut self.rng);
        self.world.prune_rigid_ids_for_empty_cells();
        self.rigid.check_splits(&mut self.world);
        let physics_dirty = self.world.take_physics_dirty_chunks();
        self.rigid.rebuild_static_colliders(&self.world, &physics_dirty);
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
        self.world.set_solid_bounds(bounds);
        self.world.mark_all_chunks_physics_dirty();
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
        render::copy_rgba_for_region(&self.world, rect)
    }

    pub fn copy_argb32_for_region(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region(&self.world, rect)
    }

    pub fn copy_argb32_for_region_chunk_step_viz(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region_chunk_step_viz(&self.world, rect)
    }

    pub fn copy_argb32_for_region_pass_batch_viz(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region_pass_batch_viz(&self.world, rect)
    }

    pub fn copy_debug_argb32_for_region(&self, rect: RectI) -> Vec<u32> {
        render::copy_debug_argb32_for_region(&self.world, rect)
    }

    pub fn copy_argb32_for_region_all_debug_views(&self, rect: RectI) -> Vec<u32> {
        render::copy_argb32_for_region_all_debug_views(&self.world, rect)
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
            self.scheduler.set_debug_chunk_step(false);
            self.chunk_step_stride_counter = 0;
            return;
        }
        self.scheduler.abort_debug_chunk_step(&mut self.world);
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
        self.accumulator = (self.accumulator + frame_dt).min(self.fixed_dt * self.max_substeps as f32);
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

    pub(crate) fn test_rigid(&self) -> &RigidBridge {
        &self.rigid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rigid::RigidBridge;
    use crate::sim::deterministic_hash;
    use crate::world::{material, Cell, ChunkCoord, MaterialId, MaterialMotion, World, CHUNK_SIZE};

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
        let lower = sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(6, 12), Vec2i::new(14, 40)));
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
        sim.paint_circle(Vec2i::new(16, 16), 4, material::STATIC);
        sim.paint_circle(Vec2i::new(16, 2), 3, material::SAND);
        let static_before = count_material(&sim, bounds, material::STATIC);

        for _ in 0..180 {
            sim.step(1.0 / 60.0);
        }

        assert_eq!(static_before, count_material(&sim, bounds, material::STATIC));
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
                ignitability: 0,
                consumption_rate: 0,
                fuel_mass: 0,
                explosion_radius: 0,
                on_heat_become: 0,
                on_death_become: 0,
                on_death_lifetime_lo: 0,
                on_death_lifetime_hi: 0,
                corrosion_max_hp: 0,
                acid_vulnerability: crate::world::AcidVulnerability::inactive(),
                acid_corrosion: crate::world::AcidCorrosionSource::inactive(),
                smolder_extinguish_material: material::EMPTY,
                smolder_extinguish_lifetime_lo: 24,
                smolder_extinguish_lifetime_hi: 64,
                smolder_burnout_ignites_neighbors: false,
                smolder_burnout_explosion_radius: 0,
                smolder_burnout_become: material::EMPTY,
                smolder_burnout_lifetime_lo: 0,
                smolder_burnout_lifetime_hi: 0,
                adjacent_transforms: [
                    crate::world::AdjacentTransformRule::inactive(),
                    crate::world::AdjacentTransformRule::inactive(),
                    crate::world::AdjacentTransformRule::inactive(),
                    crate::world::AdjacentTransformRule::inactive(),
                ],
                adjacent_influence: [crate::world::AdjacentInfluenceRule::inactive(); 8],
                neighbor_spawns: [
                    crate::world::NeighborSpawnRule::inactive(),
                    crate::world::NeighborSpawnRule::inactive(),
                    crate::world::NeighborSpawnRule::inactive(),
                    crate::world::NeighborSpawnRule::inactive(),
                ],
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
            paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::LIQUID);
        paint_cell(&mut sim, Vec2i::new(10, 11), material::SAND);

        for _ in 0..240 {
            sim.step(1.0 / 60.0);
            let c = sim.cell(Vec2i::new(10, 11));
            if c.material == material::SAND && (c.flags & cell_flags::WET) != 0 {
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
            paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
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
            paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
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
            paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
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
            paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
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
        sim.paint_circle(Vec2i::new(16, 16), 0, material::STATIC);
        assert_eq!(count_material(&sim, bounds, material::STATIC), 1);
        for _ in 0..80_000 {
            sim.step(1.0 / 60.0);
        }
        assert_eq!(count_material(&sim, bounds, material::STATIC), 0);
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
            sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(16, 8), Vec2i::new(16, 8)))[0],
            material::PLANT
        );
    }

    #[test]
    fn fire_adjacent_transform_vaporizes_water_and_costs_lifetime() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        // 5x5 static shell with fire surrounding water on all cardinal sides.
        for y in 8..=12 {
            for x in 8..=12 {
                paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
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
                paint_cell(
                    &mut sim,
                    Vec2i::new(16 + dx, 16 + dy),
                    material::STATIC,
                );
            }
        }
        paint_cell(&mut sim, Vec2i::new(16, 16), material::STEAM);

        for _ in 0..200 {
            sim.step(1.0 / 60.0);
        }

        assert_eq!(
            sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(16, 16), Vec2i::new(16, 16)))[0],
            material::LIQUID
        );
        assert_eq!(count_material(&sim, bounds, material::STEAM), 0);
    }

    #[test]
    fn lava_trapped_cools_to_sand() {
        let mut sim = Simulation::new(SimulationConfig::default());
        let bounds = RectI::new(Vec2i::new(0, 0), Vec2i::new(31, 31));
        sim.set_solid_bounds(bounds);
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                paint_cell(
                    &mut sim,
                    Vec2i::new(16 + dx, 16 + dy),
                    material::STATIC,
                );
            }
        }
        paint_cell(&mut sim, Vec2i::new(16, 16), material::LAVA);

        for _ in 0..60_000 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, bounds, material::LAVA) == 0 {
                break;
            }
        }

        assert_eq!(
            sim.copy_palette_indices_for_region(RectI::new(Vec2i::new(16, 16), Vec2i::new(16, 16)))[0],
            material::SAND
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
    fn lava_adjacent_transform_vaporizes_water() {
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
            paint_cell(&mut sim, Vec2i::new(x, y), material::STATIC);
        }
        paint_cell(&mut sim, Vec2i::new(10, 10), material::LAVA);
        paint_cell(&mut sim, Vec2i::new(11, 10), material::LIQUID);

        let probe = RectI::new(Vec2i::new(9, 9), Vec2i::new(12, 11));
        let mut saw_steam = false;
        for _ in 0..256 {
            sim.step(1.0 / 60.0);
            if count_material(&sim, probe, material::STEAM) >= 1 {
                saw_steam = true;
                break;
            }
        }
        assert!(saw_steam, "lava should eventually vaporize adjacent water (chance < 100%)");
        assert_eq!(count_material(&sim, probe, material::LIQUID), 0);
        assert_eq!(count_material(&sim, probe, material::LAVA), 1);
    }

    fn test_dirt_cell() -> Cell {
        Cell {
            material: material::DIRT,
            flags: 0,
            velocity: 0,
            lifetime: 0,
            variant: 0,
            scorch: 0,
        }
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
        world.set_solid_bounds(RectI::new(Vec2i::new(-1003, 8), Vec2i::new(-1003 + 255, 8 + 191)));
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
        world.set_solid_bounds(RectI::new(Vec2i::new(-1005, 11), Vec2i::new(-1005 + 255, 11 + 160)));
        let _ = world.take_physics_dirty_chunks();
        let mut rigid = RigidBridge::new();
        let origin = world.grid_origin();
        let y = origin.y + 7;
        let x0 = origin.x + 1;
        for dx in 0..(3 * CHUNK_SIZE + 5) {
            let p = Vec2i::new(x0 + dx, y);
            if world.get_cell(p).material == material::EMPTY {
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
        world.set_solid_bounds(RectI::new(Vec2i::new(-1007, 9), Vec2i::new(-1007 + 255, 9 + 160)));
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
            world.set_cell(p, Cell::default());
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
        world.set_solid_bounds(RectI::new(Vec2i::new(-1002, 20), Vec2i::new(-1002 + 200, 20 + 255)));
        let _ = world.take_physics_dirty_chunks();
        let mut rigid = RigidBridge::new();
        let origin = world.grid_origin();
        let x = origin.x + 5;
        let y0 = origin.y + 2;
        for dy in 0..(2 * CHUNK_SIZE + 4) {
            let p = Vec2i::new(x, y0 + dy);
            if world.get_cell(p).material == material::EMPTY {
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
}
