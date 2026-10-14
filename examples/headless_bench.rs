//! Headless step-timing benchmarks for laptop FPS close-out.
//!
//! Small tiers keep historical numbers; `desktop_*` tiers match
//! `sandbox_window` (640×360 viewport, ≤700×400 active dense grid).

use std::time::Instant;

use falling_everything_core::world::{material, RectI, Vec2i};
use falling_everything_core::{Simulation, SimulationConfig};

const DESKTOP_W: i32 = 640;
const DESKTOP_H: i32 = 360;
const STEPS: usize = 600;
/// 16.67ms frame budget at 60 Hz (micros).
const FRAME_BUDGET_US: f64 = 16_666.0;

fn main() {
    // Historical small-world tiers (256×256 solid bounds).
    run_tier("sparse", STEPS, setup_small, |sim| {
        sim.paint_circle(Vec2i::new(64, 16), 8, material::SAND);
        sim.paint_circle(Vec2i::new(64, 10), 6, material::LIQUID);
    });
    run_tier("medium", STEPS, setup_small, |sim| {
        sim.paint_circle(Vec2i::new(64, 18), 18, material::SAND);
        sim.paint_circle(Vec2i::new(64, 14), 16, material::LIQUID);
        sim.paint_circle(Vec2i::new(64, 50), 10, material::GAS);
        let _rb =
            sim.spawn_rigid_body_rect(Vec2i::new(40, 48), Vec2i::new(80, 58), material::RIGID);
    });
    run_tier("full_churn", STEPS, setup_small, |sim| {
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

    // Desktop-scale: same active-grid path as sandbox_window.
    run_tier("desktop_idle", STEPS, setup_desktop, |sim| {
        // Light terrain-like strip only — idle / sparse play.
        for x in (-200..200).step_by(4) {
            sim.paint_circle(Vec2i::new(x, 80), 2, material::STONE);
        }
        sim.paint_circle(Vec2i::new(0, 40), 12, material::SAND);
    });
    run_tier("desktop_paint_storm", STEPS, setup_desktop, |sim| {
        // Dense sand + water + oil columns across most of the viewport (paint storm).
        for x in (-280..280).step_by(28) {
            sim.paint_circle(Vec2i::new(x, -40), 22, material::SAND);
            sim.paint_circle(Vec2i::new(x + 10, -80), 18, material::LIQUID);
            sim.paint_circle(Vec2i::new(x - 8, -120), 14, material::OIL);
        }
        // Floor so matter keeps churning instead of falling forever.
        for x in -320..320 {
            sim.paint_circle(Vec2i::new(x, 140), 1, material::STONE);
        }
    });
    run_tier("desktop_recipe_storm", STEPS, setup_desktop, |sim| {
        // Lava + water + fire-adjacent chaos (recipe feel under load).
        for x in (-200..200).step_by(40) {
            sim.paint_circle(Vec2i::new(x, -20), 16, material::LAVA);
            sim.paint_circle(Vec2i::new(x + 16, -50), 16, material::LIQUID);
            sim.paint_circle(Vec2i::new(x, -90), 10, material::WOOD);
            sim.paint_circle(Vec2i::new(x + 8, -110), 6, material::FIRE);
        }
        for x in -280..280 {
            sim.paint_circle(Vec2i::new(x, 130), 1, material::STONE);
        }
    });

    let (hw, hh) = Simulation::active_grid_half_extents(DESKTOP_W, DESKTOP_H);
    println!(
        "[meta] desktop viewport={}x{} active_grid={}x{} cap={}x{} frame_budget_us={:.0}",
        DESKTOP_W,
        DESKTOP_H,
        hw * 2,
        hh * 2,
        Simulation::ACTIVE_SIM_MAX_W,
        Simulation::ACTIVE_SIM_MAX_H,
        FRAME_BUDGET_US
    );
}

fn setup_small(sim: &mut Simulation) {
    sim.set_solid_bounds(RectI::new(Vec2i::new(0, 0), Vec2i::new(255, 255)));
}

fn setup_desktop(sim: &mut Simulation) {
    let center = Vec2i::new(0, 0);
    sim.ensure_active_grid(center, DESKTOP_W, DESKTOP_H);
    let (half_w, half_h) = Simulation::active_grid_half_extents(DESKTOP_W, DESKTOP_H);
    sim.set_solid_bounds(RectI::new(
        Vec2i::new(center.x - half_w, center.y - half_h),
        Vec2i::new(center.x + half_w - 1, center.y + half_h - 1),
    ));
    sim.set_focus(center);
}

fn run_tier(
    name: &str,
    steps: usize,
    setup: impl FnOnce(&mut Simulation),
    paint: impl FnOnce(&mut Simulation),
) {
    let mut sim = Simulation::new(SimulationConfig::default());
    setup(&mut sim);
    paint(&mut sim);
    let mut per_step_us = Vec::with_capacity(steps);
    for i in 0..steps {
        let t0 = Instant::now();
        // Mild camera drift exercises relocate path without dominating cost.
        if name.starts_with("desktop_") {
            sim.set_focus(Vec2i::new((i as i32 / 16) % 8, (i as i32 / 24) % 6));
            sim.relocate_if_needed(Vec2i::new(0, 0), DESKTOP_W, DESKTOP_H);
        } else {
            sim.set_focus(Vec2i::new(i as i32 / 8, i as i32 / 8));
        }
        sim.advance_frame(1.0 / 60.0);
        per_step_us.push(t0.elapsed().as_micros() as f64);
    }
    per_step_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = per_step_us[per_step_us.len() / 2];
    let p95 = per_step_us[(per_step_us.len() * 95) / 100];
    let max = *per_step_us
        .last()
        .expect("at least one step");
    let total_us: f64 = per_step_us.iter().sum();
    let avg = total_us / steps as f64;
    // Rough equivalent sustained FPS if step were the only cost.
    let equiv_fps = if avg > 0.0 { 1_000_000.0 / avg } else { 0.0 };
    let under_budget = p95 < FRAME_BUDGET_US;
    println!(
        "[{}] steps={} avg_us={:.2} p50_us={:.2} p95_us={:.2} max_us={:.2} ~fps={:.1} p95_under_16ms={}",
        name,
        steps,
        avg,
        p50,
        p95,
        max,
        equiv_fps,
        under_budget
    );
}
