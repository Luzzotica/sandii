use falling_everything_core::world::{RectI, Vec2i};
use falling_everything_core::{Simulation, SimulationConfig};

#[derive(Debug, Clone)]
pub struct TextureUpdate {
    pub position: Vec2i,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct GodotSimulationAdapter {
    simulation: Simulation,
}

impl GodotSimulationAdapter {
    pub fn new(config: SimulationConfig) -> Self {
        Self {
            simulation: Simulation::new(config),
        }
    }

    pub fn simulation_mut(&mut self) -> &mut Simulation {
        &mut self.simulation
    }

    pub fn step_and_collect_texture_updates(&mut self, dt: f32) -> Vec<TextureUpdate> {
        self.simulation.step(dt);
        let dirty = self.simulation.get_dirty_chunks();
        let mut out = Vec::with_capacity(dirty.len());
        for chunk in dirty {
            let rect = RectI::new(chunk.rect.min, chunk.rect.max);
            let pixel_region = self.simulation.copy_rgba_for_region(rect);
            out.push(TextureUpdate {
                position: rect.min,
                width: pixel_region.width,
                height: pixel_region.height,
                rgba: pixel_region.rgba,
            });
        }
        out
    }
}
