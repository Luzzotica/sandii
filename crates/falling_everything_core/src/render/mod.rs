use crate::world::{MaterialId, RectI, Vec2i, World};

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
        .all_chunks()
        .filter_map(|(coord, dirty)| {
            dirty.map(|rect| DirtyChunkView {
                chunk: Vec2i::new(coord.x, coord.y),
                rect,
            })
        })
        .collect()
}

pub fn copy_rgba_for_region(world: &World, rect: RectI) -> PixelRegion {
    let width = (rect.max.x - rect.min.x + 1).max(0) as usize;
    let height = (rect.max.y - rect.min.y + 1).max(0) as usize;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            let color = material_to_rgba(world.get_cell(Vec2i::new(x, y)).material);
            rgba.extend_from_slice(&color);
        }
    }
    PixelRegion { width, height, rgba }
}

pub fn copy_palette_indices_for_region(world: &World, rect: RectI) -> Vec<u16> {
    let mut indices = Vec::new();
    for y in rect.min.y..=rect.max.y {
        for x in rect.min.x..=rect.max.x {
            indices.push(world.get_cell(Vec2i::new(x, y)).material);
        }
    }
    indices
}

fn material_to_rgba(material: MaterialId) -> [u8; 4] {
    static PALETTE: std::sync::LazyLock<[[u8; 4]; crate::world::material::MAX_MATERIALS]> =
        std::sync::LazyLock::new(crate::materials::builtin_palette_rgba);
    PALETTE[material as usize]
}
