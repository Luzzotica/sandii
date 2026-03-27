//! Sandii WASM shell: canvas + DOM controls, `requestAnimationFrame` loop.

mod game;
mod material_blurbs;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use falling_everything_core::materials;
use falling_everything_core::world::material;
use falling_everything_core::world::MaterialId;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    window, CanvasRenderingContext2d, Document, Element, Event, HtmlCanvasElement, HtmlElement,
    HtmlInputElement, ImageData, KeyboardEvent, MouseEvent,
};

use game::{
    brush_cap_for_mode, cell_flags_line, cell_name_at, clamp_brush_to_mode, is_rigid_body_eligible,
    FrameInput, GameState, InteractionMode, DISP_H, DISP_W,
};

/// Material/mode clicks are queued here so handlers never `borrow_mut` [`App`] while RAF holds it.
enum UiAction {
    SelectMaterial(MaterialId),
    SetMode(InteractionMode),
    SetBrushRadius(i32),
    ClearWorld,
}

struct UiRefs {
    mat_container: Element,
    mat_buttons: Vec<HtmlElement>,
    mode_buttons: Vec<HtmlElement>,
    mode_help: HtmlElement,
    brush_slider: HtmlInputElement,
    brush_value: HtmlElement,
    material_title: HtmlElement,
    material_body: HtmlElement,
    selection_mode: HtmlElement,
    selection_sep: HtmlElement,
    selection_mat_wrap: HtmlElement,
    selection_swatch: HtmlElement,
    selection_name: HtmlElement,
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
    last_ui_mode: InteractionMode,
    last_ui_mat: MaterialId,
    last_ui_brush: i32,
}

impl App {
    fn new(input: Rc<RefCell<FrameInput>>, pending_ui: Rc<RefCell<VecDeque<UiAction>>>) -> Self {
        let perf = window().unwrap().performance().unwrap().now();
        Self {
            game: GameState::new(),
            input,
            pending_ui,
            prev_input: FrameInput::default(),
            last_perf_ms: perf,
            fps_frames: 0,
            fps_last_ms: perf,
            fps_show: 0,
            rgba_scratch: vec![0u8; DISP_W * DISP_H * 4],
            last_ui_mode: InteractionMode::Draw,
            last_ui_mat: material::SAND,
            last_ui_brush: 4,
        }
    }

    fn apply_pending_ui(&mut self) -> bool {
        let batch: Vec<UiAction> = {
            let mut q = self.pending_ui.borrow_mut();
            q.drain(..).collect()
        };
        if batch.is_empty() {
            return false;
        }
        for a in batch {
            match a {
                UiAction::SelectMaterial(id) => self.game.set_selected_material(id),
                UiAction::SetMode(m) => self.game.set_interaction_mode(m),
                UiAction::SetBrushRadius(r) => {
                    self.game.brush_radius = clamp_brush_to_mode(r, self.game.interaction_mode);
                }
                UiAction::ClearWorld => self.game.clear_world(),
            }
        }
        true
    }

    fn tick(&mut self, ctx: &CanvasRenderingContext2d, stats: &HtmlElement, ui: &UiRefs) {
        let ui_from_queue = self.apply_pending_ui();
        let now = window().unwrap().performance().unwrap().now();
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
        self.game.step_frame(dt, &cur, &self.prev_input);
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

        fill_argb_to_rgba_bytes(&self.game.frame, &mut self.rgba_scratch);
        let data = ImageData::new_with_u8_clamped_array_and_sh(
            wasm_bindgen::Clamped(&mut self.rgba_scratch),
            DISP_W as u32,
            DISP_H as u32,
        )
        .expect("ImageData");
        ctx.put_image_data(&data, 0.0, 0.0).expect("put_image_data");

        let mut hud = String::new();
        hud.push_str(&format!(
            "FPS {} | parallel {} | ",
            self.fps_show,
            self.game.sim.is_parallel()
        ));
        if cur.mouse_x >= 0.0 {
            let w = self.game.world_from_mouse(cur.mouse_x, cur.mouse_y);
            hud.push_str(&cell_name_at(&self.game.sim, w));
            hud.push_str(" | ");
            hud.push_str(&cell_flags_line(&self.game.sim, w));
        }
        stats.set_inner_text(&hud);

        if ui_from_queue
            || self.game.interaction_mode != self.last_ui_mode
            || self.game.selected != self.last_ui_mat
            || self.game.brush_radius != self.last_ui_brush
        {
            sync_ui(ui, &self.game);
            self.last_ui_mode = self.game.interaction_mode;
            self.last_ui_mat = self.game.selected;
            self.last_ui_brush = self.game.brush_radius;
        }

        self.prev_input = cur;
    }
}

fn fill_argb_to_rgba_bytes(frame: &[u32], out: &mut [u8]) {
    for (i, &p) in frame.iter().enumerate() {
        let o = i * 4;
        out[o] = ((p >> 16) & 0xff) as u8;
        out[o + 1] = ((p >> 8) & 0xff) as u8;
        out[o + 2] = (p & 0xff) as u8;
        out[o + 3] = ((p >> 24) & 0xff) as u8;
    }
}

fn argb_to_css(argb: u32) -> String {
    let r = (argb >> 16) & 0xff;
    let g = (argb >> 8) & 0xff;
    let b = argb & 0xff;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

fn mode_display_name(mode: InteractionMode) -> &'static str {
    match mode {
        InteractionMode::Draw => "Draw",
        InteractionMode::RigidBody => "Rigid",
        InteractionMode::Explosion => "Explosion",
        InteractionMode::Heat => "Heat",
        InteractionMode::Cool => "Cool",
    }
}

fn mode_help_text(mode: InteractionMode) -> &'static str {
    match mode {
        InteractionMode::Draw => "Draw: left paint, right erase. Shift+drag rectangles.",
        InteractionMode::RigidBody => {
            "Rigid: left-drag stroke, release to spawn. Only solid materials (see grid)."
        }
        InteractionMode::Explosion => {
            "Explosion: left click to blast. Brush size = radius. Material picker hidden."
        }
        InteractionMode::Heat => {
            "Heat: left paint or drag to add temperature. Material picker hidden."
        }
        InteractionMode::Cool => "Cool: left paint or drag to remove heat. Material picker hidden.",
    }
}

fn sync_ui(ui: &UiRefs, game: &GameState) {
    use InteractionMode::*;

    let show_materials = matches!(game.interaction_mode, Draw | RigidBody);

    if let Some(he) = ui.mat_container.dyn_ref::<HtmlElement>() {
        let _ = he
            .style()
            .set_property("display", if show_materials { "grid" } else { "none" });
    }

    if show_materials {
        if let Some(def) = materials::BUILTINS.iter().find(|d| d.id == game.selected) {
            ui.material_title.set_inner_text(def.name);
        } else {
            ui.material_title.set_inner_text("Material");
        }
        ui.material_body
            .set_inner_text(material_blurbs::material_description(game.selected));
    } else {
        ui.material_title.set_inner_text("Materials");
        ui.material_body
            .set_inner_text(material_blurbs::MODE_NO_MATERIAL_PICKER);
    }

    for btn in &ui.mat_buttons {
        let id_str = btn.get_attribute("data-mat-id").unwrap_or_default();
        let id: MaterialId = id_str.parse().unwrap_or(0);
        let rigid = game.interaction_mode == RigidBody;
        let eligible = materials::BUILTINS
            .iter()
            .find(|d| d.id == id)
            .map(is_rigid_body_eligible)
            .unwrap_or(false);
        if rigid && !eligible {
            let _ = btn.set_attribute("disabled", "");
        } else {
            let _ = btn.remove_attribute("disabled");
        }
        let mut cls = String::from("mat-btn");
        if game.selected == id && show_materials {
            cls.push_str(" mat-btn--selected");
        }
        if rigid && !eligible {
            cls.push_str(" mat-btn--ineligible");
        }
        btn.set_class_name(&cls);
    }

    for btn in &ui.mode_buttons {
        let idx_str = btn.get_attribute("data-mode-idx").unwrap_or_default();
        let idx: usize = idx_str.parse().unwrap_or(0);
        let active = InteractionMode::ALL.get(idx).copied() == Some(game.interaction_mode);
        btn.set_class_name(if active {
            "mode-btn mode-btn--active"
        } else {
            "mode-btn"
        });
    }

    ui.mode_help
        .set_inner_text(mode_help_text(game.interaction_mode));

    let cap = brush_cap_for_mode(game.interaction_mode);
    let _ = ui.brush_slider.set_attribute("min", "1");
    let _ = ui.brush_slider.set_attribute("max", &cap.to_string());
    ui.brush_slider
        .set_value(&game.brush_radius.clamp(1, cap).to_string());
    ui.brush_value
        .set_inner_text(&game.brush_radius.clamp(1, cap).to_string());

    ui.selection_mode
        .set_inner_text(mode_display_name(game.interaction_mode));
    if show_materials {
        let _ = ui.selection_sep.remove_attribute("hidden");
        let _ = ui.selection_mat_wrap.remove_attribute("hidden");
        if let Some(def) = materials::BUILTINS.iter().find(|d| d.id == game.selected) {
            ui.selection_name.set_inner_text(def.name);
            let _ = ui
                .selection_swatch
                .style()
                .set_property("background", &argb_to_css(def.color_argb));
        } else {
            ui.selection_name.set_inner_text("—");
            let _ = ui
                .selection_swatch
                .style()
                .set_property("background", "#444");
        }
    } else {
        let _ = ui.selection_sep.set_attribute("hidden", "");
        let _ = ui.selection_mat_wrap.set_attribute("hidden", "");
    }
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

fn create_mat_btn(
    document: &Document,
    def: &materials::MaterialDef,
) -> Result<HtmlElement, JsValue> {
    let btn: HtmlElement = document.create_element("button")?.dyn_into()?;
    btn.set_attribute("type", "button")?;
    btn.set_attribute("data-mat-id", &def.id.to_string())?;
    btn.set_class_name("mat-btn");

    let swatch: HtmlElement = document.create_element("span")?.dyn_into()?;
    swatch.set_class_name("mat-swatch");
    swatch
        .style()
        .set_property("background", &argb_to_css(def.color_argb))?;

    let name_el: HtmlElement = document.create_element("span")?.dyn_into()?;
    name_el.set_class_name("mat-name");
    name_el.set_text_content(Some(def.name));

    btn.append_child(&swatch)?;
    btn.append_child(&name_el)?;
    Ok(btn)
}

fn create_mode_btn(document: &Document, idx: usize, label: &str) -> Result<HtmlElement, JsValue> {
    let btn: HtmlElement = document.create_element("button")?.dyn_into()?;
    btn.set_attribute("type", "button")?;
    btn.set_attribute("data-mode-idx", &idx.to_string())?;
    btn.set_class_name("mode-btn");
    btn.set_text_content(Some(label));
    Ok(btn)
}

fn fit_canvas(canvas: &HtmlCanvasElement) {
    let win = window().unwrap();
    let wrap = win
        .document()
        .unwrap()
        .get_element_by_id("game-wrap")
        .unwrap();
    let rect = wrap.get_bounding_client_rect();
    let w = rect.width();
    let h = rect.height();
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let aspect = DISP_W as f64 / DISP_H as f64;
    let (css_w, css_h) = if w / h > aspect {
        (h * aspect, h)
    } else {
        (w, w / aspect)
    };
    let s = canvas.style();
    let _ = s.set_property("width", &format!("{css_w:.0}px"));
    let _ = s.set_property("height", &format!("{css_h:.0}px"));
}

fn run() -> Result<(), JsValue> {
    let document: Document = window().unwrap().document().unwrap();

    let canvas = document
        .get_element_by_id("sandii")
        .expect("#sandii canvas")
        .dyn_into::<HtmlCanvasElement>()?;
    canvas.set_width(DISP_W as u32);
    canvas.set_height(DISP_H as u32);
    canvas.set_tab_index(0);
    let canvas = Rc::new(canvas);

    fit_canvas(&canvas);

    let ctx = canvas
        .get_context("2d")?
        .unwrap()
        .dyn_into::<CanvasRenderingContext2d>()?;
    ctx.set_image_smoothing_enabled(false);

    let stats = document
        .get_element_by_id("stats")
        .expect("#stats")
        .dyn_into::<HtmlElement>()?;

    let mat_container = document
        .get_element_by_id("panel-materials")
        .expect("#panel-materials")
        .dyn_into::<Element>()?;
    mat_container.set_inner_html("");

    let mode_row = document
        .get_element_by_id("panel-modes")
        .expect("#panel-modes")
        .dyn_into::<Element>()?;
    mode_row.set_inner_html("");

    let mode_help = document
        .get_element_by_id("mode-help")
        .expect("#mode-help")
        .dyn_into::<HtmlElement>()?;

    let brush_slider = document
        .get_element_by_id("brush-slider")
        .expect("#brush-slider")
        .dyn_into::<HtmlInputElement>()?;

    let brush_value = document
        .get_element_by_id("brush-value")
        .expect("#brush-value")
        .dyn_into::<HtmlElement>()?;

    let material_title = document
        .get_element_by_id("material-detail-title")
        .expect("#material-detail-title")
        .dyn_into::<HtmlElement>()?;

    let material_body = document
        .get_element_by_id("material-detail-body")
        .expect("#material-detail-body")
        .dyn_into::<HtmlElement>()?;

    let selection_mode = document
        .get_element_by_id("selection-mode")
        .expect("#selection-mode")
        .dyn_into::<HtmlElement>()?;
    let selection_sep = document
        .get_element_by_id("selection-sep")
        .expect("#selection-sep")
        .dyn_into::<HtmlElement>()?;
    let selection_mat_wrap = document
        .get_element_by_id("selection-mat-wrap")
        .expect("#selection-mat-wrap")
        .dyn_into::<HtmlElement>()?;
    let selection_swatch = document
        .get_element_by_id("selection-swatch")
        .expect("#selection-swatch")
        .dyn_into::<HtmlElement>()?;
    let selection_name = document
        .get_element_by_id("selection-name")
        .expect("#selection-name")
        .dyn_into::<HtmlElement>()?;

    let mut mat_buttons: Vec<HtmlElement> = Vec::new();
    for i in 0..materials::paintable_builtin_count() {
        let def = materials::paintable_builtin_at(i).expect("paintable builtin");
        let btn = create_mat_btn(&document, def)?;
        mat_container.append_child(&btn)?;
        mat_buttons.push(btn);
    }

    let mode_labels = ["Draw", "Rigid", "Explosion", "Heat", "Cool"];
    let mut mode_buttons: Vec<HtmlElement> = Vec::new();
    for (i, label) in mode_labels.iter().enumerate() {
        let btn = create_mode_btn(&document, i, label)?;
        mode_row.append_child(&btn)?;
        mode_buttons.push(btn);
    }

    let ui = Rc::new(UiRefs {
        mat_container,
        mat_buttons,
        mode_buttons,
        mode_help,
        brush_slider,
        brush_value,
        material_title,
        material_body,
        selection_mode,
        selection_sep,
        selection_mat_wrap,
        selection_swatch,
        selection_name,
    });

    let input = Rc::new(RefCell::new(FrameInput::default()));
    let pending_ui = Rc::new(RefCell::new(VecDeque::new()));
    let app = Rc::new(RefCell::new(App::new(input.clone(), pending_ui.clone())));

    for btn in &ui.mat_buttons {
        let pq2 = pending_ui.clone();
        let id: MaterialId = btn
            .get_attribute("data-mat-id")
            .unwrap_or_default()
            .parse()
            .unwrap_or(0);
        let btn_el = btn.clone();
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            if btn_el.has_attribute("disabled") {
                return;
            }
            pq2.borrow_mut().push_back(UiAction::SelectMaterial(id));
        }) as Box<dyn FnMut(_)>);
        btn.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    for btn in &ui.mode_buttons {
        let pq2 = pending_ui.clone();
        let idx: usize = btn
            .get_attribute("data-mode-idx")
            .unwrap_or_default()
            .parse()
            .unwrap_or(0);
        let m = InteractionMode::ALL[idx];
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            pq2.borrow_mut().push_back(UiAction::SetMode(m));
        }) as Box<dyn FnMut(_)>);
        btn.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let pq2 = pending_ui.clone();
        let slider_el = ui.brush_slider.clone();
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            if let Ok(v) = slider_el.value().parse::<i32>() {
                pq2.borrow_mut().push_back(UiAction::SetBrushRadius(v));
            }
        }) as Box<dyn FnMut(_)>);
        ui.brush_slider
            .add_event_listener_with_callback("input", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let clear_btn: HtmlElement = document
            .get_element_by_id("clear-world")
            .expect("#clear-world")
            .dyn_into()?;
        let pq2 = pending_ui.clone();
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            pq2.borrow_mut().push_back(UiAction::ClearWorld);
        }) as Box<dyn FnMut(_)>);
        clear_btn.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    sync_ui(&ui, &app.borrow().game);

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
                0 => g.left_down = true,
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

    {
        let inp = input.clone();
        let closure = Closure::wrap(Box::new(move |e: KeyboardEvent| {
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
            g.left_down = false;
            g.right_down = false;
            g.middle_down = false;
        }) as Box<dyn FnMut(_)>);
        window()
            .unwrap()
            .add_event_listener_with_callback("mouseup", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let c2 = canvas.clone();
        let closure = Closure::wrap(Box::new(move |_e: Event| {
            fit_canvas(&c2);
        }) as Box<dyn FnMut(_)>);
        window()
            .unwrap()
            .add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref())?;
        closure.forget();
    }

    {
        let a = app.clone();
        let c = ctx.clone();
        let s = stats.clone();
        let u = ui.clone();
        let closure = Rc::new(RefCell::new(None::<Closure<dyn FnMut()>>));
        let g = closure.clone();
        *closure.borrow_mut() = Some(Closure::wrap(Box::new(move || {
            {
                let mut app_b = a.borrow_mut();
                app_b.tick(&c, &s, &u);
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
