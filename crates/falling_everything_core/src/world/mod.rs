use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Vec2i {
    pub x: i32,
    pub y: i32,
}

impl Vec2i {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectI {
    pub min: Vec2i,
    pub max: Vec2i,
}

impl RectI {
    pub const fn new(min: Vec2i, max: Vec2i) -> Self {
        Self { min, max }
    }

    pub fn from_point(p: Vec2i) -> Self {
        Self { min: p, max: p }
    }

    pub fn expand_to_include(&mut self, p: Vec2i) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn contains(&self, p: Vec2i) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn intersects(&self, other: RectI) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }
}

pub type MaterialId = u16;

pub mod material {
    use super::MaterialId;

    pub const EMPTY: MaterialId = 0;
    pub const SAND: MaterialId = 1;
    pub const LIQUID: MaterialId = 2;
    pub const GAS: MaterialId = 3;
    pub const STATIC: MaterialId = 4;
    pub const RIGID: MaterialId = 5;
    pub const LIGHT_LIQUID: MaterialId = 6;
    pub const HEAVY_LIQUID: MaterialId = 7;
    pub const MAX_MATERIALS: usize = 512;
}

#[derive(Debug, Clone, Copy)]
pub struct MaterialRule {
    pub lateral_spread: u8,
    pub miscible: bool,
}

impl Default for MaterialRule {
    fn default() -> Self {
        Self {
            lateral_spread: 1,
            miscible: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionOutcome {
    None,
    Swap,
    Transform(MaterialId, MaterialId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Solid,
    Liquid,
    Gas,
}

#[derive(Debug, Clone, Copy)]
pub struct MaterialProps {
    pub density: i16,
    pub phase: Phase,
    pub viscosity: u8,
    pub inert: bool,
    pub max_speed: u8,
    pub acceleration: u8,
}

impl Default for MaterialProps {
    fn default() -> Self {
        Self {
            density: 0,
            phase: Phase::Solid,
            viscosity: 255,
            inert: true,
            max_speed: 0,
            acceleration: 0,
        }
    }
}

fn default_material_props() -> [MaterialProps; material::MAX_MATERIALS] {
    crate::materials::builtin_props()
}

fn default_material_rules() -> [MaterialRule; material::MAX_MATERIALS] {
    crate::materials::builtin_rules()
}

fn default_reactions() -> Vec<ReactionOutcome> {
    vec![ReactionOutcome::None; material::MAX_MATERIALS * material::MAX_MATERIALS]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub material: MaterialId,
    pub color: u16,
    pub flags: u8,
    pub velocity: i8,
    pub rigid_id: Option<u32>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            material: material::EMPTY,
            color: 0,
            flags: 0,
            velocity: 0,
            rigid_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Chunk {
    cells: Vec<Cell>,
    dirty: Option<RectI>,
}

impl Chunk {
    fn new(chunk_size: i32) -> Self {
        Self {
            cells: vec![Cell::default(); (chunk_size * chunk_size) as usize],
            dirty: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WorldEvent {
    RegionLoaded(Vec2i),
    RegionSaved(Vec2i),
}

#[derive(Debug, Serialize, Deserialize)]
struct RegionFile {
    chunks: Vec<(ChunkCoord, Chunk)>,
}

pub struct World {
    chunk_size: i32,
    region_size: i32,
    chunks: HashMap<ChunkCoord, Chunk>,
    active_regions: HashSet<Vec2i>,
    active_radius_regions: i32,
    focus: Vec2i,
    storage_dir: Option<PathBuf>,
    solid_bounds: Option<RectI>,
    material_props: [MaterialProps; material::MAX_MATERIALS],
    material_rules: [MaterialRule; material::MAX_MATERIALS],
    reactions: Vec<ReactionOutcome>,
}

impl World {
    pub fn new(chunk_size: i32, region_size: i32) -> Self {
        Self {
            chunk_size,
            region_size,
            chunks: HashMap::new(),
            active_regions: HashSet::new(),
            active_radius_regions: 1,
            focus: Vec2i::new(0, 0),
            storage_dir: None,
            solid_bounds: None,
            material_props: default_material_props(),
            material_rules: default_material_rules(),
            reactions: default_reactions(),
        }
    }

    pub fn chunk_size(&self) -> i32 {
        self.chunk_size
    }

    pub fn set_storage_dir(&mut self, dir: PathBuf) {
        self.storage_dir = Some(dir);
    }

    pub fn set_solid_bounds(&mut self, bounds: RectI) {
        self.solid_bounds = Some(bounds);
    }

    pub fn material_props(&self, id: MaterialId) -> MaterialProps {
        self.material_props
            .get(id as usize)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_material_props(&mut self, id: MaterialId, props: MaterialProps) {
        if let Some(slot) = self.material_props.get_mut(id as usize) {
            *slot = props;
        }
    }

    pub fn material_rule(&self, id: MaterialId) -> MaterialRule {
        self.material_rules
            .get(id as usize)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_material_rule(&mut self, id: MaterialId, rule: MaterialRule) {
        if let Some(slot) = self.material_rules.get_mut(id as usize) {
            *slot = rule;
        }
    }

    pub fn reaction(&self, from: MaterialId, to: MaterialId) -> ReactionOutcome {
        let idx = from as usize * material::MAX_MATERIALS + to as usize;
        self.reactions.get(idx).copied().unwrap_or(ReactionOutcome::None)
    }

    pub fn set_reaction(&mut self, from: MaterialId, to: MaterialId, reaction: ReactionOutcome) {
        let idx = from as usize * material::MAX_MATERIALS + to as usize;
        if let Some(slot) = self.reactions.get_mut(idx) {
            *slot = reaction;
        }
    }

    pub fn active_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn sleeping_chunk_count(&self) -> usize {
        0
    }

    pub fn set_focus(&mut self, focus: Vec2i) -> Vec<WorldEvent> {
        self.focus = focus;
        self.sync_streaming_regions()
    }

    pub fn all_chunks(&self) -> impl Iterator<Item = (&ChunkCoord, &Option<RectI>)> {
        self.chunks.iter().map(|(coord, chunk)| (coord, &chunk.dirty))
    }

    pub fn active_chunk_coords_for_pass(&self, pass: usize) -> Vec<ChunkCoord> {
        let px = (pass & 1) as i32;
        let py = ((pass >> 1) & 1) as i32;
        let mut coords: Vec<ChunkCoord> = self
            .chunks
            .keys()
            .filter(|coord| (coord.x & 1) == px && (coord.y & 1) == py)
            .copied()
            .collect();
        coords.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
        coords
    }

    pub fn chunk_dirty_rect(&self, coord: ChunkCoord) -> Option<RectI> {
        self.chunks.get(&coord).and_then(|c| c.dirty)
    }

    pub fn clear_chunk_dirty(&mut self, coord: ChunkCoord) {
        if let Some(chunk) = self.chunks.get_mut(&coord) {
            chunk.dirty = None;
        }
    }

    pub fn get_cell(&self, p: Vec2i) -> Cell {
        if let Some(bounds) = self.solid_bounds {
            if !bounds.contains(p) {
                return Cell {
                    material: material::STATIC,
                    color: material::STATIC,
                    flags: 0,
                    velocity: 0,
                    rigid_id: None,
                };
            }
        }
        let (chunk_coord, local) = self.to_chunk_local(p);
        self.chunks
            .get(&chunk_coord)
            .map(|chunk| chunk.cells[self.flat_index(local)])
            .unwrap_or_default()
    }

    pub fn set_cell(&mut self, p: Vec2i, cell: Cell) {
        if let Some(bounds) = self.solid_bounds {
            if !bounds.contains(p) {
                return;
            }
        }
        let (chunk_coord, local) = self.to_chunk_local(p);
        let idx = self.flat_index(local);
        let cs = self.chunk_size;
        let chunk = self
            .chunks
            .entry(chunk_coord)
            .or_insert_with(|| Chunk::new(cs));
        chunk.cells[idx] = cell;
    }

    pub fn paint_circle(&mut self, center: Vec2i, radius: i32, material: MaterialId) {
        let r2 = radius * radius;
        for y in (center.y - radius)..=(center.y + radius) {
            for x in (center.x - radius)..=(center.x + radius) {
                let dx = x - center.x;
                let dy = y - center.y;
                if dx * dx + dy * dy <= r2 {
                    self.set_cell(
                        Vec2i::new(x, y),
                        Cell {
                            material,
                            color: material,
                            flags: 0,
                            velocity: 0,
                            rigid_id: None,
                        },
                    );
                }
            }
        }
    }

    pub fn try_swap(&mut self, from: Vec2i, to: Vec2i) -> bool {
        let from_cell = self.get_cell(from);
        let to_cell = self.get_cell(to);
        if to_cell.material != material::EMPTY {
            return false;
        }
        self.set_cell(to, from_cell);
        self.set_cell(from, Cell::default());
        true
    }

    pub fn finish_frame(&mut self) {
        let cs = self.chunk_size;
        for (&coord, chunk) in self.chunks.iter_mut() {
            let min = Vec2i::new(coord.x * cs, coord.y * cs);
            let max = Vec2i::new(min.x + cs - 1, min.y + cs - 1);
            chunk.dirty = Some(RectI::new(min, max));
        }
        let _ = self.sync_streaming_regions();
    }

    pub fn dirty_world_bounds(&self) -> Option<RectI> {
        let mut out: Option<RectI> = None;
        for chunk in self.chunks.values() {
            if let Some(rect) = chunk.dirty {
                match &mut out {
                    Some(agg) => {
                        agg.expand_to_include(rect.min);
                        agg.expand_to_include(rect.max);
                    }
                    None => out = Some(rect),
                }
            }
        }
        out
    }

    pub fn bounds_for_chunk(&self, coord: ChunkCoord) -> RectI {
        let min = Vec2i::new(coord.x * self.chunk_size, coord.y * self.chunk_size);
        let max = Vec2i::new(min.x + self.chunk_size - 1, min.y + self.chunk_size - 1);
        RectI::new(min, max)
    }

    pub fn clear_outside_rect(&mut self, bounds: RectI) {
        let all_coords: Vec<ChunkCoord> = self.chunks.keys().copied().collect();
        let mut remove_chunks = Vec::new();

        for coord in all_coords {
            let chunk_bounds = self.bounds_for_chunk(coord);
            if !chunk_bounds.intersects(bounds) {
                remove_chunks.push(coord);
                continue;
            }

            let mut dirty_points = Vec::new();
            for y in chunk_bounds.min.y..=chunk_bounds.max.y {
                for x in chunk_bounds.min.x..=chunk_bounds.max.x {
                    let p = Vec2i::new(x, y);
                    if bounds.contains(p) {
                        continue;
                    }
                    let cell = self.get_cell(p);
                    if cell.material != material::EMPTY {
                        dirty_points.push(p);
                    }
                }
            }

            for p in dirty_points {
                self.set_cell(p, Cell::default());
            }
        }

        for coord in remove_chunks {
            self.chunks.remove(&coord);
        }
    }

    fn to_chunk_local(&self, p: Vec2i) -> (ChunkCoord, Vec2i) {
        let chunk_x = div_floor(p.x, self.chunk_size);
        let chunk_y = div_floor(p.y, self.chunk_size);
        let local_x = p.x - chunk_x * self.chunk_size;
        let local_y = p.y - chunk_y * self.chunk_size;
        (
            ChunkCoord {
                x: chunk_x,
                y: chunk_y,
            },
            Vec2i::new(local_x, local_y),
        )
    }

    fn flat_index(&self, local: Vec2i) -> usize {
        (local.y * self.chunk_size + local.x) as usize
    }

    fn sync_streaming_regions(&mut self) -> Vec<WorldEvent> {
        let focus_region = self.region_coord(self.focus);
        let mut desired = HashSet::new();
        for ry in -self.active_radius_regions..=self.active_radius_regions {
            for rx in -self.active_radius_regions..=self.active_radius_regions {
                desired.insert(Vec2i::new(focus_region.x + rx, focus_region.y + ry));
            }
        }

        let mut events = Vec::new();
        let to_unload: Vec<Vec2i> = self
            .active_regions
            .difference(&desired)
            .copied()
            .collect();
        for region in to_unload {
            self.save_region(region);
            self.unload_region(region);
            self.active_regions.remove(&region);
            events.push(WorldEvent::RegionSaved(region));
        }

        let to_load: Vec<Vec2i> = desired
            .difference(&self.active_regions)
            .copied()
            .collect();
        for region in to_load {
            self.load_region(region);
            self.active_regions.insert(region);
            events.push(WorldEvent::RegionLoaded(region));
        }

        events
    }

    fn region_coord(&self, p: Vec2i) -> Vec2i {
        Vec2i::new(div_floor(p.x, self.region_size), div_floor(p.y, self.region_size))
    }

    fn save_region(&self, region: Vec2i) {
        let Some(storage_dir) = &self.storage_dir else {
            return;
        };
        let _ = fs::create_dir_all(storage_dir);

        let mut region_chunks = Vec::new();
        for (coord, chunk) in &self.chunks {
            let chunk_origin = Vec2i::new(coord.x * self.chunk_size, coord.y * self.chunk_size);
            if self.region_coord(chunk_origin) == region {
                region_chunks.push((*coord, chunk.clone()));
            }
        }

        let payload = RegionFile { chunks: region_chunks };
        if let Ok(encoded) = bincode::serialize(&payload) {
            let path = storage_dir.join(format!("region_{}_{}.bin", region.x, region.y));
            let _ = fs::write(path, encoded);
        }
    }

    fn load_region(&mut self, region: Vec2i) {
        let Some(storage_dir) = &self.storage_dir else {
            return;
        };
        let path = storage_dir.join(format!("region_{}_{}.bin", region.x, region.y));
        let Ok(bytes) = fs::read(path) else {
            return;
        };
        let Ok(payload) = bincode::deserialize::<RegionFile>(&bytes) else {
            return;
        };
        for (coord, chunk) in payload.chunks {
            self.chunks.insert(coord, chunk);
        }
    }

    fn unload_region(&mut self, region: Vec2i) {
        let keys: Vec<ChunkCoord> = self
            .chunks
            .keys()
            .filter(|coord| {
                let origin = Vec2i::new(coord.x * self.chunk_size, coord.y * self.chunk_size);
                self.region_coord(origin) == region
            })
            .copied()
            .collect();
        for key in keys {
            self.chunks.remove(&key);
        }
    }
}

fn div_floor(a: i32, b: i32) -> i32 {
    let mut q = a / b;
    let r = a % b;
    if r != 0 && ((r > 0) != (b > 0)) {
        q -= 1;
    }
    q
}
