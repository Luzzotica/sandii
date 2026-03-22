use std::time::Instant;

use falling_everything_core::materials;
use falling_everything_core::world::{material, MaterialId, RectI, Vec2i};
use falling_everything_core::{Simulation, SimulationConfig, SimulationStats};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Scale, Window, WindowOptions};

const WIDTH: usize = 1280;
const HEIGHT: usize = 720;
const TARGET_FPS: usize = 60;
const CULL_MARGIN: i32 = 80;

/// Top-left material palette (must match `draw_ui_hint` / `draw_material_palette`).
struct PaletteLayout {
    panel_x: usize,
    panel_y: usize,
    panel_w: usize,
    panel_h: usize,
    swatch_x0: usize,
    swatch_y: usize,
    swatch_w: usize,
    swatch_h: usize,
    stride: usize,
    count: usize,
}

impl PaletteLayout {
    fn new() -> Self {
        let count = materials::BUILTINS.len();
        let panel_w = count * 16 + 70;
        Self {
            panel_x: 4,
            panel_y: 4,
            panel_w,
            panel_h: 26,
            swatch_x0: 8,
            swatch_y: 8,
            swatch_w: 12,
            swatch_h: 14,
            stride: 16,
            count,
        }
    }

    fn panel_contains(&self, mx: i32, my: i32) -> bool {
        mx >= self.panel_x as i32
            && mx < (self.panel_x + self.panel_w) as i32
            && my >= self.panel_y as i32
            && my < (self.panel_y + self.panel_h) as i32
    }

    fn material_at(&self, mx: i32, my: i32) -> Option<MaterialId> {
        for i in 0..self.count {
            let sx = self.swatch_x0 + i * self.stride;
            let sy = self.swatch_y;
            if mx >= sx as i32
                && mx < (sx + self.swatch_w) as i32
                && my >= sy as i32
                && my < (sy + self.swatch_h) as i32
            {
                return Some(materials::BUILTINS[i].id);
            }
        }
        None
    }
}

fn main() {
    let mut sim = Simulation::new(SimulationConfig {
        deterministic: false,
        ..SimulationConfig::default()
    });
    sim.paint_circle(
        Vec2i::new((WIDTH / 2) as i32, (HEIGHT / 2) as i32),
        80,
        material::STATIC,
    );
    sim.set_solid_bounds(RectI::new(
        Vec2i::new(0, 0),
        Vec2i::new((WIDTH - 1) as i32, (HEIGHT - 1) as i32),
    ));

    let mat_list: String = materials::BUILTINS.iter().enumerate()
        .map(|(i, d)| format!("{} {}", i, d.name))
        .collect::<Vec<_>>()
        .join(" ");
    let title = format!(
        "Sandii Sandbox - {} | R Rigid | C Clear | P Parallel | T Pass viz | D 1-pass debug",
        mat_list
    );
    let mut window = Window::new(
        &title,
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
    let mut last_left_paint: Option<Vec2i> = None;
    let mut last_right_paint: Option<Vec2i> = None;
    let mut last_step = Instant::now();
    let mut fps_last = Instant::now();
    let mut fps_frames: u32 = 0;
    let mut fps_display: u32 = 0;
    let mut render_ms: f32;
    let mut sim_stats: SimulationStats;

    // Prime full frame once.
    blit_full_world(&sim, &mut frame);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        if window.is_key_pressed(Key::D, KeyRepeat::No) {
            sim.set_debug_full_world_single_pass(!sim.debug_full_world_single_pass());
        }
        handle_material_shortcuts(&window, &mut selected);
        handle_brush_shortcuts(&window, &mut brush_radius);
        if window.is_key_pressed(Key::P, KeyRepeat::No) {
            let next_parallel = !sim.is_parallel();
            sim.set_parallel(next_parallel);
        }
        if window.is_key_pressed(Key::T, KeyRepeat::No) {
            let next = !sim.debug_pass_enabled();
            sim.set_debug_pass_enabled(next);
            blit_full_world(&sim, &mut frame);
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
            let layout = PaletteLayout::new();
            let mx_i = mx as i32;
            let my_i = my as i32;

            if layout.panel_contains(mx_i, my_i) {
                last_left_paint = None;
                last_right_paint = None;
                if window.get_mouse_down(MouseButton::Left) {
                    if let Some(id) = layout.material_at(mx_i, my_i) {
                        selected = id;
                    }
                }
            } else {
                if window.get_mouse_down(MouseButton::Left) {
                    if let Some(prev) = last_left_paint {
                        sim.paint_line_brush(prev, p, brush_radius, selected);
                    } else {
                        sim.paint_circle(p, brush_radius, selected);
                    }
                    last_left_paint = Some(p);
                } else {
                    last_left_paint = None;
                }
                if window.get_mouse_down(MouseButton::Right) {
                    if let Some(prev) = last_right_paint {
                        sim.paint_line_brush(prev, p, brush_radius, material::EMPTY);
                    } else {
                        sim.paint_circle(p, brush_radius, material::EMPTY);
                    }
                    last_right_paint = Some(p);
                } else {
                    last_right_paint = None;
                }
                if window.is_key_pressed(Key::R, KeyRepeat::No) {
                    let min = Vec2i::new(p.x - 6, p.y - 4);
                    let max = Vec2i::new(p.x + 6, p.y + 4);
                    let _ = sim.spawn_rigid_body_rect(min, max, material::RIGID);
                }
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
        draw_fps_top_right(
            &mut frame,
            fps_display,
            sim.is_parallel(),
            sim.debug_full_world_single_pass(),
        );
        draw_perf_hud(&mut frame, sim_stats, render_ms);

        window
            .update_with_buffer(&frame, WIDTH, HEIGHT)
            .expect("failed to update frame");

    }
}

fn blit_full_world(sim: &Simulation, frame: &mut [u32]) {
    let rect = RectI::new(Vec2i::new(0, 0), Vec2i::new((WIDTH - 1) as i32, (HEIGHT - 1) as i32));
    let pixels = if sim.debug_pass_enabled() {
        sim.copy_debug_argb32_for_region(rect)
    } else {
        sim.copy_argb32_for_region(rect)
    };
    frame[..pixels.len()].copy_from_slice(&pixels);
}

fn blit_dirty_regions(sim: &Simulation, frame: &mut [u32]) {
    let debug = sim.debug_pass_enabled();
    if debug {
        blit_full_world(sim, frame);
        return;
    }
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
        let pixels = sim.copy_argb32_for_region(rect);
        let w = (max.x - min.x + 1) as usize;
        let h = (max.y - min.y + 1) as usize;
        for y in 0..h {
            let src = y * w;
            let dst = (min.y as usize + y) * WIDTH + min.x as usize;
            frame[dst..dst + w].copy_from_slice(&pixels[src..src + w]);
        }
    }
}

fn handle_material_shortcuts(window: &Window, selected: &mut MaterialId) {
    // Avoid R (rigid), P (parallel), D (1-pass debug), C (clear), [, ] (brush). Comma = slot that was D.
    const KEYS: &[Key] = &[
        Key::Key0, Key::Key1, Key::Key2, Key::Key3, Key::Key4,
        Key::Key5, Key::Key6, Key::Key7, Key::Key8, Key::Key9,
        Key::Q, Key::W, Key::E, Key::A, Key::S, Key::Comma, Key::F, Key::G,
        Key::H, Key::J, Key::K, Key::L, Key::Z, Key::X, Key::V, Key::B, Key::N, Key::M,
    ];
    for (i, &key) in KEYS.iter().enumerate() {
        if i >= materials::BUILTINS.len() {
            break;
        }
        if window.is_key_pressed(key, KeyRepeat::No) {
            *selected = materials::BUILTINS[i].id;
            return;
        }
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

fn draw_ui_hint(frame: &mut [u32], material: MaterialId, brush_radius: i32) {
    let layout = PaletteLayout::new();
    draw_panel(frame, layout.panel_x, layout.panel_y, layout.panel_w, layout.panel_h, 0xAA101010);
    draw_material_palette(frame, material);
    draw_brush_meter(frame, brush_radius);
}

fn draw_material_palette(frame: &mut [u32], selected: MaterialId) {
    let layout = PaletteLayout::new();
    let mut x = layout.swatch_x0;
    for (idx, def) in materials::BUILTINS.iter().enumerate() {
        let color = def.color_argb;
        draw_panel(frame, x, layout.swatch_y, layout.swatch_w, layout.swatch_h, color);
        if def.id == selected {
            draw_rect_outline(frame, x - 1, 7, 14, 16, 0xFFFFFFFF);
        }
        if idx < 10 {
            draw_digit(frame, idx as u8, x + 3, 10, 0xFF000000);
        }
        x += layout.stride;
    }
}

fn draw_brush_meter(frame: &mut [u32], brush_radius: i32) {
    let layout = PaletteLayout::new();
    let meter_x = layout.swatch_x0 + layout.count * layout.stride + 4;
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

fn draw_fps_top_right(frame: &mut [u32], fps: u32, parallel: bool, full_world_debug: bool) {
    let digits = fps.to_string().len().max(1);
    let text_w = digits * 4 + 18;
    let x = WIDTH.saturating_sub(text_w + 6);
    draw_panel(frame, x, 4, text_w, 12, 0xAA101010);
    let mode_color = if full_world_debug {
        0xFFFF9800
    } else if parallel {
        0xFF4CAF50
    } else {
        0xFFE53935
    };
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
