use std::time::Instant;

use falling_everything_core::materials;
use falling_everything_core::world::{material, MaterialId, RectI, Vec2i};
use falling_everything_core::{Simulation, SimulationConfig, SimulationStats};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Scale, Window, WindowOptions};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;
const TARGET_FPS: usize = 60;
const CULL_MARGIN: i32 = 80;
fn palette_argb() -> [u32; material::MAX_MATERIALS] {
    materials::builtin_palette_argb()
}

fn main() {
    let mut sim = Simulation::new(SimulationConfig {
        deterministic: false,
        ..SimulationConfig::default()
    });
    sim.paint_circle(Vec2i::new(320, 180), 40, material::STATIC);
    sim.set_solid_bounds(RectI::new(
        Vec2i::new(0, 0),
        Vec2i::new((WIDTH - 1) as i32, (HEIGHT - 1) as i32),
    ));

    let mut window = Window::new(
        "Sandii Sandbox - 0 Empty 1 Sand 2 Water 3 Gas 4 Static 5 Rigid 6 LightW 7 HeavyW | R Spawn Body | C Clear",
        WIDTH,
        HEIGHT,
        WindowOptions {
            scale: Scale::X1,
            resize: false,
            ..WindowOptions::default()
        },
    )
    .expect("failed to create window");
    window.set_target_fps(TARGET_FPS);

    let mut frame = vec![0u32; WIDTH * HEIGHT];
    let mut selected: MaterialId = material::SAND;
    let mut brush_radius: i32 = 4;
    let mut last_step = Instant::now();
    let mut fps_last = Instant::now();
    let mut fps_frames: u32 = 0;
    let mut fps_display: u32 = 0;
    let mut render_ms: f32;
    let mut sim_stats: SimulationStats;

    // Prime full frame once.
    blit_full_world(&sim, &mut frame);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        handle_material_shortcuts(&window, &mut selected);
        handle_brush_shortcuts(&window, &mut brush_radius);
        if window.is_key_pressed(Key::P, KeyRepeat::No) {
            let next_parallel = !sim.is_parallel();
            sim.set_parallel(next_parallel);
        }

        if window.is_key_pressed(Key::C, KeyRepeat::No) {
            sim.paint_circle(
                Vec2i::new((WIDTH / 2) as i32, (HEIGHT / 2) as i32),
                10_000,
                material::EMPTY,
            );
        }

        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Clamp) {
            let p = Vec2i::new(mx as i32, my as i32);
            if window.get_mouse_down(MouseButton::Left) {
                sim.paint_circle(p, brush_radius, selected);
            }
            if window.get_mouse_down(MouseButton::Right) {
                sim.paint_circle(p, brush_radius, material::EMPTY);
            }
            if window.is_key_pressed(Key::R, KeyRepeat::No) {
                let min = Vec2i::new(p.x - 6, p.y - 4);
                let max = Vec2i::new(p.x + 6, p.y + 4);
                let _ = sim.spawn_rigid_body_rect(min, max, material::RIGID);
            }
        }

        let now = Instant::now();
        let dt = (now - last_step).as_secs_f32().clamp(1.0 / 240.0, 1.0 / 15.0);
        last_step = now;
        sim_stats = sim.advance_frame(dt);
        sim.set_focus(Vec2i::new((WIDTH / 2) as i32, (HEIGHT / 2) as i32));
        sim.despawn_outside(RectI::new(
            Vec2i::new(-CULL_MARGIN, -CULL_MARGIN),
            Vec2i::new(WIDTH as i32 - 1 + CULL_MARGIN, HEIGHT as i32 - 1 + CULL_MARGIN),
        ));

        let render_start = Instant::now();
        blit_dirty_regions(&sim, &mut frame);
        render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

        fps_frames = fps_frames.saturating_add(1);
        let fps_elapsed = fps_last.elapsed().as_secs_f32();
        if fps_elapsed >= 0.25 {
            fps_display = (fps_frames as f32 / fps_elapsed).round() as u32;
            fps_frames = 0;
            fps_last = Instant::now();
        }
        draw_ui_hint(&mut frame, selected, brush_radius);
        draw_fps_top_right(&mut frame, fps_display, sim.is_parallel());
        draw_perf_hud(&mut frame, sim_stats, render_ms);

        window
            .update_with_buffer(&frame, WIDTH, HEIGHT)
            .expect("failed to update frame");

    }
}

fn blit_full_world(sim: &Simulation, frame: &mut [u32]) {
    let rect = RectI::new(Vec2i::new(0, 0), Vec2i::new((WIDTH - 1) as i32, (HEIGHT - 1) as i32));
    let palette_indices = sim.copy_palette_indices_for_region(rect);
    palette_to_argb32(&palette_indices, frame);
}

fn blit_dirty_regions(sim: &Simulation, frame: &mut [u32]) {
    let dirty = sim.get_dirty_chunks();
    if dirty.is_empty() {
        return;
    }
    if dirty.len() > 96 {
        blit_full_world(sim, frame);
        return;
    }
    for chunk in dirty {
        let mut min = chunk.rect.min;
        let mut max = chunk.rect.max;
        min.x = min.x.clamp(0, (WIDTH - 1) as i32);
        min.y = min.y.clamp(0, (HEIGHT - 1) as i32);
        max.x = max.x.clamp(0, (WIDTH - 1) as i32);
        max.y = max.y.clamp(0, (HEIGHT - 1) as i32);
        if min.x > max.x || min.y > max.y {
            continue;
        }
        let rect = RectI::new(min, max);
        let indices = sim.copy_palette_indices_for_region(rect);
        let w = (max.x - min.x + 1) as usize;
        let h = (max.y - min.y + 1) as usize;
        for y in 0..h {
            let src = y * w;
            let dst = (min.y as usize + y) * WIDTH + min.x as usize;
            for x in 0..w {
                let idx = indices[src + x] as usize;
                frame[dst + x] = PALETTE_ARGB.get(idx).copied().unwrap_or(0xFFFF00FF);
            }
        }
    }
}

fn handle_material_shortcuts(window: &Window, selected: &mut MaterialId) {
    if window.is_key_pressed(Key::Key0, KeyRepeat::No) {
        *selected = material::EMPTY;
    } else if window.is_key_pressed(Key::Key1, KeyRepeat::No) {
        *selected = material::SAND;
    } else if window.is_key_pressed(Key::Key2, KeyRepeat::No) {
        *selected = material::LIQUID;
    } else if window.is_key_pressed(Key::Key3, KeyRepeat::No) {
        *selected = material::GAS;
    } else if window.is_key_pressed(Key::Key4, KeyRepeat::No) {
        *selected = material::STATIC;
    } else if window.is_key_pressed(Key::Key5, KeyRepeat::No) {
        *selected = material::RIGID;
    } else if window.is_key_pressed(Key::Key6, KeyRepeat::No) {
        *selected = material::LIGHT_LIQUID;
    } else if window.is_key_pressed(Key::Key7, KeyRepeat::No) {
        *selected = material::HEAVY_LIQUID;
    }
}

fn handle_brush_shortcuts(window: &Window, brush_radius: &mut i32) {
    if window.is_key_pressed(Key::LeftBracket, KeyRepeat::Yes) {
        *brush_radius = (*brush_radius - 1).max(1);
    }
    if window.is_key_pressed(Key::RightBracket, KeyRepeat::Yes) {
        *brush_radius = (*brush_radius + 1).min(32);
    }
}

fn palette_to_argb32(src: &[u16], dst: &mut [u32]) {
    for (i, px) in dst.iter_mut().enumerate() {
        let idx = src[i] as usize;
        *px = PALETTE_ARGB.get(idx).copied().unwrap_or(0xFFFF00FF);
    }
}

fn draw_ui_hint(frame: &mut [u32], material: MaterialId, brush_radius: i32) {
    draw_panel(frame, 4, 4, 180, 26, 0xAA101010);
    draw_material_palette(frame, material);
    draw_brush_meter(frame, brush_radius);
}

fn draw_material_palette(frame: &mut [u32], selected: MaterialId) {
    let materials = [
        (0u8, material::EMPTY),
        (1u8, material::SAND),
        (2u8, material::LIQUID),
        (3u8, material::GAS),
        (4u8, material::STATIC),
        (5u8, material::RIGID),
        (6u8, material::LIGHT_LIQUID),
        (7u8, material::HEAVY_LIQUID),
    ];
    let mut x = 8usize;
    for (idx, mat) in materials {
        let color = material_color(mat);
        draw_panel(frame, x, 8, 14, 14, color);
        if mat == selected {
            draw_rect_outline(frame, x - 1, 7, 16, 16, 0xFFFFFFFF);
        }
        draw_digit(frame, idx, x + 4, 10, 0xFF000000);
        x += 18;
    }
}

fn draw_brush_meter(frame: &mut [u32], brush_radius: i32) {
    let meter_x = 120usize;
    let meter_y = 10usize;
    let w = 56usize;
    let fill = ((brush_radius.clamp(1, 32) as usize) * w) / 32;
    draw_rect_outline(frame, meter_x, meter_y, w, 8, 0xFFFFFFFF);
    draw_panel(frame, meter_x + 1, meter_y + 1, fill.saturating_sub(2), 6, 0xFFFFD166);

    let tens = ((brush_radius / 10) % 10).max(0) as u8;
    let ones = (brush_radius % 10).max(0) as u8;
    if brush_radius >= 10 {
        draw_digit(frame, tens, meter_x + w + 6, meter_y + 1, 0xFFFFFFFF);
    }
    draw_digit(frame, ones, meter_x + w + 10, meter_y + 1, 0xFFFFFFFF);
}

fn material_color(material: MaterialId) -> u32 {
    PALETTE_ARGB[material as usize]
}

fn draw_panel(frame: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    for yy in y..(y + h).min(HEIGHT) {
        for xx in x..(x + w).min(WIDTH) {
            frame[yy * WIDTH + xx] = color;
        }
    }
}

fn draw_rect_outline(frame: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    for xx in x..(x + w).min(WIDTH) {
        if y < HEIGHT {
            frame[y * WIDTH + xx] = color;
        }
        let y2 = y + h - 1;
        if y2 < HEIGHT {
            frame[y2 * WIDTH + xx] = color;
        }
    }
    for yy in y..(y + h).min(HEIGHT) {
        if x < WIDTH {
            frame[yy * WIDTH + x] = color;
        }
        let x2 = x + w - 1;
        if x2 < WIDTH {
            frame[yy * WIDTH + x2] = color;
        }
    }
}

fn draw_digit(frame: &mut [u32], digit: u8, x: usize, y: usize, color: u32) {
    let glyph = match digit {
        0 => [0b111, 0b101, 0b101, 0b101, 0b111],
        1 => [0b010, 0b110, 0b010, 0b010, 0b111],
        2 => [0b111, 0b001, 0b111, 0b100, 0b111],
        3 => [0b111, 0b001, 0b111, 0b001, 0b111],
        4 => [0b101, 0b101, 0b111, 0b001, 0b001],
        5 => [0b111, 0b100, 0b111, 0b001, 0b111],
        6 => [0b111, 0b100, 0b111, 0b101, 0b111],
        7 => [0b111, 0b001, 0b001, 0b001, 0b001],
        8 => [0b111, 0b101, 0b111, 0b101, 0b111],
        9 => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => [0, 0, 0, 0, 0],
    };
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..3 {
            if (bits >> (2 - col)) & 1 == 1 {
                let px = x + col;
                let py = y + row;
                if px < WIDTH && py < HEIGHT {
                    frame[py * WIDTH + px] = color;
                }
            }
        }
    }
}

fn draw_number(frame: &mut [u32], value: u32, x: usize, y: usize, color: u32) {
    let s = value.to_string();
    let mut xx = x;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            draw_digit(frame, b - b'0', xx, y, color);
            xx += 4;
        }
    }
}

fn draw_fps_top_right(frame: &mut [u32], fps: u32, parallel: bool) {
    let digits = fps.to_string().len().max(1);
    let text_w = digits * 4 + 18;
    let x = WIDTH.saturating_sub(text_w + 6);
    draw_panel(frame, x, 4, text_w, 12, 0xAA101010);
    let mode_color = if parallel { 0xFF4CAF50 } else { 0xFFE53935 };
    draw_panel(frame, x + 2, 6, 8, 8, mode_color);
    draw_number(frame, fps, x + 14, 7, 0xFFFFFFFF);
}

fn draw_perf_hud(frame: &mut [u32], stats: SimulationStats, render_ms: f32) {
    let x = WIDTH.saturating_sub(140);
    draw_panel(frame, x, 18, 136, 18, 0xAA101010);
    draw_number(frame, stats.sim_ms.round() as u32, x + 4, 21, 0xFF8BC34A);
    draw_number(frame, render_ms.round() as u32, x + 24, 21, 0xFF03A9F4);
    draw_number(frame, stats.active_chunks as u32, x + 50, 21, 0xFFFFC107);
    draw_number(frame, stats.sleeping_chunks as u32, x + 84, 21, 0xFFB0BEC5);
    draw_number(frame, stats.substeps, x + 116, 21, 0xFFFFFFFF);
}
