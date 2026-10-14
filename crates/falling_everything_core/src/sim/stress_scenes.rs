//! Custom stress scenes + integration tests for chunks, electricity, asleep sand.

#[cfg(test)]
mod tests {
    use crate::rigid::{collider_polygons_from_pixels, triangle_count_for_pixels};
    use crate::world::{cell_flags, material, RectI, Vec2i, CHUNK_H, CHUNK_W};
    use crate::{Simulation, SimulationConfig};

    fn sim_small() -> Simulation {
        Simulation::new(SimulationConfig {
            region_size: 256,
            deterministic: true,
            seed: 42,
            ..Default::default()
        })
    }

    fn paint(sim: &mut Simulation, p: Vec2i, mat: u16) {
        sim.paint_circle(p, 0, mat);
    }

    #[test]
    fn chunk_aspect_is_16_by_9() {
        assert_eq!(CHUNK_W * 9, CHUNK_H * 16);
        assert_ne!(CHUNK_W, CHUNK_H, "work units should be 16:9 rectangles, not squares");
        assert_eq!((CHUNK_W, CHUNK_H), (64, 36));
        let sim = sim_small();
        assert_eq!(sim.test_world().chunk_w(), CHUNK_W);
        assert_eq!(sim.test_world().chunk_h(), CHUNK_H);
    }

    /// Regression: Y work-units must use CHUNK_H (36), not square CHUNK_W (64).
    #[test]
    fn dense_chunk_coord_uses_rectangular_extents() {
        let sim = sim_small();
        let w = sim.test_world();
        // Within first chunk: y in 0..36
        let c0 = w
            .dense_chunk_coord_for_cell(Vec2i::new(10, 20))
            .expect("in grid");
        assert_eq!(c0.x, 0);
        assert_eq!(c0.y, 0);
        // y=40 → second vertical chunk only if H=36 (would still be y=0 if square 64).
        let c1 = w
            .dense_chunk_coord_for_cell(Vec2i::new(10, 40))
            .expect("in grid");
        assert_eq!(c1.x, 0);
        assert_eq!(c1.y, 1, "y=40 must land in chunk row 1 with CHUNK_H=36");
        // x=70 → second horizontal chunk (W=64).
        let c2 = w
            .dense_chunk_coord_for_cell(Vec2i::new(70, 10))
            .expect("in grid");
        assert_eq!(c2.x, 1);
        assert_eq!(c2.y, 0);
        let b = w.bounds_for_chunk(c1);
        assert!(b.contains(Vec2i::new(10, 40)));
        assert!(!b.contains(Vec2i::new(10, 20)));
    }

    #[test]
    fn electricity_travels_through_water_bridge() {
        let mut sim = sim_small();
        let y = 40;
        // Floor so water does not fall out of the bridge.
        for x in 8..22 {
            paint(&mut sim, Vec2i::new(x, y + 1), material::STONE);
            paint(&mut sim, Vec2i::new(x, y + 2), material::STONE);
        }
        for x in 10..12 {
            paint(&mut sim, Vec2i::new(x, y), material::METAL);
        }
        for x in 12..18 {
            paint(&mut sim, Vec2i::new(x, y), material::LIQUID);
        }
        for x in 18..20 {
            paint(&mut sim, Vec2i::new(x, y), material::METAL);
        }
        // Free spark beside metal (empty-only visitor), not painted onto metal.
        paint(&mut sim, Vec2i::new(9, y), material::ELECTRICITY);

        let mut saw_right = false;
        let mut saw_water_charged = false;
        for _ in 0..60 {
            sim.step(1.0 / 60.0);
            for x in 12..18 {
                let c = sim.test_world().get_cell(Vec2i::new(x, y));
                if c.has_flag(cell_flags::ELECTRIFIED) {
                    saw_water_charged = true;
                }
                // Medium must never become ELECTRICITY material.
                assert_ne!(
                    c.material(),
                    material::ELECTRICITY,
                    "water bridge replaced by electricity at x={x}"
                );
            }
            for x in 17..20 {
                let c = sim.test_world().get_cell(Vec2i::new(x, y));
                assert_ne!(c.material(), material::ELECTRICITY);
                if c.has_flag(cell_flags::ELECTRIFIED) {
                    saw_right = true;
                    break;
                }
            }
            if saw_right {
                break;
            }
        }
        assert!(
            saw_water_charged,
            "charge should enter the water bridge cells"
        );
        assert!(
            saw_right,
            "electricity should travel through the water bridge to the right metal"
        );
    }

    /// After charge dies, metal stays metal; water is not permanently converted to ELECTRICITY.
    #[test]
    fn electricity_restores_conductor_medium() {
        let mut sim = sim_small();
        let y = 35;
        // Closed trough so water cannot drain (walls + floor).
        for x in 6..22 {
            paint(&mut sim, Vec2i::new(x, y + 1), material::STONE);
        }
        for yy in (y - 1)..=(y + 1) {
            paint(&mut sim, Vec2i::new(6, yy), material::STONE);
            paint(&mut sim, Vec2i::new(21, yy), material::STONE);
        }
        for x in 8..14 {
            paint(&mut sim, Vec2i::new(x, y), material::METAL);
        }
        for x in 14..20 {
            paint(&mut sim, Vec2i::new(x, y), material::LIQUID);
        }
        paint(&mut sim, Vec2i::new(7, y), material::ELECTRICITY);
        // Paint charge directly onto metal (must not replace metal).
        paint(&mut sim, Vec2i::new(10, y), material::ELECTRICITY);
        assert_eq!(
            sim.test_world().get_cell(Vec2i::new(10, y)).material(),
            material::METAL,
            "paint electricity onto metal must keep metal"
        );
        assert!(sim
            .test_world()
            .get_cell(Vec2i::new(10, y))
            .has_flag(cell_flags::ELECTRIFIED));

        for _ in 0..120 {
            sim.step(1.0 / 60.0);
            // Inert metal bar must never become ELECTRICITY material.
            for x in 8..14 {
                let c = sim.test_world().get_cell(Vec2i::new(x, y));
                assert_eq!(
                    c.material(),
                    material::METAL,
                    "metal replaced by mat {} at x={x}",
                    c.material()
                );
            }
        }

        // No permanent ELECTRICITY material trail on the conductor row (metal + water band).
        for x in 8..20 {
            let m = sim.test_world().get_cell(Vec2i::new(x, y)).material();
            assert_ne!(
                m,
                material::ELECTRICITY,
                "permanent ELECTRICITY trail at x={x}"
            );
            assert!(
                m == material::METAL || m == material::LIQUID || m == material::EMPTY,
                "unexpected mat {m} on conductor row at x={x}"
            );
        }
    }

    /// Metal path alone (no water) still conducts — baseline for the water-bridge case.
    #[test]
    fn electricity_travels_along_metal_rail() {
        let mut sim = sim_small();
        let y = 30;
        for x in 5..25 {
            paint(&mut sim, Vec2i::new(x, y + 1), material::STONE);
            paint(&mut sim, Vec2i::new(x, y), material::METAL);
        }
        // Spark in empty cell touching the rail.
        paint(&mut sim, Vec2i::new(5, y - 1), material::ELECTRICITY);
        let mut far = false;
        for _ in 0..50 {
            sim.step(1.0 / 60.0);
            let c = sim.test_world().get_cell(Vec2i::new(22, y));
            assert_eq!(c.material(), material::METAL);
            if c.has_flag(cell_flags::ELECTRIFIED) {
                far = true;
                break;
            }
        }
        assert!(far, "electrified metal rail should carry charge end-to-end");
    }

    #[test]
    fn active_sim_half_extents_respect_fps_cap() {
        let (hw, hh) = crate::Simulation::active_grid_half_extents(640, 360);
        assert!(hw * 2 <= crate::Simulation::ACTIVE_SIM_MAX_W);
        assert!(hh * 2 <= crate::Simulation::ACTIVE_SIM_MAX_H);
        assert!(hw * 2 >= 640);
        assert!(hh * 2 >= 360);
        // Old 3× buffer was 1920×1080 — must not return.
        assert!(hw * 2 < 1200, "must not use 3× screen width, got {}", hw * 2);
    }

    #[test]
    fn sand_falls_asleep_and_joins_static_colliders() {
        let mut sim = sim_small();
        for x in 20..40 {
            paint(&mut sim, Vec2i::new(x, 50), material::STONE);
        }
        for x in 25..35 {
            for y in 30..50 {
                paint(&mut sim, Vec2i::new(x, y), material::SAND);
            }
        }
        for _ in 0..200 {
            sim.step(1.0 / 60.0);
        }
        let mut asleep = 0usize;
        let mut total_sand = 0usize;
        for y in 20..55 {
            for x in 20..45 {
                let c = sim.test_world().get_cell(Vec2i::new(x, y));
                if c.material() == material::SAND {
                    total_sand += 1;
                    if c.has_flag(cell_flags::ASLEEP) {
                        asleep += 1;
                    }
                }
            }
        }
        assert!(total_sand > 50, "sand should still exist, got {total_sand}");
        assert!(
            asleep > 10,
            "expected many asleep sand grains after settle, got {asleep}/{total_sand}"
        );
        let mut found_collider = false;
        for y in 20..55 {
            for x in 20..45 {
                let p = Vec2i::new(x, y);
                let c = sim.test_world().get_cell(p);
                if c.has_flag(cell_flags::ASLEEP)
                    && sim.test_world().cell_contributes_static_collider(p, c)
                {
                    found_collider = true;
                    break;
                }
            }
            if found_collider {
                break;
            }
        }
        assert!(found_collider, "asleep sand should contribute static colliders");
    }

    #[test]
    fn wake_asleep_when_neighbor_changes() {
        let mut sim = sim_small();
        for x in 10..20 {
            paint(&mut sim, Vec2i::new(x, 30), material::STONE);
        }
        paint(&mut sim, Vec2i::new(15, 29), material::SAND);
        for _ in 0..80 {
            sim.step(1.0 / 60.0);
        }
        let mut asleep_pos = None;
        for y in 25..32 {
            for x in 12..18 {
                let c = sim.test_world().get_cell(Vec2i::new(x, y));
                if c.material() == material::SAND && c.has_flag(cell_flags::ASLEEP) {
                    asleep_pos = Some(Vec2i::new(x, y));
                }
            }
        }
        let pos = asleep_pos.expect("sand should fall asleep on stone");
        sim.test_world_mut()
            .wake_asleep_neighbors(Vec2i::new(pos.x + 1, pos.y));
        // Also wake the cell itself if we touch it via neighbor of a dig
        sim.test_world_mut().wake_asleep_neighbors(pos);
        let c2 = sim.test_world().get_cell(pos);
        // wake_asleep_neighbors wakes *neighbors* of the dig point, not the dig point.
        // Dig next to asleep sand:
        sim.test_world_mut()
            .set_cell(Vec2i::new(pos.x + 1, pos.y), crate::world::Cell::new());
        let c3 = sim.test_world().get_cell(pos);
        assert!(
            !c3.has_flag(cell_flags::ASLEEP) || c3.has_flag(cell_flags::IS_FREE_FALLING),
            "adjacent set_cell should wake asleep sand (flags={:?})",
            c3.flags()
        );
        let _ = c2;
    }

    #[test]
    fn stress_scene_mixed_chaos_stays_stable() {
        let mut sim = sim_small();
        for x in 5..100 {
            paint(&mut sim, Vec2i::new(x, 80), material::STONE);
        }
        sim.paint_rect_filled(Vec2i::new(10, 10), Vec2i::new(39, 29), material::SAND);
        sim.paint_rect_filled(Vec2i::new(45, 10), Vec2i::new(69, 24), material::LIQUID);
        for x in 20..30 {
            paint(&mut sim, Vec2i::new(x, 40), material::METAL);
        }
        paint(&mut sim, Vec2i::new(20, 38), material::ELECTRICITY);
        sim.paint_rect_filled(Vec2i::new(75, 50), Vec2i::new(89, 69), material::OIL);
        sim.spawn_rigid_body_rect(Vec2i::new(50, 5), Vec2i::new(58, 12), material::WOOD);

        for _ in 0..120 {
            sim.step(1.0 / 60.0);
        }
        let mut any = false;
        for y in 0..100 {
            for x in 0..110 {
                if sim.test_world().get_cell(Vec2i::new(x, y)).material() != material::EMPTY {
                    any = true;
                    break;
                }
            }
            if any {
                break;
            }
        }
        assert!(any);
        let _ = RectI::new(Vec2i::new(0, 0), Vec2i::new(1, 1));
    }

    /// Desktop-scale paint storm: dense sand/water on ≤700×400 active grid must not panic.
    #[test]
    fn paint_storm_desktop_scale_stays_stable() {
        let mut sim = Simulation::new(SimulationConfig {
            region_size: 256,
            deterministic: true,
            seed: 7,
            ..Default::default()
        });
        let center = Vec2i::new(0, 0);
        let vw = 640i32;
        let vh = 360i32;
        sim.ensure_active_grid(center, vw, vh);
        let (hw, hh) = crate::Simulation::active_grid_half_extents(vw, vh);
        assert!(hw * 2 <= crate::Simulation::ACTIVE_SIM_MAX_W);
        assert!(hh * 2 <= crate::Simulation::ACTIVE_SIM_MAX_H);
        sim.set_solid_bounds(RectI::new(
            Vec2i::new(center.x - hw, center.y - hh),
            Vec2i::new(center.x + hw - 1, center.y + hh - 1),
        ));
        for x in (-280..280).step_by(32) {
            sim.paint_circle(Vec2i::new(x, -40), 20, material::SAND);
            sim.paint_circle(Vec2i::new(x + 12, -80), 16, material::LIQUID);
        }
        for x in -300..300 {
            paint(&mut sim, Vec2i::new(x, 130), material::STONE);
        }
        for _ in 0..90 {
            sim.step(1.0 / 60.0);
        }
        let mut matter = 0usize;
        for y in -hh..hh {
            for x in -hw..hw {
                if sim.test_world().get_cell(Vec2i::new(x, y)).material() != material::EMPTY {
                    matter += 1;
                }
            }
        }
        assert!(matter > 500, "storm should leave substantial matter, got {matter}");
        assert_eq!(sim.grid_width(), hw * 2);
        assert_eq!(sim.grid_height(), hh * 2);
    }

    #[test]
    fn boundary_stress_shapes_triangle_budget() {
        let mut pixels = Vec::new();
        for y in 0..40 {
            for x in 0..10 {
                pixels.push((x, y));
            }
        }
        for y in 0..10 {
            for x in 0..40 {
                pixels.push((x, y));
            }
        }
        let t = triangle_count_for_pixels(&pixels);
        assert!(t >= 2);
        assert!(t < 80, "L should not explode triangles, got {t}");
        let polys = collider_polygons_from_pixels(&pixels);
        assert_eq!(polys.len(), 1);
    }
}
