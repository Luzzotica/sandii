//! Discrete thermal diffusion on a 2D grid.
//!
//! Each step applies an explicit **5-point stencil** Laplacian-style exchange: for each neighbor,
//! flux is proportional to `k` (thermal conductivity) and inversely to **volumetric heat capacity**
//! ρ·c_p on the cell being updated. Empty cells participate as **air** using [`material::EMPTY`]
//! builtin props from [`crate::materials::empty`].
//!
//! **Glossary (grid fields vs physics):** `thermal_conductivity` is **k** [W/(m·K)] in spirit;
//! `volumetric_heat_capacity` is **ρ·c_p** per cell (not specific heat **c_p** alone). One explicit
//! conduction pass per tick. Burning chemistry heat may subsample ticks (`CHEMISTRY_HEAT_PERIOD` in the sim).
//! `EMPTY` at stored temperature `0` is treated as
//! ambient air for flux so fresh grid slots do not behave as absolute zero.
//!
//! Ambient relaxation uses a coarse divisor plus a **stochastic** 1 K nudge when `|T−T_amb|` is
//! small, so we do not equilibrate to room temperature in a single tick (integer Kelvin storage).

use rand::rngs::SmallRng;
use rand::Rng;

use crate::world::{cell_flags, material, Cell, MaterialProps, Vec2i, World};

use super::SimGrids;

/// Resolve material props for temperature conduction. `EMPTY` uses air-like props; `STONE` does not conduct.
#[inline]
pub(crate) fn thermal_props_for(cell: Cell, world: &World) -> Option<MaterialProps> {
    let mat = cell.material();
    if mat == material::STONE {
        return None;
    }
    if mat == material::EMPTY {
        return Some(world.material_props(material::EMPTY));
    }
    Some(world.material_props(mat))
}

/// Extra flux scale for large |ΔT| so hot cells exchange heat quickly with cold neighbors.
#[inline]
fn gradient_boost(diff: i32) -> i32 {
    256 + diff.abs().min(2048)
}

/// Linear relaxation `ΔT = (T_amb − T) / AMBIENT_RELAX_DIV` when magnitude ≥ 1.
const AMBIENT_RELAX_DIV: i32 = 2048;

/// When `|T_amb − T|` is small, integer division yields 0; nudge 1 K with this average period (ticks).
const NEAR_AMBIENT_NUDGE_PERIOD: u32 = 28;

/// Extra global relaxation toward ambient at ~10 Hz (every 6 ticks at 60 Hz sim): removes stray heat from the world.
const AMBIENT_GLOBAL_TICK_PERIOD: u64 = 6;
const AMBIENT_GLOBAL_EXTRA_DIV: i32 = 384;

/// [`material::LIQUID`] uses `min(k_self, k_neighbor)` like everything else, which lets slow conductors
/// (e.g. lava) bottleneck cooling. Add this to **k** when water is on either side of the edge so hot
/// solids lose temperature much faster to water (convection / boiling carrying heat away).
const LIQUID_EDGE_THERMAL_BONUS: i32 = 112;

pub(crate) fn step_temperature(
    sg: &SimGrids,
    world: &World,
    p: Vec2i,
    rng: &mut SmallRng,
    sim_tick: u64,
) {
    let cell = sg.get(p);
    let Some(props) = thermal_props_for(cell, world) else {
        return;
    };
    let ambient = world.ambient_temperature_k() as i32;
    // Fresh grid slots use `Cell::new()` (EMPTY @ 0 K); treat as ambient air for conduction.
    let effective_temp = |c: Cell| -> i32 {
        if c.material() == material::EMPTY && c.temperature() == 0 {
            ambient
        } else {
            c.temperature() as i32
        }
    };
    let mut temp = effective_temp(cell);

    for &(dx, dy) in &[(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
        let np = Vec2i::new(p.x + dx, p.y + dy);
        let nc = sg.get(np);
        let Some(np_props) = thermal_props_for(nc, world) else {
            continue;
        };
        let mut k = (props.thermal_conductivity as i32).min(np_props.thermal_conductivity as i32);
        if cell.material() == material::LIQUID || nc.material() == material::LIQUID {
            k = (k + LIQUID_EDGE_THERMAL_BONUS).min(255);
        }
        let diff = effective_temp(nc) - temp;
        let c_vol = (props.volumetric_heat_capacity as i32).max(1);
        let base = (diff * k) / (c_vol * 16);
        let transfer = (base * gradient_boost(diff)) / 256;
        temp += transfer;
    }

    let d_amb = ambient - temp;
    let relax = d_amb / AMBIENT_RELAX_DIV;
    if relax != 0 {
        temp += relax;
    } else if d_amb != 0 && rng.gen_ratio(1, NEAR_AMBIENT_NUDGE_PERIOD) {
        temp += d_amb.signum();
    }

    if sim_tick % AMBIENT_GLOBAL_TICK_PERIOD == 0 {
        let d_amb = ambient - temp;
        let pull = d_amb / AMBIENT_GLOBAL_EXTRA_DIV;
        if pull != 0 {
            temp += pull;
        }
    }

    let new_temp = temp.clamp(0, crate::cell64::MAX_TEMPERATURE as i32) as u16;
    if new_temp != cell.temperature() {
        sg.set_temperature(p, new_temp);
    }
}

/// Phase change: swaps material and temperature; keeps lifetime, most flags, velocities, variant.
/// On **melt**, strips rigid-body flags and [`Cell::rigid_source_material`] so the cell can move in
/// [`crate::sim::step_pixel`] (rigid pixels otherwise skip liquid/gas/granular motion).
pub(crate) fn check_phase_transition(sg: &SimGrids, world: &World, p: Vec2i) {
    let cell = sg.get(p);
    let props = world.material_props(cell.material());
    let temp = cell.temperature();

    if props.freeze_temperature > 0
        && temp <= props.freeze_temperature
        && props.freeze_into != material::EMPTY
    {
        let into = props.freeze_into;
        let new_cell = cell.with_material(into).with_temperature(temp);
        sg.set_cell(p, new_cell);
    } else if props.melt_temperature > 0
        && temp >= props.melt_temperature
        && props.melt_into != material::EMPTY
    {
        let into = props.melt_into;
        let new_cell = cell
            .with_material(into)
            .with_temperature(temp)
            .with_flags(cell.flags() & !(cell_flags::RIGID_PIXEL | cell_flags::RIGID_BODY_SIM))
            .with_rigid_source_material(0);
        sg.set_cell(p, new_cell);
    }
}
