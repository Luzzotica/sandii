use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use nalgebra::{point, vector};
use rand::Rng;
use rapier2d::prelude::*;

use crate::world::cell_flags;
use crate::world::{material, Cell, ChunkCoord, MaterialId, RectI, Vec2i, World, CHUNK_SIZE};

fn rigid_body_cell_at_spawn(world: &World, _p: Vec2i, material_id: MaterialId, variant: u8) -> Cell {
    let props = world.material_props(material_id);
    let mut cell = Cell::default();
    cell.material = material_id;
    cell.variant = variant;
    cell.flags = 0;
    cell.velocity = 0;
    cell.scorch = 0;
    if material_id == material::LAVA && props.fuel_mass > 0 {
        cell.flags |= cell_flags::ON_FIRE;
        cell.lifetime = props.fuel_mass;
    } else {
        cell.lifetime = World::initial_lifetime_for(material_id, &props);
    }
    cell
}

#[derive(Debug, Clone, Copy)]
pub struct RigidBodySpec {
    pub min: Vec2i,
    pub max: Vec2i,
    pub material: MaterialId,
}

#[derive(Clone)]
pub(crate) struct PixelAnchor {
    pub local: Vec2i,
    pub cell: Cell,
}

pub(crate) struct PixelRigidBody {
    pub id: u32,
    pub anchors: Vec<PixelAnchor>,
    pub handle: rapier2d::prelude::RigidBodyHandle,
    pub needs_collider_refresh: bool,
}

pub struct RigidBridge {
    gravity: Vector<Real>,
    pipeline: PhysicsPipeline,
    integration_parameters: IntegrationParameters,
    islands: IslandManager,
    broad_phase: BroadPhaseMultiSap,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    bodies_by_id: HashMap<u32, PixelRigidBody>,
    pre_physics_positions: HashMap<u32, HashSet<(i32, i32)>>,
    next_id: u32,

    static_body_handle: RigidBodyHandle,
    static_chunk_colliders: HashMap<ChunkCoord, Vec<ColliderHandle>>,
    static_chunk_hashes: HashMap<ChunkCoord, u64>,
}

impl RigidBridge {
    pub fn new() -> Self {
        let mut bodies = RigidBodySet::new();
        let static_body = RigidBodyBuilder::fixed().build();
        let static_body_handle = bodies.insert(static_body);

        Self {
            gravity: vector![0.0, 200.0],
            pipeline: PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseMultiSap::new(),
            narrow_phase: NarrowPhase::new(),
            bodies,
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            bodies_by_id: HashMap::new(),
            pre_physics_positions: HashMap::new(),
            next_id: 1,
            static_body_handle,
            static_chunk_colliders: HashMap::new(),
            static_chunk_hashes: HashMap::new(),
        }
    }

    pub fn spawn_from_world_rect(&mut self, world: &mut World, spec: RigidBodySpec) -> u32 {
        let mut positions = Vec::new();
        for y in spec.min.y..=spec.max.y {
            for x in spec.min.x..=spec.max.x {
                positions.push(Vec2i::new(x, y));
            }
        }
        self.spawn_from_pixels(world, &positions, spec.material)
    }

    pub fn spawn_from_pixels(
        &mut self,
        world: &mut World,
        positions: &[Vec2i],
        material_id: MaterialId,
    ) -> u32 {
        if positions.is_empty() {
            return 0;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let sum_x: i64 = positions.iter().map(|p| p.x as i64).sum();
        let sum_y: i64 = positions.iter().map(|p| p.y as i64).sum();
        let n = positions.len() as i64;
        let center = Vec2i::new((sum_x / n) as i32, (sum_y / n) as i32);

        let mut rng = rand::thread_rng();
        let mut anchors = Vec::new();
        for &p in positions {
            let cell = rigid_body_cell_at_spawn(world, p, material_id, rng.gen_range(0..=40));
            world.set_cell(p, cell);
            world.set_rigid_id(p, id);
            anchors.push(PixelAnchor {
                local: Vec2i::new(p.x - center.x, p.y - center.y),
                cell,
            });
        }

        let local_positions: Vec<(i32, i32)> = positions
            .iter()
            .map(|p| (p.x - center.x, p.y - center.y))
            .collect();
        let contours = extract_contours_from_pixels(&local_positions);

        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![center.x as f32, center.y as f32])
            .build();
        let handle = self.bodies.insert(rb);

        if let Some(outer) = contours
            .iter()
            .max_by(|a, b| {
                polygon_area(a)
                    .abs()
                    .partial_cmp(&polygon_area(b).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        {
            let mut simplified = simplify_douglas_peucker(outer, 0.5);
            if polygon_area(&simplified) < 0.0 {
                simplified.reverse();
            }
            let triangles = triangulate_ear_clip(&simplified);
            for tri in triangles {
                let shape = SharedShape::triangle(tri[0], tri[1], tri[2]);
                let collider = ColliderBuilder::new(shape)
                    .density(5.0)
                    .friction(0.6)
                    .restitution(0.2)
                    .build();
                self.colliders
                    .insert_with_parent(collider, handle, &mut self.bodies);
            }
        }

        self.bodies_by_id
            .insert(id, PixelRigidBody { id, anchors, handle, needs_collider_refresh: false });
        id
    }

    pub fn spawn_from_circle(
        &mut self,
        world: &mut World,
        positions: &[Vec2i],
        material_id: MaterialId,
        radius: f32,
    ) -> u32 {
        if positions.is_empty() {
            return 0;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let sum_x: i64 = positions.iter().map(|p| p.x as i64).sum();
        let sum_y: i64 = positions.iter().map(|p| p.y as i64).sum();
        let n = positions.len() as i64;
        let center = Vec2i::new((sum_x / n) as i32, (sum_y / n) as i32);

        let mut rng = rand::thread_rng();
        let mut anchors = Vec::new();
        for &p in positions {
            let cell = rigid_body_cell_at_spawn(world, p, material_id, rng.gen_range(0..=40));
            world.set_cell(p, cell);
            world.set_rigid_id(p, id);
            anchors.push(PixelAnchor {
                local: Vec2i::new(p.x - center.x, p.y - center.y),
                cell,
            });
        }

        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![center.x as f32, center.y as f32])
            .build();
        let handle = self.bodies.insert(rb);

        let collider = ColliderBuilder::ball(radius)
            .density(5.0)
            .friction(0.6)
            .restitution(0.2)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);

        self.bodies_by_id
            .insert(id, PixelRigidBody { id, anchors, handle, needs_collider_refresh: false });
        id
    }

    /// Record each body's current world-pixel positions (pre-physics).
    /// Body pixels are set to STATIC so the sim step treats them as inert
    /// solid walls — sand/water can't enter or displace them.
    pub fn record_positions(&mut self, world: &mut World) {
        self.pre_physics_positions.clear();
        for rigid in self.bodies_by_id.values() {
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = body.translation();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();
            let mut positions = HashSet::new();
            for anchor in &rigid.anchors {
                let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
                let key = (wp.x, wp.y);
                if positions.insert(key) {
                    if world.get_rigid_id(wp) == Some(rigid.id) {
                        let logical = anchor.cell;
                        let mut cell = logical;
                        cell.material = material::STATIC;
                        // Only molten on-fire lava uses the heat proxy; cooled sand/etc. stay plain STATIC.
                        if logical.material == material::LAVA
                            && (logical.flags & cell_flags::ON_FIRE) != 0
                            && logical.lifetime > 0
                        {
                            cell.flags |= cell_flags::RIGID_BODY_SIM;
                        } else {
                            cell.flags &= !cell_flags::RIGID_BODY_SIM;
                        }
                        world.set_cell(wp, cell);
                    }
                }
            }
            self.pre_physics_positions.insert(rigid.id, positions);
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.integration_parameters.dt = dt.max(0.0001);
        self.pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            None,
            &(),
            &(),
        );
    }

    /// After the physics step, diff old vs new positions for each body.
    /// Clear cells the body is leaving; place pixels where it is entering.
    pub fn sync_pixels_to_physics(
        &mut self,
        world: &mut World,
    ) {
        let empty_set = HashSet::new();
        let mut drag_list: Vec<(RigidBodyHandle, u32)> = Vec::new();

        // Pull sim results (lifetime, lava→sand, etc.) into anchors before we write back.
        let ids_handles: Vec<(u32, rapier2d::prelude::RigidBodyHandle)> = self
            .bodies_by_id
            .iter()
            .map(|(&id, rb)| (id, rb.handle))
            .collect();
        for (id, handle) in ids_handles {
            let Some(body) = self.bodies.get(handle) else {
                continue;
            };
            let pos = *body.translation();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();
            let Some(rigid) = self.bodies_by_id.get_mut(&id) else {
                continue;
            };
            let mut material_changed = false;
            for anchor in &mut rigid.anchors {
                let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
                if world.get_rigid_id(wp) != Some(id) {
                    continue;
                }
                let wc = world.get_cell(wp);
                let before_mat = anchor.cell.material;
                if wc.material == material::STATIC {
                    // `record_positions` uses STATIC as a sim placeholder; the real material lives on the anchor.
                    // Copy ticked fields (lifetime, flags) but keep logical material until the sim replaces
                    // the cell with something else (e.g. lava burnout → sand).
                    anchor.cell = Cell {
                        material: anchor.cell.material,
                        flags: wc.flags & !cell_flags::RIGID_BODY_SIM,
                        lifetime: wc.lifetime,
                        velocity: wc.velocity,
                        variant: wc.variant,
                        scorch: wc.scorch,
                    };
                } else {
                    anchor.cell = wc;
                    if anchor.cell.material != before_mat {
                        material_changed = true;
                    }
                }
            }
            if material_changed {
                rigid.needs_collider_refresh = true;
            }
        }

        for rigid in self.bodies_by_id.values() {
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = *body.translation();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let new_cells: HashMap<(i32, i32), Cell> = {
                let mut map = HashMap::new();
                for anchor in &rigid.anchors {
                    let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
                    map.entry((wp.x, wp.y)).or_insert(anchor.cell);
                }
                map
            };

            let old_cells = self.pre_physics_positions.get(&rigid.id).unwrap_or(&empty_set);

            // Clear cells the body is leaving (old but not new)
            for &(ox, oy) in old_cells {
                if !new_cells.contains_key(&(ox, oy)) {
                    let wp = Vec2i::new(ox, oy);
                    if world.get_rigid_id(wp) == Some(rigid.id) {
                        world.set_cell(wp, Cell::default());
                        world.clear_rigid_id(wp);
                    }
                }
            }

            // Place/update cells the body now occupies
            let new_keys: HashSet<(i32, i32)> = new_cells.keys().copied().collect();
            let search_radius = (rigid.anchors.len() as f32).sqrt().ceil() as i32 + 4;
            let mut displaced_count: u32 = 0;

            for (&key, &cell) in &new_cells {
                let wp = Vec2i::new(key.0, key.1);

                if world.get_rigid_id(wp) == Some(rigid.id) {
                    world.set_cell(wp, cell);
                    continue;
                }

                let occupant = world.get_cell(wp);

                if occupant.material == material::EMPTY {
                    world.set_cell(wp, cell);
                    world.set_rigid_id(wp, rigid.id);
                    continue;
                }

                if world.material_props(occupant.material).inert() {
                    continue;
                }

                // Entering a cell with non-inert material — displace it upward
                displaced_count += 1;
                if let Some(dest) = find_empty_above(world, wp, &new_keys, search_radius) {
                    world.set_cell(dest, occupant);
                    world.set_cell(wp, cell);
                    world.set_rigid_id(wp, rigid.id);
                }
            }

            if displaced_count > 0 {
                drag_list.push((rigid.handle, displaced_count));
            }
        }

        for (handle, count) in drag_list {
            if let Some(body) = self.bodies.get_mut(handle) {
                let factor = 1.0 / (1.0 + count as f32 * 0.05);
                let lv = *body.linvel() * factor;
                let av = body.angvel() * factor;
                body.set_linvel(lv, true);
                body.set_angvel(av, true);
            }
        }
    }

    /// Returns world-space line segments for every collider triangle in the
    /// physics world (both dynamic bodies and static terrain).
    pub fn debug_collider_lines(&self) -> Vec<[(f32, f32); 2]> {
        let mut lines = Vec::new();
        for (_handle, collider) in self.colliders.iter() {
            let (tx, ty, angle) = if let Some(parent_handle) = collider.parent() {
                if let Some(body) = self.bodies.get(parent_handle) {
                    (body.translation().x, body.translation().y, body.rotation().angle())
                } else {
                    continue;
                }
            } else {
                let p = collider.position();
                (p.translation.x, p.translation.y, p.rotation.angle())
            };
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let col_rel = collider.position_wrt_parent().map(|iso| {
                (iso.translation.x, iso.translation.y, iso.rotation.angle())
            }).unwrap_or((0.0, 0.0, 0.0));

            let to_world = |lx: f32, ly: f32| -> (f32, f32) {
                let cos_c = col_rel.2.cos();
                let sin_c = col_rel.2.sin();
                let rx = lx * cos_c - ly * sin_c + col_rel.0;
                let ry = lx * sin_c + ly * cos_c + col_rel.1;
                let wx = rx * cos_a - ry * sin_a + tx;
                let wy = rx * sin_a + ry * cos_a + ty;
                (wx, wy)
            };

            if let Some(tri) = collider.shape().as_triangle() {
                let a = to_world(tri.a.x, tri.a.y);
                let b = to_world(tri.b.x, tri.b.y);
                let c = to_world(tri.c.x, tri.c.y);
                lines.push([a, b]);
                lines.push([b, c]);
                lines.push([c, a]);
            } else if let Some(ball) = collider.shape().as_ball() {
                const SEGMENTS: usize = 32;
                for i in 0..SEGMENTS {
                    let a0 = (i as f32) * std::f32::consts::TAU / SEGMENTS as f32;
                    let a1 = ((i + 1) as f32) * std::f32::consts::TAU / SEGMENTS as f32;
                    let p0 = to_world(ball.radius * a0.cos(), ball.radius * a0.sin());
                    let p1 = to_world(ball.radius * a1.cos(), ball.radius * a1.sin());
                    lines.push([p0, p1]);
                }
            } else if let Some(cuboid) = collider.shape().as_cuboid() {
                let hw = cuboid.half_extents.x;
                let hh = cuboid.half_extents.y;
                let a = to_world(-hw, -hh);
                let b = to_world(hw, -hh);
                let c = to_world(hw, hh);
                let d = to_world(-hw, hh);
                lines.push([a, b]);
                lines.push([b, c]);
                lines.push([c, d]);
                lines.push([d, a]);
            }
        }
        lines
    }

    pub fn cull_outside(&mut self, world: &mut World, bounds: RectI) {
        let mut ids_to_remove = Vec::new();
        for (id, rigid) in &self.bodies_by_id {
            let Some(body) = self.bodies.get(rigid.handle) else {
                ids_to_remove.push(*id);
                continue;
            };
            let center = Vec2i::new(body.translation().x.round() as i32, body.translation().y.round() as i32);
            if !bounds.contains(center) {
                ids_to_remove.push(*id);
            }
        }

        for id in ids_to_remove {
            if let Some(rigid) = self.bodies_by_id.remove(&id) {
                self.clear_pixels_for_body(world, &rigid);
                self.bodies.remove(
                    rigid.handle,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    true,
                );
                self.pre_physics_positions.remove(&id);
            }
        }

        let bounds_min_cx = bounds.min.x.div_euclid(CHUNK_SIZE);
        let bounds_min_cy = bounds.min.y.div_euclid(CHUNK_SIZE);
        let bounds_max_cx = bounds.max.x.div_euclid(CHUNK_SIZE);
        let bounds_max_cy = bounds.max.y.div_euclid(CHUNK_SIZE);
        let stale: Vec<ChunkCoord> = self
            .static_chunk_colliders
            .keys()
            .copied()
            .filter(|c| c.x < bounds_min_cx || c.x > bounds_max_cx || c.y < bounds_min_cy || c.y > bounds_max_cy)
            .collect();
        for coord in stale {
            self.remove_static_chunk(coord);
        }
    }

    fn clear_pixels_for_body(&self, world: &mut World, rigid: &PixelRigidBody) {
        let Some(body) = self.bodies.get(rigid.handle) else {
            return;
        };
        let pos = body.translation();
        let angle = body.rotation().angle();
        let cos_a = angle.cos();
        let sin_a = angle.sin();
        for anchor in &rigid.anchors {
            let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
            if world.get_rigid_id(wp) == Some(rigid.id) {
                world.set_cell(wp, Cell::default());
                world.clear_rigid_id(wp);
            }
        }
    }

    // --- Static terrain colliders (per-chunk) ---

    pub fn rebuild_static_colliders(&mut self, world: &World, dirty_chunks: &[ChunkCoord]) {
        for &coord in dirty_chunks {
            let bounds = world.bounds_for_chunk(coord);
            let info = analyze_inert_cells(world, &bounds);

            self.remove_static_chunk(coord);
            self.static_chunk_hashes.insert(coord, info.hash);

            if info.inert_count == 0 {
                continue;
            }

            let mut handles = Vec::new();
            let rects = build_run_rectangles(world, &bounds);
            for (cx, cy, hw, hh) in rects {
                let collider = ColliderBuilder::cuboid(hw, hh)
                    .translation(vector![cx, cy])
                    .friction(0.5)
                    .restitution(0.0)
                    .build();
                let handle = self.colliders.insert_with_parent(
                    collider,
                    self.static_body_handle,
                    &mut self.bodies,
                );
                handles.push(handle);
            }

            if !handles.is_empty() {
                self.static_chunk_colliders.insert(coord, handles);
            }
        }
    }

    fn remove_static_chunk(&mut self, coord: ChunkCoord) {
        if let Some(handles) = self.static_chunk_colliders.remove(&coord) {
            for h in handles {
                self.colliders.remove(h, &mut self.islands, &mut self.bodies, true);
            }
        }
        self.static_chunk_hashes.remove(&coord);
    }

    pub fn clear_all_static_colliders(&mut self) {
        let coords: Vec<ChunkCoord> = self.static_chunk_colliders.keys().copied().collect();
        for coord in coords {
            self.remove_static_chunk(coord);
        }
    }

    /// Number of Rapier collider handles attached to the static body for this chunk (test / diagnostics).
    pub fn static_collider_count_for_chunk(&self, coord: ChunkCoord) -> usize {
        self.static_chunk_colliders
            .get(&coord)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    // --- Dynamic body collider refresh ---

    pub fn rebuild_dirty_dynamic_colliders(&mut self) {
        let ids: Vec<u32> = self
            .bodies_by_id
            .iter()
            .filter_map(|(&id, rb)| if rb.needs_collider_refresh { Some(id) } else { None })
            .collect();

        for id in ids {
            let Some(rigid) = self.bodies_by_id.get_mut(&id) else {
                continue;
            };
            rigid.needs_collider_refresh = false;

            if rigid.anchors.is_empty() {
                continue;
            }

            let handle = rigid.handle;
            let attached: Vec<ColliderHandle> = self
                .bodies
                .get(handle)
                .map(|b| b.colliders().to_vec())
                .unwrap_or_default();
            for ch in attached {
                self.colliders.remove(ch, &mut self.islands, &mut self.bodies, true);
            }

            let local_positions: Vec<(i32, i32)> = rigid
                .anchors
                .iter()
                .map(|a| (a.local.x, a.local.y))
                .collect();
            let contours = extract_contours_from_pixels(&local_positions);

            if let Some(outer) = contours
                .iter()
                .max_by(|a, b| {
                    polygon_area(a)
                        .abs()
                        .partial_cmp(&polygon_area(b).abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            {
                let mut simplified = simplify_douglas_peucker(outer, 0.5);
                if polygon_area(&simplified) < 0.0 {
                    simplified.reverse();
                }
                let triangles = triangulate_ear_clip(&simplified);
                for tri in triangles {
                    let shape = SharedShape::triangle(tri[0], tri[1], tri[2]);
                    let collider = ColliderBuilder::new(shape)
                        .density(5.0)
                        .friction(0.6)
                        .restitution(0.2)
                        .build();
                    self.colliders
                        .insert_with_parent(collider, handle, &mut self.bodies);
                }
            }
        }
    }

    pub fn check_splits(&mut self, world: &mut World) {
        const DAMAGE_SPEED: f32 = 80.0;

        let body_ids: Vec<u32> = self.bodies_by_id.keys().copied().collect();
        let mut splits_to_process = Vec::new();

        for body_id in body_ids {
            let Some(rigid) = self.bodies_by_id.get(&body_id) else {
                continue;
            };
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = *body.translation();
            let linvel = *body.linvel();
            let angvel = body.angvel();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let speed = (linvel.x * linvel.x + linvel.y * linvel.y).sqrt()
                + angvel.abs() * 5.0;

            let mut surviving = Vec::new();
            let mut has_env_missing = false;
            let mut has_impact_missing = false;

            for anchor in &rigid.anchors {
                let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
                let rid = world.get_rigid_id(wp);
                let mat = world.get_cell(wp).material;

                if rid == Some(body_id) {
                    surviving.push(anchor.clone());
                    continue;
                }

                if mat == material::EMPTY {
                    // Sim removed this pixel (acid, fire, etc.) — always drop anchor.
                    has_env_missing = true;
                    continue;
                }

                if rid.is_some() || world.material_props(mat).inert() {
                    // Another rigid body or terrain blocking this slot — keep anchor.
                    surviving.push(anchor.clone());
                } else {
                    // Non-inert occupant without our rigid_id (e.g. violent displacement).
                    has_impact_missing = true;
                }
            }

            let should_split =
                has_env_missing || (has_impact_missing && speed >= DAMAGE_SPEED);
            if should_split {
                splits_to_process.push((body_id, surviving, pos, linvel, angvel, angle));
            }
        }

        for (body_id, surviving, pos, linvel, angvel, angle) in splits_to_process {
            if surviving.is_empty() {
                self.remove_body_physics(body_id);
                continue;
            }

            let components = find_connected_components(&surviving);
            if components.len() <= 1 {
                if let Some(rigid) = self.bodies_by_id.get_mut(&body_id) {
                    rigid.anchors = surviving;
                    rigid.needs_collider_refresh = true;
                }
                continue;
            }

            let cos_a = angle.cos();
            let sin_a = angle.sin();

            for anchor in &surviving {
                let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
                if world.get_rigid_id(wp) == Some(body_id) {
                    world.set_cell(wp, Cell::default());
                    world.clear_rigid_id(wp);
                }
            }
            self.remove_body_physics(body_id);

            for component in components {
                let world_positions: Vec<Vec2i> = component
                    .iter()
                    .map(|a| rotated_world_pos(pos.x, pos.y, cos_a, sin_a, a.local))
                    .collect();
                let mat = component[0].cell.material;
                let new_id = self.spawn_from_pixels(world, &world_positions, mat);
                if let Some(new_rigid) = self.bodies_by_id.get(&new_id) {
                    if let Some(new_body) = self.bodies.get_mut(new_rigid.handle) {
                        new_body.set_linvel(linvel, true);
                        new_body.set_angvel(angvel, true);
                    }
                }
            }
        }
    }

    fn remove_body_physics(&mut self, body_id: u32) {
        if let Some(rigid) = self.bodies_by_id.remove(&body_id) {
            self.bodies.remove(
                rigid.handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
            self.pre_physics_positions.remove(&body_id);
        }
    }
}

fn find_connected_components(anchors: &[PixelAnchor]) -> Vec<Vec<PixelAnchor>> {
    let anchor_set: HashSet<(i32, i32)> = anchors.iter().map(|a| (a.local.x, a.local.y)).collect();
    let anchor_map: HashMap<(i32, i32), usize> = anchors
        .iter()
        .enumerate()
        .map(|(i, a)| ((a.local.x, a.local.y), i))
        .collect();
    let mut visited: HashSet<(i32, i32)> = HashSet::new();
    let mut components = Vec::new();

    for anchor in anchors {
        let key = (anchor.local.x, anchor.local.y);
        if visited.contains(&key) {
            continue;
        }

        let mut component = Vec::new();
        let mut stack = vec![key];
        while let Some(pos) = stack.pop() {
            if !anchor_set.contains(&pos) || visited.contains(&pos) {
                continue;
            }
            visited.insert(pos);
            if let Some(&idx) = anchor_map.get(&pos) {
                component.push(anchors[idx].clone());
            }
            stack.push((pos.0 + 1, pos.1));
            stack.push((pos.0 - 1, pos.1));
            stack.push((pos.0, pos.1 + 1));
            stack.push((pos.0, pos.1 - 1));
        }

        if !component.is_empty() {
            components.push(component);
        }
    }

    components
}

fn extract_contours_from_pixels(local_positions: &[(i32, i32)]) -> Vec<Vec<(f32, f32)>> {
    let filled: HashSet<(i32, i32)> = local_positions.iter().copied().collect();

    // Boundary edges using doubled integer coords to avoid FP comparison during chaining.
    // Pixel (px,py) corners: TL=(2px-1,2py-1) TR=(2px+1,2py-1) BR=(2px+1,2py+1) BL=(2px-1,2py+1)
    let mut edge_map: HashMap<(i32, i32), (i32, i32)> = HashMap::new();

    for &(px, py) in local_positions {
        let tl = (2 * px - 1, 2 * py - 1);
        let tr = (2 * px + 1, 2 * py - 1);
        let br = (2 * px + 1, 2 * py + 1);
        let bl = (2 * px - 1, 2 * py + 1);

        if !filled.contains(&(px, py - 1)) {
            edge_map.insert(tl, tr);
        }
        if !filled.contains(&(px + 1, py)) {
            edge_map.insert(tr, br);
        }
        if !filled.contains(&(px, py + 1)) {
            edge_map.insert(br, bl);
        }
        if !filled.contains(&(px - 1, py)) {
            edge_map.insert(bl, tl);
        }
    }

    let mut loops = Vec::new();
    let mut visited: HashSet<(i32, i32)> = HashSet::new();
    let starts: Vec<(i32, i32)> = edge_map.keys().copied().collect();

    for start in starts {
        if visited.contains(&start) {
            continue;
        }
        let mut loop_points = Vec::new();
        let mut current = start;
        loop {
            if visited.contains(&current) && current != start {
                break;
            }
            visited.insert(current);
            loop_points.push((current.0 as f32 * 0.5, current.1 as f32 * 0.5));
            if let Some(&next) = edge_map.get(&current) {
                current = next;
                if current == start {
                    break;
                }
            } else {
                break;
            }
        }
        if loop_points.len() >= 3 {
            loops.push(loop_points);
        }
    }

    loops
}

fn polygon_area(polygon: &[(f32, f32)]) -> f32 {
    let n = polygon.len();
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += polygon[i].0 * polygon[j].1;
        area -= polygon[j].0 * polygon[i].1;
    }
    area * 0.5
}

fn simplify_douglas_peucker(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let first = points[0];
    let last = points[points.len() - 1];

    let mut max_dist = 0.0f32;
    let mut max_idx = 0;
    for i in 1..points.len() - 1 {
        let d = perpendicular_distance(points[i], first, last);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    if max_dist > epsilon {
        let left = simplify_douglas_peucker(&points[..=max_idx], epsilon);
        let right = simplify_douglas_peucker(&points[max_idx..], epsilon);
        let mut result = left;
        result.extend_from_slice(&right[1..]);
        result
    } else {
        vec![first, last]
    }
}

fn perpendicular_distance(point: (f32, f32), line_start: (f32, f32), line_end: (f32, f32)) -> f32 {
    let dx = line_end.0 - line_start.0;
    let dy = line_end.1 - line_start.1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-10 {
        let px = point.0 - line_start.0;
        let py = point.1 - line_start.1;
        return (px * px + py * py).sqrt();
    }
    let numerator = ((line_end.0 - line_start.0) * (line_start.1 - point.1)
        - (line_start.0 - point.0) * (line_end.1 - line_start.1))
        .abs();
    numerator / len_sq.sqrt()
}

fn triangulate_ear_clip(polygon: &[(f32, f32)]) -> Vec<[Point<Real>; 3]> {
    if polygon.len() < 3 {
        return Vec::new();
    }

    let mut indices: Vec<usize> = (0..polygon.len()).collect();
    let mut triangles = Vec::new();

    while indices.len() > 2 {
        let n = indices.len();
        let mut found_ear = false;

        for i in 0..n {
            let prev = indices[(i + n - 1) % n];
            let curr = indices[i];
            let next = indices[(i + 1) % n];

            let a = polygon[prev];
            let b = polygon[curr];
            let c = polygon[next];

            let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if cross <= 0.0 {
                continue;
            }

            let mut has_point_inside = false;
            for j in 0..n {
                if j == (i + n - 1) % n || j == i || j == (i + 1) % n {
                    continue;
                }
                let p = polygon[indices[j]];
                if point_in_triangle(p, a, b, c) {
                    has_point_inside = true;
                    break;
                }
            }

            if !has_point_inside {
                triangles.push([point![a.0, a.1], point![b.0, b.1], point![c.0, c.1]]);
                indices.remove(i);
                found_ear = true;
                break;
            }
        }

        if !found_ear {
            for i in 1..indices.len() - 1 {
                let a = polygon[indices[0]];
                let b = polygon[indices[i]];
                let c = polygon[indices[i + 1]];
                triangles.push([point![a.0, a.1], point![b.0, b.1], point![c.0, c.1]]);
            }
            break;
        }
    }

    triangles
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = sign_2d(p, a, b);
    let d2 = sign_2d(p, b, c);
    let d3 = sign_2d(p, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn sign_2d(p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) -> f32 {
    (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
}

struct ChunkInertInfo {
    hash: u64,
    inert_count: u32,
}

fn is_static_terrain(world: &World, pos: Vec2i) -> bool {
    let cell = world.get_cell(pos);
    world.cell_contributes_static_collider(pos, cell)
}

fn analyze_inert_cells(world: &World, bounds: &RectI) -> ChunkInertInfo {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut inert_count: u32 = 0;
    for y in bounds.min.y..=bounds.max.y {
        for x in bounds.min.x..=bounds.max.x {
            let is_inert = is_static_terrain(world, Vec2i::new(x, y));
            is_inert.hash(&mut hasher);
            if is_inert {
                inert_count += 1;
            }
        }
    }
    ChunkInertInfo { hash: hasher.finish(), inert_count }
}

fn rotated_world_pos(tx: f32, ty: f32, cos_a: f32, sin_a: f32, local: Vec2i) -> Vec2i {
    let lx = local.x as f32;
    let ly = local.y as f32;
    Vec2i::new(
        (tx + lx * cos_a - ly * sin_a).round() as i32,
        (ty + lx * sin_a + ly * cos_a).round() as i32,
    )
}

/// Scan rows of a chunk for contiguous runs of inert cells and merge vertically
/// into rectangles. Returns (center_x, center_y, half_width, half_height) for each.
fn build_run_rectangles(world: &World, bounds: &RectI) -> Vec<(f32, f32, f32, f32)> {
    let mut active: Vec<(i32, i32, i32)> = Vec::new(); // (start_x, end_x, start_y)
    let mut rects = Vec::new();

    // +1 extra iteration to flush remaining active runs
    for y in bounds.min.y..=bounds.max.y + 1 {
        let mut row_runs: Vec<(i32, i32)> = Vec::new();
        if y <= bounds.max.y {
            let mut run_start: Option<i32> = None;
            for x in bounds.min.x..=bounds.max.x {
                let solid = is_static_terrain(world, Vec2i::new(x, y));
                if solid {
                    if run_start.is_none() {
                        run_start = Some(x);
                    }
                } else if let Some(sx) = run_start {
                    row_runs.push((sx, x - 1));
                    run_start = None;
                }
            }
            if let Some(sx) = run_start {
                row_runs.push((sx, bounds.max.x));
            }
        }

        let mut matched_active = vec![false; active.len()];
        let mut matched_row = vec![false; row_runs.len()];

        for (ai, &(asx, aex, _)) in active.iter().enumerate() {
            for (ri, &(rsx, rex)) in row_runs.iter().enumerate() {
                if !matched_row[ri] && asx == rsx && aex == rex {
                    matched_active[ai] = true;
                    matched_row[ri] = true;
                    break;
                }
            }
        }

        for (ai, &(asx, aex, asy)) in active.iter().enumerate() {
            if !matched_active[ai] {
                let hw = (aex - asx + 1) as f32 / 2.0;
                let hh = (y - asy) as f32 / 2.0;
                let cx = asx as f32 + hw;
                let cy = asy as f32 + hh;
                rects.push((cx, cy, hw, hh));
            }
        }

        let mut new_active: Vec<(i32, i32, i32)> = Vec::new();
        for (ai, &run) in active.iter().enumerate() {
            if matched_active[ai] {
                new_active.push(run);
            }
        }
        for (ri, &(rsx, rex)) in row_runs.iter().enumerate() {
            if !matched_row[ri] {
                new_active.push((rsx, rex, y));
            }
        }
        active = new_active;
    }

    rects
}

fn find_empty_above(
    world: &World,
    pos: Vec2i,
    body_cells: &HashSet<(i32, i32)>,
    max_radius: i32,
) -> Option<Vec2i> {
    for r in 1..=max_radius {
        for dy in -r..0 {
            for dx in -r..=r {
                if dx.abs() != r && dy != -r {
                    continue;
                }
                let candidate = Vec2i::new(pos.x + dx, pos.y + dy);
                if body_cells.contains(&(candidate.x, candidate.y)) {
                    continue;
                }
                if world.get_cell(candidate).material == material::EMPTY {
                    return Some(candidate);
                }
            }
        }
    }
    None
}
