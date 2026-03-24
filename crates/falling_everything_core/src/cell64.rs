use serde::{Deserialize, Serialize};

pub const MATERIAL_BITS: u32 = 10;
pub const TEMPERATURE_BITS: u32 = 12;
pub const VX_BITS: u32 = 4;
pub const VY_BITS: u32 = 4;
pub const LIFETIME_BITS: u32 = 8;
pub const VARIANT_BITS: u32 = 2;
pub const FLAGS_BITS: u32 = 8;
pub const FRAME_BITS: u32 = 2;
pub const RIGID_SOURCE_MAT_BITS: u32 = 10;
pub const SPARE_BITS: u32 = 4;

const MATERIAL_SHIFT: u32 = 0;
const TEMPERATURE_SHIFT: u32 = MATERIAL_SHIFT + MATERIAL_BITS;
const VX_SHIFT: u32 = TEMPERATURE_SHIFT + TEMPERATURE_BITS;
const VY_SHIFT: u32 = VX_SHIFT + VX_BITS;
const LIFETIME_SHIFT: u32 = VY_SHIFT + VY_BITS;
const VARIANT_SHIFT: u32 = LIFETIME_SHIFT + LIFETIME_BITS;
const FLAGS_SHIFT: u32 = VARIANT_SHIFT + VARIANT_BITS;
const FRAME_SHIFT: u32 = FLAGS_SHIFT + FLAGS_BITS;
const RIGID_SOURCE_MAT_SHIFT: u32 = FRAME_SHIFT + FRAME_BITS;

const MATERIAL_MASK: u64 = ((1u64 << MATERIAL_BITS) - 1) << MATERIAL_SHIFT;
const TEMPERATURE_MASK: u64 = ((1u64 << TEMPERATURE_BITS) - 1) << TEMPERATURE_SHIFT;
const VX_MASK: u64 = ((1u64 << VX_BITS) - 1) << VX_SHIFT;
const VY_MASK: u64 = ((1u64 << VY_BITS) - 1) << VY_SHIFT;
const LIFETIME_MASK: u64 = ((1u64 << LIFETIME_BITS) - 1) << LIFETIME_SHIFT;
const VARIANT_MASK: u64 = ((1u64 << VARIANT_BITS) - 1) << VARIANT_SHIFT;
const FLAGS_MASK: u64 = ((1u64 << FLAGS_BITS) - 1) << FLAGS_SHIFT;
const FRAME_MASK: u64 = ((1u64 << FRAME_BITS) - 1) << FRAME_SHIFT;
const RIGID_SOURCE_MAT_MASK: u64 = ((1u64 << RIGID_SOURCE_MAT_BITS) - 1) << RIGID_SOURCE_MAT_SHIFT;

pub const MAX_MATERIAL_ID: u16 = (1 << MATERIAL_BITS) - 1;
pub const MAX_TEMPERATURE: u16 = (1 << TEMPERATURE_BITS) - 1;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[repr(transparent)]
pub struct Cell(pub u64);

impl std::fmt::Debug for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cell")
            .field("material", &self.material())
            .field("temperature", &self.temperature())
            .field("vx", &self.velocity_x())
            .field("vy", &self.velocity_y())
            .field("lifetime", &self.lifetime())
            .field("variant", &self.variant())
            .field("flags", &self.flags())
            .field("frame", &self.frame_tracker())
            .finish()
    }
}

impl Cell {
    #[inline]
    pub const fn new() -> Self {
        Self(0)
    }

    // --- Accessors ---

    #[inline]
    pub const fn material(self) -> u16 {
        ((self.0 & MATERIAL_MASK) >> MATERIAL_SHIFT) as u16
    }

    #[inline]
    pub const fn temperature(self) -> u16 {
        ((self.0 & TEMPERATURE_MASK) >> TEMPERATURE_SHIFT) as u16
    }

    #[inline]
    pub fn velocity_x(self) -> i8 {
        decode_signed_4(((self.0 & VX_MASK) >> VX_SHIFT) as u8)
    }

    #[inline]
    pub fn velocity_y(self) -> i8 {
        decode_signed_4(((self.0 & VY_MASK) >> VY_SHIFT) as u8)
    }

    #[inline]
    pub const fn lifetime(self) -> u8 {
        ((self.0 & LIFETIME_MASK) >> LIFETIME_SHIFT) as u8
    }

    #[inline]
    pub const fn variant(self) -> u8 {
        ((self.0 & VARIANT_MASK) >> VARIANT_SHIFT) as u8
    }

    #[inline]
    pub const fn flags(self) -> u8 {
        ((self.0 & FLAGS_MASK) >> FLAGS_SHIFT) as u8
    }

    #[inline]
    pub const fn frame_tracker(self) -> u8 {
        ((self.0 & FRAME_MASK) >> FRAME_SHIFT) as u8
    }

    /// Original material stored in spare bits when a rigid body cell becomes STATIC proxy.
    /// Returns 0 (EMPTY) for non-rigid cells.
    #[inline]
    pub const fn rigid_source_material(self) -> u16 {
        ((self.0 & RIGID_SOURCE_MAT_MASK) >> RIGID_SOURCE_MAT_SHIFT) as u16
    }

    // --- Setters (in-place mutation) ---

    #[inline]
    pub fn set_material(&mut self, id: u16) {
        debug_assert!(id <= MAX_MATERIAL_ID);
        self.0 = (self.0 & !MATERIAL_MASK) | (((id as u64) << MATERIAL_SHIFT) & MATERIAL_MASK);
    }

    #[inline]
    pub fn set_temperature(&mut self, t: u16) {
        let t = t.min(MAX_TEMPERATURE);
        self.0 = (self.0 & !TEMPERATURE_MASK) | (((t as u64) << TEMPERATURE_SHIFT) & TEMPERATURE_MASK);
    }

    #[inline]
    pub fn set_velocity_x(&mut self, vx: i8) {
        let nibble = encode_signed_4(vx);
        self.0 = (self.0 & !VX_MASK) | (((nibble as u64) << VX_SHIFT) & VX_MASK);
    }

    #[inline]
    pub fn set_velocity_y(&mut self, vy: i8) {
        let nibble = encode_signed_4(vy);
        self.0 = (self.0 & !VY_MASK) | (((nibble as u64) << VY_SHIFT) & VY_MASK);
    }

    #[inline]
    pub fn set_lifetime(&mut self, lifetime: u8) {
        self.0 = (self.0 & !LIFETIME_MASK) | (((lifetime as u64) << LIFETIME_SHIFT) & LIFETIME_MASK);
    }

    #[inline]
    pub fn set_variant(&mut self, variant: u8) {
        let v = variant & 0b11;
        self.0 = (self.0 & !VARIANT_MASK) | (((v as u64) << VARIANT_SHIFT) & VARIANT_MASK);
    }

    #[inline]
    pub fn set_flags(&mut self, flags: u8) {
        self.0 = (self.0 & !FLAGS_MASK) | (((flags as u64) << FLAGS_SHIFT) & FLAGS_MASK);
    }

    #[inline]
    pub fn set_frame_tracker(&mut self, frame: u8) {
        let v = frame & 0b11;
        self.0 = (self.0 & !FRAME_MASK) | (((v as u64) << FRAME_SHIFT) & FRAME_MASK);
    }

    #[inline]
    pub fn set_rigid_source_material(&mut self, id: u16) {
        debug_assert!(id <= MAX_MATERIAL_ID);
        self.0 = (self.0 & !RIGID_SOURCE_MAT_MASK) | (((id as u64) << RIGID_SOURCE_MAT_SHIFT) & RIGID_SOURCE_MAT_MASK);
    }

    // --- Flag helpers ---

    #[inline]
    pub fn has_flag(self, flag: u8) -> bool {
        self.flags() & flag != 0
    }

    #[inline]
    pub fn or_flags(&mut self, flag: u8) {
        self.set_flags(self.flags() | flag);
    }

    #[inline]
    pub fn clear_flag_bits(&mut self, flag: u8) {
        self.set_flags(self.flags() & !flag);
    }

    // --- Builder pattern ---

    #[inline]
    pub const fn with_material(mut self, id: u16) -> Self {
        self.0 = (self.0 & !MATERIAL_MASK) | (((id as u64) << MATERIAL_SHIFT) & MATERIAL_MASK);
        self
    }

    #[inline]
    pub const fn with_temperature(mut self, t: u16) -> Self {
        let t_clamped = if t > MAX_TEMPERATURE { MAX_TEMPERATURE } else { t };
        self.0 = (self.0 & !TEMPERATURE_MASK) | (((t_clamped as u64) << TEMPERATURE_SHIFT) & TEMPERATURE_MASK);
        self
    }

    #[inline]
    pub fn with_velocity_x(mut self, vx: i8) -> Self {
        self.set_velocity_x(vx);
        self
    }

    #[inline]
    pub fn with_velocity_y(mut self, vy: i8) -> Self {
        self.set_velocity_y(vy);
        self
    }

    #[inline]
    pub const fn with_lifetime(mut self, lifetime: u8) -> Self {
        self.0 = (self.0 & !LIFETIME_MASK) | (((lifetime as u64) << LIFETIME_SHIFT) & LIFETIME_MASK);
        self
    }

    #[inline]
    pub const fn with_variant(mut self, variant: u8) -> Self {
        let v = variant & 0b11;
        self.0 = (self.0 & !VARIANT_MASK) | (((v as u64) << VARIANT_SHIFT) & VARIANT_MASK);
        self
    }

    #[inline]
    pub const fn with_flags(mut self, flags: u8) -> Self {
        self.0 = (self.0 & !FLAGS_MASK) | (((flags as u64) << FLAGS_SHIFT) & FLAGS_MASK);
        self
    }

    #[inline]
    pub const fn with_frame_tracker(mut self, frame: u8) -> Self {
        let v = frame & 0b11;
        self.0 = (self.0 & !FRAME_MASK) | (((v as u64) << FRAME_SHIFT) & FRAME_MASK);
        self
    }

    #[inline]
    pub const fn with_rigid_source_material(mut self, id: u16) -> Self {
        self.0 = (self.0 & !RIGID_SOURCE_MAT_MASK) | (((id as u64) << RIGID_SOURCE_MAT_SHIFT) & RIGID_SOURCE_MAT_MASK);
        self
    }
}

#[inline]
fn decode_signed_4(n: u8) -> i8 {
    let v = n & 0x0F;
    if (v & 0x08) != 0 {
        (v as i8) - 16
    } else {
        v as i8
    }
}

#[inline]
fn encode_signed_4(v: i8) -> u8 {
    (v.clamp(-8, 7) as i16 as u8) & 0x0F
}

const _: () = assert!(std::mem::size_of::<Cell>() == 8);
const _: () = assert!(
    MATERIAL_BITS + TEMPERATURE_BITS + VX_BITS + VY_BITS + LIFETIME_BITS + VARIANT_BITS + FLAGS_BITS + FRAME_BITS + RIGID_SOURCE_MAT_BITS + SPARE_BITS == 64
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_nibble_roundtrip() {
        for v in -8..=7 {
            assert_eq!(decode_signed_4(encode_signed_4(v)), v);
        }
    }

    #[test]
    fn field_isolation() {
        let c = Cell::new()
            .with_material(1023)
            .with_temperature(4095)
            .with_lifetime(255)
            .with_variant(3)
            .with_flags(0xFF)
            .with_frame_tracker(3)
            .with_velocity_x(-8)
            .with_velocity_y(7)
            .with_rigid_source_material(999);

        assert_eq!(c.material(), 1023);
        assert_eq!(c.temperature(), 4095);
        assert_eq!(c.lifetime(), 255);
        assert_eq!(c.variant(), 3);
        assert_eq!(c.flags(), 0xFF);
        assert_eq!(c.frame_tracker(), 3);
        assert_eq!(c.velocity_x(), -8);
        assert_eq!(c.velocity_y(), 7);
        assert_eq!(c.rigid_source_material(), 999);
    }

    #[test]
    fn default_is_zero() {
        let c = Cell::new();
        assert_eq!(c.0, 0);
        assert_eq!(c.material(), 0);
        assert_eq!(c.temperature(), 0);
        assert_eq!(c.lifetime(), 0);
    }

    #[test]
    fn setter_mutates_in_place() {
        let mut c = Cell::new().with_material(5);
        assert_eq!(c.material(), 5);
        c.set_temperature(100);
        assert_eq!(c.temperature(), 100);
        assert_eq!(c.material(), 5);
        c.set_lifetime(42);
        assert_eq!(c.lifetime(), 42);
        assert_eq!(c.temperature(), 100);
    }

    #[test]
    fn flag_helpers() {
        let mut c = Cell::new();
        c.or_flags(0b0101);
        assert!(c.has_flag(0b0001));
        assert!(c.has_flag(0b0100));
        assert!(!c.has_flag(0b0010));
        c.clear_flag_bits(0b0001);
        assert!(!c.has_flag(0b0001));
        assert!(c.has_flag(0b0100));
    }

    #[test]
    fn temperature_clamps() {
        let c = Cell::new().with_temperature(9999);
        assert_eq!(c.temperature(), MAX_TEMPERATURE);
    }
}
