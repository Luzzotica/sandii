use std::collections::{HashMap, HashSet};

use nalgebra::{point, vector};
use rapier2d::prelude::*;

use crate::sim::{Particle, ParticleSim};
use crate::world::{material, Cell, MaterialId, Vec2i, World};
use crate::SimulationEvent;

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
    extracted_world_positions: HashMap<u32, Vec<(Vec2i, Cell)>>,
    next_id: u32,
}

impl RigidBridge {
    pub fn new() -> Self {
        Self {
            gravity: vector![0.0, 9.81],
            pipeline: PhysicsPipeline::new(),
            integration_parameters: IntegrationParameters::default(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseMultiSap::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            bodies_by_id: HashMap::new(),
            extracted_world_positions: HashMap::new(),
            next_id: 1,
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

        let mut anchors = Vec::new();
        for &p in positions {
            let mut cell = world.get_cell(p);
            cell.material = material_id;
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
                let collider = ColliderBuilder::new(shape).density(1.0).build();
                self.colliders
                    .insert_with_parent(collider, handle, &mut self.bodies);
            }
        }

        self.bodies_by_id
            .insert(id, PixelRigidBody { id, anchors, handle });
        id
    }

    pub fn extract_from_world(&mut self, world: &mut World) {
        self.extracted_world_positions.clear();
        for rigid in self.bodies_by_id.values() {
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = body.translation();
            let mut extracted = Vec::new();
            for anchor in &rigid.anchors {
                let wp = Vec2i::new(pos.x.round() as i32 + anchor.local.x, pos.y.round() as i32 + anchor.local.y);
                extracted.push((wp, anchor.cell));
                world.set_cell(wp, Cell::default());
                world.clear_rigid_id(wp);
            }
            self.extracted_world_positions.insert(rigid.id, extracted);
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

    pub fn reinsert_into_world(
        &mut self,
        world: &mut World,
        particles: &mut ParticleSim,
        events: &mut Vec<SimulationEvent>,
    ) {
        for rigid in self.bodies_by_id.values() {
            let Some(body) = self.bodies.get(rigid.handle) else {
                continue;
            };
            let pos = body.translation();
            for anchor in &rigid.anchors {
                let wp = Vec2i::new(pos.x.round() as i32 + anchor.local.x, pos.y.round() as i32 + anchor.local.y);
                let occupant = world.get_cell(wp);
                if occupant.material != material::EMPTY && world.get_rigid_id(wp) != Some(rigid.id) {
                    particles.spawn(Particle {
                        pos: (wp.x as f32, wp.y as f32),
                        vel: (0.0, -20.0),
                        cell: occupant,
                        lifetime: 0.75,
                    });
                }
                world.set_cell(wp, anchor.cell);
                world.set_rigid_id(wp, rigid.id);
                events.push(SimulationEvent::PixelEjectedToParticle { at: wp });
            }
        }
    }

    pub fn cull_outside(&mut self, world: &mut World, bounds: crate::world::RectI) {
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
                self.extracted_world_positions.remove(&id);
            }
        }
    }

    fn clear_pixels_for_body(&self, world: &mut World, rigid: &PixelRigidBody) {
        let Some(body) = self.bodies.get(rigid.handle) else {
            return;
        };
        let pos = body.translation();
        for anchor in &rigid.anchors {
            let wp = Vec2i::new(pos.x.round() as i32 + anchor.local.x, pos.y.round() as i32 + anchor.local.y);
            if world.get_rigid_id(wp) == Some(rigid.id) {
                world.set_cell(wp, Cell::default());
                world.clear_rigid_id(wp);
            }
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

            let mut surviving = Vec::new();
            let mut has_missing = false;

            for anchor in &rigid.anchors {
                let wp = Vec2i::new(
                    pos.x.round() as i32 + anchor.local.x,
                    pos.y.round() as i32 + anchor.local.y,
                );
                if world.get_rigid_id(wp) == Some(body_id) {
                    surviving.push(anchor.clone());
                } else {
                    has_missing = true;
                }
            }

            if has_missing {
                splits_to_process.push((body_id, surviving, pos, linvel, angvel));
            }
        }

        for (body_id, surviving, pos, linvel, angvel) in splits_to_process {
            if surviving.is_empty() {
                self.remove_body_physics(body_id);
                continue;
            }

            let components = find_connected_components(&surviving);
            if components.len() <= 1 {
                if let Some(rigid) = self.bodies_by_id.get_mut(&body_id) {
                    rigid.anchors = surviving;
                }
                continue;
            }

            for anchor in &surviving {
                let wp = Vec2i::new(
                    pos.x.round() as i32 + anchor.local.x,
                    pos.y.round() as i32 + anchor.local.y,
                );
                if world.get_rigid_id(wp) == Some(body_id) {
                    world.set_cell(wp, Cell::default());
                    world.clear_rigid_id(wp);
                }
            }
            self.remove_body_physics(body_id);

            for component in components {
                let world_positions: Vec<Vec2i> = component
                    .iter()
                    .map(|a| {
                        Vec2i::new(
                            pos.x.round() as i32 + a.local.x,
                            pos.y.round() as i32 + a.local.y,
                        )
                    })
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
            self.extracted_world_positions.remove(&body_id);
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
