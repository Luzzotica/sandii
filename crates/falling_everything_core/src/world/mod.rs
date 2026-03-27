use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use crate::cell64::Cell;
use crate::cell64::{MAX_MATERIAL_ID, MAX_TEMPERATURE};

mod store;
use store::{DenseCellStore, SpatialHashChunkStore};

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

    pub fn intersection(&self, other: &RectI) -> Option<RectI> {
        let min_x = self.min.x.max(other.min.x);
        let min_y = self.min.y.max(other.min.y);
        let max_x = self.max.x.min(other.max.x);
        let max_y = self.max.y.min(other.max.y);
        if min_x <= max_x && min_y <= max_y {
            Some(RectI::new(
                Vec2i::new(min_x, min_y),
                Vec2i::new(max_x, max_y),
            ))
        } else {
            None
        }
    }
}

pub type MaterialId = u16;
pub const CHUNK_W: i32 = 32;
pub const CHUNK_H: i32 = 32;
pub const CHUNK_SIZE: i32 = 32;
pub const CHUNK_AREA: usize = (CHUNK_W as usize) * (CHUNK_H as usize);
/// How many ticks a chunk stays awake after the last wake signal.
/// Gives settling cascades (sand piles, liquid spreading) time to propagate
/// before sleeping the chunk.
pub const WAKE_COOLDOWN: u8 = 16;

const _: () = assert!(CHUNK_W == CHUNK_H && CHUNK_W == CHUNK_SIZE);

pub mod material {
    use super::MaterialId;

    pub const EMPTY: MaterialId = 0;
    pub const SAND: MaterialId = 1;
    pub const LIQUID: MaterialId = 2;
    pub const GAS: MaterialId = 3;
    pub const STONE: MaterialId = 4;
    pub const RIGID: MaterialId = 5;
    pub const LIGHT_LIQUID: MaterialId = 6;
    pub const HEAVY_LIQUID: MaterialId = 7;
    pub const OIL: MaterialId = 8;
    pub const FIRE: MaterialId = 9;
    pub const WAX: MaterialId = 10;
    pub const C4: MaterialId = 11;
    pub const SMOKE: MaterialId = 12;
    pub const PLANT: MaterialId = 13;
    pub const ACID: MaterialId = 14;
    pub const EMBER: MaterialId = 15;
    pub const WOOD: MaterialId = 16;
    pub const STEAM: MaterialId = 17;
    pub const LAVA: MaterialId = 18;
    pub const TORCH: MaterialId = 19;
    pub const WELL: MaterialId = 20;
    pub const SPOUT: MaterialId = 21;
    pub const DIRT: MaterialId = 22;
    pub const GRASS: MaterialId = 23;
    pub const OBSIDIAN: MaterialId = 24;
    pub const ICE: MaterialId = 25;
    pub const MELTED_WAX: MaterialId = 26;
    pub const MAX_MATERIALS: usize = 1024;
}

pub mod chunk_local_step {
    pub const LEFT: i32 = -1;
    pub const RIGHT: i32 = 1;
    pub const UP: i32 = -super::CHUNK_W;
    pub const DOWN: i32 = super::CHUNK_W;
    pub const UP_LEFT: i32 = UP + LEFT;
    pub const UP_RIGHT: i32 = UP + RIGHT;
    pub const DOWN_LEFT: i32 = DOWN + LEFT;
    pub const DOWN_RIGHT: i32 = DOWN + RIGHT;
}

#[inline]
pub const fn chunk_local_index(x: i32, y: i32) -> usize {
    (y as usize) * (CHUNK_W as usize) + (x as usize)
}

#[derive(Debug, Clone, Copy)]
pub struct MaterialRule {
    pub lateral_spread: u8,
    /// Reserved for future tuning. **Not** used for lateral liquid–liquid motion: the sim does not
    /// swap adjacent liquids sideways (see `sim::can_displace`, `MoveIntent::Lateral`).
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

/// How a material participates in movement and displacement (one variant per distinct sim path).
///
/// This groups [`Phase`], `inert`, and the numeric fields that only apply to certain kinds of matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialMotion {
    /// Solid anchor: no granular / liquid / gas stepping (still runs fire, adjacent transforms, spawns).
    InertSolid,
    /// Solid grains or rigid chunks (granular / sand-style stepping).
    SolidGranular {
        viscosity: u8,
        max_speed: u8,
        acceleration: u8,
        inertial_resistance: u8,
    },
    /// Flowing liquid.
    Liquid {
        viscosity: u8,
        max_speed: u8,
        acceleration: u8,
        extinguishes_fire: bool,
    },
    /// Buoyant gas, smoke, steam, fire path, etc.
    Gas {
        viscosity: u8,
        max_speed: u8,
        acceleration: u8,
    },
}

impl MaterialMotion {
    #[inline]
    pub const fn phase(self) -> Phase {
        match self {
            Self::InertSolid | Self::SolidGranular { .. } => Phase::Solid,
            Self::Liquid { .. } => Phase::Liquid,
            Self::Gas { .. } => Phase::Gas,
        }
    }

    #[inline]
    pub const fn inert(self) -> bool {
        matches!(self, Self::InertSolid)
    }

    #[inline]
    pub const fn viscosity(self) -> u8 {
        match self {
            Self::InertSolid => 255,
            Self::SolidGranular { viscosity, .. }
            | Self::Liquid { viscosity, .. }
            | Self::Gas { viscosity, .. } => viscosity,
        }
    }

    #[inline]
    pub const fn max_speed(self) -> u8 {
        match self {
            Self::InertSolid => 0,
            Self::SolidGranular { max_speed, .. }
            | Self::Liquid { max_speed, .. }
            | Self::Gas { max_speed, .. } => max_speed,
        }
    }

    #[inline]
    pub const fn acceleration(self) -> u8 {
        match self {
            Self::InertSolid => 0,
            Self::SolidGranular { acceleration, .. }
            | Self::Liquid { acceleration, .. }
            | Self::Gas { acceleration, .. } => acceleration,
        }
    }

    #[inline]
    pub const fn inertial_resistance(self) -> u8 {
        match self {
            Self::InertSolid => 255,
            Self::SolidGranular {
                inertial_resistance,
                ..
            } => inertial_resistance,
            Self::Liquid { .. } | Self::Gas { .. } => 255,
        }
    }

    #[inline]
    pub const fn extinguishes_fire(self) -> bool {
        match self {
            Self::Liquid {
                extinguishes_fire, ..
            } => extinguishes_fire,
            _ => false,
        }
    }
}

/// Max rules per material for [`MaterialProps::adjacent_transforms`].
pub const MAX_ADJACENT_TRANSFORM_RULES: usize = 4;

/// When this material is stepped, each neighbor may be replaced: `from` → `to` with the given odds.
///
/// Slots with `chance_percent == 0` are ignored. Rules are checked in order per neighbor; the first
/// matching `from` that succeeds the roll applies, then that neighbor is skipped for remaining rules.
///
/// `to: EMPTY` clears the neighbor with [`Cell::default()`].
///
/// On success, the neighbor is written first, then if [`Self::actor_lifetime_delta`] is non-zero the
/// source cell at the stepped position is re-read and its [`Cell::lifetime`] is reduced (saturating).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjacentTransformRule {
    pub from: MaterialId,
    pub to: MaterialId,
    /// 0 = disabled. 1–100 = percent success (`roll in 0..100` must be `< chance_percent`).
    pub chance_percent: u8,
    /// If true, only the four cardinals; if false, all eight neighbors.
    pub cardinal_neighbors_only: bool,
    /// Subtract from the acting cell's `lifetime` after a successful transform (`0` = no effect).
    pub actor_lifetime_delta: u8,
}

impl AdjacentTransformRule {
    pub const fn inactive() -> Self {
        Self {
            from: material::EMPTY,
            to: material::EMPTY,
            chance_percent: 0,
            cardinal_neighbors_only: true,
            actor_lifetime_delta: 0,
        }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.chance_percent > 0
    }
}

impl Default for AdjacentTransformRule {
    fn default() -> Self {
        Self::inactive()
    }
}

pub type AdjacentTransformRules = [AdjacentTransformRule; MAX_ADJACENT_TRANSFORM_RULES];

/// Max spawn rules per material for [`MaterialProps::neighbor_spawns`].
pub const MAX_NEIGHBOR_SPAWN_RULES: usize = 4;

/// Each tick, with [`Self::chance`] / 256 odds, try to place [`Self::spawn_material`] in a random empty
/// 8-neighbor (one successful spawn per rule per tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeighborSpawnRule {
    pub spawn_material: MaterialId,
    /// 0 = disabled. Otherwise `gen_ratio(chance, 256)` each tick.
    pub chance: u8,
    /// Half-open lifetime for spawned cell when `hi > lo`; else [`World::initial_lifetime_for`].
    pub lifetime_lo: u8,
    pub lifetime_hi: u8,
    pub spawn_flags: u8,
}

impl NeighborSpawnRule {
    pub const fn inactive() -> Self {
        Self {
            spawn_material: material::EMPTY,
            chance: 0,
            lifetime_lo: 0,
            lifetime_hi: 0,
            spawn_flags: 0,
        }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.spawn_material != material::EMPTY && self.chance > 0
    }
}

impl Default for NeighborSpawnRule {
    fn default() -> Self {
        Self::inactive()
    }
}

pub type NeighborSpawnRules = [NeighborSpawnRule; MAX_NEIGHBOR_SPAWN_RULES];

/// Whether this material can be damaged by adjacent corrosive liquids and at what odds per tick.
///
/// Inactive when `affected == false` or `chance_percent == 0`. Roll in `sim`: `rng.gen_range(0..100) < chance_percent.min(100)`.
/// Neighbor HP for multi-hit melt is [`MaterialProps::corrosion_max_hp`] + [`Cell::lifetime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcidVulnerability {
    pub affected: bool,
    pub chance_percent: u8,
}

impl AcidVulnerability {
    pub const fn inactive() -> Self {
        Self {
            affected: false,
            chance_percent: 0,
        }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.affected && self.chance_percent > 0
    }
}

impl Default for AcidVulnerability {
    fn default() -> Self {
        Self::inactive()
    }
}

/// Damage dealt by a **corrosive liquid** (e.g. acid) to acid-vulnerable neighbors and cost to its own `lifetime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcidCorrosionSource {
    pub neighbor_damage: u8,
    pub self_lifetime_cost: u8,
}

impl AcidCorrosionSource {
    pub const fn inactive() -> Self {
        Self {
            neighbor_damage: 0,
            self_lifetime_cost: 0,
        }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.neighbor_damage != 0 || self.self_lifetime_cost != 0
    }
}

impl Default for AcidCorrosionSource {
    fn default() -> Self {
        Self::inactive()
    }
}

/// Max rules per material for [`MaterialProps::adjacent_influence`].
pub const MAX_ADJACENT_INFLUENCE_RULES: usize = 8;

/// How [`AdjacentInfluenceRule`] updates the neighbor [`Cell::lifetime`] when not using a transition spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfluenceVictimLifetime {
    Unchanged,
    Set(u8),
    /// Use [`MaterialProps::fuel_mass`] on the victim (smolder ignition).
    UseVictimFuelMass,
}

/// How the **source** cell at `p` changes after a successful influence on a neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfluenceSourceEffect {
    None,
    ClearSourceCell,
    AddSourceLifetime(u8),
    SubtractSourceLifetime(u8),
}

/// Probabilistic neighbor flag / lifetime effects from a stepped cell (fire spread, water wetting, etc.).
///
/// Checked in order per neighbor; first matching `victim` that passes the roll applies.
///
/// **Roll** (`sim::try_adjacent_influence_on_neighbor`): `heat_factor` is passed by the caller (e.g. fire uses
/// `lifetime / 255`, ember uses `ember ignitability / 255`, water uses `1.0`). With `requires_victim_ignitability`,
/// success probability is `(chance_percent / 100) * heat_factor * (victim ignitability / 255)` (capped at `1.0`),
/// except when victim `ignitability == 255` the random check always succeeds (legacy “always ignite”; `chance_percent` does not gate it).
/// Without `requires_victim_ignitability`, use `(chance_percent / 100) * heat_factor`. Flags are applied as
/// `(old | flags_or) & !flags_clear` unless a transition spawn replaces the cell.
/// Optional [`Self::empty_neighbor_spawn`] runs after any successful apply (including flag-only updates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjacentInfluenceRule {
    pub victim: MaterialId,
    /// Base success scale 1–100; combined with `heat_factor` and optionally victim `ignitability` (see sim).
    pub chance_percent: u8,
    /// If true, only the four cardinals; if false, all eight offsets (same winding as burn spread).
    pub cardinal_neighbors_only: bool,
    /// When true, roll uses `(chance/100) * heat_factor * (victim.ignitability/255)` (victim 255 uses only chance×heat).
    /// When false, roll uses `(chance/100) * heat_factor` (for wet/dry sand, etc.).
    pub requires_victim_ignitability: bool,
    /// If non-zero, rule only matches when `(victim.flags & mask) != 0` (e.g. only wet sand for drying).
    pub require_victim_flags_any: u8,
    /// If non-zero, skip when `(victim.flags & mask) != 0` (e.g. do not wet already-wet sand).
    pub exclude_victim_flags_any: u8,
    pub flags_or: u8,
    pub flags_clear: u8,
    pub victim_lifetime: InfluenceVictimLifetime,
    pub source_effect: InfluenceSourceEffect,
    /// If non-zero: when this mask goes from set to clear, replace victim with `spawn_on_clear_material`.
    pub if_cleared_mask: u8,
    pub spawn_on_clear_material: MaterialId,
    /// If non-zero: when this mask goes from clear to set, replace victim with `spawn_on_set_material`.
    pub if_set_mask: u8,
    pub spawn_on_set_material: MaterialId,
    /// Half-open lifetime for transition spawns; if `hi <= lo`, sim uses [`World::initial_lifetime_for`].
    pub spawn_lifetime_lo: u8,
    pub spawn_lifetime_hi: u8,
    /// If non-`EMPTY`, after applying this rule to the victim, try to place this material in one **random** empty
    /// 8-neighbor of the victim (e.g. steam when drying wet sand without replacing the sand cell).
    pub empty_neighbor_spawn: MaterialId,
    /// Half-open lifetime for [`Self::empty_neighbor_spawn`]; if `hi <= lo`, sim uses [`World::initial_lifetime_for`].
    pub empty_neighbor_spawn_lifetime_lo: u8,
    pub empty_neighbor_spawn_lifetime_hi: u8,
}

impl AdjacentInfluenceRule {
    pub const fn inactive() -> Self {
        Self {
            victim: material::EMPTY,
            chance_percent: 0,
            cardinal_neighbors_only: false,
            requires_victim_ignitability: false,
            require_victim_flags_any: 0,
            exclude_victim_flags_any: 0,
            flags_or: 0,
            flags_clear: 0,
            victim_lifetime: InfluenceVictimLifetime::Unchanged,
            source_effect: InfluenceSourceEffect::None,
            if_cleared_mask: 0,
            spawn_on_clear_material: material::EMPTY,
            if_set_mask: 0,
            spawn_on_set_material: material::EMPTY,
            spawn_lifetime_lo: 0,
            spawn_lifetime_hi: 0,
            empty_neighbor_spawn: material::EMPTY,
            empty_neighbor_spawn_lifetime_lo: 0,
            empty_neighbor_spawn_lifetime_hi: 0,
        }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.victim != material::EMPTY && self.chance_percent > 0
    }
}

impl Default for AdjacentInfluenceRule {
    fn default() -> Self {
        Self::inactive()
    }
}

pub type AdjacentInfluenceRules = [AdjacentInfluenceRule; MAX_ADJACENT_INFLUENCE_RULES];

pub mod cell_flags {
    pub const IS_FREE_FALLING: u8 = 1 << 0;
    pub const WET: u8 = 1 << 1;
    pub const ELECTRIFIED: u8 = 1 << 2;
    /// Legacy flag kept for backward compatibility. Prefer `RIGID_PIXEL`.
    pub const RIGID_BODY_SIM: u8 = 1 << 3;
    /// Marks a cell as belonging to a rigid body. The cell keeps its **real** material so the
    /// full sim rules (reactions, adjacent transforms, temperature, phase transitions) apply.
    /// Movement (granular fall, liquid flow, gas rise) is blocked by checking this flag in
    /// `step_pixel` and `try_displace`.
    pub const RIGID_PIXEL: u8 = 1 << 4;
    /// Solid (or other) fuel is actively burning: loses `lifetime` via `consumption_rate` and emits heat.
    /// Set when temperature crosses [`MaterialProps::autoignition_temperature`] or when ignited by neighbors.
    /// Molten fuels with `autoignition_temperature == 0` (e.g. lava) do not use this flag.
    pub const ON_FIRE: u8 = 1 << 5;
}

#[derive(Debug, Clone, Copy)]
pub struct MaterialProps {
    pub density: i16,
    pub motion: MaterialMotion,
    /// 0 = never ignites. 255 = always ignites from adjacent `FIRE` (skips heat-scaled roll).
    pub ignitability: u8,
    /// While [`cell_flags::ON_FIRE`] is set, probability weight out of 256 to lose 1 `lifetime` (fuel HP) per tick.
    pub consumption_rate: u8,
    /// If > 0, successful ignition seeds `lifetime` with this value and sets [`cell_flags::ON_FIRE`] (see [`crate::sim::is_burning`]) instead of immediate replacement.
    /// If 0, heat replaces the cell using `smolder_burnout_become` + `smolder_burnout_lifetime_*` (same as smolder burnout); when those are unset, becomes `FIRE` with heat-weakened lifetime.
    pub fuel_mass: u8,
    /// Resistance to ray-style explosions: higher = more blast energy absorbed per hit. `0` = ray strength is not reduced when passing through (still may clear the cell).
    pub durability: u8,
    pub explosion_radius: u8,
    pub on_heat_become: MaterialId,
    pub on_death_become: MaterialId,
    /// Half-open range for replacement [`Cell::lifetime`] when this material dies into `on_death_become`
    /// (e.g. `FIRE` → smoke). If `hi <= lo`, the sim uses a built-in default range instead.
    pub on_death_lifetime_lo: u8,
    pub on_death_lifetime_hi: u8,
    /// For materials that take multiple acid hits: max HP stored in [`Cell::lifetime`] when painted.
    /// `0` = first successful corrosive hit erases the cell (no bar). Hit chance and “affected” are [`Self::acid_vulnerability`]; damage/cost come from the adjacent corrosive liquid’s [`AcidCorrosionSource`].
    pub corrosion_max_hp: u8,

    /// When active ([`AcidVulnerability::is_active`]), this material can be damaged by adjacent corrosive liquids at `chance_percent` per tick.
    pub acid_vulnerability: AcidVulnerability,

    /// When non-inactive on a corrosive liquid, [`sim`](crate::sim) applies these values to vulnerable neighbors each successful hit.
    pub acid_corrosion: AcidCorrosionSource,

    // --- Smolder elimination (`ON_FIRE` + `fuel_mass`): burnout or fully quenched by water ---
    /// Cell becomes this material when smolder ends. `EMPTY` means `material::SMOKE`.
    pub smolder_extinguish_material: MaterialId,
    /// Half-open range for replacement [`Cell::lifetime`]: `rng.gen_range(lo..hi)`.
    pub smolder_extinguish_lifetime_lo: u8,
    pub smolder_extinguish_lifetime_hi: u8,
    /// On **burnout** (fuel consumed), flash-ignite neighbors using the same rules as fire spread.
    pub smolder_burnout_ignites_neighbors: bool,
    /// On **burnout** only: enqueue an explosion here (`0` = none). Skips placing the replacement cell at `p` (blast clears the cell).
    pub smolder_burnout_explosion_radius: u8,
    /// When set, replaces smolder on **burnout** instead of `smolder_extinguish_*` (e.g. plant → fire, wood → ember).
    /// Same fields apply when `fuel_mass == 0` and heat **instantly** replaces the cell (spread / flash ignite).
    /// `EMPTY` = smolder uses extinguish material/range; instant heat defaults to `FIRE` + weakened spread lifetime.
    pub smolder_burnout_become: MaterialId,
    /// Half-open lifetime range (`rng.gen_range(lo..hi)`) for `smolder_burnout_become` (smolder burnout and instant heat when `fuel_mass == 0`).
    pub smolder_burnout_lifetime_lo: u8,
    pub smolder_burnout_lifetime_hi: u8,

    /// Probabilistic neighbor replacement each tick (runs even when [`Self::inert`] is true).
    pub adjacent_transforms: AdjacentTransformRules,

    /// Neighbor flag/lifetime rules (fire spread, water wetting, drying wet sand, etc.).
    pub adjacent_influence: AdjacentInfluenceRules,

    /// Spawn materials into random empty 8-neighbors (smolder, ember, and inert props like torches).
    pub neighbor_spawns: NeighborSpawnRules,

    /// How much collision stress a rigid-body pixel made of this material can take before anchors are
    /// dropped against **inert** world cells with no `rigid_id` (terrain squeeze). Higher = tougher.
    /// Overlap with non-inert materials (liquids, sand, etc.) always removes anchors regardless of this value.
    /// Tuned relative to [`crate::rigid::STRUCTURAL_STRESS_SCALE`].
    pub structure_integrity: f32,

    // --- Temperature system (Kelvin; 0 allowed for EMPTY / unused) ---
    /// Temperature when spawned/painted (e.g. lava ~1473 K, water ~293 K).
    pub base_temperature: u16,
    /// 0–255 scale: thermal conductivity **k** [W/(m·K)] in relative units.
    pub thermal_conductivity: u8,
    /// **Volumetric heat capacity** ρ·c_p in relative units (not mass-specific c_p alone). Not stored on [`Cell`].
    /// Larger = more energy to change temperature by 1 K per cell (thermal inertia).
    pub volumetric_heat_capacity: u16,
    /// Minimum temperature for sustained smolder/burn (`fuel_mass` + `lifetime`). 0 = never ignites.
    pub autoignition_temperature: u16,
    /// Above this temperature, the cell melts into `melt_into` (e.g. ice→water, water→steam). 0 = never melts.
    pub melt_temperature: u16,
    /// Material to become when melting (e.g. ICE → LIQUID).
    pub melt_into: MaterialId,
    /// Below this temperature, the cell freezes into `freeze_into` (e.g. water→ice, steam→water). 0 = never freezes.
    pub freeze_temperature: u16,
    /// Material to become when freezing (e.g. LAVA → OBSIDIAN).
    pub freeze_into: MaterialId,
    /// Local heat source while burning: abstract units; ΔT from chemistry ≈ `rate * 256 / volumetric_heat_capacity` when a chemistry step runs (subsampled; see sim `CHEMISTRY_HEAT_PERIOD`).
    pub heat_generation_rate: u16,
    /// Kelvin-equivalent pulse spread to **cardinal neighbors** when the cell first catches fire (`ON_FIRE`).
    /// Scaled like chemistry: `ΔT_neighbor ≈ pulse * 256 / neighbor_volumetric_heat_capacity`. `0` = derive from `heat_generation_rate`.
    /// Negative values cool neighbors (“alchemical” endothermic ignition).
    pub ignition_thermal_pulse: i16,
}

impl Default for MaterialProps {
    fn default() -> Self {
        Self::default_const()
    }
}

impl MaterialProps {
    /// Same values as [`Default`], usable in `const` (e.g. `MaterialProps { density: 1, ..MaterialProps::default_const() }`).
    pub const fn default_const() -> Self {
        Self {
            density: 0,
            motion: MaterialMotion::InertSolid,
            ignitability: 0,
            consumption_rate: 0,
            fuel_mass: 0,
            durability: 0,
            explosion_radius: 0,
            on_heat_become: material::EMPTY,
            on_death_become: material::EMPTY,
            on_death_lifetime_lo: 0,
            on_death_lifetime_hi: 0,
            corrosion_max_hp: 0,
            // Most materials are acid-vulnerable unless overridden (e.g. EMPTY, ACID in builtins).
            acid_vulnerability: AcidVulnerability {
                affected: true,
                chance_percent: 50,
            },
            acid_corrosion: AcidCorrosionSource::inactive(),
            smolder_extinguish_material: material::EMPTY,
            smolder_extinguish_lifetime_lo: 24,
            smolder_extinguish_lifetime_hi: 64,
            smolder_burnout_ignites_neighbors: false,
            smolder_burnout_explosion_radius: 0,
            smolder_burnout_become: material::EMPTY,
            smolder_burnout_lifetime_lo: 0,
            smolder_burnout_lifetime_hi: 0,
            adjacent_transforms: [
                AdjacentTransformRule::inactive(),
                AdjacentTransformRule::inactive(),
                AdjacentTransformRule::inactive(),
                AdjacentTransformRule::inactive(),
            ],
            adjacent_influence: [
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
                AdjacentInfluenceRule::inactive(),
            ],
            neighbor_spawns: [
                NeighborSpawnRule::inactive(),
                NeighborSpawnRule::inactive(),
                NeighborSpawnRule::inactive(),
                NeighborSpawnRule::inactive(),
            ],
            structure_integrity: 1.0,
            base_temperature: 293,
            thermal_conductivity: 50,
            volumetric_heat_capacity: 128,
            autoignition_temperature: 0,
            melt_temperature: 0,
            melt_into: material::EMPTY,
            freeze_temperature: 0,
            freeze_into: material::EMPTY,
            heat_generation_rate: 0,
            ignition_thermal_pulse: 0,
        }
    }

    #[inline]
    pub const fn phase(self) -> Phase {
        self.motion.phase()
    }

    #[inline]
    pub const fn inert(self) -> bool {
        self.motion.inert()
    }

    #[inline]
    pub const fn viscosity(self) -> u8 {
        self.motion.viscosity()
    }

    #[inline]
    pub const fn max_speed(self) -> u8 {
        self.motion.max_speed()
    }

    #[inline]
    pub const fn acceleration(self) -> u8 {
        self.motion.acceleration()
    }

    #[inline]
    pub const fn inertial_resistance(self) -> u8 {
        self.motion.inertial_resistance()
    }

    #[inline]
    pub const fn extinguishes_fire(self) -> bool {
        self.motion.extinguishes_fire()
    }

    #[inline]
    pub fn has_adjacent_transforms(self) -> bool {
        self.adjacent_transforms.iter().any(|r| r.is_active())
    }

    #[inline]
    pub fn has_neighbor_spawns(self) -> bool {
        self.neighbor_spawns.iter().any(|r| r.is_active())
    }

    #[inline]
    pub fn has_acid_corrosion(self) -> bool {
        self.acid_corrosion.is_active()
    }

    #[inline]
    pub fn has_adjacent_influence(self) -> bool {
        self.adjacent_influence.iter().any(|r| r.is_active())
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

const STONE_CELL: Cell = Cell::new().with_material(material::STONE);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkCoord {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy)]
pub enum WorldEvent {
    RegionLoaded(Vec2i),
    RegionSaved(Vec2i),
}

#[derive(Debug, Serialize, Deserialize)]
struct RegionFile {
    #[serde(default = "region_file_version")]
    version: u8,
    chunk_size: i32,
    cells: Vec<(ChunkCoord, Vec<Cell>)>,
}

const fn region_file_version() -> u8 {
    2
}

pub struct World {
    chunk_size: i32,
    region_size: i32,

    store: DenseCellStore,
    hash_chunks: SpatialHashChunkStore,

    chunks_x: i32,
    chunks_y: i32,
    chunk_dirty: Vec<bool>,
    /// Marks chunks whose inert-solid content changed (for static physics collider rebuild).
    chunk_physics_dirty: Vec<bool>,
    /// Per-chunk cooldown counter: >0 means awake. Decremented each tick; any
    /// wake signal resets to `WAKE_COOLDOWN`. Prevents rapid sleep/wake cycling.
    chunk_awake: Vec<u8>,
    chunk_awake_next: Vec<bool>,

    active_regions: HashSet<Vec2i>,
    active_radius_regions: i32,
    focus: Vec2i,
    storage_dir: Option<PathBuf>,
    solid_bounds: Option<RectI>,

    material_props: [MaterialProps; material::MAX_MATERIALS],
    material_rules: [MaterialRule; material::MAX_MATERIALS],
    reactions: Vec<ReactionOutcome>,
    rigid_ids: HashMap<Vec2i, u32>,

    debug_pass_enabled: bool,
    debug_pass: Vec<u8>,

    /// When stepping one chunk at a time, the chunk currently being processed (checkerboard pass in `.1`).
    debug_chunk_highlight: Option<(ChunkCoord, u8)>,

    /// Draw faint outlines for all chunks in each checkerboard pass (same color = same pass = parallel batch in thread-pool mode).
    debug_pass_batch_outlines: bool,

    /// Ambient temperature (Kelvin) used by thermal relaxation in the sim.
    ambient_temperature_k: u16,
}

fn cell_contributes_static_collider_refs(
    store: &DenseCellStore,
    material_props: &[MaterialProps; material::MAX_MATERIALS],
    rigid_ids: &HashMap<Vec2i, u32>,
    p: Vec2i,
    cell: Cell,
) -> bool {
    if store.grid_index(p).is_none() {
        return false;
    }
    material_props
        .get(cell.material() as usize)
        .copied()
        .unwrap_or_default()
        .inert()
        && cell.material() != material::EMPTY
        && !rigid_ids.contains_key(&p)
        && !cell.has_flag(cell_flags::RIGID_PIXEL)
}

impl World {
    #[inline]
    fn assert_supported_material_id(id: MaterialId) {
        debug_assert!(
            id <= MAX_MATERIAL_ID,
            "material id {} exceeds packed u10 limit {}",
            id,
            MAX_MATERIAL_ID
        );
    }

    pub fn new(chunk_size: i32, region_size: i32) -> Self {
        assert_eq!(
            chunk_size, CHUNK_SIZE,
            "World chunk size is fixed at {}; got {}",
            CHUNK_SIZE, chunk_size
        );
        let w = region_size;
        let h = region_size;
        let origin = Vec2i::new(0, 0);
        let cx = (w + CHUNK_SIZE - 1) / CHUNK_SIZE;
        let cy = (h + CHUNK_SIZE - 1) / CHUNK_SIZE;

        Self {
            chunk_size: CHUNK_SIZE,
            region_size,
            store: DenseCellStore::new(w, h, origin),
            hash_chunks: SpatialHashChunkStore::with_capacity((cx * cy).max(1) as usize * 4),
            chunks_x: cx,
            chunks_y: cy,
            chunk_dirty: vec![true; (cx * cy) as usize],
            chunk_physics_dirty: vec![true; (cx * cy) as usize],
            chunk_awake: vec![WAKE_COOLDOWN; (cx * cy) as usize],
            chunk_awake_next: vec![false; (cx * cy) as usize],
            active_regions: HashSet::new(),
            active_radius_regions: 1,
            focus: Vec2i::new(0, 0),
            storage_dir: None,
            solid_bounds: None,
            material_props: default_material_props(),
            material_rules: default_material_rules(),
            reactions: default_reactions(),
            rigid_ids: HashMap::new(),
            debug_pass_enabled: false,
            debug_pass: Vec::new(),
            debug_chunk_highlight: None,
            debug_pass_batch_outlines: false,
            ambient_temperature_k: 293,
        }
    }

    /// Ambient temperature for thermal relaxation (Kelvin).
    #[inline]
    pub fn ambient_temperature_k(&self) -> u16 {
        self.ambient_temperature_k
    }

    pub fn set_ambient_temperature_k(&mut self, k: u16) {
        self.ambient_temperature_k = k;
    }

    pub fn chunk_size(&self) -> i32 {
        self.chunk_size
    }

    pub fn grid_width(&self) -> i32 {
        self.store.width()
    }

    pub fn grid_height(&self) -> i32 {
        self.store.height()
    }

    pub fn grid_origin(&self) -> Vec2i {
        self.store.origin()
    }

    pub fn set_storage_dir(&mut self, dir: PathBuf) {
        self.storage_dir = Some(dir);
    }

    pub fn set_solid_bounds(&mut self, bounds: RectI) {
        self.solid_bounds = Some(bounds);
        let new_w = bounds.max.x - bounds.min.x + 1;
        let new_h = bounds.max.y - bounds.min.y + 1;
        if new_w != self.store.width()
            || new_h != self.store.height()
            || bounds.min != self.store.origin()
        {
            self.store.resize(new_w, new_h, bounds.min);
            let cx = (new_w + CHUNK_SIZE - 1) / CHUNK_SIZE;
            let cy = (new_h + CHUNK_SIZE - 1) / CHUNK_SIZE;
            self.chunks_x = cx;
            self.chunks_y = cy;
            self.hash_chunks = SpatialHashChunkStore::with_capacity((cx * cy).max(1) as usize * 4);
            let n = (cx * cy) as usize;
            self.chunk_dirty = vec![true; n];
            self.chunk_physics_dirty = vec![true; n];
            self.chunk_awake = vec![WAKE_COOLDOWN; n];
            self.chunk_awake_next = vec![false; n];
        }
    }

    /// Re-center the dense simulation grid around `center` with the given half-extents.
    /// Cells leaving the new area are saved to hash_chunks; cells entering are loaded
    /// from hash_chunks if present.
    /// Recenters the dense grid. Returns `true` if the store origin/size changed.
    pub fn relocate_around(&mut self, center: Vec2i, half_w: i32, half_h: i32) -> bool {
        let new_origin = Vec2i::new(center.x - half_w, center.y - half_h);
        let new_w = half_w * 2;
        let new_h = half_h * 2;

        if new_origin == self.store.origin()
            && new_w == self.store.width()
            && new_h == self.store.height()
        {
            return false;
        }

        let old_origin = self.store.origin();
        let old_w = self.store.width();
        let old_h = self.store.height();
        let old_rect = RectI::new(
            old_origin,
            Vec2i::new(old_origin.x + old_w - 1, old_origin.y + old_h - 1),
        );
        let new_rect = RectI::new(
            new_origin,
            Vec2i::new(new_origin.x + new_w - 1, new_origin.y + new_h - 1),
        );

        // Save outgoing cells (in old area but NOT in new area) to hash_chunks.
        // A chunk that partially overlaps both areas must still save its non-overlap cells.
        let old_base_cx = div_floor(old_origin.x, CHUNK_SIZE);
        let old_base_cy = div_floor(old_origin.y, CHUNK_SIZE);
        let old_chunks_x = (old_w + CHUNK_SIZE - 1) / CHUNK_SIZE;
        let old_chunks_y = (old_h + CHUNK_SIZE - 1) / CHUNK_SIZE;

        for lcy in 0..old_chunks_y {
            for lcx in 0..old_chunks_x {
                let coord = ChunkCoord {
                    x: old_base_cx + lcx,
                    y: old_base_cy + lcy,
                };
                let chunk_min = Vec2i::new(coord.x * CHUNK_SIZE, coord.y * CHUNK_SIZE);
                let chunk_max =
                    Vec2i::new(chunk_min.x + CHUNK_SIZE - 1, chunk_min.y + CHUNK_SIZE - 1);
                let chunk_rect = RectI::new(chunk_min, chunk_max);

                let clipped = chunk_rect.intersection(&old_rect).unwrap_or(chunk_rect);
                let mut has_content = false;
                for y in clipped.min.y..=clipped.max.y {
                    for x in clipped.min.x..=clipped.max.x {
                        if new_rect.contains(Vec2i::new(x, y)) {
                            continue; // overlap — preserved by relocate
                        }
                        if let Some(cell) = self.store.get_read(Vec2i::new(x, y)) {
                            if cell.material() != material::EMPTY {
                                has_content = true;
                                break;
                            }
                        }
                    }
                    if has_content {
                        break;
                    }
                }
                if !has_content {
                    continue;
                }
                self.hash_chunks.ensure_chunk(coord);
                for y in clipped.min.y..=clipped.max.y {
                    for x in clipped.min.x..=clipped.max.x {
                        if new_rect.contains(Vec2i::new(x, y)) {
                            continue;
                        }
                        if let Some(cell) = self.store.get_read(Vec2i::new(x, y)) {
                            if cell.material() != material::EMPTY {
                                let lx = x - coord.x * CHUNK_SIZE;
                                let ly = y - coord.y * CHUNK_SIZE;
                                let local_idx = chunk_local_index(lx, ly);
                                if let Some(slab) = self.hash_chunks.get_chunk_mut(coord) {
                                    slab[local_idx] = cell;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Relocate dense store (copies overlap region automatically).
        let (_old_read, _old_origin, _old_w, _old_h) =
            self.store.relocate(new_origin, new_w, new_h);

        // Load incoming cells from hash_chunks (in new area but NOT in old area).
        // Partial-overlap chunks must still load their non-overlap portion.
        let new_base_cx = div_floor(new_origin.x, CHUNK_SIZE);
        let new_base_cy = div_floor(new_origin.y, CHUNK_SIZE);
        let new_chunks_x = (new_w + CHUNK_SIZE - 1) / CHUNK_SIZE;
        let new_chunks_y = (new_h + CHUNK_SIZE - 1) / CHUNK_SIZE;

        let mut loaded_coords = Vec::new();
        for lcy in 0..new_chunks_y {
            for lcx in 0..new_chunks_x {
                let coord = ChunkCoord {
                    x: new_base_cx + lcx,
                    y: new_base_cy + lcy,
                };
                let chunk_min = Vec2i::new(coord.x * CHUNK_SIZE, coord.y * CHUNK_SIZE);
                let chunk_max =
                    Vec2i::new(chunk_min.x + CHUNK_SIZE - 1, chunk_min.y + CHUNK_SIZE - 1);
                let chunk_rect = RectI::new(chunk_min, chunk_max);

                if let Some(slab) = self.hash_chunks.get_chunk(coord) {
                    let slab_copy: Vec<Cell> = slab.to_vec();
                    let clipped = chunk_rect.intersection(&new_rect).unwrap_or(chunk_rect);
                    for y in clipped.min.y..=clipped.max.y {
                        for x in clipped.min.x..=clipped.max.x {
                            if old_rect.contains(Vec2i::new(x, y)) {
                                continue; // overlap — already copied by relocate
                            }
                            let lx = x - coord.x * CHUNK_SIZE;
                            let ly = y - coord.y * CHUNK_SIZE;
                            let local_idx = chunk_local_index(lx, ly);
                            let cell = slab_copy[local_idx];
                            if cell.material() != material::EMPTY {
                                self.store.set_read(Vec2i::new(x, y), cell);
                            }
                        }
                    }
                    // Only remove hash chunk if it's fully inside the new dense area
                    // (all its cells are now covered by the dense store).
                    if chunk_rect.min.x >= new_rect.min.x
                        && chunk_rect.max.x <= new_rect.max.x
                        && chunk_rect.min.y >= new_rect.min.y
                        && chunk_rect.max.y <= new_rect.max.y
                    {
                        loaded_coords.push(coord);
                    }
                }
            }
        }
        for coord in loaded_coords {
            self.hash_chunks.remove_chunk(coord);
        }

        // Rebuild chunk metadata.
        self.chunks_x = new_chunks_x;
        self.chunks_y = new_chunks_y;
        let n = (new_chunks_x * new_chunks_y) as usize;
        self.chunk_dirty = vec![true; n];
        self.chunk_physics_dirty = vec![true; n];
        self.chunk_awake = vec![WAKE_COOLDOWN; n];
        self.chunk_awake_next = vec![false; n];

        self.solid_bounds = None;

        // Rebuild debug pass buffer if active.
        if self.debug_pass_enabled {
            self.debug_pass = vec![0xFF; (new_w * new_h) as usize];
        }
        true
    }

    pub fn material_props(&self, id: MaterialId) -> MaterialProps {
        self.material_props
            .get(id as usize)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_material_props(&mut self, id: MaterialId, props: MaterialProps) {
        Self::assert_supported_material_id(id);
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
        Self::assert_supported_material_id(id);
        if let Some(slot) = self.material_rules.get_mut(id as usize) {
            *slot = rule;
        }
    }

    pub fn reaction(&self, from: MaterialId, to: MaterialId) -> ReactionOutcome {
        let idx = from as usize * material::MAX_MATERIALS + to as usize;
        self.reactions
            .get(idx)
            .copied()
            .unwrap_or(ReactionOutcome::None)
    }

    pub fn set_reaction(&mut self, from: MaterialId, to: MaterialId, reaction: ReactionOutcome) {
        Self::assert_supported_material_id(from);
        Self::assert_supported_material_id(to);
        let idx = from as usize * material::MAX_MATERIALS + to as usize;
        if let Some(slot) = self.reactions.get_mut(idx) {
            *slot = reaction;
        }
    }

    pub fn get_rigid_id(&self, p: Vec2i) -> Option<u32> {
        self.rigid_ids.get(&p).copied()
    }

    /// Positions currently listed in `rigid_ids` for `body_id` (for sync / cleanup).
    pub fn rigid_ids_iter_for_body(&self, body_id: u32) -> impl Iterator<Item = Vec2i> + '_ {
        self.rigid_ids
            .iter()
            .filter(move |(_, id)| **id == body_id)
            .map(|(p, _)| *p)
    }

    pub fn set_rigid_id(&mut self, p: Vec2i, id: u32) {
        let cell = self.get_cell(p);
        let before = self.cell_contributes_static_collider(p, cell);
        self.rigid_ids.insert(p, id);
        let after = self.cell_contributes_static_collider(p, cell);
        if before != after {
            self.mark_chunk_physics_dirty_for(p);
        }
    }

    pub fn clear_rigid_id(&mut self, p: Vec2i) {
        let cell = self.get_cell(p);
        let before = self.cell_contributes_static_collider(p, cell);
        self.rigid_ids.remove(&p);
        let after = self.cell_contributes_static_collider(p, cell);
        if before != after {
            self.mark_chunk_physics_dirty_for(p);
        }
    }

    /// Clears every cell that still claims `body_id` (including morph-close fill pixels that are not
    /// represented in [`crate::rigid::PixelRigidBody::anchors`]). Call when removing that body from
    /// the physics bridge so stale STONE / rigid pixels cannot outlive the body.
    pub fn purge_rigid_body_ownership(&mut self, body_id: u32) {
        let positions: Vec<Vec2i> = self
            .rigid_ids
            .iter()
            .filter(|(_, id)| **id == body_id)
            .map(|(p, _)| *p)
            .collect();
        for p in positions {
            self.set_cell(p, Cell::new());
        }
    }

    pub fn active_chunk_count(&self) -> usize {
        self.chunk_awake.iter().filter(|&&a| a > 0).count()
    }

    pub fn sleeping_chunk_count(&self) -> usize {
        self.chunk_awake.iter().filter(|&&a| a == 0).count()
    }

    fn chunk_local_index(&self, cx: i32, cy: i32) -> Option<usize> {
        if cx >= 0 && cx < self.chunks_x && cy >= 0 && cy < self.chunks_y {
            Some((cy * self.chunks_x + cx) as usize)
        } else {
            None
        }
    }

    pub fn advance_awake_flags(&mut self) {
        for i in 0..self.chunk_awake.len() {
            if self.chunk_awake_next[i] {
                self.chunk_awake[i] = WAKE_COOLDOWN;
            } else {
                self.chunk_awake[i] = self.chunk_awake[i].saturating_sub(1);
            }
            self.chunk_awake_next[i] = false;
        }
    }

    pub fn wake_chunk_at(&mut self, p: Vec2i) {
        let origin = self.store.origin();
        let cx = div_floor(p.x - origin.x, self.chunk_size);
        let cy = div_floor(p.y - origin.y, self.chunk_size);
        self.wake_chunk_local(cx, cy);
    }

    fn wake_chunk_local(&mut self, cx: i32, cy: i32) {
        if let Some(idx) = self.chunk_local_index(cx, cy) {
            self.chunk_awake_next[idx] = true;
        }
    }

    pub fn wake_chunk_and_neighbors(&mut self, p: Vec2i) {
        let origin = self.store.origin();
        let cx = div_floor(p.x - origin.x, self.chunk_size);
        let cy = div_floor(p.y - origin.y, self.chunk_size);
        for dy in -1..=1 {
            for dx in -1..=1 {
                self.wake_chunk_local(cx + dx, cy + dy);
            }
        }
    }

    pub fn is_chunk_awake(&self, coord: ChunkCoord) -> bool {
        let origin = self.store.origin();
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        let cx = coord.x - base_cx;
        let cy = coord.y - base_cy;
        self.chunk_local_index(cx, cy)
            .map(|i| self.chunk_awake[i] > 0)
            .unwrap_or(false)
    }

    pub fn chunk_awake_ptr(&self) -> *const u8 {
        self.chunk_awake.as_ptr()
    }

    pub fn chunk_awake_next_ptr(&self) -> *mut bool {
        self.chunk_awake_next.as_ptr() as *mut bool
    }

    pub fn chunks_x(&self) -> i32 {
        self.chunks_x
    }

    pub fn chunks_y(&self) -> i32 {
        self.chunks_y
    }

    pub fn set_debug_pass_enabled(&mut self, enabled: bool) {
        self.debug_pass_enabled = enabled;
        let total = (self.store.width() * self.store.height()) as usize;
        if enabled && self.debug_pass.len() != total {
            self.debug_pass = vec![0xFF; total];
        }
    }

    pub fn debug_pass_enabled(&self) -> bool {
        self.debug_pass_enabled
    }

    pub fn debug_pass_ptr(&mut self) -> *mut u8 {
        self.debug_pass.as_mut_ptr()
    }

    pub fn get_debug_pass(&self, p: Vec2i) -> u8 {
        self.grid_index(p)
            .and_then(|i| self.debug_pass.get(i).copied())
            .unwrap_or(0xFF)
    }

    pub fn debug_chunk_highlight(&self) -> Option<(ChunkCoord, u8)> {
        self.debug_chunk_highlight
    }

    pub(crate) fn set_debug_chunk_highlight(&mut self, highlight: Option<(ChunkCoord, u8)>) {
        self.debug_chunk_highlight = highlight;
    }

    pub fn debug_pass_batch_outlines(&self) -> bool {
        self.debug_pass_batch_outlines
    }

    pub fn set_debug_pass_batch_outlines(&mut self, enabled: bool) {
        self.debug_pass_batch_outlines = enabled;
    }

    pub fn set_focus(&mut self, focus: Vec2i) -> Vec<WorldEvent> {
        self.focus = focus;
        self.sync_streaming_regions()
    }

    fn grid_index(&self, p: Vec2i) -> Option<usize> {
        self.store.grid_index(p)
    }

    pub fn get_cell(&self, p: Vec2i) -> Cell {
        self.store.get_read(p).unwrap_or(STONE_CELL)
    }

    /// Whether this cell is included in the static terrain Rapier colliders (fixed body).
    pub(crate) fn cell_contributes_static_collider(&self, p: Vec2i, cell: Cell) -> bool {
        cell_contributes_static_collider_refs(
            &self.store,
            &self.material_props,
            &self.rigid_ids,
            p,
            cell,
        )
    }

    /// Mark every chunk for a static Rapier collider rebuild on the next [`Self::take_physics_dirty_chunks`].
    pub fn mark_all_chunks_physics_dirty(&mut self) {
        for d in self.chunk_physics_dirty.iter_mut() {
            *d = true;
        }
    }

    pub fn set_cell(&mut self, p: Vec2i, mut cell: Cell) {
        if cell.material() == material::EMPTY && cell.temperature() == 0 {
            cell = cell.with_temperature(self.ambient_temperature_k());
        }
        Self::assert_supported_material_id(cell.material());
        let old = self.store.get_read(p);
        if self.store.set_read(p, cell) {
            let (coord, idx) = SpatialHashChunkStore::split_world_to_chunk(p);
            self.hash_chunks.ensure_chunk(coord);
            if let Some(chunk) = self.hash_chunks.get_chunk_mut(coord) {
                if idx < chunk.len() {
                    chunk[idx] = cell;
                }
            }
            self.mark_chunk_dirty_for(p);
            self.wake_chunk_and_neighbors(p);

            let before = old.map_or(false, |c| self.cell_contributes_static_collider(p, c));
            self.clear_rigid_id(p);
            let after = self.cell_contributes_static_collider(p, cell);
            if before != after {
                self.mark_chunk_physics_dirty_for(p);
            }
        }
    }

    fn mark_chunk_dirty_for(&mut self, p: Vec2i) {
        let origin = self.store.origin();
        let cx = div_floor(p.x - origin.x, self.chunk_size);
        let cy = div_floor(p.y - origin.y, self.chunk_size);
        if cx >= 0 && cx < self.chunks_x && cy >= 0 && cy < self.chunks_y {
            self.chunk_dirty[(cy * self.chunks_x + cx) as usize] = true;
        }
    }

    fn mark_chunk_physics_dirty_for(&mut self, p: Vec2i) {
        let origin = self.store.origin();
        let cx = div_floor(p.x - origin.x, self.chunk_size);
        let cy = div_floor(p.y - origin.y, self.chunk_size);
        if cx >= 0 && cx < self.chunks_x && cy >= 0 && cy < self.chunks_y {
            self.chunk_physics_dirty[(cy * self.chunks_x + cx) as usize] = true;
        }
    }

    /// Drain and return all chunk coordinates whose static-terrain collider mask may have changed
    /// since the last call (`set_cell`, sim commit in `finish_sim`, or rigid id toggles).
    pub fn take_physics_dirty_chunks(&mut self) -> Vec<ChunkCoord> {
        let origin = self.store.origin();
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        let mut out = Vec::new();
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                let idx = (cy * self.chunks_x + cx) as usize;
                if self.chunk_physics_dirty[idx] {
                    self.chunk_physics_dirty[idx] = false;
                    out.push(ChunkCoord {
                        x: cx + base_cx,
                        y: cy + base_cy,
                    });
                }
            }
        }
        out
    }

    /// Return chunk coords for all currently-awake chunks.
    pub fn awake_chunk_coords(&self) -> Vec<ChunkCoord> {
        let origin = self.store.origin();
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        let mut out = Vec::new();
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                let idx = (cy * self.chunks_x + cx) as usize;
                if self.chunk_awake[idx] > 0 {
                    out.push(ChunkCoord {
                        x: cx + base_cx,
                        y: cy + base_cy,
                    });
                }
            }
        }
        out
    }

    pub fn paint_circle(&mut self, center: Vec2i, radius: i32, mat: MaterialId) {
        Self::assert_supported_material_id(mat);
        let props = self.material_props(mat);
        let initial_lifetime = Self::initial_lifetime_for(mat, &props);
        let cell = Cell::new()
            .with_material(mat)
            .with_lifetime(initial_lifetime)
            .with_temperature(props.base_temperature);
        let r2 = radius * radius;
        for y in (center.y - radius)..=(center.y + radius) {
            for x in (center.x - radius)..=(center.x + radius) {
                let dx = x - center.x;
                let dy = y - center.y;
                if dx * dx + dy * dy <= r2 {
                    self.set_cell(Vec2i::new(x, y), cell);
                }
            }
        }
    }

    pub fn paint_rect_filled(&mut self, min: Vec2i, max: Vec2i, mat: MaterialId) {
        Self::assert_supported_material_id(mat);
        let props = self.material_props(mat);
        let initial_lifetime = Self::initial_lifetime_for(mat, &props);
        let cell = Cell::new()
            .with_material(mat)
            .with_lifetime(initial_lifetime)
            .with_temperature(props.base_temperature);
        let x0 = min.x.min(max.x);
        let x1 = min.x.max(max.x);
        let y0 = min.y.min(max.y);
        let y1 = min.y.max(max.y);
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set_cell(Vec2i::new(x, y), cell);
            }
        }
    }

    /// Add `delta` to temperature for every non-`EMPTY` / non-`STONE` cell in a filled disk.
    /// Does not clear rigid-body IDs (unlike [`Self::paint_circle`]).
    pub fn adjust_temperature_disk(&mut self, center: Vec2i, radius: i32, delta: i32) {
        let max_t = MAX_TEMPERATURE as i32;
        let r2 = radius * radius;
        for y in (center.y - radius)..=(center.y + radius) {
            for x in (center.x - radius)..=(center.x + radius) {
                let dx = x - center.x;
                let dy = y - center.y;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let p = Vec2i::new(x, y);
                let Some(mut c) = self.store.get_read(p) else {
                    continue;
                };
                let m = c.material();
                if m == material::EMPTY || m == material::STONE {
                    continue;
                }
                let t = c.temperature() as i32;
                let nt = (t + delta).clamp(0, max_t) as u16;
                if nt == c.temperature() {
                    continue;
                }
                c.set_temperature(nt);
                if self.store.set_read(p, c) {
                    let (coord, idx) = SpatialHashChunkStore::split_world_to_chunk(p);
                    self.hash_chunks.ensure_chunk(coord);
                    if let Some(chunk) = self.hash_chunks.get_chunk_mut(coord) {
                        if idx < chunk.len() {
                            chunk[idx] = c;
                        }
                    }
                    self.mark_chunk_dirty_for(p);
                    self.wake_chunk_and_neighbors(p);
                }
            }
        }
    }

    /// Same as [`Self::adjust_temperature_disk`] for an axis-aligned filled rectangle.
    pub fn adjust_temperature_rect_filled(&mut self, min: Vec2i, max: Vec2i, delta: i32) {
        let max_t = MAX_TEMPERATURE as i32;
        let x0 = min.x.min(max.x);
        let x1 = min.x.max(max.x);
        let y0 = min.y.min(max.y);
        let y1 = min.y.max(max.y);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Vec2i::new(x, y);
                let Some(mut c) = self.store.get_read(p) else {
                    continue;
                };
                let m = c.material();
                if m == material::EMPTY || m == material::STONE {
                    continue;
                }
                let t = c.temperature() as i32;
                let nt = (t + delta).clamp(0, max_t) as u16;
                if nt == c.temperature() {
                    continue;
                }
                c.set_temperature(nt);
                if self.store.set_read(p, c) {
                    let (coord, idx) = SpatialHashChunkStore::split_world_to_chunk(p);
                    self.hash_chunks.ensure_chunk(coord);
                    if let Some(chunk) = self.hash_chunks.get_chunk_mut(coord) {
                        if idx < chunk.len() {
                            chunk[idx] = c;
                        }
                    }
                    self.mark_chunk_dirty_for(p);
                    self.wake_chunk_and_neighbors(p);
                }
            }
        }
    }

    pub(crate) fn initial_lifetime_for(mat: MaterialId, props: &MaterialProps) -> u8 {
        if mat == material::FIRE {
            return 72;
        }
        if mat == material::SMOKE {
            return 48;
        }
        if mat == material::STEAM {
            return 60;
        }
        if mat == material::ACID {
            return 255;
        }
        if mat == material::EMBER {
            return 160;
        }
        if props.corrosion_max_hp > 0 {
            return props.corrosion_max_hp;
        }
        if props.fuel_mass > 0 {
            return 0;
        }
        if props.phase() == Phase::Gas {
            return 0;
        }
        if props.durability > 0 {
            return props.durability;
        }
        0
    }

    pub fn try_swap(&mut self, from: Vec2i, to: Vec2i) -> bool {
        let from_cell = self.get_cell(from);
        let to_cell = self.get_cell(to);
        if to_cell.material() != material::EMPTY {
            return false;
        }
        self.set_cell(to, from_cell);
        self.set_cell(from, Cell::new());
        true
    }

    // --- Double-buffer lifecycle ---

    pub fn prepare_sim(&mut self) {
        self.store.prepare_sim();
    }

    pub fn finish_sim(&mut self) {
        self.mark_physics_dirty_from_simulation_diff();
        self.store.finish_sim();
    }

    fn mark_physics_dirty_from_simulation_diff(&mut self) {
        let store = &self.store;
        let material_props = &self.material_props;
        let rigid_ids = &self.rigid_ids;
        let chunk_awake_next = self.chunk_awake_next.as_slice();

        let (read_buf, write_buf) = store.read_write_cell_slices();
        debug_assert_eq!(read_buf.len(), write_buf.len());
        let origin = store.origin();
        let width = store.width();
        let height = store.height();
        let chunks_x = self.chunks_x;
        let chunks_y = self.chunks_y;
        let chunk_size = self.chunk_size;

        let mut mark_indices: Vec<usize> = Vec::new();

        for cy in 0..chunks_y {
            for cx in 0..chunks_x {
                let idx = (cy * chunks_x + cx) as usize;
                if !chunk_awake_next[idx] {
                    continue;
                }
                let mut dirty = false;
                'scan: for ly in 0..chunk_size {
                    for lx in 0..chunk_size {
                        let x = origin.x + cx * chunk_size + lx;
                        let y = origin.y + cy * chunk_size + ly;
                        let lx_g = x - origin.x;
                        let ly_g = y - origin.y;
                        if lx_g < 0 || lx_g >= width || ly_g < 0 || ly_g >= height {
                            continue;
                        }
                        let i = (ly_g * width + lx_g) as usize;
                        let old_c = read_buf[i];
                        let new_c = write_buf[i];
                        let p = Vec2i::new(x, y);
                        let before = cell_contributes_static_collider_refs(
                            store,
                            material_props,
                            rigid_ids,
                            p,
                            old_c,
                        );
                        let after = cell_contributes_static_collider_refs(
                            store,
                            material_props,
                            rigid_ids,
                            p,
                            new_c,
                        );
                        if before != after {
                            dirty = true;
                            break 'scan;
                        }
                    }
                }
                if dirty {
                    mark_indices.push(idx);
                }
            }
        }

        for idx in mark_indices {
            self.chunk_physics_dirty[idx] = true;
        }
    }

    /// After a sim step, remove stale rigid-body ownership for cells the sim cleared to EMPTY.
    /// Without this, `sync_pixels_to_physics` would keep re-drawing body pixels on top of acid etc.
    pub fn prune_rigid_ids_for_empty_cells(&mut self) {
        let keys: Vec<Vec2i> = self.rigid_ids.keys().copied().collect();
        for p in keys {
            if self.get_cell(p).material() == material::EMPTY {
                self.clear_rigid_id(p);
            }
        }
    }

    /// Cells that still list a `rigid_id` but whose committed material moves as liquid or gas (fire,
    /// ember, smoke, water, acid, etc.). Sim commits via the double buffer without updating `rigid_ids`;
    /// the rigid bridge must carve these voxels and clear ownership or they stay glued to the body.
    pub(crate) fn collect_liquid_gas_rigid_hits(&self) -> Vec<(Vec2i, u32)> {
        let mut out = Vec::new();
        for (p, id) in &self.rigid_ids {
            let m = self.get_cell(*p).material();
            if m == material::EMPTY {
                continue;
            }
            let motion = self.material_props(m).motion;
            if matches!(
                motion,
                MaterialMotion::Liquid { .. } | MaterialMotion::Gas { .. }
            ) {
                out.push((*p, *id));
            }
        }
        out
    }

    pub fn read_cells(&self) -> &[Cell] {
        self.store.read_cells()
    }

    pub fn write_cells_mut(&mut self) -> &mut [Cell] {
        self.store.write_cells_mut()
    }

    pub fn write_cells_ptr(&mut self) -> *mut Cell {
        self.store.write_cells_ptr()
    }

    pub fn write_cells_len(&self) -> usize {
        self.store.write_cells_len()
    }

    // --- Chunk queries ---

    pub fn all_chunk_dirty_rects(&self) -> Vec<(ChunkCoord, RectI)> {
        let mut out = Vec::new();
        let origin = self.store.origin();
        let width = self.store.width();
        let height = self.store.height();
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                let idx = (cy * self.chunks_x + cx) as usize;
                if self.chunk_dirty[idx] {
                    let coord = ChunkCoord {
                        x: cx + base_cx,
                        y: cy + base_cy,
                    };
                    let min_x = origin.x + cx * self.chunk_size;
                    let min_y = origin.y + cy * self.chunk_size;
                    let max_x = (min_x + self.chunk_size - 1).min(origin.x + width - 1);
                    let max_y = (min_y + self.chunk_size - 1).min(origin.y + height - 1);
                    out.push((
                        coord,
                        RectI::new(Vec2i::new(min_x, min_y), Vec2i::new(max_x, max_y)),
                    ));
                }
            }
        }
        out
    }

    pub fn active_chunk_coords_for_pass(&self, pass: usize) -> Vec<ChunkCoord> {
        let px = (pass & 1) as i32;
        let py = ((pass >> 1) & 1) as i32;
        let origin = self.store.origin();
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        let mut coords = Vec::new();
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                let idx = (cy * self.chunks_x + cx) as usize;
                if self.chunk_awake[idx] == 0 {
                    continue;
                }
                let abs_cx = base_cx + cx;
                let abs_cy = base_cy + cy;
                if (abs_cx & 1) == px && (abs_cy & 1) == py {
                    coords.push(ChunkCoord {
                        x: abs_cx,
                        y: abs_cy,
                    });
                }
            }
        }
        coords.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
        coords
    }

    /// World-space rectangle for one dense-grid chunk.
    ///
    /// [`ChunkCoord`] values from [`Self::take_physics_dirty_chunks`], [`Self::active_chunk_coords_for_pass`],
    /// and the simulation scheduler use `coord = local_index + div_floor(origin, chunk_size)` so that
    /// stepping and static colliders stay aligned with the store even when `origin` is not a multiple of
    /// [`CHUNK_SIZE`]. This must **not** use `coord * chunk_size` alone.
    pub fn bounds_for_chunk(&self, coord: ChunkCoord) -> RectI {
        let origin = self.store.origin();
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        let lcx = coord.x - base_cx;
        let lcy = coord.y - base_cy;
        let w = self.store.width();
        let h = self.store.height();
        if lcx < 0 || lcy < 0 || lcx >= self.chunks_x || lcy >= self.chunks_y || w <= 0 || h <= 0 {
            return RectI::new(origin, origin);
        }
        let min_x = origin.x + lcx * self.chunk_size;
        let min_y = origin.y + lcy * self.chunk_size;
        let max_x = (min_x + self.chunk_size - 1).min(origin.x + w - 1);
        let max_y = (min_y + self.chunk_size - 1).min(origin.y + h - 1);
        RectI::new(Vec2i::new(min_x, min_y), Vec2i::new(max_x, max_y))
    }

    /// Chunk coordinate used with [`Self::bounds_for_chunk`] for a world cell inside the dense store.
    #[cfg(test)]
    pub(crate) fn dense_chunk_coord_for_cell(&self, p: Vec2i) -> Option<ChunkCoord> {
        if self.store.grid_index(p).is_none() {
            return None;
        }
        let origin = self.store.origin();
        let lcx = div_floor(p.x - origin.x, self.chunk_size);
        let lcy = div_floor(p.y - origin.y, self.chunk_size);
        if lcx < 0 || lcy < 0 || lcx >= self.chunks_x || lcy >= self.chunks_y {
            return None;
        }
        let base_cx = div_floor(origin.x, self.chunk_size);
        let base_cy = div_floor(origin.y, self.chunk_size);
        Some(ChunkCoord {
            x: lcx + base_cx,
            y: lcy + base_cy,
        })
    }

    pub fn finish_frame(&mut self) {
        for d in self.chunk_dirty.iter_mut() {
            *d = true;
        }
        let _ = self.sync_streaming_regions();
    }

    pub fn dirty_world_bounds(&self) -> Option<RectI> {
        let width = self.store.width();
        let height = self.store.height();
        let origin = self.store.origin();
        if width > 0 && height > 0 {
            Some(RectI::new(
                origin,
                Vec2i::new(origin.x + width - 1, origin.y + height - 1),
            ))
        } else {
            None
        }
    }

    pub fn clear_outside_rect(&mut self, bounds: RectI) {
        self.store.clear_outside_rect(bounds);
        self.hash_chunks.remove_outside_world_rect(bounds);
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
        let to_unload: Vec<Vec2i> = self.active_regions.difference(&desired).copied().collect();
        for region in to_unload {
            self.save_region(region);
            self.active_regions.remove(&region);
            events.push(WorldEvent::RegionSaved(region));
        }

        let to_load: Vec<Vec2i> = desired.difference(&self.active_regions).copied().collect();
        for region in to_load {
            self.load_region(region);
            self.active_regions.insert(region);
            events.push(WorldEvent::RegionLoaded(region));
        }

        events
    }

    fn region_coord(&self, p: Vec2i) -> Vec2i {
        Vec2i::new(
            div_floor(p.x, self.region_size),
            div_floor(p.y, self.region_size),
        )
    }

    fn save_region(&self, region: Vec2i) {
        let Some(storage_dir) = &self.storage_dir else {
            return;
        };
        let _ = fs::create_dir_all(storage_dir);

        let region_min_x = region.x * self.region_size;
        let region_min_y = region.y * self.region_size;
        let region_max_x = region_min_x + self.region_size - 1;
        let region_max_y = region_min_y + self.region_size - 1;

        let cs = self.chunk_size;
        let mut chunk_cells = Vec::new();
        let chunk_min_cx = div_floor(region_min_x, cs);
        let chunk_max_cx = div_floor(region_max_x, cs);
        let chunk_min_cy = div_floor(region_min_y, cs);
        let chunk_max_cy = div_floor(region_max_y, cs);

        for cy in chunk_min_cy..=chunk_max_cy {
            for cx in chunk_min_cx..=chunk_max_cx {
                let coord = ChunkCoord { x: cx, y: cy };
                let mut cells = vec![Cell::new(); (cs * cs) as usize];
                for ly in 0..cs {
                    for lx in 0..cs {
                        let p = Vec2i::new(cx * cs + lx, cy * cs + ly);
                        if let Some(cell) = self.store.get_read(p) {
                            cells[(ly * cs + lx) as usize] = cell;
                        }
                    }
                }
                if cells.iter().any(|c| c.material() != material::EMPTY) {
                    chunk_cells.push((coord, cells));
                }
            }
        }

        let payload = RegionFile {
            version: region_file_version(),
            chunk_size: cs,
            cells: chunk_cells,
        };
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
        let cs = payload.chunk_size;
        for (coord, cells) in payload.cells {
            if cs == CHUNK_SIZE && cells.len() == CHUNK_AREA {
                let _ = self.hash_chunks.upsert_chunk(coord, &cells);
            }
            for ly in 0..cs {
                for lx in 0..cs {
                    let p = Vec2i::new(coord.x * cs + lx, coord.y * cs + ly);
                    let cell = cells[(ly * cs + lx) as usize];
                    if cell.material() != material::EMPTY {
                        if self.store.set_read(p, cell) {
                            let (chunk, local_idx) = SpatialHashChunkStore::split_world_to_chunk(p);
                            self.hash_chunks.ensure_chunk(chunk);
                            if let Some(slab) = self.hash_chunks.get_chunk_mut(chunk) {
                                if local_idx < slab.len() {
                                    slab[local_idx] = cell;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
impl World {
    /// Count of grid cells whose `rigid_id` is not in `alive_ids` (orphaned ownership).
    pub(crate) fn stale_rigid_reference_count(
        &self,
        alive_ids: &std::collections::HashSet<u32>,
    ) -> usize {
        self.rigid_ids
            .values()
            .filter(|id| !alive_ids.contains(id))
            .count()
    }
}

pub fn div_floor(a: i32, b: i32) -> i32 {
    let mut q = a / b;
    let r = a % b;
    if r != 0 && ((r > 0) != (b > 0)) {
        q -= 1;
    }
    q
}
