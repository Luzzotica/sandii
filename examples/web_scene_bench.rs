//! Native step timing + state hash for the web build's world (phone grid, tight bounds,
//! deterministic single-threaded scheduler, terrain, then the screen filled with liquid).
//!
//! ```bash
//! cargo run --release --example web_scene_bench            # all scenes
//! cargo run --release --example web_scene_bench -- water   # one scene
//! ```
//!
//! The `hash` column is a hash of every cell after the run. The sim is deterministic, so a
//! pure speed-up must leave it unchanged: compare it before and after a perf change.

use std::time::Instant;

use falling_everything_core::world::{material, Cell, RectI, Vec2i};
use falling_everything_core::worldgen::{self, TerrainConfig};
use falling_everything_core::{Simulation, SimulationConfig};

/// iPhone landscape (844×390 CSS px) → the web grid (`sandii_web::game::grid_for_viewport`).
const W: i32 = 422;
const H: i32 = 195;
const SURFACE_FRACTION: f64 = 0.55;
const WARMUP: usize = 60;
const STEPS: usize = 240;

fn setup() -> (Simulation, RectI) {
    let mut sim = Simulation::new(SimulationConfig {
        deterministic: true,
        ..SimulationConfig::default()
    });
    let surface_y = TerrainConfig::default().surface_y;
    let camera = Vec2i::new(-W / 2, surface_y - (H as f64 * SURFACE_FRACTION).round() as i32);
    let rect = RectI::new(camera, Vec2i::new(camera.x + W - 1, camera.y + H - 1));
    sim.set_solid_bounds(rect);
    let cfg = TerrainConfig {
        x_min: rect.min.x,
        x_max: rect.max.x,
        ..TerrainConfig::default()
    };
    for col in worldgen::generate_terrain(&cfg) {
        for &(y, mat) in &col.cells {
            let p = Vec2i::new(col.x, y);
            if rect.contains(p) {
                sim.paint_cell(p, Cell::new().with_material(mat));
            }
        }
    }
    (sim, rect)
}

fn fill(sim: &mut Simulation, rect: RectI, scene: &str) {
    let mid_x = rect.min.x + W / 2;
    let rain_max_y = rect.min.y + (H * 2) / 5;
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let p = Vec2i::new(x, y);
            if sim.cell(p).material() != material::EMPTY {
                continue;
            }
            let mat = match scene {
                "water" => material::LIQUID,
                "lava" => material::LAVA,
                "mix" if x < mid_x => material::LIQUID,
                "mix" => material::LAVA,
                "rain" if y <= rain_max_y => material::LIQUID,
                "rain" => continue,
                "idle" => continue,
                _ => panic!("unknown scene {scene}"),
            };
            sim.paint_rect_filled(p, p, mat);
        }
    }
}

fn state_hash(sim: &Simulation, rect: RectI) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            h ^= sim.cell(Vec2i::new(x, y)).0;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn main() {
    let only: Vec<String> = std::env::args().skip(1).collect();
    // Override the timed step count with SANDII_BENCH_STEPS (e.g. for a profiler).
    let steps: usize = std::env::var("SANDII_BENCH_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(STEPS);
    let scenes = ["idle", "water", "lava", "mix", "rain"];
    println!("grid {W}x{H}, {WARMUP} warm-up + {steps} timed steps per scene");
    println!("{:<6} {:>8} {:>8} {:>8}  hash", "scene", "avg ms", "p95 ms", "max ms");
    for scene in scenes {
        if !only.is_empty() && !only.iter().any(|s| s == scene) {
            continue;
        }
        let (mut sim, rect) = setup();
        fill(&mut sim, rect, scene);
        for _ in 0..WARMUP {
            sim.step(1.0 / 60.0);
        }
        let mut times = Vec::with_capacity(steps);
        for _ in 0..steps {
            let t = Instant::now();
            sim.step(1.0 / 60.0);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let avg = times.iter().sum::<f64>() / times.len() as f64;
        let mut sorted = times.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = sorted[(sorted.len() * 95) / 100];
        let max = *sorted.last().unwrap();
        println!(
            "{:<6} {:>8.3} {:>8.3} {:>8.3}  {:016x}",
            scene,
            avg,
            p95,
            max,
            state_hash(&sim, rect)
        );
    }
}
