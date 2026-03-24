use std::collections::HashMap;

use crate::rigid::RigidBridge;
use crate::world::{cell_flags, material, Cell, MaterialId, RectI, Vec2i, World};

#[derive(Debug, Clone, Copy)]
pub struct DirtyChunkView {
    pub chunk: Vec2i,
    pub rect: RectI,
}

pub struct PixelRegion {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub fn get_dirty_chunks(world: &World) -> Vec<DirtyChunkView> {
    world
        .all_chunk_dirty_rects()
        .into_iter()
        .map(|(coord, rect)| DirtyChunkView {
            chunk: Vec2i::new(coord.x, coord.y),
            rect,
        })
        .collect()
}

pub fn copy_rgba_for_region(world: &World, rigid: Option<&RigidBridge>, rect: RectI) -> PixelRegion {
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    let height = (rect.max.y - rect.min.y + 1).max(0) as usize;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let cell = world.get_cell(Vec2i::new(x, y));
            let color = cell_to_rgba(cell, x, y);
            rgba.extend_from_slice(&color);
        }
    }
    if let Some(r) = rigid {
        apply_rigid_morph_overlays_rgba(world, r, rect, &mut rgba, width);
    }
    PixelRegion { width, height, rgba }
}

fn apply_rigid_morph_overlays_rgba(
    world: &World,
    rigid: &RigidBridge,
    rect: RectI,
    rgba: &mut [u8],
    width: usize,
) {
    let mut tiles = Vec::new();
    rigid.collect_visual_morph_overlay_tiles(world, rect, &mut tiles);
    let mut by_pos: HashMap<Vec2i, Cell> = HashMap::new();
    for (_, p, c) in tiles {
        by_pos.insert(p, c);
    }
    for (p, cell) in by_pos {
        let ix = (p.x - rect.min.x) as usize;
        let iy = (p.y - rect.min.y) as usize;
        let i = iy * width + ix;
        let base = i * 4;
        if base + 3 < rgba.len() {
            let [r, g, b, a] = cell_to_rgba(cell, p.x, p.y);
            rgba[base..base + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
}

/// Returns ARGB32 pixels for a region, suitable for minifb / sandbox rendering.
pub fn copy_argb32_for_region(world: &World, rigid: Option<&RigidBridge>, rect: RectI) -> Vec<u32> {
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    let height = (rect.max.y - rect.min.y + 1).max(0) as usize;
    let mut buf = Vec::with_capacity(width * height);
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let cell = world.get_cell(Vec2i::new(x, y));
            buf.push(cell_to_argb32(cell, x, y));
        }
    }
    if let Some(r) = rigid {
        apply_rigid_morph_overlays_argb32(world, r, rect, &mut buf, width);
    }
    buf
}

fn apply_rigid_morph_overlays_argb32(
    world: &World,
    rigid: &RigidBridge,
    rect: RectI,
    buf: &mut [u32],
    width: usize,
) {
    let mut tiles = Vec::new();
    rigid.collect_visual_morph_overlay_tiles(world, rect, &mut tiles);
    let mut by_pos: HashMap<Vec2i, Cell> = HashMap::new();
    for (_, p, c) in tiles {
        by_pos.insert(p, c);
    }
    for (p, cell) in by_pos {
        let ix = (p.x - rect.min.x) as usize;
        let iy = (p.y - rect.min.y) as usize;
        let i = iy * width + ix;
        if i < buf.len() {
            buf[i] = cell_to_argb32(cell, p.x, p.y);
        }
    }
}

#[inline]
fn checker_pass_rgb(pass: usize) -> [u8; 3] {
    match pass & 3 {
        0 => [255, 40, 40],
        1 => [40, 255, 60],
        2 => [50, 120, 255],
        _ => [255, 220, 50],
    }
}

#[inline]
fn blend_argb32_toward_rgb(px: u32, rgb: [u8; 3], tint_numer: u16, tint_denom: u16) -> u32 {
    let d = tint_denom.max(1);
    let n = tint_numer.min(d);
    let inv = d - n;
    let pr = ((px >> 16) & 0xFF) as u16;
    let pg = ((px >> 8) & 0xFF) as u16;
    let pb = (px & 0xFF) as u16;
    let a = ((px >> 24) & 0xFF) as u16;
    let nr = ((pr * inv + rgb[0] as u16 * n) / d) as u8;
    let ng = ((pg * inv + rgb[1] as u16 * n) / d) as u8;
    let nb = ((pb * inv + rgb[2] as u16 * n) / d) as u8;
    let na = (a.min(255)) as u32;
    (na << 24) | ((nr as u32) << 16) | ((ng as u32) << 8) | nb as u32
}

fn draw_chunk_outline_on_buf(
    buf: &mut [u32],
    width: usize,
    region: RectI,
    chunk: RectI,
    thickness: i32,
    rgb: [u8; 3],
    blend: Option<(u16, u16)>,
) {
    let t = thickness.max(1);
    // Only scan the chunk ∩ blit region — not the whole framebuffer per chunk (was O(screen × chunks)).
    let y_lo = region.min.y.max(chunk.min.y);
    let y_hi = region.max.y.min(chunk.max.y);
    let x_lo = region.min.x.max(chunk.min.x);
    let x_hi = region.max.x.min(chunk.max.x);
    if y_lo > y_hi || x_lo > x_hi {
        return;
    }
    for y in y_lo..=y_hi {
        for x in x_lo..=x_hi {
            let edge = x < chunk.min.x + t || x > chunk.max.x - t || y < chunk.min.y + t || y > chunk.max.y - t;
            if !edge {
                continue;
            }
            let ix = (x - region.min.x) as usize;
            let iy = (y - region.min.y) as usize;
            let i = iy * width + ix;
            if i >= buf.len() {
                continue;
            }
            buf[i] = match blend {
                Some((n, d)) => blend_argb32_toward_rgb(buf[i], rgb, n, d),
                None => {
                    0xFF000000u32 | (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32
                }
            };
        }
    }
}

fn draw_all_pass_batch_outlines(world: &World, buf: &mut [u32], width: usize, region: RectI) {
    for pass in 0usize..4 {
        let rgb = checker_pass_rgb(pass);
        for coord in world.active_chunk_coords_for_pass(pass) {
            let b = world.bounds_for_chunk(coord);
            // 1px solid outline: each chunk in its checkerboard pass color.
            draw_chunk_outline_on_buf(buf, width, region, b, 1, rgb, None);
        }
    }
}

/// Same as [`copy_argb32_for_region`], then 1px outlines per chunk in checkerboard pass color (red/green/blue/yellow batches).
/// In [`crate::sim::SchedulerMode::ThreadPool`], all chunks of one color are stepped **in parallel** (one chunk per rayon task).
pub fn copy_argb32_for_region_pass_batch_viz(
    world: &World,
    rigid: Option<&RigidBridge>,
    rect: RectI,
) -> Vec<u32> {
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    let height = (rect.max.y - rect.min.y + 1).max(0) as usize;
    let mut buf = Vec::with_capacity(width * height);
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let cell = world.get_cell(Vec2i::new(x, y));
            buf.push(cell_to_argb32(cell, x, y));
        }
    }
    if let Some(r) = rigid {
        apply_rigid_morph_overlays_argb32(world, r, rect, &mut buf, width);
    }
    draw_all_pass_batch_outlines(world, &mut buf, width, rect);
    buf
}

/// Same as [`copy_argb32_for_region`], optional pass-batch outlines, then a thick outline on the chunk being stepped.
pub fn copy_argb32_for_region_chunk_step_viz(
    world: &World,
    rigid: Option<&RigidBridge>,
    rect: RectI,
) -> Vec<u32> {
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    let height = (rect.max.y - rect.min.y + 1).max(0) as usize;
    let mut buf = Vec::with_capacity(width * height);
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let cell = world.get_cell(Vec2i::new(x, y));
            buf.push(cell_to_argb32(cell, x, y));
        }
    }
    if let Some(r) = rigid {
        apply_rigid_morph_overlays_argb32(world, r, rect, &mut buf, width);
    }
    if world.debug_pass_batch_outlines() {
        draw_all_pass_batch_outlines(world, &mut buf, width, rect);
    }
    if let Some((coord, pass)) = world.debug_chunk_highlight() {
        let b = world.bounds_for_chunk(coord);
        let rgb = checker_pass_rgb(pass as usize);
        // Slightly thicker than batch lines so the active chunk is still obvious when O is on.
        draw_chunk_outline_on_buf(&mut buf, width, rect, b, 2, rgb, None);
    }
    buf
}

pub fn copy_palette_indices_for_region(world: &World, rect: RectI) -> Vec<u16> {
    let mut indices = Vec::new();
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            indices.push(world.get_cell(Vec2i::new(x, y)).material());
        }
    }
    indices
}

fn cell_to_rgba(cell: Cell, x: i32, y: i32) -> [u8; 4] {
    let temp = cell.temperature();
    if (cell.material() == material::PLANT || cell.material() == material::WOOD)
        && temp >= 250 && cell.lifetime() > 0
    {
        burning_vegetation_rgba(cell.lifetime(), x, y)
    } else {
        let [mut r, mut g, b, a] = match cell.material() {
            m if m == material::FIRE => fire_rgba(cell.lifetime(), x, y),
            m if m == material::LAVA => lava_rgba(cell.lifetime(), x, y),
            m if m == material::EMBER => ember_pile_rgba(cell.lifetime(), x, y),
            m if m == material::SMOKE => smoke_rgba(cell.lifetime(), x, y),
            m if m == material::STEAM => steam_rgba(cell.lifetime(), x, y),
            m => {
                let base = static_palette_rgba(m);
                let mut v = apply_variant(base, cell.variant());
                if cell.has_flag(cell_flags::WET) && m == material::SAND {
                    v[0] = ((v[0] as u16 * 88) / 100) as u8;
                    v[1] = ((v[1] as u16 * 92) / 100) as u8;
                    v[2] = v[2].saturating_add(20).min(255);
                }
                v
            }
        };
        if temp > 200 && cell.material() != material::FIRE && cell.material() != material::LAVA {
            let glow = ((temp as u32 - 200).min(800) * 255 / 800) as u8;
            r = r.saturating_add(glow);
            g = g.saturating_add(glow / 3);
        }
        [r, g, b, a]
    }
}

fn cell_to_argb32(cell: Cell, x: i32, y: i32) -> u32 {
    let [r, g, b, a] = cell_to_rgba(cell, x, y);
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

fn apply_variant(base: [u8; 4], variant: u8) -> [u8; 4] {
    if variant == 0 {
        return base;
    }
    let shift = (variant as i16 % 21) - 10;
    [
        (base[0] as i16 + shift).clamp(0, 255) as u8,
        (base[1] as i16 + shift).clamp(0, 255) as u8,
        (base[2] as i16 + shift).clamp(0, 255) as u8,
        base[3],
    ]
}

/// Smoldering plant / wood: high `lifetime` (more fuel left) → red/orange; low → char black.
fn burning_vegetation_rgba(life: u8, x: i32, y: i32) -> [u8; 4] {
    let t = (life as f32 / 255.0).clamp(0.0, 1.0);
    let noise = simple_hash(x, y, life as i32) as i16;
    let jitter = ((noise % 26) - 13) as i32;

    let r = (35.0 + t * 215.0) as i32 + jitter;
    let g = (22.0 + t * 95.0) as i32 + jitter / 2;
    let b = (12.0 + t * 42.0) as i32 + jitter / 3;

    [
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
        255,
    ]
}

/// Glowing ember pile: high `lifetime` → bright orange; low → dull red / near-black.
fn ember_pile_rgba(life: u8, x: i32, y: i32) -> [u8; 4] {
    let t = (life as f32 / 255.0).clamp(0.0, 1.0);
    let noise = simple_hash(x, y, life as i32) as i16;
    let jitter = ((noise % 22) - 11) as i32;

    let r = (28.0 + t * 210.0) as i32 + jitter;
    let g = (10.0 + t * 85.0) as i32 + jitter / 2;
    let b = (4.0 + t * 28.0) as i32 + jitter / 3;

    [
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
        255,
    ]
}

fn lava_rgba(life: u8, x: i32, y: i32) -> [u8; 4] {
    let noise = simple_hash(x, y, life as i32) as i16;
    let jitter = (noise % 28) - 14;
    let (r, g, b) = if life > 200 {
        (255i16, 120, 20)
    } else if life > 120 {
        (255, 85, 12)
    } else if life > 60 {
        (240, 55, 8)
    } else if life > 25 {
        (210, 40, 5)
    } else {
        (170, 30, 4)
    };
    [
        (r + jitter).clamp(0, 255) as u8,
        (g + jitter / 2).clamp(0, 255) as u8,
        (b + jitter / 3).clamp(0, 255) as u8,
        255,
    ]
}

fn fire_rgba(life: u8, x: i32, y: i32) -> [u8; 4] {
    let noise = simple_hash(x, y, life as i32) as i16;
    let jitter = (noise % 30) - 15;

    let (r, g, b) = if life > 200 {
        (255i16, 220, 50)
    } else if life > 120 {
        (255, 160, 20)
    } else if life > 60 {
        (240, 100, 10)
    } else if life > 25 {
        (200, 60, 5)
    } else {
        (150, 35, 5)
    };

    [
        (r + jitter).clamp(0, 255) as u8,
        (g + jitter / 2).clamp(0, 255) as u8,
        (b + jitter / 4).clamp(0, 255) as u8,
        255,
    ]
}

fn smoke_rgba(life: u8, x: i32, y: i32) -> [u8; 4] {
    let noise = simple_hash(x, y, life as i32) as i16;
    let jitter = (noise % 20) - 10;
    let base = 90i16 + jitter;
    let alpha = ((life as i16) * 180 / 60).clamp(20, 180);
    [
        base.clamp(40, 140) as u8,
        base.clamp(40, 140) as u8,
        base.clamp(40, 140) as u8,
        alpha as u8,
    ]
}

fn steam_rgba(life: u8, x: i32, y: i32) -> [u8; 4] {
    let noise = simple_hash(x, y, life as i32) as i16;
    let jitter = (noise % 24) - 12;
    let alpha = ((life as i16) * 200 / 72).clamp(40, 200);
    [
        (200 + jitter).clamp(160, 255) as u8,
        (230 + jitter / 2).clamp(200, 255) as u8,
        255u8,
        alpha as u8,
    ]
}

fn simple_hash(x: i32, y: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(374761393)
        .wrapping_add((y as u32).wrapping_mul(668265263))
        .wrapping_add((z as u32).wrapping_mul(2147483647));
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

/// Debug overlay: tint pixel by the pass that last processed it (0..3 checkerboard, 4 full-grid debug).
/// Pass 0xFF (unprocessed) renders as dark gray.
fn cell_to_debug_rgba(cell: Cell, x: i32, y: i32, pass: u8) -> [u8; 4] {
    let base = cell_to_rgba(cell, x, y);
    let tint: [u8; 3] = match pass {
        0 => [255, 80, 80],
        1 => [80, 255, 80],
        2 => [80, 120, 255],
        3 => [255, 220, 60],
        4 => [255, 80, 255],
        _ => [80, 80, 80],
    };
    let blend = |b: u8, t: u8| -> u8 { ((b as u16 + t as u16) / 2) as u8 };
    [blend(base[0], tint[0]), blend(base[1], tint[1]), blend(base[2], tint[2]), base[3]]
}

fn cell_to_debug_argb32(cell: Cell, x: i32, y: i32, pass: u8) -> u32 {
    let [r, g, b, a] = cell_to_debug_rgba(cell, x, y, pass);
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// ARGB32 debug overlay for a region, tinted by last-processed pass.
pub fn copy_debug_argb32_for_region(world: &World, rect: RectI) -> Vec<u32> {
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    let height = (rect.max.y - rect.min.y + 1).max(0) as usize;
    let mut buf = Vec::with_capacity(width * height);
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let p = Vec2i::new(x, y);
            let cell = world.get_cell(p);
            let pass = world.get_debug_pass(p);
            buf.push(cell_to_debug_argb32(cell, x, y, pass));
        }
    }
    buf
}

/// Per-pixel pass tint, pass-colored chunk grid, and (if set) the active chunk-step outline.
pub fn copy_argb32_for_region_all_debug_views(
    world: &World,
    rigid: Option<&RigidBridge>,
    rect: RectI,
) -> Vec<u32> {
    let mut buf = copy_debug_argb32_for_region(world, rect);
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    if let Some(r) = rigid {
        apply_rigid_morph_overlays_argb32(world, r, rect, &mut buf, width);
    }
    draw_all_pass_batch_outlines(world, &mut buf, width, rect);
    if let Some((coord, pass)) = world.debug_chunk_highlight() {
        let b = world.bounds_for_chunk(coord);
        let rgb = checker_pass_rgb(pass as usize);
        draw_chunk_outline_on_buf(&mut buf, width, rect, b, 2, rgb, None);
    }
    buf
}

fn static_palette_rgba(material: MaterialId) -> [u8; 4] {
    static PALETTE: std::sync::LazyLock<[[u8; 4]; material::MAX_MATERIALS]> =
        std::sync::LazyLock::new(crate::materials::builtin_palette_rgba);
    PALETTE[material as usize]
}
