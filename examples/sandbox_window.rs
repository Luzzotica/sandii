use std::collections::HashSet;
use std::time::Instant;

use falling_everything_core::bresenham;
use falling_everything_core::materials;
use falling_everything_core::world::{cell_flags, material, Cell, MaterialId, Phase, RectI, Vec2i};
use falling_everything_core::worldgen::{self, TerrainConfig};
use falling_everything_core::{
    base_strength_for_radius, ExplosionParams, ExplosionSpawn, Simulation, SimulationConfig,
    SimulationStats,
};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Scale, Window, WindowOptions};

const SIM_WIDTH: usize = 640;
const SIM_HEIGHT: usize = 360;
const SCALE: usize = 2;
const DISP_W: usize = SIM_WIDTH * SCALE;
const DISP_H: usize = SIM_HEIGHT * SCALE;
const TARGET_FPS: usize = 60;
const PAN_SPEED: i32 = 8;
/// Kelvin added/removed per heat or cool brush stamp (disk).
const TEMP_BRUSH_DELTA: i32 = 40;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InteractionMode {
    /// Paint / erase immediately (brush and Shift+rect paint).
    Draw,
    /// LMB drag defines pixels; rigid body spawns on mouse release. Shift+drag rect → rigid rectangle on release.
    RigidBody,
    /// LMB click: Bresenham chord explosion at cursor; `[` / `]` adjust blast radius.
    Explosion,
    /// LMB: add heat (Kelvin) in brush; Shift+rect fills rectangle.
    Heat,
    /// LMB: remove heat (Kelvin) in brush; Shift+rect fills rectangle.
    Cool,
}

fn is_rigid_body_eligible(def: &materials::MaterialDef) -> bool {
    def.props.phase() == Phase::Solid
        && def.id != material::EMPTY
        && def.id != material::STONE
        && def.id != material::RIGID
}

struct PaletteLayout {
    panel_x: usize,
    panel_y: usize,
    panel_w: usize,
    panel_h: usize,
    /// Y offset from panel top to swatch row (after header text).
    swatch_y_off: usize,
    swatch_w: usize,
    swatch_h: usize,
    stride: usize,
    count: usize,
    /// Y offset from panel top to first shortcut help line.
    help_y_off: usize,
    mode_btn_y_off: usize,
    mode_btn_h: usize,
    thermal_btn_y_off: usize,
}

impl PaletteLayout {
    const INNER: usize = 4 * SCALE;
    /// One line of 5×7 text (scale 1) + gap.
    const LINE: usize = 12;

    fn new() -> Self {
        let count = materials::paintable_builtin_count();
        let panel_w = (count * 16 + 70) * SCALE;
        let swatch_w = 12 * SCALE;
        let swatch_h = 14 * SCALE;
        let stride = 16 * SCALE;

        let header_lines = 2;
        let swatch_y_off = Self::INNER + header_lines * Self::LINE;
        let after_swatches = swatch_y_off + swatch_h + 4;
        let brush_row_h = Self::LINE;
        let help_lines = 6;
        let help_y_off = after_swatches + brush_row_h + 2;
        let mode_btn_y_off = help_y_off + help_lines * Self::LINE + 6;
        let mode_btn_h = 14 * SCALE;
        let thermal_btn_y_off = mode_btn_y_off + mode_btn_h + 4;
        let panel_h = thermal_btn_y_off + mode_btn_h + Self::INNER + 4;

        Self {
            panel_x: 4 * SCALE,
            panel_y: 4 * SCALE,
            panel_w,
            panel_h,
            swatch_y_off,
            swatch_w,
            swatch_h,
            stride,
            count,
            help_y_off,
            mode_btn_y_off,
            mode_btn_h,
            thermal_btn_y_off,
        }
    }

    fn swatch_base_x(&self) -> usize {
        self.panel_x + Self::INNER
    }

    fn swatch_base_y(&self) -> usize {
        self.panel_y + self.swatch_y_off
    }

    fn panel_contains(&self, mx: i32, my: i32) -> bool {
        mx >= self.panel_x as i32
            && mx < (self.panel_x + self.panel_w) as i32
            && my >= self.panel_y as i32
            && my < (self.panel_y + self.panel_h) as i32
    }

    fn material_at(&self, mx: i32, my: i32, mode: InteractionMode) -> Option<MaterialId> {
        let bx = self.swatch_base_x() as i32;
        let by = self.swatch_base_y() as i32;
        for i in 0..self.count {
            let sx = bx + (i * self.stride) as i32;
            if mx >= sx
                && mx < sx + self.swatch_w as i32
                && my >= by
                && my < by + self.swatch_h as i32
            {
                let def = materials::paintable_builtin_at(i).expect("paintable material");
                if mode == InteractionMode::RigidBody && !is_rigid_body_eligible(def) {
                    return None;
                }
                return Some(def.id);
            }
        }
        None
    }

    fn brush_row_y(&self) -> usize {
        self.panel_y + self.swatch_y_off + self.swatch_h + 4
    }

    /// Large clickable mode buttons at bottom of panel.
    fn mode_click(&self, mx: i32, my: i32) -> Option<InteractionMode> {
        let x0 = (self.panel_x + Self::INNER) as i32;
        let y0 = (self.panel_y + self.mode_btn_y_off) as i32;
        let w = (self.panel_w - 2 * Self::INNER) as i32;
        let h = self.mode_btn_h as i32;
        if mx < x0 || my < y0 || mx >= x0 + w || my >= y0 + h {
            return None;
        }
        let t = w / 3;
        let lx = mx - x0;
        if lx < t {
            Some(InteractionMode::Draw)
        } else if lx < 2 * t {
            Some(InteractionMode::RigidBody)
        } else {
            Some(InteractionMode::Explosion)
        }
    }

    /// Heat / cool row below main mode buttons.
    fn thermal_mode_click(&self, mx: i32, my: i32) -> Option<InteractionMode> {
        let x0 = (self.panel_x + Self::INNER) as i32;
        let y0 = (self.panel_y + self.thermal_btn_y_off) as i32;
        let w = (self.panel_w - 2 * Self::INNER) as i32;
        let h = self.mode_btn_h as i32;
        if mx < x0 || my < y0 || mx >= x0 + w || my >= y0 + h {
            return None;
        }
        let half = w / 2;
        let lx = mx - x0;
        if lx < half {
            Some(InteractionMode::Heat)
        } else {
            Some(InteractionMode::Cool)
        }
    }
}

fn viewport_rect(camera: Vec2i) -> RectI {
    RectI::new(
        camera,
        Vec2i::new(
            camera.x + SIM_WIDTH as i32 - 1,
            camera.y + SIM_HEIGHT as i32 - 1,
        ),
    )
}

fn camera_center(camera: Vec2i) -> Vec2i {
    Vec2i::new(
        camera.x + SIM_WIDTH as i32 / 2,
        camera.y + SIM_HEIGHT as i32 / 2,
    )
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

    let mat_list: String = (0..materials::paintable_builtin_count())
        .filter_map(materials::paintable_builtin_at)
        .enumerate()
        .map(|(i, d)| format!("{} {}", i, d.name))
        .collect::<Vec<_>>()
        .join(" ");
    let title = format!(
        "Sandii Sandbox - {} | Tab cycles modes (Draw/Rigid/Explode/Heat/Cool) | [] radius in Explode | Arrows pan | T/L spawn | F physics | C clear",
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
    let mut prev_left_down = false;
    let mut prev_right_down = false;
    let mut shift_left_rect_anchor: Option<Vec2i> = None;
    let mut shift_left_rect_last = Vec2i::new(0, 0);
    let mut shift_right_rect_anchor: Option<Vec2i> = None;
    let mut shift_right_rect_last = Vec2i::new(0, 0);
    let mut rigid_rect_corner: Option<Vec2i> = None;
    let mut interaction_mode = InteractionMode::Draw;
    let mut rigid_stroke: HashSet<(i32, i32)> = HashSet::new();
    let mut rigid_stroke_last: Option<Vec2i> = None;
    let mut last_step = Instant::now();
    let mut fps_last = Instant::now();
    let mut fps_frames: u32 = 0;
    let mut fps_display: u32 = 0;
    let mut render_ms: f32;
    let mut sim_stats;

    let mut camera = initial_camera;
    let mut hover_world: Option<Vec2i>;
    let mut hover_over_panel: bool;
    let mut mid_drag_origin: Option<(f32, f32)> = None;
    let mut camera_at_drag_start = camera;
    let mut show_physics = false;
    /// Full-disk erase (no ray simulation).
    let mut explosion_obliterate = false;
    /// Destroyed interior becomes selected material (when not EMPTY).
    let mut explosion_spawn_interior = false;
    /// Outer annulus uses `explosion_edge_spawn` when true.
    let mut explosion_edge_enabled = true;
    let mut explosion_edge_spawn = ExplosionSpawn {
        material: material::FIRE,
        lifetime: Some(40),
        temperature: Some(1200),
    };

    blit_full_world(&sim, &mut frame, camera);

    while window.is_open() && !window.is_key_down(Key::Escape) {
        hover_world = None;
        hover_over_panel = false;
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
                let s = sim
                    .debug_chunk_step_stride_frames()
                    .saturating_sub(1)
                    .max(1);
                sim.set_debug_chunk_step_stride_frames(s);
            }
            if window.is_key_pressed(Key::Equal, KeyRepeat::Yes) {
                let s = sim
                    .debug_chunk_step_stride_frames()
                    .saturating_add(1)
                    .min(128);
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
        if window.is_key_pressed(Key::Tab, KeyRepeat::No) {
            interaction_mode = match interaction_mode {
                InteractionMode::Draw => InteractionMode::RigidBody,
                InteractionMode::RigidBody => InteractionMode::Explosion,
                InteractionMode::Explosion => InteractionMode::Heat,
                InteractionMode::Heat => InteractionMode::Cool,
                InteractionMode::Cool => InteractionMode::Draw,
            };
            rigid_stroke.clear();
            rigid_stroke_last = None;
        }
        if interaction_mode == InteractionMode::Explosion {
            if window.is_key_pressed(Key::O, KeyRepeat::No) {
                explosion_obliterate = !explosion_obliterate;
            }
            if window.is_key_pressed(Key::I, KeyRepeat::No) {
                let shift =
                    window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
                if shift {
                    explosion_spawn_interior = false;
                } else {
                    explosion_spawn_interior = !explosion_spawn_interior;
                }
            }
            if window.is_key_pressed(Key::E, KeyRepeat::No) {
                let shift =
                    window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
                if shift {
                    explosion_edge_enabled = !explosion_edge_enabled;
                } else {
                    explosion_edge_spawn = ExplosionSpawn {
                        material: selected,
                        lifetime: None,
                        temperature: None,
                    };
                    explosion_edge_enabled = true;
                }
            }
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

        // Pass matches framebuffer coords more reliably than Clamp on some platforms (e.g. macOS).
        if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Pass) {
            let mx = mx.clamp(0.0, (DISP_W.saturating_sub(1)) as f32);
            let my = my.clamp(0.0, (DISP_H.saturating_sub(1)) as f32);
            let mx = mx as i32;
            let my = my as i32;
            let world_p = Vec2i::new(
                mx as i32 / SCALE as i32 + camera.x,
                my as i32 / SCALE as i32 + camera.y,
            );
            let layout = PaletteLayout::new();
            let sx = mx as i32;
            let sy = my as i32;
            hover_world = Some(world_p);
            hover_over_panel = layout.panel_contains(sx, sy);

            if layout.panel_contains(sx, sy) {
                last_left_paint = None;
                last_right_paint = None;
                rigid_stroke.clear();
                rigid_stroke_last = None;
                if window.get_mouse_down(MouseButton::Left) {
                    if let Some(id) = layout.material_at(sx, sy, interaction_mode) {
                        selected = id;
                    } else if !prev_left_down {
                        if let Some(m) = layout.mode_click(sx, sy) {
                            interaction_mode = m;
                            rigid_stroke.clear();
                            rigid_stroke_last = None;
                        } else if let Some(m) = layout.thermal_mode_click(sx, sy) {
                            interaction_mode = m;
                            rigid_stroke.clear();
                            rigid_stroke_last = None;
                        }
                    }
                }
            } else if mid_drag_origin.is_none() {
                let shift =
                    window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
                let left_down = window.get_mouse_down(MouseButton::Left);
                let right_down = window.get_mouse_down(MouseButton::Right);

                if shift && !layout.panel_contains(sx, sy) {
                    if left_down {
                        if shift_left_rect_anchor.is_none() {
                            shift_left_rect_anchor = Some(world_p);
                        }
                        shift_left_rect_last = world_p;
                    } else if prev_left_down {
                        if let Some(a) = shift_left_rect_anchor.take() {
                            match interaction_mode {
                                InteractionMode::Draw => {
                                    sim.paint_rect_filled(a, shift_left_rect_last, selected);
                                }
                                InteractionMode::RigidBody => {
                                    let _ = sim.spawn_rigid_body_rect(
                                        a,
                                        shift_left_rect_last,
                                        selected,
                                    );
                                }
                                InteractionMode::Explosion => {}
                                InteractionMode::Heat => {
                                    sim.adjust_temperature_rect_filled(
                                        a,
                                        shift_left_rect_last,
                                        TEMP_BRUSH_DELTA,
                                    );
                                }
                                InteractionMode::Cool => {
                                    sim.adjust_temperature_rect_filled(
                                        a,
                                        shift_left_rect_last,
                                        -TEMP_BRUSH_DELTA,
                                    );
                                }
                            }
                        }
                    }
                    if right_down {
                        if shift_right_rect_anchor.is_none() {
                            shift_right_rect_anchor = Some(world_p);
                        }
                        shift_right_rect_last = world_p;
                    } else if prev_right_down {
                        if let Some(a) = shift_right_rect_anchor.take() {
                            sim.paint_rect_filled(a, shift_right_rect_last, material::EMPTY);
                        }
                    }
                } else {
                    if !shift {
                        shift_left_rect_anchor = None;
                        shift_right_rect_anchor = None;
                    }
                    match interaction_mode {
                        InteractionMode::Draw => {
                            if left_down {
                                if let Some(prev) = last_left_paint {
                                    sim.paint_line_brush(prev, world_p, brush_radius, selected);
                                } else {
                                    sim.paint_circle(world_p, brush_radius, selected);
                                }
                                last_left_paint = Some(world_p);
                            } else {
                                last_left_paint = None;
                            }
                            if right_down {
                                if let Some(prev) = last_right_paint {
                                    sim.paint_line_brush(
                                        prev,
                                        world_p,
                                        brush_radius,
                                        material::EMPTY,
                                    );
                                } else {
                                    sim.paint_circle(world_p, brush_radius, material::EMPTY);
                                }
                                last_right_paint = Some(world_p);
                            } else {
                                last_right_paint = None;
                            }
                        }
                        InteractionMode::RigidBody => {
                            if left_down {
                                if rigid_stroke_last.is_none() {
                                    rigid_stroke.clear();
                                    collect_brush_disk(&mut rigid_stroke, world_p, brush_radius);
                                } else if let Some(prev) = rigid_stroke_last {
                                    collect_line_brush(
                                        &mut rigid_stroke,
                                        prev,
                                        world_p,
                                        brush_radius,
                                    );
                                }
                                rigid_stroke_last = Some(world_p);
                            } else if prev_left_down {
                                if !rigid_stroke.is_empty() {
                                    let positions: Vec<Vec2i> = rigid_stroke
                                        .iter()
                                        .map(|(x, y)| Vec2i::new(*x, *y))
                                        .collect();
                                    let _ = sim.spawn_rigid_body_from_pixels(&positions, selected);
                                }
                                rigid_stroke.clear();
                                rigid_stroke_last = None;
                            }
                            last_left_paint = None;
                            if right_down {
                                if let Some(prev) = last_right_paint {
                                    sim.paint_line_brush(
                                        prev,
                                        world_p,
                                        brush_radius,
                                        material::EMPTY,
                                    );
                                } else {
                                    sim.paint_circle(world_p, brush_radius, material::EMPTY);
                                }
                                last_right_paint = Some(world_p);
                            } else {
                                last_right_paint = None;
                            }
                        }
                        InteractionMode::Explosion => {
                            last_left_paint = None;
                            last_right_paint = None;
                            rigid_stroke.clear();
                            rigid_stroke_last = None;
                            if left_down && !prev_left_down {
                                let r = brush_radius.max(1);
                                let fill_on_destroy =
                                    if explosion_spawn_interior && selected != material::EMPTY {
                                        Some(ExplosionSpawn {
                                            material: selected,
                                            lifetime: None,
                                            temperature: None,
                                        })
                                    } else {
                                        None
                                    };
                                let edge_on_destroy = if explosion_edge_enabled {
                                    Some(explosion_edge_spawn)
                                } else {
                                    None
                                };
                                sim.apply_explosion(ExplosionParams {
                                    center: world_p,
                                    radius: r,
                                    base_strength: base_strength_for_radius(r),
                                    obliterate_disk: explosion_obliterate,
                                    fill_on_destroy,
                                    edge_on_destroy,
                                    edge_band_inward: 2,
                                });
                            }
                        }
                        InteractionMode::Heat => {
                            last_right_paint = None;
                            rigid_stroke.clear();
                            rigid_stroke_last = None;
                            if left_down {
                                if let Some(prev) = last_left_paint {
                                    adjust_temperature_line_brush(
                                        &mut sim,
                                        prev,
                                        world_p,
                                        brush_radius,
                                        TEMP_BRUSH_DELTA,
                                    );
                                } else {
                                    sim.adjust_temperature_disk(
                                        world_p,
                                        brush_radius,
                                        TEMP_BRUSH_DELTA,
                                    );
                                }
                                last_left_paint = Some(world_p);
                            } else {
                                last_left_paint = None;
                            }
                        }
                        InteractionMode::Cool => {
                            last_right_paint = None;
                            rigid_stroke.clear();
                            rigid_stroke_last = None;
                            if left_down {
                                if let Some(prev) = last_left_paint {
                                    adjust_temperature_line_brush(
                                        &mut sim,
                                        prev,
                                        world_p,
                                        brush_radius,
                                        -TEMP_BRUSH_DELTA,
                                    );
                                } else {
                                    sim.adjust_temperature_disk(
                                        world_p,
                                        brush_radius,
                                        -TEMP_BRUSH_DELTA,
                                    );
                                }
                                last_left_paint = Some(world_p);
                            } else {
                                last_left_paint = None;
                            }
                        }
                    }
                }

                if shift
                    && window.is_key_pressed(Key::R, KeyRepeat::No)
                    && !layout.panel_contains(sx, sy)
                {
                    if let Some(a) = rigid_rect_corner.take() {
                        let mat = match interaction_mode {
                            InteractionMode::Draw
                            | InteractionMode::Explosion
                            | InteractionMode::Heat
                            | InteractionMode::Cool => material::RIGID,
                            InteractionMode::RigidBody => selected,
                        };
                        let _ = sim.spawn_rigid_body_rect(a, world_p, mat);
                    } else {
                        rigid_rect_corner = Some(world_p);
                    }
                }
                if window.is_key_pressed(Key::T, KeyRepeat::No) {
                    let _ = sim.spawn_rigid_body_circle(world_p, brush_radius, material::RIGID);
                }
                if window.is_key_pressed(Key::L, KeyRepeat::No) {
                    let _ = sim.spawn_rigid_body_circle_with_temp(
                        world_p,
                        brush_radius,
                        material::OBSIDIAN,
                        Some(1373),
                    );
                }
            }
        }

        let now = Instant::now();
        let dt = (now - last_step)
            .as_secs_f32()
            .clamp(1.0 / 240.0, 1.0 / 15.0);
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
        if interaction_mode == InteractionMode::RigidBody {
            draw_rigid_mode_chrome(&mut frame);
            if !rigid_stroke.is_empty() {
                draw_rigid_stroke_preview(&mut frame, camera, &rigid_stroke);
            }
        }
        render_ms = render_start.elapsed().as_secs_f32() * 1000.0;

        fps_frames = fps_frames.saturating_add(1);
        let fps_elapsed = fps_last.elapsed().as_secs_f32();
        if fps_elapsed >= 0.25 {
            fps_display = (fps_frames as f32 / fps_elapsed).round() as u32;
            fps_frames = 0;
            fps_last = Instant::now();
        }
        draw_ui_hint(
            &mut frame,
            selected,
            brush_radius,
            interaction_mode,
            explosion_obliterate,
            explosion_spawn_interior,
            explosion_edge_enabled,
            explosion_edge_spawn.material,
        );
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
        draw_cell_inspector_hud(&mut frame, &sim, hover_world, hover_over_panel);

        prev_left_down = window.get_mouse_down(MouseButton::Left);
        prev_right_down = window.get_mouse_down(MouseButton::Right);

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
        if i >= materials::paintable_builtin_count() {
            break;
        }
        if window.is_key_pressed(key, KeyRepeat::No) {
            *selected = materials::paintable_builtin_at(i)
                .expect("paintable material")
                .id;
            return;
        }
    }
}

fn collect_brush_disk(into: &mut HashSet<(i32, i32)>, center: Vec2i, radius: i32) {
    let r2 = radius * radius;
    for y in (center.y - radius)..=(center.y + radius) {
        for x in (center.x - radius)..=(center.x + radius) {
            let dx = x - center.x;
            let dy = y - center.y;
            if dx * dx + dy * dy <= r2 {
                into.insert((x, y));
            }
        }
    }
}

fn collect_line_brush(into: &mut HashSet<(i32, i32)>, a: Vec2i, b: Vec2i, radius: i32) {
    for p in bresenham::bresenham_line(a, b) {
        collect_brush_disk(into, p, radius);
    }
}

fn adjust_temperature_line_brush(
    sim: &mut Simulation,
    a: Vec2i,
    b: Vec2i,
    brush_radius: i32,
    delta: i32,
) {
    for p in bresenham::bresenham_line(a, b) {
        sim.adjust_temperature_disk(p, brush_radius, delta);
    }
}

/// Orange frame so rigid mode is obvious even before the first drag.
fn draw_rigid_mode_chrome(frame: &mut [u32]) {
    let th = (2 * SCALE).max(3);
    let c = 0xFFFF9800;
    let w = DISP_W;
    let h = DISP_H;
    for t in 0..th {
        for x in 0..w {
            frame[t * DISP_W + x] = c;
            frame[(h - 1 - t) * DISP_W + x] = c;
        }
    }
    for t in 0..th {
        for y in 0..h {
            frame[y * DISP_W + t] = c;
            frame[y * DISP_W + (w - 1 - t)] = c;
        }
    }
}

/// 5×7 pixel font (bit 4 = left column). Used for sandbox HUD labels.
fn glyph_5x7_rows(c: u8) -> Option<[u8; 7]> {
    let c = c.to_ascii_uppercase();
    Some(match c {
        b'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x15, 0x13, 0x12, 0x0D],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        b'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x02, 0x04, 0x08, 0x10, 0x10, 0x1F],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        _ => return None,
    })
}

fn text_width_5x7(s: &str, ps: usize) -> usize {
    let mut w = 0usize;
    for b in s.bytes() {
        if b == b' ' {
            w += 3 * ps;
        } else if glyph_5x7_rows(b).is_some() {
            w += 6 * ps;
        }
    }
    w
}

fn draw_str_5x7(frame: &mut [u32], s: &str, mut x: usize, y: usize, color: u32, ps: usize) {
    for b in s.bytes() {
        if b == b' ' {
            x += 3 * ps;
            continue;
        }
        let Some(rows) = glyph_5x7_rows(b) else {
            continue;
        };
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..5 {
                if (bits >> (4 - col)) & 1 == 0 {
                    continue;
                }
                for dy in 0..ps {
                    for dx in 0..ps {
                        let px = x + col * ps + dx;
                        let py = y + row * ps + dy;
                        if px < DISP_W && py < DISP_H {
                            frame[py * DISP_W + px] = color;
                        }
                    }
                }
            }
        }
        x += 6 * ps;
    }
}

fn draw_rigid_stroke_preview(frame: &mut [u32], camera: Vec2i, stroke: &HashSet<(i32, i32)>) {
    let hi = 0xCCFFEB3B;
    for &(wx, wy) in stroke {
        let sx = wx - camera.x;
        let sy = wy - camera.y;
        if sx < 0 || sy < 0 || sx >= SIM_WIDTH as i32 || sy >= SIM_HEIGHT as i32 {
            continue;
        }
        let px = sx as usize * SCALE;
        let py = sy as usize * SCALE;
        for oy in 0..SCALE {
            for ox in 0..SCALE {
                let pxx = px + ox;
                let pyy = py + oy;
                if pxx < DISP_W && pyy < DISP_H {
                    frame[pyy * DISP_W + pxx] = hi;
                }
            }
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

fn draw_ui_hint(
    frame: &mut [u32],
    material: MaterialId,
    brush_radius: i32,
    mode: InteractionMode,
    explosion_obliterate: bool,
    explosion_spawn_interior: bool,
    explosion_edge_enabled: bool,
    explosion_edge_material: MaterialId,
) {
    let layout = PaletteLayout::new();
    draw_panel(
        frame,
        layout.panel_x,
        layout.panel_y,
        layout.panel_w,
        layout.panel_h,
        0xCC101018,
    );
    let tx = layout.panel_x + PaletteLayout::INNER;
    let mut ty = layout.panel_y + PaletteLayout::INNER;
    draw_str_5x7(frame, "CLICK A COLORED SQUARE", tx, ty, 0xFFFFFFFF, 1);
    ty += PaletteLayout::LINE;
    draw_str_5x7(frame, "TO CHOOSE BRUSH MATERIAL", tx, ty, 0xFFB3E5FC, 1);
    draw_material_palette(frame, material, &layout, mode);
    draw_brush_row(frame, brush_radius, &layout);
    draw_help_lines(
        frame,
        &layout,
        mode,
        explosion_obliterate,
        explosion_spawn_interior,
        explosion_edge_enabled,
        explosion_edge_material,
    );
    draw_mode_buttons(frame, &layout, mode);
    draw_thermal_buttons(frame, &layout, mode);
}

fn draw_help_lines(
    frame: &mut [u32],
    layout: &PaletteLayout,
    mode: InteractionMode,
    explosion_obliterate: bool,
    explosion_spawn_interior: bool,
    explosion_edge_enabled: bool,
    explosion_edge_material: MaterialId,
) {
    let tx = layout.panel_x + PaletteLayout::INNER;
    let mut ty = layout.panel_y + layout.help_y_off;
    draw_str_5x7(frame, "SHIFT LEFT DRAG  RECT PAINT", tx, ty, 0xFFFFE082, 1);
    ty += PaletteLayout::LINE;
    draw_str_5x7(frame, "SHIFT RIGHT DRAG RECT ERASE", tx, ty, 0xFF90CAF9, 1);
    ty += PaletteLayout::LINE;
    draw_str_5x7(
        frame,
        "SHIFT R TWICE      RIGID RECT",
        tx,
        ty,
        0xFFA5D6A7,
        1,
    );
    ty += PaletteLayout::LINE;
    draw_str_5x7(frame, "EXPLODE LMB      [] RADIUS", tx, ty, 0xFFFFAB91, 1);
    ty += PaletteLayout::LINE;
    if mode == InteractionMode::Explosion {
        let nuke = if explosion_obliterate { "ON " } else { "OFF" };
        let fill = if explosion_spawn_interior {
            "ON "
        } else {
            "OFF"
        };
        let edge = if explosion_edge_enabled { "ON " } else { "OFF" };
        let edge_name = materials::BUILTINS
            .iter()
            .find(|d| d.id == explosion_edge_material)
            .map(|d| d.name)
            .unwrap_or("?");
        let line5 = format!("O NUKE {} I FILL {}", nuke, fill);
        draw_str_5x7(frame, &line5, tx, ty, 0xFFFFCC80, 1);
        ty += PaletteLayout::LINE;
        let line6 = format!("E CAPTURE EDGE {}  SHF-E RING {}", edge_name, edge);
        draw_str_5x7(frame, &line6, tx, ty, 0xFFFFCC80, 1);
    } else if mode == InteractionMode::Heat {
        draw_str_5x7(frame, "HEAT LMB ADDS TEMP", tx, ty, 0xFFFF8A65, 1);
        ty += PaletteLayout::LINE;
        draw_str_5x7(frame, "TAB CYCLE MODES", tx, ty, 0xFF9E9E9E, 1);
    } else if mode == InteractionMode::Cool {
        draw_str_5x7(frame, "COOL LMB SUB TEMP", tx, ty, 0xFF4FC3F7, 1);
        ty += PaletteLayout::LINE;
        draw_str_5x7(frame, "TAB CYCLE MODES", tx, ty, 0xFF9E9E9E, 1);
    } else {
        draw_str_5x7(frame, "TAB CYCLE MODES", tx, ty, 0xFF9E9E9E, 1);
        ty += PaletteLayout::LINE;
        draw_str_5x7(frame, "EXPLODE O I E KEYS", tx, ty, 0xFF9E9E9E, 1);
    }
}

/// Three mode segments (Draw / Rigid / Explosion). Heat and cool are on a second row.
fn draw_mode_buttons(frame: &mut [u32], layout: &PaletteLayout, mode: InteractionMode) {
    let x = layout.panel_x + PaletteLayout::INNER;
    let y = layout.panel_y + layout.mode_btn_y_off;
    let w = layout.panel_w - 2 * PaletteLayout::INNER;
    let h = layout.mode_btn_h;
    let tw = w / 3;
    let seg = [
        (InteractionMode::Draw, 0xFF2E7D32, 0xFF37474F),
        (InteractionMode::RigidBody, 0xFFE65100, 0xFF37474F),
        (InteractionMode::Explosion, 0xFF6D1B7B, 0xFF37474F),
    ];
    for (i, (m, on_c, off_c)) in seg.iter().enumerate() {
        let x0 = x + i * tw;
        let on = mode == *m;
        draw_panel(frame, x0, y, tw, h, if on { *on_c } else { *off_c });
        let border = if on { 0xFFFFFFFF } else { 0xFF888888 };
        draw_rect_outline(frame, x0, y, tw, h, border);
    }

    let ty = y + 3 * SCALE;
    let labels = [["DRAW", "PAINT"], ["RIGID", "REL."], ["BLAST", "LMB"]];
    let subcols = [0xFFC8E6C9u32, 0xFFFFE0B2u32, 0xFFE1BEE7u32];
    for i in 0..3 {
        let cxi = x + i * tw + tw / 2;
        for li in 0..2 {
            let s = labels[i][li];
            let col = if li == 0 { 0xFFFFFFFFu32 } else { subcols[i] };
            let sw = text_width_5x7(s, 1);
            draw_str_5x7(frame, s, cxi.saturating_sub(sw / 2), ty + li * 9, col, 1);
        }
    }
}

fn draw_thermal_buttons(frame: &mut [u32], layout: &PaletteLayout, mode: InteractionMode) {
    let x = layout.panel_x + PaletteLayout::INNER;
    let y = layout.panel_y + layout.thermal_btn_y_off;
    let w = layout.panel_w - 2 * PaletteLayout::INNER;
    let h = layout.mode_btn_h;
    let half = w / 2;
    let seg = [
        (InteractionMode::Heat, 0xFFD84315, 0xFF37474F),
        (InteractionMode::Cool, 0xFF0277BD, 0xFF37474F),
    ];
    for (i, (m, on_c, off_c)) in seg.iter().enumerate() {
        let x0 = x + i * half;
        let on = mode == *m;
        draw_panel(frame, x0, y, half, h, if on { *on_c } else { *off_c });
        let border = if on { 0xFFFFFFFF } else { 0xFF888888 };
        draw_rect_outline(frame, x0, y, half, h, border);
    }
    let ty = y + 3 * SCALE;
    let labels = [["HEAT", "ADD K"], ["COOL", "SUB K"]];
    let subcols = [0xFFFFCCBCu32, 0xFFB3E5FCu32];
    for i in 0..2 {
        let cxi = x + i * half + half / 2;
        for li in 0..2 {
            let s = labels[i][li];
            let col = if li == 0 { 0xFFFFFFFFu32 } else { subcols[i] };
            let sw = text_width_5x7(s, 1);
            draw_str_5x7(frame, s, cxi.saturating_sub(sw / 2), ty + li * 9, col, 1);
        }
    }
}

fn draw_material_palette(
    frame: &mut [u32],
    selected: MaterialId,
    layout: &PaletteLayout,
    mode: InteractionMode,
) {
    let mut x = layout.swatch_base_x();
    let sy = layout.swatch_base_y();
    for idx in 0..layout.count {
        let def = materials::paintable_builtin_at(idx).expect("paintable material");
        let disabled = mode == InteractionMode::RigidBody && !is_rigid_body_eligible(def);
        let color = if disabled {
            dim_argb(def.color_argb)
        } else {
            def.color_argb
        };
        draw_panel(frame, x, sy, layout.swatch_w, layout.swatch_h, color);
        if def.id == selected && !disabled {
            draw_rect_outline(
                frame,
                x.saturating_sub(SCALE),
                sy.saturating_sub(SCALE),
                layout.swatch_w + 2 * SCALE,
                layout.swatch_h + 2 * SCALE,
                0xFFFFFFFF,
            );
        }
        if idx < 10 {
            draw_digit(
                frame,
                idx as u8,
                x + 3 * SCALE,
                sy + 10 * SCALE,
                if disabled { 0xFF444444 } else { 0xFF000000 },
            );
        }
        x += layout.stride;
    }
}

fn dim_argb(c: u32) -> u32 {
    let a = c & 0xFF000000;
    let r = ((c >> 16) & 0xFF) / 4;
    let g = ((c >> 8) & 0xFF) / 4;
    let b = (c & 0xFF) / 4;
    a | (r << 16) | (g << 8) | b
}

fn draw_brush_row(frame: &mut [u32], brush_radius: i32, layout: &PaletteLayout) {
    let y = layout.brush_row_y();
    let tx = layout.swatch_base_x();
    draw_str_5x7(frame, "BRUSH SIZE", tx, y + 2, 0xFFFFD166, 1);

    let meter_w = 48 * SCALE;
    let meter_x = layout.panel_x + layout.panel_w - PaletteLayout::INNER - meter_w - 14 * SCALE;
    let bracket_lbl = "BRACKET KEYS";
    let lx = meter_x.saturating_sub(text_width_5x7(bracket_lbl, 1) + 6);
    draw_str_5x7(frame, bracket_lbl, lx, y + 2, 0xFF9E9E9E, 1);
    let fill = ((brush_radius.clamp(1, 32) as usize) * meter_w) / 32;
    draw_rect_outline(frame, meter_x, y, meter_w, 8 * SCALE, 0xFFFFFFFF);
    draw_panel(
        frame,
        meter_x + SCALE,
        y + SCALE,
        fill.saturating_sub(2 * SCALE),
        6 * SCALE,
        0xFFFFD166,
    );
    let tens = ((brush_radius / 10) % 10).max(0) as u8;
    let ones = (brush_radius % 10).max(0) as u8;
    let num_x = meter_x + meter_w + 4 * SCALE;
    if brush_radius >= 10 {
        draw_digit(frame, tens, num_x, y + 2 * SCALE, 0xFFFFFFFF);
    }
    draw_digit(
        frame,
        ones,
        num_x + if brush_radius >= 10 { 4 * SCALE } else { 0 },
        y + 2 * SCALE,
        0xFFFFFFFF,
    );
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
    let stride_extra = if chunk_stride_frames.is_some() {
        22 * SCALE
    } else {
        0
    };
    let text_w = (digits * 4 + 18) * SCALE + stride_extra;
    let x = DISP_W.saturating_sub(text_w + 6 * SCALE);
    let panel_h = if chunk_stride_frames.is_some() {
        22 * SCALE
    } else {
        12 * SCALE
    };
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
    draw_panel(
        frame,
        x + 2 * SCALE,
        6 * SCALE,
        8 * SCALE,
        8 * SCALE,
        mode_color,
    );
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

fn material_name_for_id(id: MaterialId) -> &'static str {
    materials::BUILTINS
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
        .unwrap_or("UNKNOWN")
}

fn truncate_hud_label(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max_chars.saturating_sub(2)).collect();
    t.push_str("..");
    t
}

fn digit_count_u32(n: u32) -> usize {
    if n == 0 {
        1
    } else {
        n.to_string().len()
    }
}

/// Returns horizontal pixel width consumed.
fn draw_i32_at(frame: &mut [u32], v: i32, x: usize, y: usize, color: u32) -> usize {
    if v < 0 {
        draw_str_5x7(frame, "-", x, y, color, 1);
        let abs = (v as i64).unsigned_abs().min(u32::MAX as u64) as u32;
        draw_number(frame, abs, x + 6, y, color);
        6 + digit_count_u32(abs) * 4 * SCALE
    } else {
        draw_number(frame, v as u32, x, y, color);
        digit_count_u32(v as u32) * 4 * SCALE
    }
}

fn flags_subtext(cell: Cell) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    let f = cell.flags();
    if f & cell_flags::IS_FREE_FALLING != 0 {
        parts.push("FF");
    }
    if f & cell_flags::WET != 0 {
        parts.push("WET");
    }
    if f & cell_flags::ELECTRIFIED != 0 {
        parts.push("ELEC");
    }
    if f & cell_flags::RIGID_BODY_SIM != 0 {
        parts.push("RBS");
    }
    if f & cell_flags::RIGID_PIXEL != 0 {
        parts.push("RIG");
    }
    if parts.is_empty() {
        "NONE".to_string()
    } else {
        parts.join(" ")
    }
}

fn draw_cell_inspector_hud(
    frame: &mut [u32],
    sim: &Simulation,
    hover_world: Option<Vec2i>,
    hover_over_panel: bool,
) {
    let Some(world_p) = hover_world else {
        return;
    };

    let inner = 4 * SCALE;
    let line_h = PaletteLayout::LINE;
    let ps = 1;
    let px = 4 * SCALE;
    let panel_w = 268 * SCALE;

    let mut line_count = 4usize;
    if !hover_over_panel {
        let c = sim.cell(world_p);
        if c.rigid_source_material() != 0 {
            line_count += 1;
        }
        if c.flags() != 0 {
            line_count += 1;
        }
    }

    let panel_h = inner * 2 + line_count * line_h + 4;
    let py = DISP_H.saturating_sub(panel_h + 4 * SCALE);
    draw_panel(frame, px, py, panel_w, panel_h, 0xCC101018);

    let tx = px + inner;
    let mut ty = py + inner;

    draw_str_5x7(frame, "W", tx, ty, 0xFF80CBC4, ps);
    let mut cx = tx + 6 * ps;
    cx += draw_i32_at(frame, world_p.x, cx, ty, 0xFFE0E0E0);
    draw_str_5x7(frame, " ", cx, ty, 0xFFE0E0E0, ps);
    cx += 3 * ps;
    draw_i32_at(frame, world_p.y, cx, ty, 0xFFE0E0E0);
    ty += line_h;

    if hover_over_panel {
        draw_str_5x7(frame, "OVER UI", tx, ty, 0xFFFFB74D, ps);
        return;
    }

    let c = sim.cell(world_p);
    let mat_line = truncate_hud_label(material_name_for_id(c.material()), 32);
    draw_str_5x7(frame, &mat_line, tx, ty, 0xFFFFFFFF, ps);
    ty += line_h;

    draw_str_5x7(frame, "TEMP ", tx, ty, 0xFFFFB74D, ps);
    let tw = text_width_5x7("TEMP ", ps);
    let num_x = tx + tw;
    draw_number(frame, c.temperature() as u32, num_x, ty, 0xFFE0E0E0);
    let nd = c.temperature().to_string().len();
    draw_str_5x7(frame, " K", num_x + nd * 4 * ps, ty, 0xFF9E9E9E, ps);
    ty += line_h;

    draw_str_5x7(frame, "F ", tx, ty, 0xFFFFB74D, ps);
    draw_number(
        frame,
        c.flags() as u32,
        tx + text_width_5x7("F ", ps),
        ty,
        0xFFE0E0E0,
    );
    ty += line_h;

    if c.flags() != 0 {
        let fl = truncate_hud_label(&flags_subtext(c), 40);
        draw_str_5x7(frame, &fl, tx, ty, 0xFFFFCC80, ps);
        ty += line_h;
    }

    let rsrc = c.rigid_source_material();
    if rsrc != 0 {
        draw_str_5x7(frame, "SRC ", tx, ty, 0xFFFFB74D, ps);
        let src_name = truncate_hud_label(material_name_for_id(rsrc), 28);
        draw_str_5x7(
            frame,
            &src_name,
            tx + text_width_5x7("SRC ", ps),
            ty,
            0xFFE0E0E0,
            ps,
        );
    }
}

fn draw_perf_hud(frame: &mut [u32], stats: SimulationStats, render_ms: f32) {
    let x = DISP_W.saturating_sub(140 * SCALE);
    draw_panel(frame, x, 18 * SCALE, 136 * SCALE, 18 * SCALE, 0xAA101010);
    draw_number(
        frame,
        stats.sim_ms.round() as u32,
        x + 4 * SCALE,
        21 * SCALE,
        0xFF8BC34A,
    );
    draw_number(
        frame,
        render_ms.round() as u32,
        x + 24 * SCALE,
        21 * SCALE,
        0xFF03A9F4,
    );
    draw_number(
        frame,
        stats.active_chunks as u32,
        x + 50 * SCALE,
        21 * SCALE,
        0xFFFFC107,
    );
    draw_number(
        frame,
        stats.sleeping_chunks as u32,
        x + 84 * SCALE,
        21 * SCALE,
        0xFFB0BEC5,
    );
    draw_number(
        frame,
        stats.substeps,
        x + 116 * SCALE,
        21 * SCALE,
        0xFFFFFFFF,
    );
}
