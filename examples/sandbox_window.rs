use std::time::Instant;

use falling_everything_core::bresenham;
use falling_everything_core::materials;
use falling_everything_core::world::{material, MaterialId, RectI, Vec2i};
use falling_everything_core::worldgen::{self, TerrainConfig};
use falling_everything_core::{Simulation, SimulationConfig, SimulationStats};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Scale, Window, WindowOptions};

const SIM_WIDTH: usize = 640;
const SIM_HEIGHT: usize = 360;
const SCALE: usize = 2;
const DISP_W: usize = SIM_WIDTH * SCALE;
const DISP_H: usize = SIM_HEIGHT * SCALE;
const TARGET_FPS: usize = 60;
const PAN_SPEED: i32 = 8;

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
        let panel_w = (count * 16 + 70) * SCALE;
        Self {
            panel_x: 4 * SCALE,
            panel_y: 4 * SCALE,
            panel_w,
            panel_h: 26 * SCALE,
            swatch_x0: 8 * SCALE,
            swatch_y: 8 * SCALE,
            swatch_w: 12 * SCALE,
            swatch_h: 14 * SCALE,
            stride: 16 * SCALE,
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

fn viewport_rect(camera: Vec2i) -> RectI {
    RectI::new(
        camera,
        Vec2i::new(camera.x + SIM_WIDTH as i32 - 1, camera.y + SIM_HEIGHT as i32 - 1),
    )
}

fn camera_center(camera: Vec2i) -> Vec2i {
    Vec2i::new(camera.x + SIM_WIDTH as i32 / 2, camera.y + SIM_HEIGHT as i32 / 2)
}

fn main() {
    let mut sim = Simulation::new(SimulationConfig {
        deterministic: false,
        ..SimulationConfig::default()
    });

    let terrain_cfg = TerrainConfig::default();

    let initial_camera = Vec2i::new(
        -(SIM_WIDTH as i32) / 2,
        terrain_cfg.surface_y - (SIM_HEIGHT as i32) / 3,
    );

    let half_w = SIM_WIDTH as i32 * 3 / 2;
    let half_h = SIM_HEIGHT as i32 * 3 / 2;
    sim.set_solid_bounds(RectI::new(
        Vec2i::new(
            initial_camera.x + SIM_WIDTH as i32 / 2 - half_w,
            initial_camera.y + SIM_HEIGHT as i32 / 2 - half_h,
        ),
        Vec2i::new(
            initial_camera.x + SIM_WIDTH as i32 / 2 + half_w - 1,
            initial_camera.y + SIM_HEIGHT as i32 / 2 + half_h - 1,
        ),
    ));

    worldgen::paint_terrain(&mut sim, &terrain_cfg);

    let mat_list: String = materials::BUILTINS
        .iter()
        .enumerate()
        .map(|(i, d)| format!("{} {}", i, d.name))
        .collect::<Vec<_>>()
        .join(" ");
    let title = format!(
        "Sandii Sandbox - {} | Arrows Pan | R Rigid | T Boulder | L Lava rock | F Physics | C Clear | P Parallel | D Debug | Y Slow",
        mat_list
    );
    let mut window = Window::new(
        &title,
        DISP_W,
        DISP_H,
        WindowOptions {
            scale: Scale::X1,
            resize: false,
            ..WindowOptions::default()
        },
    )
    .expect("failed to create window");
    window.set_target_fps(TARGET_FPS);

    let mut frame = vec![0u32; DISP_W * DISP_H];
    let mut selected: MaterialId = material::SAND;
    let mut brush_radius: i32 = 4;
    let mut last_left_paint: Option<Vec2i> = None;
    let mut last_right_paint: Option<Vec2i> = None;
    let mut last_step = Instant::now();
    let mut fps_last = Instant::now();
    let mut fps_frames: u32 = 0;
    let mut fps_display: u32 = 0;
    let mut render_ms: f32;
    let mut sim_stats;

    let mut camera = initial_camera;
    let mut mid_drag_origin: Option<(f32, f32)> = None;
    let mut camera_at_drag_start = camera;
    let mut show_physics = false;

    blit_full_world(&sim, &mut frame, camera);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        if window.is_key_pressed(Key::D, KeyRepeat::No) {
            sim.set_debug_views_enabled(!sim.debug_views_enabled());
            blit_full_world(&sim, &mut frame, camera);
        }
        if window.is_key_pressed(Key::Y, KeyRepeat::No) {
            sim.set_debug_chunk_step(!sim.debug_chunk_step());
            blit_full_world(&sim, &mut frame, camera);
        }
        if sim.debug_chunk_step() {
            if window.is_key_pressed(Key::Minus, KeyRepeat::Yes) {
                let s = sim.debug_chunk_step_stride_frames().saturating_sub(1).max(1);
                sim.set_debug_chunk_step_stride_frames(s);
            }
            if window.is_key_pressed(Key::Equal, KeyRepeat::Yes) {
                let s = sim.debug_chunk_step_stride_frames().saturating_add(1).min(128);
                sim.set_debug_chunk_step_stride_frames(s);
            }
        }
        if window.is_key_pressed(Key::Semicolon, KeyRepeat::No) {
            sim.set_debug_full_world_single_pass(!sim.debug_full_world_single_pass());
            blit_full_world(&sim, &mut frame, camera);
        }
        if window.is_key_pressed(Key::F, KeyRepeat::No) {
            show_physics = !show_physics;
        }
        handle_material_shortcuts(&window, &mut selected);
        handle_brush_shortcuts(&window, &mut brush_radius);
        if window.is_key_pressed(Key::P, KeyRepeat::No) {
            sim.set_parallel(!sim.is_parallel());
        }
        if window.is_key_pressed(Key::C, KeyRepeat::No) {
            sim.paint_circle(camera_center(camera), 10_000, material::EMPTY);
        }

        let mut cam_changed = false;
        if window.is_key_down(Key::Left) {
            camera.x -= PAN_SPEED;
            cam_changed = true;
        }
        if window.is_key_down(Key::Right) {
            camera.x += PAN_SPEED;
            cam_changed = true;
        }
        if window.is_key_down(Key::Up) {
            camera.y -= PAN_SPEED;
            cam_changed = true;
        }
        if window.is_key_down(Key::Down) {
            camera.y += PAN_SPEED;
            cam_changed = true;
        }

        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Pass) {
            if window.get_mouse_down(MouseButton::Middle) {
                if let Some((ox, oy)) = mid_drag_origin {
                    camera.x = camera_at_drag_start.x - ((mx - ox) / SCALE as f32) as i32;
                    camera.y = camera_at_drag_start.y - ((my - oy) / SCALE as f32) as i32;
                    cam_changed = true;
                } else {
                    mid_drag_origin = Some((mx, my));
                    camera_at_drag_start = camera;
                }
            } else {
                mid_drag_origin = None;
            }
        }

        if cam_changed {
            blit_full_world(&sim, &mut frame, camera);
        }

        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Clamp) {
            let world_p = Vec2i::new(
                mx as i32 / SCALE as i32 + camera.x,
                my as i32 / SCALE as i32 + camera.y,
            );
            let layout = PaletteLayout::new();
            let sx = mx as i32;
            let sy = my as i32;

            if layout.panel_contains(sx, sy) {
                last_left_paint = None;
                last_right_paint = None;
                if window.get_mouse_down(MouseButton::Left) {
                    if let Some(id) = layout.material_at(sx, sy) {
                        selected = id;
                    }
                }
            } else if mid_drag_origin.is_none() {
                if window.get_mouse_down(MouseButton::Left) {
                    if let Some(prev) = last_left_paint {
                        sim.paint_line_brush(prev, world_p, brush_radius, selected);
                    } else {
                        sim.paint_circle(world_p, brush_radius, selected);
                    }
                    last_left_paint = Some(world_p);
                } else {
                    last_left_paint = None;
                }
                if window.get_mouse_down(MouseButton::Right) {
                    if let Some(prev) = last_right_paint {
                        sim.paint_line_brush(prev, world_p, brush_radius, material::EMPTY);
                    } else {
                        sim.paint_circle(world_p, brush_radius, material::EMPTY);
                    }
                    last_right_paint = Some(world_p);
                } else {
                    last_right_paint = None;
                }
                if window.is_key_pressed(Key::R, KeyRepeat::No) {
                    let min = Vec2i::new(world_p.x - 6, world_p.y - 4);
                    let max = Vec2i::new(world_p.x + 6, world_p.y + 4);
                    let _ = sim.spawn_rigid_body_rect(min, max, material::RIGID);
                }
                if window.is_key_pressed(Key::T, KeyRepeat::No) {
                    let _ = sim.spawn_rigid_body_circle(world_p, brush_radius, material::RIGID);
                }
                if window.is_key_pressed(Key::L, KeyRepeat::No) {
                    let _ = sim.spawn_rigid_body_circle(world_p, brush_radius, material::LAVA);
                }
            }
        }

        let now = Instant::now();
        let dt = (now - last_step).as_secs_f32().clamp(1.0 / 240.0, 1.0 / 15.0);
        last_step = now;
        sim_stats = sim.advance_frame(dt);

        let center = camera_center(camera);
        sim.set_focus(center);
        sim.relocate_if_needed(center, SIM_WIDTH as i32, SIM_HEIGHT as i32);

        let render_start = Instant::now();
        blit_dirty_regions(&sim, &mut frame, camera);
        if show_physics {
            draw_physics_debug(&sim, &mut frame, camera);
        }
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
            sim.debug_chunk_step(),
            sim.debug_views_enabled(),
            sim.debug_chunk_step()
                .then_some(sim.debug_chunk_step_stride_frames()),
        );
        draw_perf_hud(&mut frame, sim_stats, render_ms);

        window
            .update_with_buffer(&frame, DISP_W, DISP_H)
            .expect("failed to update frame");
    }
}

fn blit_full_world(sim: &Simulation, frame: &mut [u32], camera: Vec2i) {
    let rect = viewport_rect(camera);
    let pixels = if sim.debug_views_enabled() {
        sim.copy_argb32_for_region_all_debug_views(rect)
    } else if sim.debug_chunk_step() {
        sim.copy_argb32_for_region_chunk_step_viz(rect)
    } else if sim.debug_pass_batch_outlines() {
        sim.copy_argb32_for_region_pass_batch_viz(rect)
    } else if sim.debug_pass_enabled() {
        sim.copy_debug_argb32_for_region(rect)
    } else {
        sim.copy_argb32_for_region(rect)
    };
    upscale_blit(frame, &pixels, 0, 0, SIM_WIDTH, SIM_HEIGHT);
}

fn blit_dirty_regions(sim: &Simulation, frame: &mut [u32], camera: Vec2i) {
    let debug = sim.debug_views_enabled()
        || sim.debug_chunk_step()
        || sim.debug_pass_batch_outlines()
        || sim.debug_pass_enabled();
    if debug {
        blit_full_world(sim, frame, camera);
        return;
    }
    let vp = viewport_rect(camera);
    let dirty = sim.get_dirty_chunks();
    if dirty.is_empty() {
        return;
    }
    if dirty.len() > 96 {
        blit_full_world(sim, frame, camera);
        return;
    }
    for chunk in dirty {
        let clipped = match vp.intersection(&chunk.rect) {
            Some(r) => r,
            None => continue,
        };
        let pixels = sim.copy_argb32_for_region(clipped);
        let w = (clipped.max.x - clipped.min.x + 1) as usize;
        let h = (clipped.max.y - clipped.min.y + 1) as usize;
        let screen_x = (clipped.min.x - camera.x) as usize;
        let screen_y = (clipped.min.y - camera.y) as usize;
        upscale_blit(frame, &pixels, screen_x, screen_y, w, h);
    }
}

fn upscale_blit(
    frame: &mut [u32],
    pixels: &[u32],
    screen_x: usize,
    screen_y: usize,
    w: usize,
    h: usize,
) {
    for sy in 0..h {
        for sx in 0..w {
            let color = pixels[sy * w + sx];
            let dx = (screen_x + sx) * SCALE;
            let dy = (screen_y + sy) * SCALE;
            for oy in 0..SCALE {
                let row = dy + oy;
                if row >= DISP_H {
                    break;
                }
                let base = row * DISP_W + dx;
                for ox in 0..SCALE {
                    let col = dx + ox;
                    if col < DISP_W {
                        frame[base + ox] = color;
                    }
                }
            }
        }
    }
}

fn draw_physics_debug(sim: &Simulation, frame: &mut [u32], camera: Vec2i) {
    const GREEN: u32 = 0xFF00FF40;
    let lines = sim.debug_collider_lines();
    let sf = SCALE as f32;
    for [a, b] in &lines {
        let ax = ((a.0 - camera.x as f32) * sf).round() as i32;
        let ay = ((a.1 - camera.y as f32) * sf).round() as i32;
        let bx = ((b.0 - camera.x as f32) * sf).round() as i32;
        let by = ((b.1 - camera.y as f32) * sf).round() as i32;

        if (ax < 0 && bx < 0)
            || (ax >= DISP_W as i32 && bx >= DISP_W as i32)
            || (ay < 0 && by < 0)
            || (ay >= DISP_H as i32 && by >= DISP_H as i32)
        {
            continue;
        }

        for p in bresenham::bresenham_line(Vec2i::new(ax, ay), Vec2i::new(bx, by)) {
            if p.x >= 0 && p.x < DISP_W as i32 && p.y >= 0 && p.y < DISP_H as i32 {
                frame[p.y as usize * DISP_W + p.x as usize] = GREEN;
            }
        }
    }
}

fn handle_material_shortcuts(window: &Window, selected: &mut MaterialId) {
    const KEYS: &[Key] = &[
        Key::Key0,
        Key::Key1,
        Key::Key2,
        Key::Key3,
        Key::Key4,
        Key::Key5,
        Key::Key6,
        Key::Key7,
        Key::Key8,
        Key::Key9,
        Key::Q,
        Key::W,
        Key::E,
        Key::A,
        Key::S,
        Key::Comma,
        Key::U,
        Key::G,
        Key::H,
        Key::J,
        Key::K,
        Key::L,
        Key::Z,
        Key::X,
        Key::V,
        Key::B,
        Key::N,
        Key::M,
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
    draw_panel(
        frame,
        layout.panel_x,
        layout.panel_y,
        layout.panel_w,
        layout.panel_h,
        0xAA101010,
    );
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
            draw_rect_outline(frame, x - SCALE, 7 * SCALE, 14 * SCALE, 16 * SCALE, 0xFFFFFFFF);
        }
        if idx < 10 {
            draw_digit(frame, idx as u8, x + 3 * SCALE, 10 * SCALE, 0xFF000000);
        }
        x += layout.stride;
    }
}

fn draw_brush_meter(frame: &mut [u32], brush_radius: i32) {
    let layout = PaletteLayout::new();
    let meter_x = layout.swatch_x0 + layout.count * layout.stride + 4 * SCALE;
    let meter_y = 10 * SCALE;
    let w = 56 * SCALE;
    let fill = ((brush_radius.clamp(1, 32) as usize) * w) / 32;
    draw_rect_outline(frame, meter_x, meter_y, w, 8 * SCALE, 0xFFFFFFFF);
    draw_panel(
        frame,
        meter_x + SCALE,
        meter_y + SCALE,
        fill.saturating_sub(2 * SCALE),
        6 * SCALE,
        0xFFFFD166,
    );

    let tens = ((brush_radius / 10) % 10).max(0) as u8;
    let ones = (brush_radius % 10).max(0) as u8;
    if brush_radius >= 10 {
        draw_digit(frame, tens, meter_x + w + 6 * SCALE, meter_y + SCALE, 0xFFFFFFFF);
    }
    draw_digit(frame, ones, meter_x + w + 10 * SCALE, meter_y + SCALE, 0xFFFFFFFF);
}

fn draw_panel(frame: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    for yy in y..(y + h).min(DISP_H) {
        for xx in x..(x + w).min(DISP_W) {
            frame[yy * DISP_W + xx] = color;
        }
    }
}

fn draw_rect_outline(frame: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    if w == 0 || h == 0 {
        return;
    }
    let th = SCALE;
    for t in 0..th {
        for xx in x..(x + w).min(DISP_W) {
            if y + t < DISP_H {
                frame[(y + t) * DISP_W + xx] = color;
            }
            let y2 = y + h - 1 - t;
            if y2 < DISP_H {
                frame[y2 * DISP_W + xx] = color;
            }
        }
        for yy in y..(y + h).min(DISP_H) {
            if x + t < DISP_W {
                frame[yy * DISP_W + x + t] = color;
            }
            let x2 = x + w - 1 - t;
            if x2 < DISP_W {
                frame[yy * DISP_W + x2] = color;
            }
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
                for oy in 0..SCALE {
                    for ox in 0..SCALE {
                        let px = x + col * SCALE + ox;
                        let py = y + row * SCALE + oy;
                        if px < DISP_W && py < DISP_H {
                            frame[py * DISP_W + px] = color;
                        }
                    }
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
            xx += 4 * SCALE;
        }
    }
}

fn draw_fps_top_right(
    frame: &mut [u32],
    fps: u32,
    parallel: bool,
    full_world_debug: bool,
    chunk_step: bool,
    debug_views: bool,
    chunk_stride_frames: Option<u32>,
) {
    let digits = fps.to_string().len().max(1);
    let stride_extra = if chunk_stride_frames.is_some() { 22 * SCALE } else { 0 };
    let text_w = (digits * 4 + 18) * SCALE + stride_extra;
    let x = DISP_W.saturating_sub(text_w + 6 * SCALE);
    let panel_h = if chunk_stride_frames.is_some() { 22 * SCALE } else { 12 * SCALE };
    draw_panel(frame, x, 4 * SCALE, text_w, panel_h, 0xAA101010);
    let mode_color = if full_world_debug {
        0xFFFF9800
    } else if chunk_step {
        0xFFE040FB
    } else if debug_views {
        0xFF00BCD4
    } else if parallel {
        0xFF4CAF50
    } else {
        0xFFE53935
    };
    draw_panel(frame, x + 2 * SCALE, 6 * SCALE, 8 * SCALE, 8 * SCALE, mode_color);
    draw_number(frame, fps, x + 14 * SCALE, 7 * SCALE, 0xFFFFFFFF);
    if let Some(stride) = chunk_stride_frames {
        draw_number(
            frame,
            stride,
            x + (14 + digits * 4 + 6) * SCALE,
            7 * SCALE,
            0xFFFFAB40,
        );
        let bar_x = x + (14 + digits * 4 + 2) * SCALE;
        for yy in (8 * SCALE)..(12 * SCALE) {
            for ox in 0..SCALE {
                if bar_x + ox < DISP_W && yy < DISP_H {
                    frame[yy * DISP_W + bar_x + ox] = 0xFFFFAB40;
                }
            }
        }
    }
}

fn draw_perf_hud(frame: &mut [u32], stats: SimulationStats, render_ms: f32) {
    let x = DISP_W.saturating_sub(140 * SCALE);
    draw_panel(frame, x, 18 * SCALE, 136 * SCALE, 18 * SCALE, 0xAA101010);
    draw_number(frame, stats.sim_ms.round() as u32, x + 4 * SCALE, 21 * SCALE, 0xFF8BC34A);
    draw_number(frame, render_ms.round() as u32, x + 24 * SCALE, 21 * SCALE, 0xFF03A9F4);
    draw_number(frame, stats.active_chunks as u32, x + 50 * SCALE, 21 * SCALE, 0xFFFFC107);
    draw_number(frame, stats.sleeping_chunks as u32, x + 84 * SCALE, 21 * SCALE, 0xFFB0BEC5);
    draw_number(frame, stats.substeps, x + 116 * SCALE, 21 * SCALE, 0xFFFFFFFF);
}
