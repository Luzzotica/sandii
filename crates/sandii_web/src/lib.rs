//! Sandii WASM shell: canvas + DOM controls, `requestAnimationFrame` loop.

mod game;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use falling_everything_core::materials;
use falling_everything_core::world::material;
use falling_everything_core::world::MaterialId;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    window, AddEventListenerOptions, CanvasRenderingContext2d, Document, Element, Event,
    HtmlCanvasElement, HtmlElement, HtmlInputElement, ImageData, KeyboardEvent, MouseEvent,
    PointerEvent, UrlSearchParams,
};

use game::{
    brush_cap_for_mode, cell_flags_line, cell_name_at, clamp_brush_to_mode, grid_for_viewport,
    is_rigid_body_eligible, FrameInput, GameState, InteractionMode,
};

/// How many recent materials the floating quick bar holds.
const QUICK_SLOTS: usize = 4;
/// Wait this long after the last resize before re-laying out the world (rotation settles).
const RESIZE_SETTLE_MS: f64 = 140.0;

/// Clicks are queued here so handlers never `borrow_mut` [`App`] while RAF holds it.
enum UiAction {
    /// Pick a material (from the menu or the quick bar); leaves erase / heat / blast for Draw.
    SelectMaterial(MaterialId),
    SetMode(InteractionMode),
    /// Paint with Empty.
    Erase,
    /// Quick-bar tool button: Paint ⇄ Erase; any other tool goes back to Paint.
    ToggleTool,
    SetBrushRadius(i32),
    ClearWorld,
    TogglePause,
    ToggleDebug,
}

struct UiRefs {
    mat_buttons: Vec<HtmlElement>,
    /// Menu tool buttons: `data-tool` = paint | erase | rigid | blast | heat | cool.
    tool_buttons: Vec<HtmlElement>,
    quick_tool: HtmlElement,
    quick_mats: Vec<HtmlElement>,
    brush_slider: HtmlInputElement,
    brush_value: HtmlElement,
    pause_btn: HtmlElement,
    debug_btn: HtmlElement,
    hud: HtmlElement,
    stats: HtmlElement,
}

struct App {
    game: GameState,
    /// Shared with pointer/keyboard handlers (not inside `Rc<RefCell<App>>`, so RAF cannot nest borrows).
    input: Rc<RefCell<FrameInput>>,
    pending_ui: Rc<RefCell<VecDeque<UiAction>>>,
    prev_input: FrameInput,
    last_perf_ms: f64,
    fps_frames: u32,
    fps_last_ms: f64,
    fps_show: u32,
    rgba_scratch: Vec<u8>,
    /// Background for empty cells, one colour per grid row (a soft night-sky gradient).
    sky: Vec<u32>,
    /// Last real material (not Empty), restored when leaving erase.
    last_material: MaterialId,
    recents: Vec<MaterialId>,
    /// (mode, selected, brush, paused, debug) last pushed to the DOM.
    last_ui: Option<(InteractionMode, MaterialId, i32, bool, bool)>,
    /// Debug HUD line (FPS, parallel, cell probe): off unless `?debug` or toggled with F3.
    debug_hud: Rc<Cell<bool>>,
    /// `performance.now()` of the last resize event (0 = none pending).
    resize_at: Rc<Cell<f64>>,
}

/// Frame timing for the headless benchmark (`bench/bench.mjs`): totals since the last
/// [`bench_reset`]. Cheap enough to keep on always.
#[derive(Default)]
struct BenchStats {
    frames: f64,
    /// Sum of gaps between ticks (ms) and how many gaps.
    frame_ms: f64,
    gaps: f64,
    max_frame_ms: f64,
    /// Input + simulation (`step_frame`).
    sim_ms: f64,
    /// Grid → pixels → canvas.
    render_ms: f64,
    substeps: f64,
    last_tick_ms: f64,
}

thread_local! {
    static BENCH: RefCell<BenchStats> = RefCell::new(BenchStats::default());
    static APP: RefCell<Option<Rc<RefCell<App>>>> = RefCell::new(None);
}

/// Benchmark hook: fill the screen (`water`, `lava`, `mix`, `rain`); see [`GameState::bench_fill`].
#[wasm_bindgen]
pub fn bench_fill(scene: &str) -> bool {
    APP.with(|a| {
        a.borrow()
            .as_ref()
            .map_or(false, |app| app.borrow_mut().game.bench_fill(scene))
    })
}

/// Benchmark hook: zero the frame timing totals.
#[wasm_bindgen]
pub fn bench_reset() {
    BENCH.with(|b| *b.borrow_mut() = BenchStats::default());
}

/// Benchmark hook: `[frames, frame_ms_total, gaps, max_frame_ms, sim_ms_total, render_ms_total,
/// substeps_total, grid_w, grid_h]`.
#[wasm_bindgen]
pub fn bench_stats() -> Vec<f64> {
    let (w, h) = APP.with(|a| {
        a.borrow()
            .as_ref()
            .map_or((0.0, 0.0), |app| {
                let g = &app.borrow().game;
                (g.w as f64, g.h as f64)
            })
    });
    BENCH.with(|b| {
        let b = b.borrow();
        vec![
            b.frames,
            b.frame_ms,
            b.gaps,
            b.max_frame_ms,
            b.sim_ms,
            b.render_ms,
            b.substeps,
            w,
            h,
        ]
    })
}

fn now_ms() -> f64 {
    window().unwrap().performance().unwrap().now()
}

fn sky_rows(h: usize) -> Vec<u32> {
    // Top #0d1120 → bottom #1a1c2c.
    let (r0, g0, b0) = (13.0, 17.0, 32.0);
    let (r1, g1, b1) = (26.0, 28.0, 44.0);
    (0..h)
        .map(|y| {
            let t = y as f64 / (h.max(2) - 1) as f64;
            let r = (r0 + (r1 - r0) * t) as u32;
            let g = (g0 + (g1 - g0) * t) as u32;
            let b = (b0 + (b1 - b0) * t) as u32;
            0xFF00_0000 | (r << 16) | (g << 8) | b
        })
        .collect()
}

impl App {
    fn new(
        input: Rc<RefCell<FrameInput>>,
        pending_ui: Rc<RefCell<VecDeque<UiAction>>>,
        debug_hud: Rc<Cell<bool>>,
        resize_at: Rc<Cell<f64>>,
        w: usize,
        h: usize,
    ) -> Self {
        let perf = now_ms();
        Self {
            game: GameState::new(w, h),
            input,
            pending_ui,
            prev_input: FrameInput::default(),
            last_perf_ms: perf,
            fps_frames: 0,
            fps_last_ms: perf,
            fps_show: 0,
            rgba_scratch: vec![0u8; w * h * 4],
            sky: sky_rows(h),
            last_material: material::SAND,
            recents: vec![material::SAND, material::LIQUID, material::LAVA, material::WOOD],
            last_ui: None,
            debug_hud,
            resize_at,
        }
    }

    fn back_to_paint(&mut self) {
        self.game.set_interaction_mode(InteractionMode::Draw);
        if self.game.selected == material::EMPTY {
            self.game.set_selected_material(self.last_material);
        }
    }

    fn apply_pending_ui(&mut self) {
        let batch: Vec<UiAction> = {
            let mut q = self.pending_ui.borrow_mut();
            q.drain(..).collect()
        };
        for a in batch {
            match a {
                UiAction::SelectMaterial(id) => {
                    let rigid_ok = materials::BUILTINS
                        .iter()
                        .find(|d| d.id == id)
                        .map(is_rigid_body_eligible)
                        .unwrap_or(false);
                    let keep_rigid =
                        self.game.interaction_mode == InteractionMode::RigidBody && rigid_ok;
                    if !keep_rigid {
                        self.game.set_interaction_mode(InteractionMode::Draw);
                    }
                    self.game.set_selected_material(id);
                }
                UiAction::SetMode(InteractionMode::Draw) => self.back_to_paint(),
                UiAction::SetMode(m) => {
                    if self.game.selected == material::EMPTY {
                        self.game.set_selected_material(self.last_material);
                    }
                    self.game.set_interaction_mode(m);
                }
                UiAction::Erase => {
                    self.game.set_interaction_mode(InteractionMode::Draw);
                    self.game.set_selected_material(material::EMPTY);
                }
                UiAction::ToggleTool => {
                    if self.game.interaction_mode == InteractionMode::Draw
                        && self.game.selected != material::EMPTY
                    {
                        self.game.set_selected_material(material::EMPTY);
                    } else {
                        self.back_to_paint();
                    }
                }
                UiAction::SetBrushRadius(r) => {
                    self.game.brush_radius = clamp_brush_to_mode(r, self.game.interaction_mode);
                }
                UiAction::ClearWorld => {
                    self.game.clear_world();
                    toast("Cleared");
                }
                UiAction::TogglePause => self.game.paused = !self.game.paused,
                UiAction::ToggleDebug => self.debug_hud.set(!self.debug_hud.get()),
            }
        }
    }

    /// Re-lay out the grid once resizing has settled.
    fn maybe_resize(&mut self, canvas: &HtmlCanvasElement) {
        let t = self.resize_at.get();
        if t == 0.0 || now_ms() - t < RESIZE_SETTLE_MS {
            return;
        }
        self.resize_at.set(0.0);
        let (w, h) = target_grid(canvas);
        if w == self.game.w && h == self.game.h {
            return;
        }
        self.game.resize(w, h);
        canvas.set_width(w as u32);
        canvas.set_height(h as u32);
        self.rgba_scratch = vec![0u8; w * h * 4];
        self.sky = sky_rows(h);
    }

    fn tick(&mut self, canvas: &HtmlCanvasElement, ctx: &CanvasRenderingContext2d, ui: &UiRefs) {
        self.maybe_resize(canvas);
        self.apply_pending_ui();
        let now = now_ms();
        let dt = ((now - self.last_perf_ms) / 1000.0)
            .max(1.0 / 240.0)
            .min(1.0 / 15.0) as f32;
        self.last_perf_ms = now;

        // Short borrow: DOM work below can synchronously dispatch mouse events that also touch `input`.
        let cur = self
            .input
            .try_borrow()
            .map(|b| b.clone())
            .unwrap_or_else(|_| self.prev_input.clone());
        if cur.clear_pressed {
            toast("Cleared");
        }
        let t_sim = now_ms();
        self.game.step_frame(dt, &cur, &self.prev_input);
        let t_render = now_ms();
        let drawn = self.game.render_frame();
        if let Ok(mut inp) = self.input.try_borrow_mut() {
            inp.clear_transient();
        }

        self.fps_frames = self.fps_frames.saturating_add(1);
        if now - self.fps_last_ms >= 250.0 {
            let elapsed = now - self.fps_last_ms;
            if elapsed > 0.0 {
                self.fps_show = ((self.fps_frames as f64) * 1000.0 / elapsed).round() as u32;
            }
            self.fps_frames = 0;
            self.fps_last_ms = now;
        }

        if drawn {
            fill_rgba(&self.game.frame, self.game.w, &self.sky, &mut self.rgba_scratch);
            let data = ImageData::new_with_u8_clamped_array_and_sh(
                wasm_bindgen::Clamped(&mut self.rgba_scratch),
                self.game.w as u32,
                self.game.h as u32,
            )
            .expect("ImageData");
            ctx.put_image_data(&data, 0.0, 0.0).expect("put_image_data");
        }
        let t_end = now_ms();
        BENCH.with(|b| {
            let mut b = b.borrow_mut();
            b.frames += 1.0;
            b.sim_ms += t_render - t_sim;
            b.render_ms += t_end - t_render;
            b.substeps += self.game.sim.last_stats().substeps as f64;
            if b.last_tick_ms > 0.0 {
                let gap = now - b.last_tick_ms;
                b.frame_ms += gap;
                b.max_frame_ms = b.max_frame_ms.max(gap);
                b.gaps += 1.0;
            }
            b.last_tick_ms = now;
        });

        let show_debug = self.debug_hud.get();
        if show_debug {
            let mut hud = format!(
                "FPS {} · {}×{} · parallel {}",
                self.fps_show,
                self.game.w,
                self.game.h,
                self.game.sim.is_parallel()
            );
            if cur.mouse_x >= 0.0 {
                let w = self.game.world_from_mouse(cur.mouse_x, cur.mouse_y);
                hud.push_str(" · ");
                hud.push_str(&cell_name_at(&self.game.sim, w));
                hud.push_str(" · ");
                hud.push_str(&cell_flags_line(&self.game.sim, w));
            }
            ui.stats.set_inner_text(&hud);
        }

        let state = (
            self.game.interaction_mode,
            self.game.selected,
            self.game.brush_radius,
            self.game.paused,
            show_debug,
        );
        if self.last_ui != Some(state) {
            let prev = self.last_ui;
            if self.game.selected != material::EMPTY {
                self.last_material = self.game.selected;
                if !self.recents.contains(&self.game.selected) {
                    self.recents.insert(0, self.game.selected);
                    self.recents.truncate(QUICK_SLOTS);
                }
            }
            if let Some((pm, psel, _, ppause, _)) = prev {
                if pm != state.0 || psel != state.1 {
                    toast(&tool_label(&self.game));
                } else if ppause != state.3 {
                    toast(if state.3 { "Paused" } else { "Playing" });
                }
            }
            sync_ui(ui, &self.game, &self.recents, show_debug);
            self.last_ui = Some(state);
        }

        self.prev_input = cur;
    }
}

/// ARGB → canvas RGBA bytes. Empty cells render black in the core; paint them with the sky
/// gradient instead.
fn fill_rgba(frame: &[u32], w: usize, sky: &[u32], out: &mut [u8]) {
    for ((y, row), out_row) in frame
        .chunks_exact(w.max(1))
        .enumerate()
        .zip(out.chunks_exact_mut(w.max(1) * 4))
    {
        let bg = sky.get(y).copied().unwrap_or(0xFF00_0000);
        for (&p, o) in row.iter().zip(out_row.chunks_exact_mut(4)) {
            let p = if p & 0x00FF_FFFF == 0 { bg } else { p };
            o.copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 0xff]);
        }
    }
}

fn argb_to_css(argb: u32) -> String {
    let r = (argb >> 16) & 0xff;
    let g = (argb >> 8) & 0xff;
    let b = argb & 0xff;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

fn material_name(id: MaterialId) -> &'static str {
    materials::BUILTINS
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
        .unwrap_or("Material")
}

fn material_color(id: MaterialId) -> String {
    materials::BUILTINS
        .iter()
        .find(|d| d.id == id)
        .map(|d| argb_to_css(d.color_argb))
        .unwrap_or_else(|| "#888".into())
}

/// `data-tool` name of the current tool.
fn tool_key(game: &GameState) -> &'static str {
    match game.interaction_mode {
        InteractionMode::Draw if game.selected == material::EMPTY => "erase",
        InteractionMode::Draw => "paint",
        InteractionMode::RigidBody => "rigid",
        InteractionMode::Explosion => "blast",
        InteractionMode::Heat => "heat",
        InteractionMode::Cool => "cool",
    }
}

fn tool_label(game: &GameState) -> String {
    match game.interaction_mode {
        InteractionMode::Draw if game.selected == material::EMPTY => "Erase".into(),
        InteractionMode::Draw => material_name(game.selected).into(),
        InteractionMode::RigidBody => format!("Rigid {}", material_name(game.selected).to_lowercase()),
        InteractionMode::Explosion => "Blast".into(),
        InteractionMode::Heat => "Heat".into(),
        InteractionMode::Cool => "Cool".into(),
    }
}

/// Brief label over the canvas (`window.sandiiToast` in index.html).
fn toast(text: &str) {
    let Some(w) = window() else { return };
    if let Ok(f) = js_sys::Reflect::get(&w, &JsValue::from_str("sandiiToast")) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call1(&JsValue::NULL, &JsValue::from_str(text));
        }
    }
}

fn set_class(el: &HtmlElement, class: &str, on: bool) {
    let _ = el.class_list().toggle_with_force(class, on);
}

fn sync_ui(ui: &UiRefs, game: &GameState, recents: &[MaterialId], show_debug: bool) {
    use InteractionMode::*;
    let mode = game.interaction_mode;
    let rigid = mode == RigidBody;
    let painting = matches!(mode, Draw | RigidBody) && game.selected != material::EMPTY;
    let tool = tool_key(game);

    for btn in &ui.mat_buttons {
        let id: MaterialId = btn
            .get_attribute("data-mat-id")
            .unwrap_or_default()
            .parse()
            .unwrap_or(0);
        let eligible = materials::BUILTINS
            .iter()
            .find(|d| d.id == id)
            .map(is_rigid_body_eligible)
            .unwrap_or(false);
        let off = rigid && !eligible;
        let _ = btn.toggle_attribute_with_force("disabled", off);
        set_class(btn, "on", painting && game.selected == id);
    }

    for btn in &ui.tool_buttons {
        let on = btn.get_attribute("data-tool").as_deref() == Some(tool);
        set_class(btn, "on", on);
        let _ = btn.set_attribute("aria-pressed", if on { "true" } else { "false" });
    }

    let _ = ui.quick_tool.set_attribute("data-tool", tool);
    set_class(&ui.quick_tool, "on", tool != "paint");
    let _ = ui.quick_tool.set_attribute(
        "aria-label",
        &format!("Tool: {} (tap to switch paint / erase)", tool_label(game)),
    );
    for (i, btn) in ui.quick_mats.iter().enumerate() {
        if let Some(&id) = recents.get(i) {
            let _ = btn.set_attribute("data-mat-id", &id.to_string());
            let _ = btn
                .style()
                .set_property("--swatch", &material_color(id));
            let _ = btn.set_attribute("aria-label", material_name(id));
            let _ = btn.set_attribute("title", material_name(id));
            let _ = btn.remove_attribute("hidden");
            set_class(btn, "on", painting && game.selected == id);
        } else {
            let _ = btn.set_attribute("hidden", "");
        }
    }

    let cap = brush_cap_for_mode(mode);
    let _ = ui.brush_slider.set_attribute("max", &cap.to_string());
    let v = game.brush_radius.clamp(1, cap).to_string();
    ui.brush_slider.set_value(&v);
    ui.brush_value.set_inner_text(&v);

    set_class(&ui.pause_btn, "on", game.paused);
    let _ = ui
        .pause_btn
        .set_attribute("aria-pressed", if game.paused { "true" } else { "false" });
    if let Some(label) = ui.pause_btn.query_selector(".label").ok().flatten() {
        label.set_text_content(Some(if game.paused { "Play" } else { "Pause" }));
    }
    set_class(&ui.debug_btn, "on", show_debug);
    let _ = ui
        .debug_btn
        .set_attribute("aria-pressed", if show_debug { "true" } else { "false" });
    let _ = ui.hud.toggle_attribute_with_force("hidden", !show_debug);
}

/// Grid size for the canvas' current CSS box (it covers the whole viewport).
fn target_grid(canvas: &HtmlCanvasElement) -> (usize, usize) {
    let r = canvas.get_bounding_client_rect();
    let (mut w, mut h) = (r.width(), r.height());
    if w < 1.0 || h < 1.0 {
        let win = window().unwrap();
        w = win.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(800.0);
        h = win.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(600.0);
    }
    grid_for_viewport(w, h)
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {
    window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .expect("requestAnimationFrame");
}

fn canvas_bitmap_coords(canvas: &HtmlCanvasElement, e: &MouseEvent) -> (f32, f32) {
    let rect = canvas
        .dyn_ref::<Element>()
        .expect("canvas is Element")
        .get_bounding_client_rect();
    let cw = canvas.width() as f64;
    let ch = canvas.height() as f64;
    let rw = rect.width();
    let rh = rect.height();
    if rw <= 0.0 || rh <= 0.0 {
        return (-1.0, -1.0);
    }
    let x = (f64::from(e.client_x()) - rect.left()) * (cw / rw);
    let y = (f64::from(e.client_y()) - rect.top()) * (ch / rh);
    (x as f32, y as f32)
}

/// Like [`canvas_bitmap_coords`] but clamped to the bitmap: a captured finger that slides off the
/// canvas keeps painting at the edge instead of reading as "no pointer".
fn canvas_bitmap_coords_clamped(canvas: &HtmlCanvasElement, e: &MouseEvent) -> (f32, f32) {
    let (x, y) = canvas_bitmap_coords(canvas, e);
    if x == -1.0 && y == -1.0 {
        return (x, y);
    }
    let max_x = (canvas.width() as f32 - 1.0).max(0.0);
    let max_y = (canvas.height() as f32 - 1.0).max(0.0);
    (x.clamp(0.0, max_x), y.clamp(0.0, max_y))
}

/// `?debug` (or `?debug=1`) in the page URL turns the debug HUD line on at start.
fn debug_requested() -> bool {
    let search = window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    UrlSearchParams::new_with_str(&search)
        .map(|p| p.has("debug"))
        .unwrap_or(false)
}

fn create_mat_btn(
    document: &Document,
    def: &materials::MaterialDef,
) -> Result<HtmlElement, JsValue> {
    let btn: HtmlElement = document.create_element("button")?.dyn_into()?;
    btn.set_attribute("type", "button")?;
    btn.set_attribute("data-mat-id", &def.id.to_string())?;
    btn.set_class_name("mat");
    btn.set_attribute("aria-label", def.name)?;
    btn.style()
        .set_property("--swatch", &argb_to_css(def.color_argb))?;

    let swatch: HtmlElement = document.create_element("span")?.dyn_into()?;
    swatch.set_class_name("sw");
    let name_el: HtmlElement = document.create_element("span")?.dyn_into()?;
    name_el.set_class_name("nm");
    name_el.set_text_content(Some(def.name));
    btn.append_child(&swatch)?;
    btn.append_child(&name_el)?;
    Ok(btn)
}

fn by_id<T: JsCast>(document: &Document, id: &str) -> Result<T, JsValue> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("#{id} missing")))?
        .dyn_into::<T>()
        .map_err(|_| JsValue::from_str(&format!("#{id} has the wrong type")))
}

fn all_html(document: &Document, sel: &str) -> Result<Vec<HtmlElement>, JsValue> {
    let list = document.query_selector_all(sel)?;
    let mut out = Vec::new();
    for i in 0..list.length() {
        if let Some(n) = list.item(i) {
            if let Ok(el) = n.dyn_into::<HtmlElement>() {
                out.push(el);
            }
        }
    }
    Ok(out)
}

fn on_click(el: &HtmlElement, f: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
    let closure = Closure::wrap(Box::new(f) as Box<dyn FnMut(Event)>);
    el.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
    closure.forget();
    Ok(())
}

/// True while focus is on a control (button, slider, link): Space belongs to it there.
fn typing_target(e: &KeyboardEvent) -> bool {
    e.target()
        .and_then(|t| t.dyn_into::<Element>().ok())
        .map(|el| matches!(el.tag_name().as_str(), "INPUT" | "BUTTON" | "TEXTAREA" | "SELECT" | "A"))
        .unwrap_or(false)
}

fn run() -> Result<(), JsValue> {
    let document: Document = window().unwrap().document().unwrap();

    let canvas: HtmlCanvasElement = by_id(&document, "sandii")?;
    let (gw, gh) = target_grid(&canvas);
    canvas.set_width(gw as u32);
    canvas.set_height(gh as u32);
    canvas.set_tab_index(0);
    let canvas = Rc::new(canvas);

    let ctx = canvas
        .get_context("2d")?
        .unwrap()
        .dyn_into::<CanvasRenderingContext2d>()?;
    ctx.set_image_smoothing_enabled(false);

    let mat_container: Element = by_id(&document, "materials")?;
    mat_container.set_inner_html("");
    let mut mat_buttons: Vec<HtmlElement> = Vec::new();
    for i in 0..materials::paintable_builtin_count() {
        let def = materials::paintable_builtin_at(i).expect("paintable builtin");
        if def.id == material::EMPTY {
            continue; // the Erase tool covers it
        }
        let btn = create_mat_btn(&document, def)?;
        mat_container.append_child(&btn)?;
        mat_buttons.push(btn);
    }

    let ui = Rc::new(UiRefs {
        mat_buttons,
        tool_buttons: all_html(&document, "#tools [data-tool]")?,
        quick_tool: by_id(&document, "quick-tool")?,
        quick_mats: all_html(&document, "#quick .qmat")?,
        brush_slider: by_id(&document, "brush-slider")?,
        brush_value: by_id(&document, "brush-value")?,
        pause_btn: by_id(&document, "act-pause")?,
        debug_btn: by_id(&document, "act-debug")?,
        hud: by_id(&document, "hud")?,
        stats: by_id(&document, "stats")?,
    });

    let input = Rc::new(RefCell::new(FrameInput::default()));
    let pending_ui: Rc<RefCell<VecDeque<UiAction>>> = Rc::new(RefCell::new(VecDeque::new()));
    let debug_hud = Rc::new(Cell::new(debug_requested()));
    let resize_at = Rc::new(Cell::new(0.0f64));
    let app = Rc::new(RefCell::new(App::new(
        input.clone(),
        pending_ui.clone(),
        debug_hud.clone(),
        resize_at.clone(),
        gw,
        gh,
    )));

    APP.with(|a| *a.borrow_mut() = Some(app.clone()));

    let push = {
        let pq = pending_ui.clone();
        move |a: UiAction| pq.borrow_mut().push_back(a)
    };

    for btn in &ui.mat_buttons {
        let id: MaterialId = btn
            .get_attribute("data-mat-id")
            .unwrap_or_default()
            .parse()
            .unwrap_or(0);
        let el = btn.clone();
        let p = push.clone();
        on_click(btn, move |_| {
            if !el.has_attribute("disabled") {
                p(UiAction::SelectMaterial(id));
            }
        })?;
    }
    for btn in &ui.quick_mats {
        let el = btn.clone();
        let p = push.clone();
        on_click(btn, move |_| {
            if let Some(Ok(id)) = el.get_attribute("data-mat-id").map(|v| v.parse::<MaterialId>()) {
                p(UiAction::SelectMaterial(id));
            }
        })?;
    }
    for btn in &ui.tool_buttons {
        let action_for = btn.get_attribute("data-tool").unwrap_or_default();
        let p = push.clone();
        on_click(btn, move |_| {
            p(match action_for.as_str() {
                "erase" => UiAction::Erase,
                "rigid" => UiAction::SetMode(InteractionMode::RigidBody),
                "blast" => UiAction::SetMode(InteractionMode::Explosion),
                "heat" => UiAction::SetMode(InteractionMode::Heat),
                "cool" => UiAction::SetMode(InteractionMode::Cool),
                _ => UiAction::SetMode(InteractionMode::Draw),
            })
        })?;
    }
    {
        let p = push.clone();
        on_click(&ui.quick_tool, move |_| p(UiAction::ToggleTool))?;
        let p = push.clone();
        on_click(&ui.pause_btn, move |_| p(UiAction::TogglePause))?;
        let p = push.clone();
        on_click(&ui.debug_btn, move |_| p(UiAction::ToggleDebug))?;
        let clear: HtmlElement = by_id(&document, "act-clear")?;
        let p = push.clone();
        on_click(&clear, move |_| p(UiAction::ClearWorld))?;
    }
    {
        let p = push.clone();
        let slider_el = ui.brush_slider.clone();
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            if let Ok(v) = slider_el.value().parse::<i32>() {
                p(UiAction::SetBrushRadius(v));
            }
        }) as Box<dyn FnMut(_)>);
        ui.brush_slider
            .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    sync_ui(&ui, &app.borrow().game, &app.borrow().recents, debug_hud.get());

    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: MouseEvent| {
            let (mx, my) = canvas_bitmap_coords(&c, &e);
            let mut g = inp.borrow_mut();
            g.mouse_x = mx;
            g.mouse_y = my;
            g.shift = e.shift_key();
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mousemove", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: MouseEvent| {
            if e.button() == 2 {
                e.prevent_default();
            }
            let (mx, my) = canvas_bitmap_coords(&c, &e);
            let mut g = inp.borrow_mut();
            g.mouse_x = mx;
            g.mouse_y = my;
            g.shift = e.shift_key();
            match e.button() {
                0 => {
                    g.left_down = true;
                    g.press_unseen = true;
                    g.release_pending = false;
                }
                1 => g.middle_down = true,
                2 => g.right_down = true,
                _ => {}
            }
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mousedown", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let closure = Closure::wrap(Box::new(move |e: Event| {
            e.prevent_default();
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("contextmenu", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: MouseEvent| {
            let (mx, my) = canvas_bitmap_coords(&c, &e);
            let mut g = inp.borrow_mut();
            g.mouse_x = mx;
            g.mouse_y = my;
            g.shift = e.shift_key();
            match e.button() {
                // A click shorter than a frame: let one frame see the press (explosions need it).
                0 if g.press_unseen && g.left_down => g.release_pending = true,
                0 => g.left_down = false,
                1 => g.middle_down = false,
                2 => g.right_down = false,
                _ => {}
            }
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: MouseEvent| {
            let (mx, my) = canvas_bitmap_coords(&c, &e);
            let mut g = inp.borrow_mut();
            g.mouse_x = mx;
            g.mouse_y = my;
            g.shift = e.shift_key();
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mouseenter", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let closure = Closure::wrap(Box::new(move |_e: MouseEvent| {
            let mut g = inp.borrow_mut();
            g.mouse_x = -1.0;
            g.mouse_y = -1.0;
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("mouseleave", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    // Touch and pen: Pointer Events. Mouse keeps the mouse handlers above; `preventDefault` on
    // pointerdown suppresses the compatibility mouse events a tap would otherwise also fire.
    // Only the primary pointer paints (one finger); extra fingers are ignored.
    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: PointerEvent| {
            if e.pointer_type() == "mouse" || !e.is_primary() {
                return;
            }
            e.prevent_default();
            let _ = c.set_pointer_capture(e.pointer_id());
            let _ = c.focus();
            let (mx, my) = canvas_bitmap_coords_clamped(&c, &e);
            let mut g = inp.borrow_mut();
            g.mouse_x = mx;
            g.mouse_y = my;
            g.shift = false;
            g.left_down = true;
            g.press_unseen = true;
            g.release_pending = false;
            g.hide_pointer_pending = false;
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("pointerdown", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: PointerEvent| {
            if e.pointer_type() == "mouse" || !e.is_primary() {
                return;
            }
            let mut g = inp.borrow_mut();
            if !g.left_down {
                return;
            }
            e.prevent_default();
            let (mx, my) = canvas_bitmap_coords_clamped(&c, &e);
            g.mouse_x = mx;
            g.mouse_y = my;
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("pointermove", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let c = canvas.clone();
        let closure = Closure::wrap(Box::new(move |e: PointerEvent| {
            if e.pointer_type() == "mouse" || !e.is_primary() {
                return;
            }
            e.prevent_default();
            let _ = c.release_pointer_capture(e.pointer_id());
            let mut g = inp.borrow_mut();
            if !g.left_down {
                return;
            }
            if e.type_() == "pointerup" {
                let (mx, my) = canvas_bitmap_coords_clamped(&c, &e);
                g.mouse_x = mx;
                g.mouse_y = my;
            }
            if g.press_unseen {
                // Tapped and lifted inside one frame: let a frame see the press first.
                g.release_pending = true;
            } else {
                g.left_down = false;
            }
            g.hide_pointer_pending = true;
        }) as Box<dyn FnMut(_)>);
        canvas.add_event_listener_with_callback("pointerup", closure.as_ref().unchecked_ref())?;
        canvas.add_event_listener_with_callback("pointercancel", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    // Belt and braces for browsers that still scroll/zoom on touch despite `touch-action: none`:
    // non-passive touch listeners that cancel the default gesture on the canvas.
    {
        let closure = Closure::wrap(Box::new(move |e: Event| {
            if e.cancelable() {
                e.prevent_default();
            }
        }) as Box<dyn FnMut(_)>);
        let opts = AddEventListenerOptions::new();
        opts.set_passive(false);
        for ty in ["touchstart", "touchmove", "touchend"] {
            canvas.add_event_listener_with_callback_and_add_event_listener_options(
                ty,
                closure.as_ref().unchecked_ref(),
                &opts,
            )?;
        }
        closure.forget();
    }

    {
        let inp = input.clone();
        let dbg = debug_hud.clone();
        let closure = Closure::wrap(Box::new(move |e: KeyboardEvent| {
            // Space presses a focused button / moves a slider; it only pauses from the canvas.
            if typing_target(&e) && e.code() == "Space" {
                return;
            }
            if e.code() == "F3" {
                e.prevent_default();
                if !e.repeat() {
                    dbg.set(!dbg.get());
                }
                return;
            }
            let mut g = inp.borrow_mut();
            g.shift = e.shift_key();
            match e.code().as_str() {
                "BracketLeft" => g.brush_steps -= 1,
                "BracketRight" => g.brush_steps += 1,
                "KeyC" if !e.repeat() => g.clear_pressed = true,
                "KeyP" if !e.repeat() => g.toggle_parallel_pressed = true,
                "KeyT" if !e.repeat() => g.spawn_rigid_circle_pressed = true,
                "KeyL" if !e.repeat() => g.spawn_obsidian_pressed = true,
                "KeyR" if !e.repeat() && e.shift_key() => g.rigid_rect_r_pressed = true,
                "Space" if !e.repeat() => {
                    e.prevent_default();
                    g.toggle_pause_pressed = true;
                }
                _ => {}
            }
        }) as Box<dyn FnMut(_)>);
        document.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let closure = Closure::wrap(Box::new(move |e: KeyboardEvent| {
            let mut g = inp.borrow_mut();
            g.shift = e.shift_key();
        }) as Box<dyn FnMut(_)>);
        document.add_event_listener_with_callback("keyup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let inp = input.clone();
        let closure = Closure::wrap(Box::new(move |_e: MouseEvent| {
            let mut g = inp.borrow_mut();
            if g.press_unseen && g.left_down {
                g.release_pending = true;
            } else {
                g.left_down = false;
            }
            g.right_down = false;
            g.middle_down = false;
        }) as Box<dyn FnMut(_)>);
        window()
            .unwrap()
            .add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let r = resize_at.clone();
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            r.set(now_ms().max(1.0));
        }) as Box<dyn FnMut(_)>);
        let win = window().unwrap();
        win.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())?;
        win.add_event_listener_with_callback("orientationchange", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let a = app.clone();
        let c = ctx.clone();
        let cv = canvas.clone();
        let u = ui.clone();
        let closure = Rc::new(RefCell::new(None::<Closure<dyn FnMut()>>));
        let g = closure.clone();
        *closure.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            {
                let mut app_b = a.borrow_mut();
                app_b.tick(&cv, &c, &u);
            }
            request_animation_frame(g.borrow().as_ref().unwrap());
        }) as Box<dyn FnMut()>));
        request_animation_frame(closure.borrow().as_ref().unwrap());
    }

    Ok(())
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    if let Err(e) = run() {
        web_sys::console::error_1(&e);
    }
}
