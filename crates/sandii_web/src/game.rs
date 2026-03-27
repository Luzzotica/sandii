//! World-only sandbox logic (no CPU palette); UI lives in HTML.

use std::collections::HashSet;

use falling_everything_core::bresenham;
use falling_everything_core::materials;
use falling_everything_core::world::{material, Cell, MaterialId, Phase, RectI, Vec2i};
use falling_everything_core::worldgen::{self, TerrainConfig};
use falling_everything_core::{
    base_strength_for_radius, ExplosionParams, ExplosionSpawn, Simulation, SimulationConfig,
};

/// World size in cells (~70% of 640×360). Display uses 1× scale for lighter web CPU load.
pub const SIM_WIDTH: usize = 448;
pub const SIM_HEIGHT: usize = 252;
pub const SCALE: usize = 1;
pub const DISP_W: usize = SIM_WIDTH * SCALE;
pub const DISP_H: usize = SIM_HEIGHT * SCALE;
pub const TEMP_BRUSH_DELTA: i32 = 40;
/// Max brush radius for most modes (particles, explosions, temperature).
pub const BRUSH_MAX_DEFAULT: i32 = 32;
/// Max brush radius when painting or spawning rigid / physics bodies (web UX cap).
pub const BRUSH_MAX_RIGID_PHYSICS: i32 = 6;

#[inline]
pub fn brush_cap_for_mode(mode: InteractionMode) -> i32 {
    match mode {
        InteractionMode::RigidBody => BRUSH_MAX_RIGID_PHYSICS,
        _ => BRUSH_MAX_DEFAULT,
    }
}

#[inline]
pub fn clamp_brush_to_mode(brush: i32, mode: InteractionMode) -> i32 {
    brush.clamp(1, brush_cap_for_mode(mode))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InteractionMode {
    Draw,
    RigidBody,
    Explosion,
    Heat,
    Cool,
}

impl InteractionMode {
    pub const ALL: [InteractionMode; 5] = [
        Self::Draw,
        Self::RigidBody,
        Self::Explosion,
        Self::Heat,
        Self::Cool,
    ];
}

/// Same rule as the desktop sandbox: only solid, non-empty, non-static materials can become rigid bodies.
pub fn is_rigid_body_eligible(def: &materials::MaterialDef) -> bool {
    def.props.phase() == Phase::Solid
        && def.id != material::EMPTY
        && def.id != material::STONE
        && def.id != material::RIGID
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

fn blit_full_world(sim: &Simulation, frame: &mut [u32], camera: Vec2i) {
    let rect = viewport_rect(camera);
    let pixels = sim.copy_argb32_for_region(rect);
    upscale_blit(frame, &pixels, 0, 0, SIM_WIDTH, SIM_HEIGHT);
}

/// Keyboard / pointer state sampled once per frame (from DOM events).
#[derive(Clone)]
pub struct FrameInput {
    pub mouse_x: f32,
    pub mouse_y: f32,
    pub left_down: bool,
    pub right_down: bool,
    pub middle_down: bool,
    pub shift: bool,
    /// Accumulated bracket keys this frame (negative = smaller brush).
    pub brush_steps: i32,
    pub clear_pressed: bool,
    pub toggle_parallel_pressed: bool,
    pub spawn_rigid_circle_pressed: bool,
    pub spawn_obsidian_pressed: bool,
    pub rigid_rect_r_pressed: bool,
}

impl Default for FrameInput {
    fn default() -> Self {
        Self {
            mouse_x: -1.0,
            mouse_y: -1.0,
            left_down: false,
            right_down: false,
            middle_down: false,
            shift: false,
            brush_steps: 0,
            clear_pressed: false,
            toggle_parallel_pressed: false,
            spawn_rigid_circle_pressed: false,
            spawn_obsidian_pressed: false,
            rigid_rect_r_pressed: false,
        }
    }
}

impl FrameInput {
    pub fn clear_transient(&mut self) {
        self.brush_steps = 0;
        self.clear_pressed = false;
        self.toggle_parallel_pressed = false;
        self.spawn_rigid_circle_pressed = false;
        self.spawn_obsidian_pressed = false;
        self.rigid_rect_r_pressed = false;
    }
}

pub struct GameState {
    /// Used by the WASM shell for HUD (parallel toggle, cell probe).
    pub sim: Simulation,
    pub camera: Vec2i,
    pub frame: Vec<u32>,
    pub selected: MaterialId,
    pub brush_radius: i32,
    pub interaction_mode: InteractionMode,
    last_left_paint: Option<Vec2i>,
    last_right_paint: Option<Vec2i>,
    shift_left_rect_anchor: Option<Vec2i>,
    shift_left_rect_last: Vec2i,
    shift_right_rect_anchor: Option<Vec2i>,
    shift_right_rect_last: Vec2i,
    rigid_rect_corner: Option<Vec2i>,
    rigid_stroke: HashSet<(i32, i32)>,
    rigid_stroke_last: Option<Vec2i>,
    explosion_obliterate: bool,
    explosion_spawn_interior: bool,
    explosion_edge_enabled: bool,
    explosion_edge_spawn: ExplosionSpawn,
}

impl GameState {
    pub fn new() -> Self {
        let mut sim = Simulation::new(SimulationConfig {
            deterministic: true,
            ..SimulationConfig::default()
        });

        let terrain_cfg = TerrainConfig::default();

        let initial_camera = Vec2i::new(
            -(SIM_WIDTH as i32) / 2,
            terrain_cfg.surface_y - (SIM_HEIGHT as i32) / 3,
        );

        // Tight bounds: no panning so the world equals the viewport (saves memory + CPU).
        sim.set_solid_bounds(RectI::new(
            initial_camera,
            Vec2i::new(
                initial_camera.x + SIM_WIDTH as i32 - 1,
                initial_camera.y + SIM_HEIGHT as i32 - 1,
            ),
        ));

        worldgen::paint_terrain(&mut sim, &terrain_cfg);

        // 60 Hz simulation; allow several substeps per frame to catch up after a long frame.
        sim.set_fixed_timestep(1.0 / 60.0, 4);

        Self {
            sim,
            camera: initial_camera,
            frame: vec![0u32; DISP_W * DISP_H],
            selected: material::SAND,
            brush_radius: 4,
            interaction_mode: InteractionMode::Draw,
            last_left_paint: None,
            last_right_paint: None,
            shift_left_rect_anchor: None,
            shift_left_rect_last: Vec2i::new(0, 0),
            shift_right_rect_anchor: None,
            shift_right_rect_last: Vec2i::new(0, 0),
            rigid_rect_corner: None,
            rigid_stroke: HashSet::new(),
            rigid_stroke_last: None,
            explosion_obliterate: false,
            explosion_spawn_interior: false,
            explosion_edge_enabled: true,
            explosion_edge_spawn: ExplosionSpawn {
                material: material::FIRE,
                lifetime: Some(40),
                temperature: Some(1200),
            },
        }
    }

    pub fn set_interaction_mode(&mut self, mode: InteractionMode) {
        self.interaction_mode = mode;
        self.rigid_stroke.clear();
        self.rigid_stroke_last = None;
        if mode == InteractionMode::RigidBody {
            let ok = materials::BUILTINS
                .iter()
                .find(|d| d.id == self.selected)
                .map(is_rigid_body_eligible)
                .unwrap_or(false);
            if !ok {
                if let Some(d) = materials::BUILTINS
                    .iter()
                    .find(|d| is_rigid_body_eligible(d))
                {
                    self.selected = d.id;
                }
            }
        }
        self.brush_radius = clamp_brush_to_mode(self.brush_radius, mode);
    }

    pub fn set_selected_material(&mut self, id: MaterialId) {
        self.selected = id;
    }

    /// Erase the entire visible world (same as the `C` key) and drop in-progress strokes.
    pub fn clear_world(&mut self) {
        self.sim
            .paint_circle(camera_center(self.camera), 10_000, material::EMPTY);
        self.rigid_stroke.clear();
        self.rigid_stroke_last = None;
        self.last_left_paint = None;
        self.last_right_paint = None;
        self.shift_left_rect_anchor = None;
        self.shift_right_rect_anchor = None;
        self.rigid_rect_corner = None;
    }

    pub fn world_from_mouse(&self, mx: f32, my: f32) -> Vec2i {
        let mx = mx.clamp(0.0, (DISP_W.saturating_sub(1)) as f32) as i32;
        let my = my.clamp(0.0, (DISP_H.saturating_sub(1)) as f32) as i32;
        Vec2i::new(
            mx / SCALE as i32 + self.camera.x,
            my / SCALE as i32 + self.camera.y,
        )
    }

    pub fn step_frame(&mut self, dt: f32, input: &FrameInput, prev: &FrameInput) {
        // No panning in web — camera is fixed at initial position.

        if input.brush_steps != 0 {
            let cap = brush_cap_for_mode(self.interaction_mode);
            self.brush_radius = (self.brush_radius + input.brush_steps).clamp(1, cap);
        }
        if input.clear_pressed {
            self.clear_world();
        }
        if input.toggle_parallel_pressed {
            self.sim.set_parallel(!self.sim.is_parallel());
        }
        if input.spawn_rigid_circle_pressed {
            let w = self.world_from_mouse(input.mouse_x, input.mouse_y);
            let r = self.brush_radius.min(BRUSH_MAX_RIGID_PHYSICS);
            let _ = self.sim.spawn_rigid_body_circle(w, r, material::RIGID);
        }
        if input.spawn_obsidian_pressed {
            let w = self.world_from_mouse(input.mouse_x, input.mouse_y);
            let r = self.brush_radius.min(BRUSH_MAX_RIGID_PHYSICS);
            let _ =
                self.sim
                    .spawn_rigid_body_circle_with_temp(w, r, material::OBSIDIAN, Some(1373));
        }

        let left_down = input.left_down;
        let right_down = input.right_down;
        let prev_left_down = prev.left_down;
        let prev_right_down = prev.right_down;
        let shift = input.shift;

        if input.mouse_x >= 0.0 {
            let world_p = self.world_from_mouse(input.mouse_x, input.mouse_y);

            if shift && input.rigid_rect_r_pressed {
                if let Some(a) = self.rigid_rect_corner.take() {
                    let mat = match self.interaction_mode {
                        InteractionMode::Draw
                        | InteractionMode::Explosion
                        | InteractionMode::Heat
                        | InteractionMode::Cool => material::RIGID,
                        InteractionMode::RigidBody => self.selected,
                    };
                    let _ = self.sim.spawn_rigid_body_rect(a, world_p, mat);
                } else {
                    self.rigid_rect_corner = Some(world_p);
                }
            }

            if shift {
                if left_down {
                    if self.shift_left_rect_anchor.is_none() {
                        self.shift_left_rect_anchor = Some(world_p);
                    }
                    self.shift_left_rect_last = world_p;
                } else if prev_left_down {
                    if let Some(a) = self.shift_left_rect_anchor.take() {
                        match self.interaction_mode {
                            InteractionMode::Draw => {
                                self.sim.paint_rect_filled(
                                    a,
                                    self.shift_left_rect_last,
                                    self.selected,
                                );
                            }
                            InteractionMode::RigidBody => {
                                let _ = self.sim.spawn_rigid_body_rect(
                                    a,
                                    self.shift_left_rect_last,
                                    self.selected,
                                );
                            }
                            InteractionMode::Explosion => {}
                            InteractionMode::Heat => {
                                self.sim.adjust_temperature_rect_filled(
                                    a,
                                    self.shift_left_rect_last,
                                    TEMP_BRUSH_DELTA,
                                );
                            }
                            InteractionMode::Cool => {
                                self.sim.adjust_temperature_rect_filled(
                                    a,
                                    self.shift_left_rect_last,
                                    -TEMP_BRUSH_DELTA,
                                );
                            }
                        }
                    }
                }
                if right_down {
                    if self.shift_right_rect_anchor.is_none() {
                        self.shift_right_rect_anchor = Some(world_p);
                    }
                    self.shift_right_rect_last = world_p;
                } else if prev_right_down {
                    if let Some(a) = self.shift_right_rect_anchor.take() {
                        self.sim
                            .paint_rect_filled(a, self.shift_right_rect_last, material::EMPTY);
                    }
                }
            } else {
                self.shift_left_rect_anchor = None;
                self.shift_right_rect_anchor = None;
                match self.interaction_mode {
                    InteractionMode::Draw => {
                        if left_down {
                            if let Some(prev_p) = self.last_left_paint {
                                self.sim.paint_line_brush(
                                    prev_p,
                                    world_p,
                                    self.brush_radius,
                                    self.selected,
                                );
                            } else {
                                self.sim
                                    .paint_circle(world_p, self.brush_radius, self.selected);
                            }
                            self.last_left_paint = Some(world_p);
                        } else {
                            self.last_left_paint = None;
                        }
                        if right_down {
                            if let Some(prev_p) = self.last_right_paint {
                                self.sim.paint_line_brush(
                                    prev_p,
                                    world_p,
                                    self.brush_radius,
                                    material::EMPTY,
                                );
                            } else {
                                self.sim
                                    .paint_circle(world_p, self.brush_radius, material::EMPTY);
                            }
                            self.last_right_paint = Some(world_p);
                        } else {
                            self.last_right_paint = None;
                        }
                    }
                    InteractionMode::RigidBody => {
                        if left_down {
                            if self.rigid_stroke_last.is_none() {
                                self.rigid_stroke.clear();
                                collect_brush_disk(
                                    &mut self.rigid_stroke,
                                    world_p,
                                    self.brush_radius,
                                );
                            } else if let Some(prev_p) = self.rigid_stroke_last {
                                collect_line_brush(
                                    &mut self.rigid_stroke,
                                    prev_p,
                                    world_p,
                                    self.brush_radius,
                                );
                            }
                            self.rigid_stroke_last = Some(world_p);
                        } else if prev_left_down {
                            if !self.rigid_stroke.is_empty() {
                                let positions: Vec<Vec2i> = self
                                    .rigid_stroke
                                    .iter()
                                    .map(|(x, y)| Vec2i::new(*x, *y))
                                    .collect();
                                let _ = self
                                    .sim
                                    .spawn_rigid_body_from_pixels(&positions, self.selected);
                            }
                            self.rigid_stroke.clear();
                            self.rigid_stroke_last = None;
                        }
                        self.last_left_paint = None;
                        if right_down {
                            if let Some(prev_p) = self.last_right_paint {
                                self.sim.paint_line_brush(
                                    prev_p,
                                    world_p,
                                    self.brush_radius,
                                    material::EMPTY,
                                );
                            } else {
                                self.sim
                                    .paint_circle(world_p, self.brush_radius, material::EMPTY);
                            }
                            self.last_right_paint = Some(world_p);
                        } else {
                            self.last_right_paint = None;
                        }
                    }
                    InteractionMode::Explosion => {
                        self.last_left_paint = None;
                        self.last_right_paint = None;
                        self.rigid_stroke.clear();
                        self.rigid_stroke_last = None;
                        if left_down && !prev_left_down {
                            let r = self.brush_radius.max(1);
                            let fill_on_destroy = if self.explosion_spawn_interior
                                && self.selected != material::EMPTY
                            {
                                Some(ExplosionSpawn {
                                    material: self.selected,
                                    lifetime: None,
                                    temperature: None,
                                })
                            } else {
                                None
                            };
                            let edge_on_destroy = if self.explosion_edge_enabled {
                                Some(self.explosion_edge_spawn)
                            } else {
                                None
                            };
                            self.sim.apply_explosion(ExplosionParams {
                                center: world_p,
                                radius: r,
                                base_strength: base_strength_for_radius(r),
                                obliterate_disk: self.explosion_obliterate,
                                fill_on_destroy,
                                edge_on_destroy,
                                edge_band_inward: 2,
                            });
                        }
                    }
                    InteractionMode::Heat => {
                        self.last_right_paint = None;
                        self.rigid_stroke.clear();
                        self.rigid_stroke_last = None;
                        if left_down {
                            if let Some(prev_p) = self.last_left_paint {
                                adjust_temperature_line_brush(
                                    &mut self.sim,
                                    prev_p,
                                    world_p,
                                    self.brush_radius,
                                    TEMP_BRUSH_DELTA,
                                );
                            } else {
                                self.sim.adjust_temperature_disk(
                                    world_p,
                                    self.brush_radius,
                                    TEMP_BRUSH_DELTA,
                                );
                            }
                            self.last_left_paint = Some(world_p);
                        } else {
                            self.last_left_paint = None;
                        }
                    }
                    InteractionMode::Cool => {
                        self.last_right_paint = None;
                        self.rigid_stroke.clear();
                        self.rigid_stroke_last = None;
                        if left_down {
                            if let Some(prev_p) = self.last_left_paint {
                                adjust_temperature_line_brush(
                                    &mut self.sim,
                                    prev_p,
                                    world_p,
                                    self.brush_radius,
                                    -TEMP_BRUSH_DELTA,
                                );
                            } else {
                                self.sim.adjust_temperature_disk(
                                    world_p,
                                    self.brush_radius,
                                    -TEMP_BRUSH_DELTA,
                                );
                            }
                            self.last_left_paint = Some(world_p);
                        } else {
                            self.last_left_paint = None;
                        }
                    }
                }
            }
        }

        let _stats = self.sim.advance_frame(dt);
        blit_full_world(&self.sim, &mut self.frame, self.camera);
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

/// Cell name at world position for HUD (optional).
pub fn cell_name_at(sim: &Simulation, world: Vec2i) -> String {
    let cell: Cell = sim.cell(world);
    let id = cell.material();
    if let Some(def) = materials::BUILTINS.iter().find(|d| d.id == id) {
        def.name.to_string()
    } else {
        format!("id {id}")
    }
}

pub fn cell_flags_line(sim: &Simulation, world: Vec2i) -> String {
    let cell: Cell = sim.cell(world);
    format!("flags 0x{:x}", cell.flags())
}
