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
    pub const MAX_MATERIALS: usize = 512;
}

/// Lifetime removed from fire (and similar) when an adjacent transform vaporizes a water neighbor.
pub const ADJ_ACTOR_WATER_QUENCH_LIFETIME: u8 = 48;

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
                inertial_resistance, ..
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
    pub spawn_flags: u16,
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

/// Max rules per corrosive liquid for [`MaterialProps::corrosion_adjacent`].
pub const MAX_CORROSION_ADJACENT_RULES: usize = 8;

/// When this **corrosive** material is stepped as a liquid, each cardinal neighbor may be damaged.
///
/// Slots with `chance_percent == 0` or `victim == EMPTY` are inactive. Rules are checked in order per
/// neighbor; the first matching `victim` that succeeds the roll applies.
///
/// Neighbor HP is stored in [`Cell::lifetime`]; see [`MaterialProps::corrosion_max_hp`] on the victim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrosionAdjacentRule {
    /// Neighbor material this rule applies to.
    pub victim: MaterialId,
    /// 0 = disabled. Otherwise `roll in 0..100` must be `< chance_percent` (same as [`AdjacentTransformRule`]).
    pub chance_percent: u8,
    /// Subtracted from neighbor [`Cell::lifetime`] when `corrosion_max_hp > 0`; one-shot erase when `0`.
    pub neighbor_damage: u8,
    /// Subtracted from this corrosive cell's [`Cell::lifetime`] after a successful hit.
    pub self_lifetime_cost: u8,
}

impl CorrosionAdjacentRule {
    pub const fn inactive() -> Self {
        Self {
            victim: material::EMPTY,
            chance_percent: 0,
            neighbor_damage: 0,
            self_lifetime_cost: 0,
        }
    }

    #[inline]
    pub const fn is_active(self) -> bool {
        self.victim != material::EMPTY && self.chance_percent > 0
    }
}

impl Default for CorrosionAdjacentRule {
    fn default() -> Self {
        Self::inactive()
    }
}

pub type CorrosionAdjacentRules = [CorrosionAdjacentRule; MAX_CORROSION_ADJACENT_RULES];

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
    pub require_victim_flags_any: u16,
    /// If non-zero, skip when `(victim.flags & mask) != 0` (e.g. do not wet already-wet sand).
    pub exclude_victim_flags_any: u16,
    pub flags_or: u16,
    pub flags_clear: u16,
    pub victim_lifetime: InfluenceVictimLifetime,
    pub source_effect: InfluenceSourceEffect,
    /// If non-zero: when this mask goes from set to clear, replace victim with `spawn_on_clear_material`.
    pub if_cleared_mask: u16,
    pub spawn_on_clear_material: MaterialId,
    /// If non-zero: when this mask goes from clear to set, replace victim with `spawn_on_set_material`.
    pub if_set_mask: u16,
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
    pub const IS_FREE_FALLING: u16 = 1 << 0;
    pub const ON_FIRE: u16 = 1 << 1;
    pub const WET: u16 = 1 << 2;
    pub const ELECTRIFIED: u16 = 1 << 3;
}

#[derive(Debug, Clone, Copy)]
pub struct MaterialProps {
    pub density: i16,
    pub motion: MaterialMotion,
    /// 0 = never ignites. 255 = always ignites from adjacent `FIRE` (skips heat-scaled roll).
    pub ignitability: u8,
    /// While `ON_FIRE`, probability weight out of 256 to lose 1 `lifetime` (fuel HP) per tick.
    pub consumption_rate: u8,
    /// If > 0, ignition sets `ON_FIRE` and seeds `lifetime` with this value instead of immediate replacement.
    /// If 0, heat replaces the cell using `smolder_burnout_become` + `smolder_burnout_lifetime_*` (same as smolder burnout); when those are unset, becomes `FIRE` with heat-weakened lifetime.
    pub fuel_mass: u8,
    pub explosion_radius: u8,
    pub on_heat_become: MaterialId,
    pub on_death_become: MaterialId,
    /// Half-open range for replacement [`Cell::lifetime`] when this material dies into `on_death_become`
    /// (e.g. `FIRE` → smoke). If `hi <= lo`, the sim uses a built-in default range instead.
    pub on_death_lifetime_lo: u8,
    pub on_death_lifetime_hi: u8,
    /// For materials that take multiple corrosion hits: max HP stored in [`Cell::lifetime`] when painted.
    /// `0` = first successful corrosive hit erases the cell (no bar). Corrosion rules live on the acid material.
    pub corrosion_max_hp: u8,

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

    /// Cardinal corrosion: which neighbor materials this **corrosive liquid** damages and at what odds/cost.
    pub corrosion_adjacent: CorrosionAdjacentRules,

    /// Neighbor flag/lifetime rules (fire spread, water wetting, drying wet sand, etc.).
    pub adjacent_influence: AdjacentInfluenceRules,

    /// Spawn materials into random empty 8-neighbors (smolder, ember, and inert props like torches).
    pub neighbor_spawns: NeighborSpawnRules,
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
            explosion_radius: 0,
            on_heat_become: material::EMPTY,
            on_death_become: material::EMPTY,
            on_death_lifetime_lo: 0,
            on_death_lifetime_hi: 0,
            corrosion_max_hp: 0,
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
            corrosion_adjacent: [
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
                CorrosionAdjacentRule::inactive(),
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
    pub fn has_corrosion_adjacent(self) -> bool {
        self.corrosion_adjacent.iter().any(|r| r.is_active())
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

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub material: MaterialId,
    pub flags: u16,
    pub velocity: i8,
    pub lifetime: u8,
    pub variant: u8,
    pub scorch: u8,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 8);

const STATIC_CELL: Cell = Cell {
    material: material::STATIC,
    flags: 0,
    velocity: 0,
    lifetime: 0,
    variant: 0,
    scorch: 0,
};

impl Default for Cell {
    fn default() -> Self {
        Self {
            material: material::EMPTY,
            flags: 0,
            velocity: 0,
            lifetime: 0,
            variant: 0,
            scorch: 0,
        }
    }
}

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
    chunk_size: i32,
    cells: Vec<(ChunkCoord, Vec<Cell>)>,
}

pub struct World {
    chunk_size: i32,
    region_size: i32,

    grids: [Vec<Cell>; 2],
    read_idx: usize,
    grid_width: i32,
    grid_height: i32,
    grid_origin: Vec2i,

    chunks_x: i32,
    chunks_y: i32,
    chunk_dirty: Vec<bool>,
    chunk_awake: Vec<bool>,
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
}

impl World {
    pub fn new(chunk_size: i32, region_size: i32) -> Self {
        let w = region_size;
        let h = region_size;
        let origin = Vec2i::new(0, 0);
        let total = (w * h) as usize;
        let cx = (w + chunk_size - 1) / chunk_size;
        let cy = (h + chunk_size - 1) / chunk_size;

        Self {
            chunk_size,
            region_size,
            grids: [vec![Cell::default(); total], vec![Cell::default(); total]],
            read_idx: 0,
            grid_width: w,
            grid_height: h,
            grid_origin: origin,
            chunks_x: cx,
            chunks_y: cy,
            chunk_dirty: vec![true; (cx * cy) as usize],
            chunk_awake: vec![true; (cx * cy) as usize],
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
        }
    }

    pub fn chunk_size(&self) -> i32 {
        self.chunk_size
    }

    pub fn grid_width(&self) -> i32 {
        self.grid_width
    }

    pub fn grid_height(&self) -> i32 {
        self.grid_height
    }

    pub fn grid_origin(&self) -> Vec2i {
        self.grid_origin
    }

    pub fn set_storage_dir(&mut self, dir: PathBuf) {
        self.storage_dir = Some(dir);
    }

    pub fn set_solid_bounds(&mut self, bounds: RectI) {
        self.solid_bounds = Some(bounds);
        let new_w = bounds.max.x - bounds.min.x + 1;
        let new_h = bounds.max.y - bounds.min.y + 1;
        if new_w != self.grid_width || new_h != self.grid_height || bounds.min != self.grid_origin {
            let total = (new_w * new_h) as usize;
            self.grids = [vec![Cell::default(); total], vec![Cell::default(); total]];
            self.grid_width = new_w;
            self.grid_height = new_h;
            self.grid_origin = bounds.min;
            let cx = (new_w + self.chunk_size - 1) / self.chunk_size;
            let cy = (new_h + self.chunk_size - 1) / self.chunk_size;
            self.chunks_x = cx;
            self.chunks_y = cy;
            let n = (cx * cy) as usize;
            self.chunk_dirty = vec![true; n];
            self.chunk_awake = vec![true; n];
            self.chunk_awake_next = vec![false; n];
        }
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

    pub fn get_rigid_id(&self, p: Vec2i) -> Option<u32> {
        self.rigid_ids.get(&p).copied()
    }

    pub fn set_rigid_id(&mut self, p: Vec2i, id: u32) {
        self.rigid_ids.insert(p, id);
    }

    pub fn clear_rigid_id(&mut self, p: Vec2i) {
        self.rigid_ids.remove(&p);
    }

    pub fn active_chunk_count(&self) -> usize {
        self.chunk_awake.iter().filter(|&&a| a).count()
    }

    pub fn sleeping_chunk_count(&self) -> usize {
        self.chunk_awake.iter().filter(|&&a| !a).count()
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
            self.chunk_awake[i] = self.chunk_awake_next[i];
            self.chunk_awake_next[i] = false;
        }
    }

    pub fn wake_chunk_at(&mut self, p: Vec2i) {
        let cx = div_floor(p.x - self.grid_origin.x, self.chunk_size);
        let cy = div_floor(p.y - self.grid_origin.y, self.chunk_size);
        self.wake_chunk_local(cx, cy);
    }

    fn wake_chunk_local(&mut self, cx: i32, cy: i32) {
        if let Some(idx) = self.chunk_local_index(cx, cy) {
            self.chunk_awake_next[idx] = true;
        }
    }

    pub fn wake_chunk_and_neighbors(&mut self, p: Vec2i) {
        let cx = div_floor(p.x - self.grid_origin.x, self.chunk_size);
        let cy = div_floor(p.y - self.grid_origin.y, self.chunk_size);
        for dy in -1..=1 {
            for dx in -1..=1 {
                self.wake_chunk_local(cx + dx, cy + dy);
            }
        }
    }

    pub fn is_chunk_awake(&self, coord: ChunkCoord) -> bool {
        let base_cx = div_floor(self.grid_origin.x, self.chunk_size);
        let base_cy = div_floor(self.grid_origin.y, self.chunk_size);
        let cx = coord.x - base_cx;
        let cy = coord.y - base_cy;
        self.chunk_local_index(cx, cy)
            .map(|i| self.chunk_awake[i])
            .unwrap_or(false)
    }

    pub fn chunk_awake_ptr(&self) -> *const bool {
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
        if enabled && self.debug_pass.len() != (self.grid_width * self.grid_height) as usize {
            self.debug_pass = vec![0xFF; (self.grid_width * self.grid_height) as usize];
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

    pub fn set_focus(&mut self, focus: Vec2i) -> Vec<WorldEvent> {
        self.focus = focus;
        self.sync_streaming_regions()
    }

    fn grid_index(&self, p: Vec2i) -> Option<usize> {
        let x = p.x - self.grid_origin.x;
        let y = p.y - self.grid_origin.y;
        if x < 0 || x >= self.grid_width || y < 0 || y >= self.grid_height {
            return None;
        }
        Some((y * self.grid_width + x) as usize)
    }

    pub fn get_cell(&self, p: Vec2i) -> Cell {
        match self.grid_index(p) {
            Some(idx) => self.grids[self.read_idx][idx],
            None => STATIC_CELL,
        }
    }

    pub fn set_cell(&mut self, p: Vec2i, cell: Cell) {
        if let Some(idx) = self.grid_index(p) {
            self.grids[self.read_idx][idx] = cell;
            self.mark_chunk_dirty_for(p);
            self.wake_chunk_and_neighbors(p);
        }
    }

    fn mark_chunk_dirty_for(&mut self, p: Vec2i) {
        let cx = div_floor(p.x - self.grid_origin.x, self.chunk_size);
        let cy = div_floor(p.y - self.grid_origin.y, self.chunk_size);
        if cx >= 0 && cx < self.chunks_x && cy >= 0 && cy < self.chunks_y {
            self.chunk_dirty[(cy * self.chunks_x + cx) as usize] = true;
        }
    }

    pub fn paint_circle(&mut self, center: Vec2i, radius: i32, mat: MaterialId) {
        let props = self.material_props(mat);
        let initial_lifetime = Self::initial_lifetime_for(mat, &props);
        let (paint_flags, paint_lifetime) = if mat == material::LAVA && props.fuel_mass > 0 {
            (cell_flags::ON_FIRE, props.fuel_mass)
        } else {
            (0, initial_lifetime)
        };
        let r2 = radius * radius;
        for y in (center.y - radius)..=(center.y + radius) {
            for x in (center.x - radius)..=(center.x + radius) {
                let dx = x - center.x;
                let dy = y - center.y;
                if dx * dx + dy * dy <= r2 {
                    self.set_cell(
                        Vec2i::new(x, y),
                        Cell {
                            material: mat,
                            flags: paint_flags,
                            velocity: 0,
                            lifetime: paint_lifetime,
                            variant: 0,
                            scorch: 0,
                        },
                    );
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
        0
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

    // --- Double-buffer lifecycle ---

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

    pub fn read_cells(&self) -> &[Cell] {
        &self.grids[self.read_idx]
    }

    pub fn write_cells_mut(&mut self) -> &mut [Cell] {
        let write_idx = 1 - self.read_idx;
        &mut self.grids[write_idx]
    }

    pub fn write_cells_ptr(&mut self) -> *mut Cell {
        let write_idx = 1 - self.read_idx;
        self.grids[write_idx].as_mut_ptr()
    }

    pub fn write_cells_len(&self) -> usize {
        self.grids[1 - self.read_idx].len()
    }

    // --- Chunk queries ---

    pub fn all_chunk_dirty_rects(&self) -> Vec<(ChunkCoord, RectI)> {
        let mut out = Vec::new();
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                let idx = (cy * self.chunks_x + cx) as usize;
                if self.chunk_dirty[idx] {
                    let coord = ChunkCoord {
                        x: cx + self.grid_origin.x / self.chunk_size,
                        y: cy + self.grid_origin.y / self.chunk_size,
                    };
                    let min_x = self.grid_origin.x + cx * self.chunk_size;
                    let min_y = self.grid_origin.y + cy * self.chunk_size;
                    let max_x = (min_x + self.chunk_size - 1).min(self.grid_origin.x + self.grid_width - 1);
                    let max_y = (min_y + self.chunk_size - 1).min(self.grid_origin.y + self.grid_height - 1);
                    out.push((coord, RectI::new(Vec2i::new(min_x, min_y), Vec2i::new(max_x, max_y))));
                }
            }
        }
        out
    }

    pub fn active_chunk_coords_for_pass(&self, pass: usize) -> Vec<ChunkCoord> {
        let px = (pass & 1) as i32;
        let py = ((pass >> 1) & 1) as i32;
        let base_cx = div_floor(self.grid_origin.x, self.chunk_size);
        let base_cy = div_floor(self.grid_origin.y, self.chunk_size);
        let mut coords = Vec::new();
        for cy in 0..self.chunks_y {
            for cx in 0..self.chunks_x {
                let abs_cx = base_cx + cx;
                let abs_cy = base_cy + cy;
                if (abs_cx & 1) == px && (abs_cy & 1) == py {
                    coords.push(ChunkCoord { x: abs_cx, y: abs_cy });
                }
            }
        }
        coords.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
        coords
    }

    pub fn bounds_for_chunk(&self, coord: ChunkCoord) -> RectI {
        let min = Vec2i::new(coord.x * self.chunk_size, coord.y * self.chunk_size);
        let max = Vec2i::new(min.x + self.chunk_size - 1, min.y + self.chunk_size - 1);
        RectI::new(min, max)
    }

    pub fn finish_frame(&mut self) {
        for d in self.chunk_dirty.iter_mut() {
            *d = true;
        }
        let _ = self.sync_streaming_regions();
    }

    pub fn dirty_world_bounds(&self) -> Option<RectI> {
        if self.grid_width > 0 && self.grid_height > 0 {
            Some(RectI::new(
                self.grid_origin,
                Vec2i::new(
                    self.grid_origin.x + self.grid_width - 1,
                    self.grid_origin.y + self.grid_height - 1,
                ),
            ))
        } else {
            None
        }
    }

    pub fn clear_outside_rect(&mut self, bounds: RectI) {
        for y in self.grid_origin.y..(self.grid_origin.y + self.grid_height) {
            for x in self.grid_origin.x..(self.grid_origin.x + self.grid_width) {
                let p = Vec2i::new(x, y);
                if !bounds.contains(p) {
                    if let Some(idx) = self.grid_index(p) {
                        if self.grids[self.read_idx][idx].material != material::EMPTY {
                            self.grids[self.read_idx][idx] = Cell::default();
                        }
                    }
                }
            }
        }
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
                let mut cells = vec![Cell::default(); (cs * cs) as usize];
                for ly in 0..cs {
                    for lx in 0..cs {
                        let p = Vec2i::new(cx * cs + lx, cy * cs + ly);
                        if let Some(idx) = self.grid_index(p) {
                            cells[(ly * cs + lx) as usize] = self.grids[self.read_idx][idx];
                        }
                    }
                }
                if cells.iter().any(|c| c.material != material::EMPTY) {
                    chunk_cells.push((coord, cells));
                }
            }
        }

        let payload = RegionFile {
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
            for ly in 0..cs {
                for lx in 0..cs {
                    let p = Vec2i::new(coord.x * cs + lx, coord.y * cs + ly);
                    let cell = cells[(ly * cs + lx) as usize];
                    if cell.material != material::EMPTY {
                        if let Some(idx) = self.grid_index(p) {
                            self.grids[self.read_idx][idx] = cell;
                        }
                    }
                }
            }
        }
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
