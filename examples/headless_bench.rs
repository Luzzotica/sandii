use std::time::Instant;

use falling_everything_core::world::{material, RectI, Vec2i};
use falling_everything_core::{Simulation, SimulationConfig};

fn main() {
    run_tier("sparse", 600, |sim| {
        sim.paint_circle(Vec2i::new(64, 16), 8, material::SAND);
        sim.paint_circle(Vec2i::new(64, 10), 6, material::LIQUID);
    });
    run_tier("medium", 600, |sim| {
        sim.paint_circle(Vec2i::new(64, 18), 18, material::SAND);
        sim.paint_circle(Vec2i::new(64, 14), 16, material::LIQUID);
        sim.paint_circle(Vec2i::new(64, 50), 10, material::GAS);
        let _rb =
            sim.spawn_rigid_body_rect(Vec2i::new(40, 48), Vec2i::new(80, 58), material::RIGID);
    });
    run_tier("full_churn", 600, |sim| {
        for y in (8..120).step_by(12) {
            let mat = if y % 24 == 0 {
                material::SAND
            } else {
                material::LIQUID
            };
            sim.paint_circle(Vec2i::new(64, y), 20, mat);
        }
        sim.paint_circle(Vec2i::new(64, 4), 30, material::GAS);
    });
}

fn run_tier(name: &str, steps: usize, setup: impl FnOnce(&mut Simulation)) {
    let mut sim = Simulation::new(SimulationConfig::default());
    sim.set_solid_bounds(RectI::new(Vec2i::new(0, 0), Vec2i::new(255, 255)));
    setup(&mut sim);
    let mut per_step_us = Vec::with_capacity(steps);
    for i in 0..steps {
        let t0 = Instant::now();
        sim.set_focus(Vec2i::new(i as i32 / 8, i as i32 / 8));
        sim.advance_frame(1.0 / 60.0);
        per_step_us.push(t0.elapsed().as_micros() as f64);
    }
    per_step_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = per_step_us[per_step_us.len() / 2];
    let p95 = per_step_us[(per_step_us.len() * 95) / 100];
    let rect = RectI::new(Vec2i::new(0, 0), Vec2i::new(127, 127));
    let _pixels = sim.copy_palette_indices_for_region(rect);
    let total_us: f64 = per_step_us.iter().sum();
    println!(
        "[{}] steps={} avg_us={:.2} p50_us={:.2} p95_us={:.2}",
        name,
        steps,
        total_us / steps as f64,
        p50,
        p95
    );
}
