use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};

use nalgebra::{point, vector};
use rand::Rng;
use rapier2d::prelude::*;

use crate::world::cell_flags;
use crate::world::{
    material, Cell, ChunkCoord, MaterialId, Phase, RectI, Vec2i, World, CHUNK_SIZE,
};

/// Extra radius (world units = grid pixels) for [`RigidBridge::spawn_from_circle_with_temp`] ball colliders.
const BALL_COLLIDER_RADIUS_PIXEL_PAD: f32 = 0.0;

fn rigid_body_cell_at_spawn(
    world: &World,
    _p: Vec2i,
    material_id: MaterialId,
    variant: u8,
    temp_override: Option<u16>,
) -> Cell {
    let props = world.material_props(material_id);
    let lifetime = if material_id == material::LAVA && props.fuel_mass > 0 {
        props.fuel_mass
    } else {
        World::initial_lifetime_for(material_id, &props)
    };
    let temp = temp_override.unwrap_or(props.base_temperature);
    Cell::new()
        .with_material(material_id)
        .with_variant(variant)
        .with_lifetime(lifetime)
        .with_temperature(temp)
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
    next_id: u32,

    static_body_handle: RigidBodyHandle,
    static_chunk_colliders: HashMap<ChunkCoord, Vec<ColliderHandle>>,
    static_chunk_hashes: HashMap<ChunkCoord, u64>,
    /// Fixed walls just outside the playable [`RectI`] (see [`Self::set_world_border_colliders`]).
    world_border_colliders: Vec<ColliderHandle>,
}

/// Impact stress (linear speed + angular term) at which a rigid pixel with
/// [`MaterialProps::structure_integrity`] `== 1.0` loses anchors when overlapping inert terrain without
/// `rigid_id`. Break threshold is `this * integrity` (per-anchor material).
pub const STRUCTURAL_STRESS_SCALE: f32 = 80.0;

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
            next_id: 1,
            static_body_handle,
            static_chunk_colliders: HashMap::new(),
            static_chunk_hashes: HashMap::new(),
            world_border_colliders: Vec::new(),
        }
    }

    pub fn remove_world_border_colliders(&mut self) {
        for h in self.world_border_colliders.drain(..) {
            self.colliders
                .remove(h, &mut self.islands, &mut self.bodies, true);
        }
    }

    /// Four axis-aligned boxes **outside** the map edge pixels.
    ///
    /// Each cell `(ix, iy)` is treated as a unit square centered on the integer: x ∈ [ix−½, ix+½],
    /// y ∈ [iy−½, iy+½]. Walls sit flush on the **outer** faces at ±½ past the min/max indices,
    /// so there is no gap for rigid bodies to slip through (unlike placing walls at ±½ from the
    /// min/max indices, which leaves holes on the bottom and right).
    pub fn set_world_border_colliders(&mut self, bounds: RectI) {
        self.remove_world_border_colliders();

        let min_x = bounds.min.x as f32;
        let max_x = bounds.max.x as f32;
        let min_y = bounds.min.y as f32;
        let max_y = bounds.max.y as f32;
        let w = max_x - min_x + 1.0;
        let h = max_y - min_y + 1.0;
        let cx = (min_x + max_x) * 0.5;
        let cy = (min_y + max_y) * 0.5;
        let t = 0.5_f32;

        let mut push_wall = |tx: f32, ty: f32, hx: f32, hy: f32| {
            let collider = ColliderBuilder::cuboid(hx, hy)
                .translation(vector![tx, ty])
                .friction(0.5)
                .restitution(0.0)
                .build();
            let handle = self.colliders.insert_with_parent(
                collider,
                self.static_body_handle,
                &mut self.bodies,
            );
            self.world_border_colliders.push(handle);
        };

        // Left / right: inner vertical faces at min_x − ½ and max_x + ½.
        push_wall(min_x - 1.0, cy, t, h * 0.5);
        push_wall(max_x + 1.0, cy, t, h * 0.5);
        // Top / bottom: inner horizontal faces at min_y − ½ and max_y + ½.
        push_wall(cx, min_y - 1.0, w * 0.5, t);
        push_wall(cx, max_y + 1.0, w * 0.5, t);
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
        self.spawn_from_pixels_with_temp(world, positions, material_id, None)
    }

    pub fn spawn_from_pixels_with_temp(
        &mut self,
        world: &mut World,
        positions: &[Vec2i],
        material_id: MaterialId,
        temp_override: Option<u16>,
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
            let cell = rigid_body_cell_at_spawn(
                world,
                p,
                material_id,
                rng.gen_range(0..=40),
                temp_override,
            );
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

        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![center.x as f32, center.y as f32])
            .build();
        let handle = self.bodies.insert(rb);

        build_and_insert_dynamic_pixel_colliders(
            &mut self.colliders,
            &mut self.bodies,
            handle,
            &local_positions,
        );

        self.bodies_by_id.insert(
            id,
            PixelRigidBody {
                id,
                anchors,
                handle,
                needs_collider_refresh: false,
            },
        );
        id
    }

    pub fn spawn_from_circle(
        &mut self,
        world: &mut World,
        positions: &[Vec2i],
        material_id: MaterialId,
        radius: f32,
    ) -> u32 {
        self.spawn_from_circle_with_temp(world, positions, material_id, radius, None)
    }

    pub fn spawn_from_circle_with_temp(
        &mut self,
        world: &mut World,
        positions: &[Vec2i],
        material_id: MaterialId,
        radius: f32,
        temp_override: Option<u16>,
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
            let cell = rigid_body_cell_at_spawn(
                world,
                p,
                material_id,
                rng.gen_range(0..=40),
                temp_override,
            );
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

        let collider = ColliderBuilder::ball(radius + BALL_COLLIDER_RADIUS_PIXEL_PAD)
            .density(5.0)
            .friction(0.6)
            .restitution(0.2)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);

        self.bodies_by_id.insert(
            id,
            PixelRigidBody {
                id,
                anchors,
                handle,
                needs_collider_refresh: false,
            },
        );
        id
    }

    /// Spawn a rigid body from pre-existing anchors, each with its own cell/material.
    /// `anchors_with_world_pos` provides (world_pos, anchor_cell) pairs.
    fn spawn_from_anchors(
        &mut self,
        world: &mut World,
        anchors_with_world_pos: &[(Vec2i, Cell)],
    ) -> u32 {
        if anchors_with_world_pos.is_empty() {
            return 0;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);

        let n = anchors_with_world_pos.len() as i64;
        let sum_x: i64 = anchors_with_world_pos.iter().map(|(p, _)| p.x as i64).sum();
        let sum_y: i64 = anchors_with_world_pos.iter().map(|(p, _)| p.y as i64).sum();
        let center = Vec2i::new((sum_x / n) as i32, (sum_y / n) as i32);

        let mut anchors = Vec::with_capacity(anchors_with_world_pos.len());
        let mut local_positions = Vec::with_capacity(anchors_with_world_pos.len());
        for &(wp, cell) in anchors_with_world_pos {
            world.set_cell(wp, cell);
            world.set_rigid_id(wp, id);
            let local = Vec2i::new(wp.x - center.x, wp.y - center.y);
            local_positions.push((local.x, local.y));
            anchors.push(PixelAnchor { local, cell });
        }

        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![center.x as f32, center.y as f32])
            .build();
        let handle = self.bodies.insert(rb);

        build_and_insert_dynamic_pixel_colliders(
            &mut self.colliders,
            &mut self.bodies,
            handle,
            &local_positions,
        );

        self.bodies_by_id.insert(
            id,
            PixelRigidBody {
                id,
                anchors,
                handle,
                needs_collider_refresh: false,
            },
        );
        id
    }

    /// Record each body's current world-pixel positions (pre-physics).
    /// Anchor-rounded pixels get [`cell_flags::RIGID_PIXEL`] and `rigid_id` (solids only: melted
    /// liquids/gases must simulate as loose fluid, not skip motion in [`crate::sim::step_pixel`]).
    /// Morphological hole-fill is render-only (see [`Self::collect_visual_morph_overlay_tiles`]).
    pub fn record_positions(&mut self, world: &mut World) {
        for rigid in self.bodies_by_id.values() {
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = body.translation();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let cell_map = rounded_anchor_cell_map(pos.x, pos.y, cos_a, sin_a, &rigid.anchors);

            let positions: HashSet<(i32, i32)> = cell_map.keys().copied().collect();

            for (x, y) in &positions {
                let wp = Vec2i::new(*x, *y);
                if world.get_rigid_id(wp) != Some(rigid.id) {
                    continue;
                }
                let logical = cell_map[&(*x, *y)];
                let mut cell = logical;
                let props = world.material_props(cell.material());
                if props.phase() == Phase::Solid {
                    cell.or_flags(cell_flags::RIGID_PIXEL);
                }
                world.set_cell(wp, cell);
                world.set_rigid_id(wp, rigid.id);
            }
        }
    }

    /// Morphological closing fill tiles for rendering only (not simulated / not in the grid).
    /// For each dynamic body, fills 1-cell gaps from rotation rounding; skips occupied world cells.
    pub fn collect_visual_morph_overlay_tiles(
        &self,
        world: &World,
        rect: RectI,
        out: &mut Vec<(u32, Vec2i, Cell)>,
    ) {
        let mut body_ids: Vec<u32> = self.bodies_by_id.keys().copied().collect();
        body_ids.sort_unstable();
        for id in body_ids {
            let Some(rigid) = self.bodies_by_id.get(&id) else {
                continue;
            };
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = body.translation();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();
            for ((kx, ky), mut cell) in
                compute_morph_fill_cells(&rigid.anchors, pos.x, pos.y, cos_a, sin_a)
            {
                let p = Vec2i::new(kx, ky);
                if !rect.contains(p) {
                    continue;
                }
                if world.get_cell(p).material() != material::EMPTY {
                    continue;
                }
                cell.clear_flag_bits(cell_flags::RIGID_PIXEL);
                out.push((id, p, cell));
            }
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

    /// Drop anchors whose rounded world position matches a painted cell. Call after user paint
    /// (`World::set_cell` clears `rigid_id`) so dynamic colliders match the grid. Does not run for
    /// terrain collision — see [`Self::sync_pixels_to_physics`] inert branch.
    pub fn carve_dynamic_bodies_at_world_cells(
        &mut self,
        world: &mut World,
        hits: &[(Vec2i, u32)],
    ) {
        if hits.is_empty() {
            return;
        }
        let mut by_body: HashMap<u32, Vec<Vec2i>> = HashMap::new();
        for &(wp, id) in hits {
            by_body.entry(id).or_default().push(wp);
        }
        let mut empty: Vec<u32> = Vec::new();
        for (body_id, wps) in by_body {
            let Some(rigid) = self.bodies_by_id.get_mut(&body_id) else {
                continue;
            };
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = *body.translation();
            let angle = body.rotation().angle();
            let cos_a = angle.cos();
            let sin_a = angle.sin();
            let mut locals: HashSet<Vec2i> = HashSet::new();
            for wp in wps {
                for a in &rigid.anchors {
                    if rotated_world_pos(pos.x, pos.y, cos_a, sin_a, a.local) == wp {
                        locals.insert(a.local);
                        break;
                    }
                }
            }
            if locals.is_empty() {
                continue;
            }
            rigid.anchors.retain(|a| !locals.contains(&a.local));
            rigid.needs_collider_refresh = true;
            if rigid.anchors.is_empty() {
                empty.push(body_id);
            }
        }
        for id in empty {
            self.remove_body_physics(world, id);
        }
    }

    /// After the physics step, diff old vs new positions for each body.
    /// Clear cells the body is leaving; place pixels where it is entering.
    pub fn sync_pixels_to_physics(&mut self, world: &mut World) {
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
                let before_mat = anchor.cell.material();
                if wc.has_flag(cell_flags::RIGID_PIXEL) {
                    let mut c = wc;
                    c.clear_flag_bits(cell_flags::RIGID_PIXEL);
                    if c.material() != before_mat {
                        material_changed = true;
                    }
                    anchor.cell = c;
                } else {
                    anchor.cell = wc;
                    if anchor.cell.material() != before_mat {
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

            let new_cells = rounded_anchor_cell_map(pos.x, pos.y, cos_a, sin_a, &rigid.anchors);
            let new_keys: HashSet<(i32, i32)> = new_cells.keys().copied().collect();

            // Clear every grid cell still tagged with this body that is not in the authoritative
            // anchor footprint (orphans with matching `rigid_id`, e.g. legacy morph-fill in the grid).
            let stale: Vec<Vec2i> = world
                .rigid_ids_iter_for_body(rigid.id)
                .filter(|p| !new_keys.contains(&(p.x, p.y)))
                .collect();
            for p in stale {
                world.set_cell(p, Cell::new());
            }

            // Place/update cells the body now occupies
            let search_radius = (rigid.anchors.len() as f32).sqrt().ceil() as i32 + 4;
            let mut displaced_count: u32 = 0;

            for (&key, &cell) in &new_cells {
                let wp = Vec2i::new(key.0, key.1);

                if world.get_rigid_id(wp) == Some(rigid.id) {
                    world.set_cell(wp, cell);
                    world.set_rigid_id(wp, rigid.id);
                    continue;
                }

                let occupant = world.get_cell(wp);

                if occupant.material() == material::EMPTY {
                    world.set_cell(wp, cell);
                    world.set_rigid_id(wp, rigid.id);
                    continue;
                }

                let occ_rid = world.get_rigid_id(wp);

                if occ_rid.is_some_and(|oid| oid != rigid.id) {
                    displaced_count += 1;
                    if let Some(dest) = find_empty_above(world, wp, &new_keys, search_radius) {
                        world.set_cell(dest, occupant);
                        if let Some(rid) = occ_rid {
                            world.set_rigid_id(dest, rid);
                        }
                        world.set_cell(wp, cell);
                        world.set_rigid_id(wp, rigid.id);
                    } else {
                        world.set_cell(wp, cell);
                        world.set_rigid_id(wp, rigid.id);
                    }
                    continue;
                }

                if world.material_props(occupant.material()).inert() {
                    // Do not carve here: terrain and user-painted inert solids look the same on the
                    // grid. User paint overwrite is handled in `carve_dynamic_bodies_at_world_cells`
                    // (see `Simulation` paint APIs).
                    continue;
                }

                displaced_count += 1;
                if let Some(dest) = find_empty_above(world, wp, &new_keys, search_radius) {
                    world.set_cell(dest, occupant);
                    if let Some(rid) = occ_rid {
                        world.set_rigid_id(dest, rid);
                    }
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
                    (
                        body.translation().x,
                        body.translation().y,
                        body.rotation().angle(),
                    )
                } else {
                    continue;
                }
            } else {
                let p = collider.position();
                (p.translation.x, p.translation.y, p.rotation.angle())
            };
            let cos_a = angle.cos();
            let sin_a = angle.sin();

            let col_rel = collider
                .position_wrt_parent()
                .map(|iso| (iso.translation.x, iso.translation.y, iso.rotation.angle()))
                .unwrap_or((0.0, 0.0, 0.0));

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
            let center = Vec2i::new(
                body.translation().x.round() as i32,
                body.translation().y.round() as i32,
            );
            if !bounds.contains(center) {
                ids_to_remove.push(*id);
            }
        }

        for id in ids_to_remove {
            if let Some(rigid) = self.bodies_by_id.remove(&id) {
                world.purge_rigid_body_ownership(id);
                self.bodies.remove(
                    rigid.handle,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    true,
                );
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
            .filter(|c| {
                c.x < bounds_min_cx
                    || c.x > bounds_max_cx
                    || c.y < bounds_min_cy
                    || c.y > bounds_max_cy
            })
            .collect();
        for coord in stale {
            self.remove_static_chunk(coord);
        }
    }

    // --- Static terrain colliders (per-chunk) ---

    pub fn rebuild_static_colliders(&mut self, world: &World, dirty_chunks: &[ChunkCoord]) {
        for &coord in dirty_chunks {
            let bounds = world.bounds_for_chunk(coord);
            let info = analyze_inert_cells(world, &bounds);

            if self.static_chunk_hashes.get(&coord) == Some(&info.hash) {
                continue;
            }

            self.remove_static_chunk(coord);
            self.static_chunk_hashes.insert(coord, info.hash);

            if info.inert_count == 0 {
                continue;
            }

            let positions = collect_static_positions(world, &bounds);
            let handles = build_static_triangle_colliders(
                &mut self.colliders,
                &mut self.bodies,
                self.static_body_handle,
                &positions,
            );

            if !handles.is_empty() {
                self.static_chunk_colliders.insert(coord, handles);
            }
        }
    }

    fn remove_static_chunk(&mut self, coord: ChunkCoord) {
        if let Some(handles) = self.static_chunk_colliders.remove(&coord) {
            for h in handles {
                self.colliders
                    .remove(h, &mut self.islands, &mut self.bodies, true);
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

    /// Next [`Self::rebuild_dirty_dynamic_colliders`] rebuilds colliders for every dynamic pixel body.
    pub fn mark_all_dynamic_colliders_stale(&mut self) {
        for rb in self.bodies_by_id.values_mut() {
            rb.needs_collider_refresh = true;
        }
    }

    pub fn rebuild_dirty_dynamic_colliders(&mut self) {
        let ids: Vec<u32> = self
            .bodies_by_id
            .iter()
            .filter_map(|(&id, rb)| {
                if rb.needs_collider_refresh {
                    Some(id)
                } else {
                    None
                }
            })
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
                self.colliders
                    .remove(ch, &mut self.islands, &mut self.bodies, true);
            }

            let local_positions: Vec<(i32, i32)> = rigid
                .anchors
                .iter()
                .map(|a| (a.local.x, a.local.y))
                .collect();
            build_and_insert_dynamic_pixel_colliders(
                &mut self.colliders,
                &mut self.bodies,
                handle,
                &local_positions,
            );
        }
    }

    pub fn check_splits(&mut self, world: &mut World) {
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

            let stress = (linvel.x * linvel.x + linvel.y * linvel.y).sqrt() + angvel.abs() * 5.0;

            let mut surviving = Vec::new();
            let mut has_env_missing = false;

            for anchor in &rigid.anchors {
                let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, anchor.local);
                let rid = world.get_rigid_id(wp);
                let mat = world.get_cell(wp).material();

                if rid == Some(body_id) {
                    surviving.push(anchor.clone());
                    continue;
                }

                if mat == material::EMPTY {
                    // Sim removed this pixel (acid, fire, etc.) — always drop anchor.
                    has_env_missing = true;
                    continue;
                }

                if rid.is_some() {
                    // Another rigid owns this cell — always drop our anchor.
                    has_env_missing = true;
                    continue;
                }

                let world_inert = world.material_props(mat).inert();
                if world_inert {
                    let integrity = world
                        .material_props(anchor.cell.material())
                        .structure_integrity
                        .max(0.05);
                    let break_at = STRUCTURAL_STRESS_SCALE * integrity;
                    if stress >= break_at {
                        has_env_missing = true;
                    } else {
                        surviving.push(anchor.clone());
                    }
                } else {
                    // Fluids, granular, etc. — paint and displacement always carve.
                    has_env_missing = true;
                }
            }

            let should_split = has_env_missing;
            if should_split {
                splits_to_process.push((body_id, surviving, pos, linvel, angvel, angle));
            }
        }

        for (body_id, surviving, pos, linvel, angvel, angle) in splits_to_process {
            if surviving.is_empty() {
                self.remove_body_physics(world, body_id);
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
                    world.set_cell(wp, Cell::new());
                }
            }
            self.remove_body_physics(world, body_id);

            for component in components {
                let anchors_wp: Vec<(Vec2i, Cell)> = component
                    .iter()
                    .map(|a| {
                        let wp = rotated_world_pos(pos.x, pos.y, cos_a, sin_a, a.local);
                        (wp, a.cell)
                    })
                    .collect();
                let new_id = self.spawn_from_anchors(world, &anchors_wp);
                if let Some(new_rigid) = self.bodies_by_id.get(&new_id) {
                    if let Some(new_body) = self.bodies.get_mut(new_rigid.handle) {
                        new_body.set_linvel(linvel, true);
                        new_body.set_angvel(angvel, true);
                    }
                }
            }
        }
    }

    fn remove_body_physics(&mut self, world: &mut World, body_id: u32) {
        world.purge_rigid_body_ownership(body_id);
        if let Some(rigid) = self.bodies_by_id.remove(&body_id) {
            self.bodies.remove(
                rigid.handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
        }
    }
}

#[cfg(test)]
impl RigidBridge {
    pub(crate) fn dynamic_body_count(&self) -> usize {
        self.bodies_by_id.len()
    }

    pub(crate) fn all_body_ids(&self) -> std::collections::HashSet<u32> {
        self.bodies_by_id.keys().copied().collect()
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

fn build_and_insert_dynamic_pixel_colliders(
    colliders: &mut ColliderSet,
    bodies: &mut RigidBodySet,
    handle: RigidBodyHandle,
    local_positions: &[(i32, i32)],
) {
    let mut inserted = false;
    let contours = marching_squares_contours(local_positions);
    for contour in &contours {
        if contour.len() < 3 {
            continue;
        }
        let eps = simplify_epsilon_for_contour(contour.len());
        let simplified = simplify_douglas_peucker(contour, eps);
        if polygon_area(&simplified) <= 0.0 {
            continue;
        }
        for tri in triangulate_ear_clip(&simplified) {
            let shape = SharedShape::triangle(tri[0], tri[1], tri[2]);
            let collider = ColliderBuilder::new(shape)
                .density(5.0)
                .friction(0.6)
                .restitution(0.2)
                .build();
            colliders.insert_with_parent(collider, handle, bodies);
            inserted = true;
        }
    }

    if inserted || local_positions.is_empty() {
        return;
    }

    let min_lx = local_positions.iter().map(|p| p.0).min().unwrap();
    let max_lx = local_positions.iter().map(|p| p.0).max().unwrap();
    let min_ly = local_positions.iter().map(|p| p.1).min().unwrap();
    let max_ly = local_positions.iter().map(|p| p.1).max().unwrap();
    let cx = (min_lx + max_lx) as f32 * 0.5;
    let cy = (min_ly + max_ly) as f32 * 0.5;
    let hx = ((max_lx - min_lx + 1) as f32 * 0.5).max(0.25);
    let hy = ((max_ly - min_ly + 1) as f32 * 0.5).max(0.25);
    let collider = ColliderBuilder::cuboid(hx, hy)
        .translation(vector![cx, cy])
        .density(5.0)
        .friction(0.6)
        .restitution(0.2)
        .build();
    colliders.insert_with_parent(collider, handle, bodies);
}

/// Marching-squares iso-contour extraction on a binary pixel grid.
/// Returns closed polylines in local float coords (pixel-center space).
/// Outer boundaries wind CCW (positive `polygon_area`); holes wind CW (negative).
fn marching_squares_contours(local_positions: &[(i32, i32)]) -> Vec<Vec<(f32, f32)>> {
    if local_positions.is_empty() {
        return Vec::new();
    }
    let filled: HashSet<(i32, i32)> = local_positions.iter().copied().collect();
    let min_x = local_positions.iter().map(|p| p.0).min().unwrap() - 1;
    let min_y = local_positions.iter().map(|p| p.1).min().unwrap() - 1;
    let max_x = local_positions.iter().map(|p| p.0).max().unwrap() + 1;
    let max_y = local_positions.iter().map(|p| p.1).max().unwrap() + 1;
    let w = (max_x - min_x + 1) as usize;

    let sample = |gx: i32, gy: i32| -> bool { filled.contains(&(gx, gy)) };

    // Each marching-squares cell sits between four sample points at grid positions
    // (cx, cy), (cx+1, cy), (cx, cy+1), (cx+1, cy+1).
    // Edges are keyed by (cell_index, edge_side) for cycle tracing.
    // Edge midpoints sit on the boundary between filled/empty samples.
    // cell_index = (cy - min_y) * w + (cx - min_x)

    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    struct EdgeKey {
        cell: usize,
        side: u8, // 0=top, 1=right, 2=bottom, 3=left
    }

    fn edge_midpoint(cx: i32, cy: i32, side: u8) -> (f32, f32) {
        let x = cx as f32;
        let y = cy as f32;
        match side {
            0 => (x + 0.5, y),       // top
            1 => (x + 1.0, y + 0.5), // right
            2 => (x + 0.5, y + 1.0), // bottom
            3 => (x, y + 0.5),       // left
            _ => unreachable!(),
        }
    }

    // Shift from pixel-grid to local coords: sample (gx,gy) corresponds to pixel center (gx,gy),
    // but marching cell (cx,cy) has corners at samples cx..cx+1, cy..cy+1. Edge midpoints are in
    // the same space as pixel centers, offset by (-0.5, -0.5) from corner-aligned grids — but since
    // our samples ARE pixel centers, the midpoints are already correct.

    let mut edge_next: HashMap<EdgeKey, EdgeKey> = HashMap::new();

    for cy in min_y..max_y {
        for cx in min_x..max_x {
            let tl = sample(cx, cy) as u8;
            let tr = sample(cx + 1, cy) as u8;
            let br = sample(cx + 1, cy + 1) as u8;
            let bl = sample(cx, cy + 1) as u8;
            let case = (tl << 3) | (tr << 2) | (br << 1) | bl;
            if case == 0 || case == 15 {
                continue;
            }
            let ci = ((cy - min_y) as usize) * w + ((cx - min_x) as usize);

            // For each case, list (entry_side, exit_side) pairs. The edge with the filled corner
            // on its left is the "entry", the edge with filled on its right is "exit".
            // (entry_side, exit_side) — winding keeps filled region to the RIGHT
            // of travel in screen coords (Y-down), producing CCW / positive-area
            // outer boundaries and CW / negative-area holes.
            let pairs: &[(u8, u8)] = match case {
                1 => &[(3, 2)],
                2 => &[(2, 1)],
                3 => &[(3, 1)],
                4 => &[(1, 0)],
                5 => &[(3, 0), (1, 2)], // saddle: TL+BR
                6 => &[(2, 0)],
                7 => &[(3, 0)],
                8 => &[(0, 3)],
                9 => &[(0, 2)],
                10 => &[(0, 1), (2, 3)], // saddle: TR+BL
                11 => &[(0, 1)],
                12 => &[(1, 3)],
                13 => &[(1, 2)],
                14 => &[(2, 3)],
                _ => continue,
            };

            for &(entry_side, exit_side) in pairs {
                let from = EdgeKey {
                    cell: ci,
                    side: entry_side,
                };
                // The neighbor sharing this edge
                let to = match exit_side {
                    0 => {
                        // top edge → neighbor above enters from bottom
                        let nci = (((cy - 1) - min_y) as usize) * w + ((cx - min_x) as usize);
                        EdgeKey { cell: nci, side: 2 }
                    }
                    1 => {
                        // right edge → neighbor right enters from left
                        let nci = ((cy - min_y) as usize) * w + (((cx + 1) - min_x) as usize);
                        EdgeKey { cell: nci, side: 3 }
                    }
                    2 => {
                        // bottom edge → neighbor below enters from top
                        let nci = (((cy + 1) - min_y) as usize) * w + ((cx - min_x) as usize);
                        EdgeKey { cell: nci, side: 0 }
                    }
                    3 => {
                        // left edge → neighbor left enters from right
                        let nci = ((cy - min_y) as usize) * w + (((cx - 1) - min_x) as usize);
                        EdgeKey { cell: nci, side: 1 }
                    }
                    _ => unreachable!(),
                };
                edge_next.insert(from, to);
            }
        }
    }

    // Trace closed loops
    let mut visited: HashSet<EdgeKey> = HashSet::new();
    let mut loops = Vec::new();
    let all_keys: Vec<EdgeKey> = edge_next.keys().copied().collect();
    for start in all_keys {
        if visited.contains(&start) {
            continue;
        }
        let mut poly = Vec::new();
        let mut cur = start;
        loop {
            if visited.contains(&cur) && cur != start {
                break;
            }
            visited.insert(cur);
            let row = cur.cell / w;
            let col = cur.cell % w;
            let cx = col as i32 + min_x;
            let cy = row as i32 + min_y;
            poly.push(edge_midpoint(cx, cy, cur.side));
            if let Some(&next) = edge_next.get(&cur) {
                cur = next;
                if cur == start {
                    break;
                }
            } else {
                break;
            }
        }
        if poly.len() >= 3 {
            loops.push(poly);
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

fn simplify_epsilon_for_contour(contour_len: usize) -> f32 {
    let len = contour_len as f32;
    (len / 1600.0).max(0.5)
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
    ChunkInertInfo {
        hash: hasher.finish(),
        inert_count,
    }
}

fn rotated_world_pos(tx: f32, ty: f32, cos_a: f32, sin_a: f32, local: Vec2i) -> Vec2i {
    let lx = local.x as f32;
    let ly = local.y as f32;
    Vec2i::new(
        (tx + lx * cos_a - ly * sin_a).round() as i32,
        (ty + lx * sin_a + ly * cos_a).round() as i32,
    )
}

/// 8-neighborhood dilation (Chebyshev radius 1).
fn dilate_8(keys: &HashSet<(i32, i32)>) -> HashSet<(i32, i32)> {
    let mut out = HashSet::with_capacity(keys.len() * 2);
    for &(x, y) in keys {
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                out.insert((x + dx, y + dy));
            }
        }
    }
    out
}

/// 8-neighborhood erosion: keep cells whose full 3×3 neighborhood lies in `fg`.
fn erode_8(fg: &HashSet<(i32, i32)>) -> HashSet<(i32, i32)> {
    let mut out = HashSet::new();
    for &(x, y) in fg {
        let mut ok = true;
        'nei: for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if !fg.contains(&(x + dx, y + dy)) {
                    ok = false;
                    break 'nei;
                }
            }
        }
        if ok {
            out.insert((x, y));
        }
    }
    out
}

/// Closing = erode(dilate(B)). Fills 1-cell (and some thin) interior gaps from rotation rounding.
fn morph_close_8(keys: &HashSet<(i32, i32)>) -> HashSet<(i32, i32)> {
    if keys.is_empty() {
        return HashSet::new();
    }
    let d = dilate_8(keys);
    erode_8(&d)
}

/// Pixels in `closed` that are 4-connected to any seed through cells in `closed` only.
/// Drops morph-close islands that only touch the real anchor footprint diagonally or float outside it,
/// which otherwise become gray `fill_cell` artifacts in world space.
fn closed_cells_reachable_from_seeds(
    seeds: &HashSet<(i32, i32)>,
    closed: &HashSet<(i32, i32)>,
) -> HashSet<(i32, i32)> {
    let mut out = HashSet::new();
    let mut q = VecDeque::new();
    for &p in seeds {
        if closed.contains(&p) {
            out.insert(p);
            q.push_back(p);
        }
    }
    const D4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    while let Some((x, y)) = q.pop_front() {
        for (dx, dy) in D4 {
            let n = (x + dx, y + dy);
            if closed.contains(&n) && out.insert(n) {
                q.push_back(n);
            }
        }
    }
    out
}

/// Rounded anchor positions → cell map (colliding anchors keep first cell).
fn rounded_anchor_cell_map(
    tx: f32,
    ty: f32,
    cos_a: f32,
    sin_a: f32,
    anchors: &[PixelAnchor],
) -> HashMap<(i32, i32), Cell> {
    let mut map = HashMap::with_capacity(anchors.len());
    for anchor in anchors {
        let wp = rotated_world_pos(tx, ty, cos_a, sin_a, anchor.local);
        map.entry((wp.x, wp.y)).or_insert(anchor.cell);
    }
    map
}

/// Pixels added by morphological closing (not anchor-rounded). Used for render overlays only.
fn compute_morph_fill_cells(
    anchors: &[PixelAnchor],
    tx: f32,
    ty: f32,
    cos_a: f32,
    sin_a: f32,
) -> Vec<((i32, i32), Cell)> {
    let map = rounded_anchor_cell_map(tx, ty, cos_a, sin_a, anchors);
    let anchor_keys: HashSet<(i32, i32)> = map.keys().copied().collect();
    if anchor_keys.is_empty() {
        return Vec::new();
    }
    let closed_raw = morph_close_8(&anchor_keys);
    let closed = closed_cells_reachable_from_seeds(&anchor_keys, &closed_raw);
    let new_keys: Vec<(i32, i32)> = closed
        .into_iter()
        .filter(|k| !map.contains_key(k))
        .collect();
    if new_keys.is_empty() {
        return Vec::new();
    }
    let anchor_world: Vec<((i32, i32), Cell)> = anchors
        .iter()
        .map(|a| {
            let wp = rotated_world_pos(tx, ty, cos_a, sin_a, a.local);
            ((wp.x, wp.y), a.cell)
        })
        .collect();
    let fallback = anchors.first().map(|a| a.cell).unwrap_or(Cell::new());
    let mut out = Vec::with_capacity(new_keys.len());
    for k in new_keys {
        let cell = anchor_world
            .iter()
            .min_by_key(|&&(wp, _)| (wp.0 - k.0).abs() + (wp.1 - k.1).abs())
            .map(|&(_, c)| c)
            .unwrap_or(fallback);
        out.push((k, cell));
    }
    out
}

fn collect_static_positions(world: &World, bounds: &RectI) -> Vec<(i32, i32)> {
    let mut positions = Vec::new();
    for y in bounds.min.y..=bounds.max.y {
        for x in bounds.min.x..=bounds.max.x {
            if is_static_terrain(world, Vec2i::new(x, y)) {
                positions.push((x, y));
            }
        }
    }
    positions
}

fn build_static_triangle_colliders(
    colliders: &mut ColliderSet,
    bodies: &mut RigidBodySet,
    handle: RigidBodyHandle,
    positions: &[(i32, i32)],
) -> Vec<ColliderHandle> {
    let mut handles = Vec::new();
    let contours = marching_squares_contours(positions);
    for contour in &contours {
        if contour.len() < 3 {
            continue;
        }
        let eps = simplify_epsilon_for_contour(contour.len());
        let simplified = simplify_douglas_peucker(contour, eps);
        if polygon_area(&simplified) <= 0.0 {
            continue;
        }
        for tri in triangulate_ear_clip(&simplified) {
            let shape = SharedShape::triangle(tri[0], tri[1], tri[2]);
            let collider = ColliderBuilder::new(shape)
                .friction(0.5)
                .restitution(0.0)
                .build();
            let h = colliders.insert_with_parent(collider, handle, bodies);
            handles.push(h);
        }
    }

    if !handles.is_empty() || positions.is_empty() {
        return handles;
    }

    let min_x = positions.iter().map(|p| p.0).min().unwrap();
    let max_x = positions.iter().map(|p| p.0).max().unwrap();
    let min_y = positions.iter().map(|p| p.1).min().unwrap();
    let max_y = positions.iter().map(|p| p.1).max().unwrap();
    let cx = (min_x + max_x) as f32 * 0.5;
    let cy = (min_y + max_y) as f32 * 0.5;
    let hx = ((max_x - min_x + 1) as f32 * 0.5).max(0.25);
    let hy = ((max_y - min_y + 1) as f32 * 0.5).max(0.25);
    let collider = ColliderBuilder::cuboid(hx, hy)
        .translation(vector![cx, cy])
        .friction(0.5)
        .restitution(0.0)
        .build();
    let h = colliders.insert_with_parent(collider, handle, bodies);
    handles.push(h);
    handles
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
                if world.get_cell(candidate).material() == material::EMPTY {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod morph_tests {
    use super::*;

    #[test]
    fn marching_squares_two_disconnected_islands() {
        let p = [
            (0, 0),
            (1, 0),
            (0, 1),
            (1, 1),
            (5, 0),
            (6, 0),
            (5, 1),
            (6, 1),
        ];
        let loops = marching_squares_contours(&p);
        assert_eq!(
            loops.len(),
            2,
            "two separated groups should each get a boundary loop"
        );
        for l in &loops {
            assert!(
                polygon_area(l) > 0.0,
                "outer boundary should be CCW (positive area)"
            );
        }
    }

    #[test]
    fn marching_squares_single_pixel() {
        let loops = marching_squares_contours(&[(0, 0)]);
        assert_eq!(loops.len(), 1);
        assert!(polygon_area(&loops[0]) > 0.0);
    }

    #[test]
    fn marching_squares_hole_produces_two_loops() {
        // 5x5 ring with hollow center (3x3 hole)
        let mut pixels = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                if x >= 1 && x <= 3 && y >= 1 && y <= 3 {
                    continue; // hole
                }
                pixels.push((x, y));
            }
        }
        let loops = marching_squares_contours(&pixels);
        assert_eq!(loops.len(), 2, "ring should produce outer + hole loop");
        let mut areas: Vec<f32> = loops.iter().map(|l| polygon_area(l)).collect();
        areas.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert!(areas[0] > 0.0, "outer boundary positive");
        assert!(areas[1] < 0.0, "hole boundary negative");
    }

    #[test]
    fn morph_close_8_fills_center_of_3x3_ring() {
        let mut keys = HashSet::new();
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                keys.insert((dx, dy));
            }
        }
        let closed_raw = morph_close_8(&keys);
        let closed = closed_cells_reachable_from_seeds(&keys, &closed_raw);
        assert!(closed.contains(&(0, 0)));
        assert_eq!(closed.len(), 9);
    }

    #[test]
    fn closed_reachable_drops_cells_not_four_connected_to_seeds() {
        let seeds: HashSet<_> = [(0, 0), (1, 0)].into_iter().collect();
        let mut closed = morph_close_8(&seeds);
        closed.insert((20, 20));
        let kept = closed_cells_reachable_from_seeds(&seeds, &closed);
        assert!(!kept.contains(&(20, 20)));
        assert!(kept.contains(&(0, 0)));
    }
}
