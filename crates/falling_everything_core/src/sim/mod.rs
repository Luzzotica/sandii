use rand::rngs::SmallRng;
use rand::Rng;
use rand::SeedableRng;
use rayon::prelude::*;

use crate::world::{
    material, Cell, ChunkCoord, MaterialProps, MaterialRule, Phase, ReactionOutcome, RectI, Vec2i, World,
};
use crate::SimulationEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMode {
    SingleThreadSeeded,
    ThreadPool,
}

pub struct Scheduler {
    mode: SchedulerMode,
    seed: u64,
    tick: u64,
    single_scratch: ScratchBuffers,
}

impl Scheduler {
    pub fn new(mode: SchedulerMode, seed: u64) -> Self {
        Self {
            mode,
            seed,
            tick: 0,
            single_scratch: ScratchBuffers::default(),
        }
    }

    pub fn mode(&self) -> SchedulerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: SchedulerMode) {
        self.mode = mode;
    }

    pub fn step_world(&mut self, world: &mut World, rng: &mut SmallRng) {
        self.tick = self.tick.saturating_add(1);
        if matches!(self.mode, SchedulerMode::SingleThreadSeeded) {
            *rng = SmallRng::seed_from_u64(self.seed ^ self.tick);
        }

        for pass in 0..4 {
            let coords = world.active_chunk_coords_for_pass(pass);
            match self.mode {
                SchedulerMode::SingleThreadSeeded => {
                    for coord in coords {
                        let mutations =
                            Scheduler::collect_chunk_mutations(world, coord, rng, &mut self.single_scratch);
                        for mutation in mutations {
                            world.set_cell(mutation.pos, mutation.cell);
                        }
                        world.clear_chunk_dirty(coord);
                    }
                }
                SchedulerMode::ThreadPool => {
                    let tick = self.tick;
                    let seed = self.seed;
                    let mutations_per_chunk: Vec<Vec<Mutation>> = coords
                        .par_iter()
                        .map_init(ScratchBuffers::default, |scratch, coord| {
                            let mut local_rng =
                                SmallRng::seed_from_u64(seed ^ tick ^ ((coord.x as u64) << 32) ^ coord.y as u64);
                            Scheduler::collect_chunk_mutations(world, *coord, &mut local_rng, scratch)
                        })
                        .collect();

                    for (coord, mutations) in coords.into_iter().zip(mutations_per_chunk.into_iter()) {
                        for mutation in mutations {
                            world.set_cell(mutation.pos, mutation.cell);
                        }
                        world.clear_chunk_dirty(coord);
                    }
                }
            }
        }
    }

    fn collect_chunk_mutations(
        world: &World,
        coord: ChunkCoord,
        rng: &mut SmallRng,
        scratch_buffers: &mut ScratchBuffers,
    ) -> Vec<Mutation> {
        let bounds = world.bounds_for_chunk(coord);
        const MARGIN: i32 = 8;
        let expanded = RectI {
            min: Vec2i::new(bounds.min.x - MARGIN, bounds.min.y - MARGIN),
            max: Vec2i::new(bounds.max.x + MARGIN, bounds.max.y + MARGIN),
        };

        let mut scratch = ScratchGrid::from_world(world, expanded, scratch_buffers);
        for y in (expanded.min.y..=expanded.max.y).rev() {
            let reverse_x = rng.gen_bool(0.5);
            if reverse_x {
                for x in (expanded.min.x..=expanded.max.x).rev() {
                    step_pixel(world, &mut scratch, Vec2i::new(x, y), rng);
                }
            } else {
                for x in expanded.min.x..=expanded.max.x {
                    step_pixel(world, &mut scratch, Vec2i::new(x, y), rng);
                }
            }
        }
        scratch.into_mutations()
    }
}

#[derive(Clone, Copy)]
struct Mutation {
    pos: Vec2i,
    cell: Cell,
}

#[derive(Default)]
struct ScratchBuffers {
    cells: Vec<Cell>,
    original: Vec<Cell>,
    moved: Vec<bool>,
}

struct ScratchGrid<'a> {
    rect: RectI,
    width: usize,
    cells: &'a mut Vec<Cell>,
    original: &'a mut Vec<Cell>,
    moved: &'a mut Vec<bool>,
}

impl<'a> ScratchGrid<'a> {
    fn from_world(world: &World, rect: RectI, buffers: &'a mut ScratchBuffers) -> Self {
        let width = (rect.max.x - rect.min.x + 1) as usize;
        let height = (rect.max.y - rect.min.y + 1) as usize;
        let len = width * height;

        buffers.cells.clear();
        buffers.cells.reserve(len);
        for y in rect.min.y..=rect.max.y {
            for x in rect.min.x..=rect.max.x {
                buffers.cells.push(world.get_cell(Vec2i::new(x, y)));
            }
        }
        buffers.original.clear();
        buffers.original.extend_from_slice(&buffers.cells);
        buffers.moved.clear();
        buffers.moved.resize(len, false);

        Self {
            rect,
            width,
            cells: &mut buffers.cells,
            original: &mut buffers.original,
            moved: &mut buffers.moved,
        }
    }

    fn index(&self, p: Vec2i) -> Option<usize> {
        if p.x < self.rect.min.x || p.x > self.rect.max.x || p.y < self.rect.min.y || p.y > self.rect.max.y {
            return None;
        }
        let lx = (p.x - self.rect.min.x) as usize;
        let ly = (p.y - self.rect.min.y) as usize;
        Some(ly * self.width + lx)
    }

    fn get(&self, p: Vec2i) -> Cell {
        self.index(p)
            .and_then(|i| self.cells.as_slice().get(i).copied())
            .unwrap_or(Cell::default())
    }

    fn was_moved(&self, p: Vec2i) -> bool {
        self.index(p)
            .and_then(|i| self.moved.get(i).copied())
            .unwrap_or(false)
    }

    fn set_velocity(&mut self, p: Vec2i, vel: i8) {
        if let Some(idx) = self.index(p) {
            self.cells[idx].velocity = vel;
        }
    }

    fn try_displace(&mut self, world: &World, from: Vec2i, to: Vec2i, intent: MoveIntent, rng: &mut SmallRng) -> bool {
        let Some(from_idx) = self.index(from) else {
            return false;
        };
        let Some(to_idx) = self.index(to) else {
            return false;
        };
        let from_cell = self.cells[from_idx];
        let to_cell = self.cells[to_idx];
        if from_cell.material == material::EMPTY {
            return false;
        }

        let from_props = world.material_props(from_cell.material);
        let to_props = world.material_props(to_cell.material);
        let from_rule = world.material_rule(from_cell.material);
        let to_rule = world.material_rule(to_cell.material);
        if from_props.inert || to_props.inert {
            return false;
        }

        let reaction = world.reaction(from_cell.material, to_cell.material);
        if let ReactionOutcome::Transform(from_to, to_to) = reaction {
            self.cells[to_idx].material = from_to;
            self.cells[from_idx].material = to_to;
            self.moved[to_idx] = true;
            self.moved[from_idx] = true;
            return true;
        }
        if reaction == ReactionOutcome::Swap {
            self.cells[to_idx] = from_cell;
            self.cells[from_idx] = to_cell;
            self.moved[to_idx] = true;
            self.moved[from_idx] = true;
            return true;
        }

        if !can_displace(from_props, from_rule, to_cell, to_props, to_rule, intent, rng) {
            return false;
        }

        self.cells[to_idx] = from_cell;
        self.cells[from_idx] = if to_cell.material == material::EMPTY {
            Cell::default()
        } else {
            to_cell
        };
        self.moved[to_idx] = true;
        self.moved[from_idx] = true;
        true
    }

    fn into_mutations(self) -> Vec<Mutation> {
        let mut out = Vec::new();
        for (i, (new_cell, old_cell)) in self
            .cells
            .iter()
            .copied()
            .zip(self.original.iter().copied())
            .enumerate()
        {
            if new_cell == old_cell {
                continue;
            }
            let x = (i % self.width) as i32 + self.rect.min.x;
            let y = (i / self.width) as i32 + self.rect.min.y;
            out.push(Mutation {
                pos: Vec2i::new(x, y),
                cell: new_cell,
            });
        }
        out
    }
}

#[derive(Clone, Copy)]
enum MoveIntent {
    VerticalDown,
    VerticalUp,
    Lateral,
}

fn can_displace(
    from_props: MaterialProps,
    _from_rule: MaterialRule,
    to_cell: Cell,
    to_props: MaterialProps,
    to_rule: MaterialRule,
    intent: MoveIntent,
    rng: &mut SmallRng,
) -> bool {
    if to_cell.material == material::EMPTY {
        return true;
    }

    match intent {
        MoveIntent::VerticalDown => {
            if from_props.density <= to_props.density {
                return false;
            }
            let diff = (from_props.density - to_props.density) as f32 / 256.0;
            rng.gen::<f32>() < diff.clamp(0.05, 0.6)
        }
        MoveIntent::VerticalUp => {
            if from_props.density >= to_props.density {
                return false;
            }
            let diff = (to_props.density - from_props.density) as f32 / 256.0;
            rng.gen::<f32>() < diff.clamp(0.05, 0.6)
        }
        MoveIntent::Lateral => match (from_props.phase, to_props.phase) {
            (Phase::Gas, Phase::Gas) => true,
            (Phase::Liquid, Phase::Liquid) => to_rule.miscible,
            (Phase::Liquid, Phase::Gas) => true,
            _ => false,
        },
    }
}

fn step_pixel(world: &World, scratch: &mut ScratchGrid, p: Vec2i, rng: &mut SmallRng) {
    let cell = scratch.get(p);
    if cell.material == material::EMPTY {
        return;
    }
    let props = world.material_props(cell.material);
    if props.inert {
        return;
    }
    match props.phase {
        Phase::Solid => step_sand_scratch(world, scratch, p, rng),
        Phase::Liquid => step_liquid_scratch(world, scratch, p, rng),
        Phase::Gas => step_gas_scratch(world, scratch, p, rng),
    }
}

fn step_sand_scratch(world: &World, scratch: &mut ScratchGrid, p: Vec2i, rng: &mut SmallRng) {
    if scratch.was_moved(p) {
        return;
    }
    let cell = scratch.get(p);
    let props = world.material_props(cell.material);
    let new_vel = ((cell.velocity as i16) + props.acceleration as i16).min(props.max_speed as i16) as i8;
    scratch.set_velocity(p, new_vel);

    let steps = (new_vel as i32).max(1);
    let mut current = p;

    for _ in 0..steps {
        let down = Vec2i::new(current.x, current.y + 1);
        if scratch.try_displace(world, current, down, MoveIntent::VerticalDown, rng) {
            current = down;
            continue;
        }
        let left_first = rng.gen_bool(0.5);
        let dl = Vec2i::new(current.x - 1, current.y + 1);
        let dr = Vec2i::new(current.x + 1, current.y + 1);
        let (first, second) = if left_first { (dl, dr) } else { (dr, dl) };
        if scratch.try_displace(world, current, first, MoveIntent::VerticalDown, rng) {
            current = first;
        } else if scratch.try_displace(world, current, second, MoveIntent::VerticalDown, rng) {
            current = second;
        } else {
            scratch.set_velocity(current, 0);
            break;
        }
    }

    if current == p {
        let below = Vec2i::new(p.x, p.y + 1);
        let below_cell = scratch.get(below);
        let below_props = world.material_props(below_cell.material);
        if below_props.phase == Phase::Liquid && rng.gen_ratio(1, 5) {
            let dir = if rng.gen_bool(0.5) { -1 } else { 1 };
            let side = Vec2i::new(p.x + dir, p.y);
            if scratch.get(side).material == material::EMPTY {
                let _ = scratch.try_displace(world, p, side, MoveIntent::Lateral, rng);
            }
        }
    }
}

fn step_liquid_scratch(world: &World, scratch: &mut ScratchGrid, p: Vec2i, rng: &mut SmallRng) {
    if scratch.was_moved(p) {
        return;
    }
    let cell = scratch.get(p);
    let props = world.material_props(cell.material);
    let new_vel = ((cell.velocity as i16) + props.acceleration as i16).min(props.max_speed as i16) as i8;
    scratch.set_velocity(p, new_vel);

    let steps = (new_vel as i32).max(1);
    let mut current = p;

    for _ in 0..steps {
        let down = Vec2i::new(current.x, current.y + 1);
        if scratch.try_displace(world, current, down, MoveIntent::VerticalDown, rng) {
            current = down;
            continue;
        }
        let left_first = rng.gen_bool(0.5);
        let dl = Vec2i::new(current.x - 1, current.y + 1);
        let dr = Vec2i::new(current.x + 1, current.y + 1);
        let (first, second) = if left_first { (dl, dr) } else { (dr, dl) };
        if scratch.try_displace(world, current, first, MoveIntent::VerticalDown, rng) {
            current = first;
        } else if scratch.try_displace(world, current, second, MoveIntent::VerticalDown, rng) {
            current = second;
        } else {
            scratch.set_velocity(current, 0);
            break;
        }
    }

    let below_current = Vec2i::new(current.x, current.y + 1);
    let supported = scratch.get(below_current).material != material::EMPTY;
    if supported {
        let lateral_chance = (255 - props.viscosity as i32).clamp(0, 255) as u32;
        if !rng.gen_ratio(lateral_chance + 1, 256) {
            return;
        }
        let rule = world.material_rule(scratch.get(current).material);
        let spread = rule.lateral_spread.max(1) as i32;

        let left_target = scan_lateral_target(scratch, current, -1, spread);
        let right_target = scan_lateral_target(scratch, current, 1, spread);

        let dir = match (left_target, right_target) {
            (Some(l), Some(r)) => {
                if l < r { -1 }
                else if r < l { 1 }
                else if rng.gen_bool(0.5) { -1 } else { 1 }
            }
            (Some(_), None) => -1,
            (None, Some(_)) => 1,
            (None, None) => {
                let above = Vec2i::new(current.x, current.y - 1);
                let above_props = world.material_props(scratch.get(above).material);
                if above_props.phase == Phase::Liquid || above_props.phase == Phase::Solid {
                    if rng.gen_bool(0.5) { -1 } else { 1 }
                } else {
                    return;
                }
            }
        };

        let max_move = spread.min(2);
        for i in 1..=max_move {
            let side = Vec2i::new(current.x + dir * i, current.y);
            if scratch.try_displace(world, current, side, MoveIntent::Lateral, rng) {
                current = side;
            } else {
                break;
            }
        }
    }
}

fn scan_lateral_target(scratch: &ScratchGrid, from: Vec2i, dir: i32, max_dist: i32) -> Option<i32> {
    for i in 1..=max_dist {
        let p = Vec2i::new(from.x + dir * i, from.y);
        let cell = scratch.get(p);
        if cell.material == material::EMPTY {
            return Some(i);
        }
        let below = Vec2i::new(p.x, p.y + 1);
        let below_cell = scratch.get(below);
        if below_cell.material == material::EMPTY {
            return Some(i);
        }
    }
    None
}

fn step_gas_scratch(world: &World, scratch: &mut ScratchGrid, p: Vec2i, rng: &mut SmallRng) {
    if scratch.was_moved(p) {
        return;
    }
    let cell = scratch.get(p);
    let props = world.material_props(cell.material);
    let new_vel = ((cell.velocity as i16) + props.acceleration as i16).min(props.max_speed as i16) as i8;
    scratch.set_velocity(p, new_vel);

    let steps = (new_vel as i32).max(1);
    let mut current = p;

    for _ in 0..steps {
        let up = Vec2i::new(current.x, current.y - 1);
        if scratch.try_displace(world, current, up, MoveIntent::VerticalUp, rng) {
            current = up;
            continue;
        }
        let left_first = rng.gen_bool(0.5);
        let ul = Vec2i::new(current.x - 1, current.y - 1);
        let ur = Vec2i::new(current.x + 1, current.y - 1);
        let (first, second) = if left_first { (ul, ur) } else { (ur, ul) };
        if scratch.try_displace(world, current, first, MoveIntent::VerticalUp, rng) {
            current = first;
        } else if scratch.try_displace(world, current, second, MoveIntent::VerticalUp, rng) {
            current = second;
        } else {
            scratch.set_velocity(current, 0);
            break;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub pos: (f32, f32),
    pub vel: (f32, f32),
    pub cell: Cell,
    pub lifetime: f32,
}

pub struct ParticleSim {
    particles: Vec<Particle>,
    gravity: f32,
}

impl ParticleSim {
    pub fn new() -> Self {
        Self {
            particles: Vec::new(),
            gravity: 50.0,
        }
    }

    pub fn spawn(&mut self, particle: Particle) {
        self.particles.push(particle);
    }

    pub fn step(&mut self, dt: f32, world: &mut World, events: &mut Vec<SimulationEvent>) {
        let mut survivors = Vec::with_capacity(self.particles.len());
        for mut particle in self.particles.drain(..) {
            particle.vel.1 += self.gravity * dt;
            particle.pos.0 += particle.vel.0 * dt;
            particle.pos.1 += particle.vel.1 * dt;
            particle.lifetime -= dt;

            let cell_pos = Vec2i::new(particle.pos.0.round() as i32, particle.pos.1.round() as i32);
            if particle.lifetime <= 0.0 || world.get_cell(cell_pos).material == material::EMPTY {
                if world.get_cell(cell_pos).material == material::EMPTY {
                    world.set_cell(cell_pos, particle.cell);
                } else {
                    survivors.push(particle);
                }
            } else {
                survivors.push(particle);
            }
            events.push(SimulationEvent::PixelEjectedToParticle { at: cell_pos });
        }
        self.particles = survivors;
    }

    pub fn cull_outside(&mut self, bounds: RectI) {
        self.particles.retain(|particle| {
            let p = Vec2i::new(particle.pos.0.round() as i32, particle.pos.1.round() as i32);
            bounds.contains(p)
        });
    }
}

pub fn deterministic_hash(world: &World) -> Option<u64> {
    let bounds: RectI = world.dirty_world_bounds()?;
    let mut hash: u64 = 1469598103934665603;
    for y in bounds.min.y..=bounds.max.y {
        for x in bounds.min.x..=bounds.max.x {
            let c = world.get_cell(Vec2i::new(x, y));
            hash ^= c.material as u64;
            hash = hash.wrapping_mul(1099511628211);
            hash ^= c.color as u64;
            hash = hash.wrapping_mul(1099511628211);
            hash ^= c.velocity as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
    }
    Some(hash)
}
