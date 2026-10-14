//! Pixel → collider boundary pipeline: marching squares → Douglas–Peucker → ear-clip triangles.
//!
//! Public for unit tests. Dynamic and static rigid colliders both go through here.

use nalgebra::point;
use rapier2d::prelude::{Point, Real};
use std::collections::{HashMap, HashSet};

/// Build simplified outer/hole polygons from filled pixel local positions.
/// Outer loops wind CCW (positive area); holes CW (negative).
pub fn collider_polygons_from_pixels(local_positions: &[(i32, i32)]) -> Vec<Vec<(f32, f32)>> {
    let contours = marching_squares_contours(local_positions);
    let mut out = Vec::with_capacity(contours.len());
    for contour in contours {
        if contour.len() < 3 {
            continue;
        }
        let simplified = simplify_contour(&contour);
        if simplified.len() < 3 {
            continue;
        }
        // Keep degenerate-area polygons out of triangulation.
        if polygon_area(&simplified).abs() < 1e-4 {
            continue;
        }
        out.push(simplified);
    }
    out
}

/// Marching-squares iso-contour extraction on a binary pixel grid.
/// Returns closed polylines in local float coords (edge midpoints between pixel centers).
pub fn marching_squares_contours(local_positions: &[(i32, i32)]) -> Vec<Vec<(f32, f32)>> {
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

    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    struct EdgeKey {
        cell: usize,
        side: u8, // 0=top, 1=right, 2=bottom, 3=left
    }

    fn edge_midpoint(cx: i32, cy: i32, side: u8) -> (f32, f32) {
        let x = cx as f32;
        let y = cy as f32;
        match side {
            0 => (x + 0.5, y),
            1 => (x + 1.0, y + 0.5),
            2 => (x + 0.5, y + 1.0),
            3 => (x, y + 0.5),
            _ => unreachable!(),
        }
    }

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

            // Filled region stays to the RIGHT of travel (Y-down) → CCW outer / CW holes.
            let pairs: &[(u8, u8)] = match case {
                1 => &[(3, 2)],
                2 => &[(2, 1)],
                3 => &[(3, 1)],
                4 => &[(1, 0)],
                5 => &[(3, 0), (1, 2)],
                6 => &[(2, 0)],
                7 => &[(3, 0)],
                8 => &[(0, 3)],
                9 => &[(0, 2)],
                10 => &[(0, 1), (2, 3)],
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
                let to = match exit_side {
                    0 => {
                        let nci = (((cy - 1) - min_y) as usize) * w + ((cx - min_x) as usize);
                        EdgeKey { cell: nci, side: 2 }
                    }
                    1 => {
                        let nci = ((cy - min_y) as usize) * w + (((cx + 1) - min_x) as usize);
                        EdgeKey { cell: nci, side: 3 }
                    }
                    2 => {
                        let nci = (((cy + 1) - min_y) as usize) * w + ((cx - min_x) as usize);
                        EdgeKey { cell: nci, side: 0 }
                    }
                    3 => {
                        let nci = ((cy - min_y) as usize) * w + (((cx - 1) - min_x) as usize);
                        EdgeKey { cell: nci, side: 1 }
                    }
                    _ => unreachable!(),
                };
                edge_next.insert(from, to);
            }
        }
    }

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
            // Drop exact consecutive duplicates from tracing.
            poly = dedupe_consecutive(&poly);
            if poly.len() >= 3 {
                loops.push(poly);
            }
        }
    }
    loops
}

/// Adaptive closed-contour simplification (collinear cull + closed Douglas–Peucker).
pub fn simplify_contour(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let cleaned = remove_collinear(points, 1e-3);
    if cleaned.len() < 3 {
        return cleaned;
    }
    let eps = simplify_epsilon_for_contour(cleaned.len());
    let simplified = simplify_douglas_peucker_closed(&cleaned, eps);
    let simplified = remove_collinear(&simplified, 1e-3);
    if simplified.len() < 3 {
        // Never return an unusable ring — fall back to cleaned contour.
        cleaned
    } else {
        simplified
    }
}

/// Epsilon scales gently with contour size but stays pixel-aware.
/// Small shapes stay faithful; large ones still reduce triangle spam.
///
/// Marching-squares edge midpoints put axis-aligned rects on an 8-vertex “chamfered”
/// ring (half-pixel corner cuts). Epsilon ≥ ~0.51 collapses those to 4 corners.
pub fn simplify_epsilon_for_contour(contour_len: usize) -> f32 {
    let len = contour_len as f32;
    (0.52 + len / 900.0).clamp(0.52, 1.35)
}

/// Closed-polyline Douglas–Peucker: pick diameter endpoints, simplify both arcs, rejoin.
pub fn simplify_douglas_peucker_closed(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
    let n = points.len();
    if n <= 3 {
        return points.to_vec();
    }

    // Find the pair of vertices with maximum separation (stable split for closed rings).
    let mut i0 = 0usize;
    let mut i1 = 1usize;
    let mut best = -1.0f32;
    for a in 0..n {
        for b in (a + 1)..n {
            let dx = points[a].0 - points[b].0;
            let dy = points[a].1 - points[b].1;
            let d = dx * dx + dy * dy;
            if d > best {
                best = d;
                i0 = a;
                i1 = b;
            }
        }
    }

    // Build two open chains from i0 → i1 and i1 → i0 (wrapping).
    let chain_a = chain_between(points, i0, i1);
    let chain_b = chain_between(points, i1, i0);
    let simp_a = simplify_douglas_peucker_open(&chain_a, epsilon);
    let simp_b = simplify_douglas_peucker_open(&chain_b, epsilon);

    let mut out = simp_a;
    // Skip duplicated junction at i1.
    if simp_b.len() > 1 {
        out.extend_from_slice(&simp_b[1..]);
    }
    // Drop duplicated closure if last ≈ first.
    if out.len() > 1 {
        let first = out[0];
        let last = *out.last().unwrap();
        if (first.0 - last.0).abs() < 1e-5 && (first.1 - last.1).abs() < 1e-5 {
            out.pop();
        }
    }
    out
}

fn chain_between(points: &[(f32, f32)], from: usize, to: usize) -> Vec<(f32, f32)> {
    let n = points.len();
    let mut out = Vec::with_capacity(n);
    let mut i = from;
    loop {
        out.push(points[i]);
        if i == to {
            break;
        }
        i = (i + 1) % n;
    }
    out
}

pub fn simplify_douglas_peucker_open(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
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
        let left = simplify_douglas_peucker_open(&points[..=max_idx], epsilon);
        let right = simplify_douglas_peucker_open(&points[max_idx..], epsilon);
        let mut result = left;
        result.extend_from_slice(&right[1..]);
        result
    } else {
        vec![first, last]
    }
}

pub fn perpendicular_distance(point: (f32, f32), line_start: (f32, f32), line_end: (f32, f32)) -> f32 {
    let dx = line_end.0 - line_start.0;
    let dy = line_end.1 - line_start.1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-10 {
        let px = point.0 - line_start.0;
        let py = point.1 - line_start.1;
        return (px * px + py * py).sqrt();
    }
    let numerator = (dx * (line_start.1 - point.1) - (line_start.0 - point.0) * dy).abs();
    numerator / len_sq.sqrt()
}

pub fn polygon_area(polygon: &[(f32, f32)]) -> f32 {
    let n = polygon.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += polygon[i].0 * polygon[j].1;
        area -= polygon[j].0 * polygon[i].1;
    }
    area * 0.5
}

/// Ear-clip triangulation for a simple polygon (CCW or CW).
pub fn triangulate_ear_clip(polygon: &[(f32, f32)]) -> Vec<[Point<Real>; 3]> {
    if polygon.len() < 3 {
        return Vec::new();
    }

    let mut indices: Vec<usize> = (0..polygon.len()).collect();
    let mut triangles = Vec::new();
    let area_sign = polygon_area(polygon).signum();
    if area_sign == 0.0 {
        return Vec::new();
    }

    let mut guard = 0;
    while indices.len() > 2 {
        guard += 1;
        if guard > polygon.len() * polygon.len() + 8 {
            break;
        }
        let n = indices.len();
        let mut found_ear = false;

        for i in 0..n {
            let prev = indices[(i + n - 1) % n];
            let curr = indices[i];
            let next = indices[(i + 1) % n];

            let a = polygon[prev];
            let b = polygon[curr];
            let c = polygon[next];

            // Ear must match winding (cross product sign).
            let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if cross * area_sign <= 1e-8 {
                continue;
            }

            let mut contains = false;
            for &idx in &indices {
                if idx == prev || idx == curr || idx == next {
                    continue;
                }
                if point_in_triangle(polygon[idx], a, b, c) {
                    contains = true;
                    break;
                }
            }
            if contains {
                continue;
            }

            triangles.push([
                point![a.0, a.1],
                point![b.0, b.1],
                point![c.0, c.1],
            ]);
            indices.remove(i);
            found_ear = true;
            break;
        }

        if !found_ear {
            break;
        }
    }
    triangles
}

fn point_in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let sign = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(has_neg && has_pos)
}

/// Point-in-polygon (ray cast). Boundary counts as inside.
/// Used by boundary unit tests (and available for diagnostics).
#[allow(dead_code)]
pub fn point_in_polygon(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let pi = poly[i];
        let pj = poly[j];
        let intersect = ((pi.1 > p.1) != (pj.1 > p.1))
            && (p.0 < (pj.0 - pi.0) * (p.1 - pi.1) / (pj.1 - pi.1 + f32::EPSILON) + pi.0);
        if intersect {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn remove_collinear(points: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)> {
    let n = points.len();
    if n <= 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = points[(i + n - 1) % n];
        let curr = points[i];
        let next = points[(i + 1) % n];
        let cross = (curr.0 - prev.0) * (next.1 - prev.1) - (curr.1 - prev.1) * (next.0 - prev.0);
        if cross.abs() > eps {
            out.push(curr);
        }
    }
    if out.len() < 3 {
        points.to_vec()
    } else {
        out
    }
}

fn dedupe_consecutive(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(points.len());
    for &p in points {
        if out
            .last()
            .map(|q: &(f32, f32)| (q.0 - p.0).abs() > 1e-6 || (q.1 - p.1).abs() > 1e-6)
            .unwrap_or(true)
        {
            out.push(p);
        }
    }
    if out.len() > 1 {
        let first = out[0];
        let last = *out.last().unwrap();
        if (first.0 - last.0).abs() < 1e-6 && (first.1 - last.1).abs() < 1e-6 {
            out.pop();
        }
    }
    out
}

/// Filled axis-aligned rectangle of pixels (inclusive).
/// Used by boundary unit tests.
#[allow(dead_code)]
pub fn rect_pixels(x0: i32, y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            v.push((x, y));
        }
    }
    v
}

/// Count triangles that would be inserted for these pixels.
pub fn triangle_count_for_pixels(local_positions: &[(i32, i32)]) -> usize {
    let mut n = 0;
    for poly in collider_polygons_from_pixels(local_positions) {
        // Only triangulate outer (positive area) rings for collider insertion;
        // holes need different handling in the engine — count all for diagnostics.
        n += triangulate_ear_clip(&poly).len();
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_set(pixels: &[(i32, i32)]) -> HashSet<(i32, i32)> {
        pixels.iter().copied().collect()
    }

    fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
        let abx = b.0 - a.0;
        let aby = b.1 - a.1;
        let apx = p.0 - a.0;
        let apy = p.1 - a.1;
        let ab2 = abx * abx + aby * aby;
        if ab2 < 1e-12 {
            return (apx * apx + apy * apy).sqrt();
        }
        let t = ((apx * abx + apy * aby) / ab2).clamp(0.0, 1.0);
        let qx = a.0 + t * abx;
        let qy = a.1 + t * aby;
        ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
    }

    fn near_poly(p: (f32, f32), poly: &[(f32, f32)], tol: f32) -> bool {
        if point_in_polygon(p, poly) {
            return true;
        }
        let n = poly.len();
        for i in 0..n {
            if dist_to_segment(p, poly[i], poly[(i + 1) % n]) <= tol {
                return true;
            }
        }
        false
    }

    /// Every filled pixel center should fall inside (or on) the positive-area outer ring(s)
    /// and outside negative-area holes. Edge-midpoint MS + RDP can leave centers on the rim;
    /// allow a half-pixel edge tolerance.
    fn assert_pixels_covered(pixels: &[(i32, i32)], polys: &[Vec<(f32, f32)>]) {
        let outers: Vec<_> = polys
            .iter()
            .filter(|p| polygon_area(p) > 0.0)
            .cloned()
            .collect();
        let holes: Vec<_> = polys
            .iter()
            .filter(|p| polygon_area(p) < 0.0)
            .cloned()
            .collect();
        assert!(!outers.is_empty(), "expected at least one outer polygon");
        for &(x, y) in pixels {
            let c = (x as f32, y as f32);
            let in_outer = outers.iter().any(|p| near_poly(c, p, 0.55));
            let in_hole = holes.iter().any(|p| point_in_polygon(c, p));
            assert!(
                in_outer && !in_hole,
                "pixel ({x},{y}) not covered by simplified boundary (outer={in_outer} hole={in_hole})"
            );
        }
    }

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
        assert_eq!(loops.len(), 2);
        for l in &loops {
            assert!(polygon_area(l) > 0.0);
        }
    }

    #[test]
    fn marching_squares_single_pixel() {
        let loops = marching_squares_contours(&[(0, 0)]);
        assert_eq!(loops.len(), 1);
        assert!(polygon_area(&loops[0]) > 0.0);
        // Single pixel midpoints form a diamond of area 0.5 (edge-midpoint MS).
        let a = polygon_area(&loops[0]).abs();
        assert!((a - 0.5).abs() < 0.05, "area={a}");
    }

    #[test]
    fn marching_squares_hole_produces_two_loops() {
        let mut pixels = Vec::new();
        for y in 0..5 {
            for x in 0..5 {
                if x >= 1 && x <= 3 && y >= 1 && y <= 3 {
                    continue;
                }
                pixels.push((x, y));
            }
        }
        let loops = marching_squares_contours(&pixels);
        assert_eq!(loops.len(), 2);
        let mut areas: Vec<f32> = loops.iter().map(|l| polygon_area(l)).collect();
        areas.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert!(areas[0] > 0.0);
        assert!(areas[1] < 0.0);
    }

    #[test]
    fn rectangle_simplifies_to_four_corners() {
        let pixels = rect_pixels(0, 0, 9, 4); // 10×5 rect
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys.len(), 1);
        assert_eq!(
            polys[0].len(),
            4,
            "axis-aligned rect should collapse to 4 verts, got {:?}",
            polys[0]
        );
        let area = polygon_area(&polys[0]).abs();
        // Edge-midpoint MS + corner collapse → slightly under nominal 10×5=50.
        assert!(
            area > 40.0 && area < 52.0,
            "area={area}, poly={:?}",
            polys[0]
        );
        assert_pixels_covered(&pixels, &polys);
    }

    #[test]
    fn l_shape_preserves_reentrant_corner() {
        // L: horizontal bar + vertical bar
        let mut pixels = rect_pixels(0, 0, 6, 1);
        pixels.extend(rect_pixels(0, 0, 1, 6));
        let set = filled_set(&pixels);
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys.len(), 1);
        let poly = &polys[0];
        // Must keep more than a box (L is concave).
        assert!(
            poly.len() >= 6,
            "L-shape should keep reentrant verts, got {} {:?}",
            poly.len(),
            poly
        );
        // Inner corner region near (2,2) should be outside the solid L.
        assert!(
            !point_in_polygon((3.0, 3.0), poly) || set.contains(&(3, 3)),
            "point in the L notch should not be solid"
        );
        assert_pixels_covered(&pixels, &polys);
    }

    #[test]
    fn long_thin_bar_does_not_collapse() {
        let pixels = rect_pixels(0, 0, 40, 0); // 41×1
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys.len(), 1);
        assert!(polys[0].len() >= 4);
        let area = polygon_area(&polys[0]).abs();
        assert!(area > 30.0, "bar area collapsed: {area}");
        assert_pixels_covered(&pixels, &polys);
    }

    #[test]
    fn two_by_two_block() {
        let pixels = rect_pixels(0, 0, 1, 1);
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].len(), 4);
        assert_pixels_covered(&pixels, &polys);
    }

    #[test]
    fn ring_hole_coverage() {
        let mut pixels = Vec::new();
        for y in 0..7 {
            for x in 0..7 {
                if x >= 2 && x <= 4 && y >= 2 && y <= 4 {
                    continue;
                }
                pixels.push((x, y));
            }
        }
        let polys = collider_polygons_from_pixels(&pixels);
        assert!(polys.len() >= 2, "ring needs outer+hole");
        assert_pixels_covered(&pixels, &polys);
        // Hole center empty
        let holes: Vec<_> = polys.iter().filter(|p| polygon_area(p) < 0.0).collect();
        assert!(!holes.is_empty());
        assert!(
            point_in_polygon((3.0, 3.0), holes[0]),
            "hole polygon should contain the hollow center"
        );
    }

    #[test]
    fn simplify_reduces_large_blob_vertex_count() {
        // Big filled circle-ish disk
        let mut pixels = Vec::new();
        let r = 20i32;
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r * r {
                    pixels.push((x, y));
                }
            }
        }
        let raw = marching_squares_contours(&pixels);
        assert_eq!(raw.len(), 1);
        let raw_len = raw[0].len();
        let simp = simplify_contour(&raw[0]);
        assert!(
            simp.len() < raw_len,
            "expected simplification {simp} vs raw {raw_len}",
            simp = simp.len()
        );
        assert!(simp.len() >= 8, "disk should not become a triangle");
        let polys = collider_polygons_from_pixels(&pixels);
        assert_pixels_covered(&pixels, &polys);
    }

    #[test]
    fn triangulation_covers_rectangle_with_two_tris() {
        let pixels = rect_pixels(0, 0, 3, 3);
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys[0].len(), 4, "rect poly verts {:?}", polys[0]);
        let tris = triangulate_ear_clip(&polys[0]);
        assert_eq!(tris.len(), 2, "quad → 2 triangles, got {}", tris.len());
    }

    #[test]
    fn triangle_count_reasonable_for_rect_vs_disk() {
        let rect = rect_pixels(0, 0, 15, 15);
        let rect_t = triangle_count_for_pixels(&rect);
        assert_eq!(rect_t, 2, "rect should be 2 tris, got {rect_t}");

        let mut disk = Vec::new();
        for y in -12..=12 {
            for x in -12..=12 {
                if x * x + y * y <= 12 * 12 {
                    disk.push((x, y));
                }
            }
        }
        let disk_t = triangle_count_for_pixels(&disk);
        assert!(disk_t >= 6, "disk needs several tris, got {disk_t}");
        assert!(disk_t < 200, "disk should not explode tris, got {disk_t}");
    }

    #[test]
    fn closed_rdp_does_not_open_ring() {
        // Unit square as closed polyline (no duplicate first/last).
        let sq = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let s = simplify_douglas_peucker_closed(&sq, 0.5);
        assert_eq!(s.len(), 4);
        assert!((polygon_area(&s).abs() - 100.0).abs() < 1e-3);
    }

    #[test]
    fn rdp_keeps_spike_beyond_epsilon() {
        // Flat line with a tall spike in the middle.
        let pts = [(0.0, 0.0), (5.0, 0.0), (5.0, 3.0), (5.0, 0.0), (10.0, 0.0)];
        // Open RDP on a polyline with spike
        let open = [
            (0.0, 0.0),
            (2.5, 0.0),
            (5.0, 3.0),
            (7.5, 0.0),
            (10.0, 0.0),
        ];
        let s = simplify_douglas_peucker_open(&open, 0.5);
        assert!(
            s.iter().any(|p| p.1 > 2.0),
            "spike should be retained: {:?}",
            s
        );
        let _ = pts;
    }

    #[test]
    fn empty_pixels_yield_no_polygons() {
        assert!(collider_polygons_from_pixels(&[]).is_empty());
        assert_eq!(triangle_count_for_pixels(&[]), 0);
    }

    #[test]
    fn diagonal_stair_stays_connected_coverage() {
        // Diagonal staircase 8 steps — classic over-simplify victim.
        let mut pixels = Vec::new();
        for i in 0..8 {
            pixels.push((i, i));
            pixels.push((i + 1, i));
            pixels.push((i, i + 1));
        }
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys.len(), 1);
        assert_pixels_covered(&pixels, &polys);
    }
}
