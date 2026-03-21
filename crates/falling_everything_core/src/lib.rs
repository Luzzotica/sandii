pub mod materials;
pub mod render;
pub mod rigid;
pub mod sim;
pub mod world;

use rand::rngs::SmallRng;
use rand::SeedableRng;
use std::time::Instant;

use render::{DirtyChunkView, PixelRegion};
use rigid::{RigidBodySpec, RigidBridge};
use sim::{ParticleSim, Scheduler, SchedulerMode};
use world::{MaterialId, MaterialProps, MaterialRule, ReactionOutcome, RectI, Vec2i, World};

#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub chunk_size: i32,
    pub region_size: i32,
    pub seed: u64,
    pub deterministic: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            chunk_size: 32,
            region_size: 512,
            seed: 1,
            deterministic: true,
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
        let scheduler_mode = if config.deterministic {
            SchedulerMode::SingleThreadSeeded
        } else {
            SchedulerMode::ThreadPool
        };

        Self {
            world: World::new(config.chunk_size, config.region_size),
            particles: ParticleSim::new(),
            rigid: RigidBridge::new(),
            scheduler: Scheduler::new(scheduler_mode, config.seed),
            rng: SmallRng::seed_from_u64(config.seed),
            events: Vec::new(),
            fixed_dt: 1.0 / 60.0,
            accumulator: 0.0,
            max_substeps: 4,
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

    pub fn spawn_rigid_body_rect(&mut self, min: Vec2i, max: Vec2i, material: MaterialId) -> u32 {
        self.rigid
            .spawn_from_world_rect(&mut self.world, RigidBodySpec { min, max, material })
    }

    pub fn spawn_rigid_body_from_pixels(&mut self, positions: &[Vec2i], material: MaterialId) -> u32 {
        self.rigid
            .spawn_from_pixels(&mut self.world, positions, material)
    }

    pub fn step(&mut self, dt: f32) {
        self.rigid.check_splits(&mut self.world);
        self.rigid.extract_from_world(&mut self.world);
        self.scheduler.step_world(&mut self.world, &mut self.rng);
        self.rigid.step(dt);
        self.rigid
            .reinsert_into_world(&mut self.world, &mut self.particles, &mut self.events);
        self.particles.step(dt, &mut self.world, &mut self.events);
        self.world.finish_frame();
    }

    pub fn despawn_outside(&mut self, bounds: RectI) {
        self.rigid.cull_outside(&mut self.world, bounds);
        self.particles.cull_outside(bounds);
        self.world.clear_outside_rect(bounds);
    }

    pub fn set_solid_bounds(&mut self, bounds: RectI) {
        self.world.set_solid_bounds(bounds);
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

    pub fn set_fixed_timestep(&mut self, dt: f32, max_substeps: u32) {
        self.fixed_dt = dt.max(1.0 / 240.0);
        self.max_substeps = max_substeps.max(1);
    }

    pub fn advance_frame(&mut self, frame_dt: f32) -> SimulationStats {
        self.accumulator = (self.accumulator + frame_dt).min(self.fixed_dt * self.max_substeps as f32);
        let start = Instant::now();
        let mut substeps = 0u32;
        while self.accumulator >= self.fixed_dt && substeps < self.max_substeps {
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
mod tests {
    use super::*;
    use crate::sim::deterministic_hash;
    use crate::world::{material, Phase};

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
        assert_eq!(before_liquid, count_material(&sim, bounds, material::LIQUID));
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
                phase: Phase::Gas,
                viscosity: 0,
                inert: false,
                max_speed: 4,
                acceleration: 1,
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
}
