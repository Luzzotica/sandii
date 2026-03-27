use std::collections::HashMap;

use super::{Cell, ChunkCoord, RectI, Vec2i, CHUNK_AREA, CHUNK_SIZE};

pub struct DenseCellStore {
    grids: [Vec<Cell>; 2],
    read_idx: usize,
    width: i32,
    height: i32,
    origin: Vec2i,
}

impl DenseCellStore {
    pub fn new(width: i32, height: i32, origin: Vec2i) -> Self {
        let total = (width * height) as usize;
        Self {
            grids: [vec![Cell::new(); total], vec![Cell::new(); total]],
            read_idx: 0,
            width,
            height,
            origin,
        }
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    pub fn origin(&self) -> Vec2i {
        self.origin
    }

    pub fn resize(&mut self, width: i32, height: i32, origin: Vec2i) {
        if self.width == width && self.height == height && self.origin == origin {
            return;
        }
        let total = (width * height) as usize;
        self.grids = [vec![Cell::new(); total], vec![Cell::new(); total]];
        self.read_idx = 0;
        self.width = width;
        self.height = height;
        self.origin = origin;
    }

    #[inline]
    pub fn grid_index(&self, p: Vec2i) -> Option<usize> {
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y;
        if x < 0 || x >= self.width || y < 0 || y >= self.height {
            return None;
        }
        Some((y * self.width + x) as usize)
    }

    #[inline]
    pub fn get_read(&self, p: Vec2i) -> Option<Cell> {
        self.grid_index(p).map(|idx| self.grids[self.read_idx][idx])
    }

    #[inline]
    pub fn set_read(&mut self, p: Vec2i, cell: Cell) -> bool {
        if let Some(idx) = self.grid_index(p) {
            self.grids[self.read_idx][idx] = cell;
            true
        } else {
            false
        }
    }

    pub fn prepare_sim(&mut self) {
        let (src, dst) = if self.read_idx == 0 {
            let (a, b) = self.grids.split_at_mut(1);
            (a[0].as_slice(), b[0].as_mut_slice())
        } else {
            let (a, b) = self.grids.split_at_mut(1);
            (b[0].as_slice(), a[0].as_mut_slice())
        };
        dst.copy_from_slice(src);
    }

    pub fn finish_sim(&mut self) {
        self.read_idx = 1 - self.read_idx;
    }

    #[inline]
    pub fn read_cells(&self) -> &[Cell] {
        &self.grids[self.read_idx]
    }

    /// Read buffer (committed state) and write buffer (in-progress sim) before [`Self::finish_sim`].
    #[inline]
    pub fn read_write_cell_slices(&self) -> (&[Cell], &[Cell]) {
        let ri = self.read_idx;
        let wi = 1 - ri;
        (&self.grids[ri], &self.grids[wi])
    }

    #[inline]
    pub fn write_cells_ptr(&mut self) -> *mut Cell {
        let write_idx = 1 - self.read_idx;
        self.grids[write_idx].as_mut_ptr()
    }

    #[inline]
    pub fn write_cells_mut(&mut self) -> &mut [Cell] {
        let write_idx = 1 - self.read_idx;
        &mut self.grids[write_idx]
    }

    #[inline]
    pub fn write_cells_len(&self) -> usize {
        self.grids[1 - self.read_idx].len()
    }

    /// Move the grid to a new origin+size, preserving cells in the overlapping region.
    /// Non-overlapping cells are zeroed. Returns the old read buffer and old (origin, w, h)
    /// so the caller can extract outgoing chunk data.
    pub fn relocate(
        &mut self,
        new_origin: Vec2i,
        new_w: i32,
        new_h: i32,
    ) -> (Vec<Cell>, Vec2i, i32, i32) {
        let old_origin = self.origin;
        let old_w = self.width;
        let old_h = self.height;
        let old_read = self.grids[self.read_idx].clone();

        let new_total = (new_w * new_h) as usize;
        let mut new_grid = vec![Cell::new(); new_total];

        let overlap_min_x = old_origin.x.max(new_origin.x);
        let overlap_min_y = old_origin.y.max(new_origin.y);
        let overlap_max_x = (old_origin.x + old_w - 1).min(new_origin.x + new_w - 1);
        let overlap_max_y = (old_origin.y + old_h - 1).min(new_origin.y + new_h - 1);

        if overlap_min_x <= overlap_max_x && overlap_min_y <= overlap_max_y {
            let row_len = (overlap_max_x - overlap_min_x + 1) as usize;
            for y in overlap_min_y..=overlap_max_y {
                let old_start =
                    ((y - old_origin.y) * old_w + (overlap_min_x - old_origin.x)) as usize;
                let new_start =
                    ((y - new_origin.y) * new_w + (overlap_min_x - new_origin.x)) as usize;
                new_grid[new_start..new_start + row_len]
                    .copy_from_slice(&old_read[old_start..old_start + row_len]);
            }
        }

        self.grids = [new_grid.clone(), new_grid];
        self.read_idx = 0;
        self.width = new_w;
        self.height = new_h;
        self.origin = new_origin;

        (old_read, old_origin, old_w, old_h)
    }

    pub fn clear_outside_rect(&mut self, bounds: RectI) {
        for y in self.origin.y..(self.origin.y + self.height) {
            for x in self.origin.x..(self.origin.x + self.width) {
                let p = Vec2i::new(x, y);
                if !bounds.contains(p) {
                    if let Some(idx) = self.grid_index(p) {
                        if self.grids[self.read_idx][idx].material() != super::material::EMPTY {
                            self.grids[self.read_idx][idx] = Cell::new();
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChunkPool {
    slabs: Vec<Vec<Cell>>,
    free: Vec<usize>,
}

impl ChunkPool {
    pub fn with_capacity(chunk_capacity: usize) -> Self {
        let mut slabs = Vec::with_capacity(chunk_capacity);
        let mut free = Vec::with_capacity(chunk_capacity);
        for i in 0..chunk_capacity {
            slabs.push(vec![Cell::new(); CHUNK_AREA]);
            free.push(i);
        }
        Self { slabs, free }
    }

    pub fn checkout(&mut self) -> usize {
        if let Some(idx) = self.free.pop() {
            idx
        } else {
            let idx = self.slabs.len();
            self.slabs.push(vec![Cell::new(); CHUNK_AREA]);
            idx
        }
    }

    pub fn release(&mut self, idx: usize) {
        if let Some(slab) = self.slabs.get_mut(idx) {
            slab.fill(Cell::new());
            self.free.push(idx);
        }
    }

    pub fn slab(&self, idx: usize) -> Option<&[Cell]> {
        self.slabs.get(idx).map(Vec::as_slice)
    }

    pub fn slab_mut(&mut self, idx: usize) -> Option<&mut [Cell]> {
        self.slabs.get_mut(idx).map(Vec::as_mut_slice)
    }
}

#[derive(Debug, Clone)]
pub struct SpatialHashChunkStore {
    pub pool: ChunkPool,
    handles: HashMap<ChunkCoord, usize>,
}

impl SpatialHashChunkStore {
    pub fn with_capacity(chunk_capacity: usize) -> Self {
        Self {
            pool: ChunkPool::with_capacity(chunk_capacity),
            handles: HashMap::new(),
        }
    }

    pub fn chunk_count(&self) -> usize {
        self.handles.len()
    }

    pub fn ensure_chunk(&mut self, coord: ChunkCoord) {
        if self.handles.contains_key(&coord) {
            return;
        }
        let handle = self.pool.checkout();
        self.handles.insert(coord, handle);
    }

    pub fn remove_chunk(&mut self, coord: ChunkCoord) {
        if let Some(handle) = self.handles.remove(&coord) {
            self.pool.release(handle);
        }
    }

    pub fn get_chunk(&self, coord: ChunkCoord) -> Option<&[Cell]> {
        let handle = *self.handles.get(&coord)?;
        self.pool.slab(handle)
    }

    pub fn get_chunk_mut(&mut self, coord: ChunkCoord) -> Option<&mut [Cell]> {
        let handle = *self.handles.get(&coord)?;
        self.pool.slab_mut(handle)
    }

    pub fn upsert_chunk(&mut self, coord: ChunkCoord, cells: &[Cell]) -> bool {
        if cells.len() != CHUNK_AREA {
            return false;
        }
        self.ensure_chunk(coord);
        if let Some(dst) = self.get_chunk_mut(coord) {
            dst.copy_from_slice(cells);
            true
        } else {
            false
        }
    }

    pub fn chunk_coords(&self) -> impl Iterator<Item = ChunkCoord> + '_ {
        self.handles.keys().copied()
    }

    pub fn remove_outside_world_rect(&mut self, bounds: RectI) {
        let mut to_remove = Vec::new();
        for coord in self.handles.keys().copied() {
            let min = Vec2i::new(coord.x * CHUNK_SIZE, coord.y * CHUNK_SIZE);
            let max = Vec2i::new(min.x + CHUNK_SIZE - 1, min.y + CHUNK_SIZE - 1);
            let chunk_rect = RectI::new(min, max);
            if !chunk_rect.intersects(bounds) {
                to_remove.push(coord);
            }
        }
        for coord in to_remove {
            self.remove_chunk(coord);
        }
    }

    #[inline]
    pub fn split_world_to_chunk(p: Vec2i) -> (ChunkCoord, usize) {
        let cx = super::div_floor(p.x, CHUNK_SIZE);
        let cy = super::div_floor(p.y, CHUNK_SIZE);
        let lx = p.x - cx * CHUNK_SIZE;
        let ly = p.y - cy * CHUNK_SIZE;
        (
            ChunkCoord { x: cx, y: cy },
            super::chunk_local_index(lx, ly),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::material;

    #[test]
    fn pool_checkout_and_release() {
        let mut pool = ChunkPool::with_capacity(2);
        let a = pool.checkout();
        let b = pool.checkout();
        assert!(a != b);
        let c = pool.checkout(); // grows the pool
        assert!(c != a && c != b);
        pool.release(a);
        let d = pool.checkout();
        assert_eq!(d, a); // reuses freed slab
        pool.release(b);
        pool.release(c);
        pool.release(d);
    }

    #[test]
    fn spatial_store_chunk_lifecycle() {
        let mut s = SpatialHashChunkStore::with_capacity(1);
        let c = ChunkCoord { x: 0, y: 0 };
        s.ensure_chunk(c);
        assert_eq!(s.chunk_count(), 1);
        {
            let chunk = s.get_chunk_mut(c).expect("chunk");
            chunk[0] = Cell::new().with_material(material::SAND);
        }
        assert_eq!(s.get_chunk(c).expect("chunk")[0].material(), material::SAND);
        s.remove_chunk(c);
        assert_eq!(s.chunk_count(), 0);
    }
}
