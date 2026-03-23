use crate::world::Cell;

pub const TYPE_BITS: u32 = 10;
pub const CTYPE_BITS: u32 = 10;
pub const VX_BITS: u32 = 4;
pub const VY_BITS: u32 = 4;
pub const LIFETIME_BITS: u32 = 8;
pub const FRAME_BITS: u32 = 2;
pub const FLAGS_BITS: u32 = 26;

const TYPE_SHIFT: u32 = 0;
const CTYPE_SHIFT: u32 = TYPE_SHIFT + TYPE_BITS;
const VX_SHIFT: u32 = CTYPE_SHIFT + CTYPE_BITS;
const VY_SHIFT: u32 = VX_SHIFT + VX_BITS;
const LIFETIME_SHIFT: u32 = VY_SHIFT + VY_BITS;
const FRAME_SHIFT: u32 = LIFETIME_SHIFT + LIFETIME_BITS;
const FLAGS_SHIFT: u32 = FRAME_SHIFT + FRAME_BITS;

const TYPE_MASK: u64 = ((1u64 << TYPE_BITS) - 1) << TYPE_SHIFT;
const CTYPE_MASK: u64 = ((1u64 << CTYPE_BITS) - 1) << CTYPE_SHIFT;
const VX_MASK: u64 = ((1u64 << VX_BITS) - 1) << VX_SHIFT;
const VY_MASK: u64 = ((1u64 << VY_BITS) - 1) << VY_SHIFT;
const LIFETIME_MASK: u64 = ((1u64 << LIFETIME_BITS) - 1) << LIFETIME_SHIFT;
const FRAME_MASK: u64 = ((1u64 << FRAME_BITS) - 1) << FRAME_SHIFT;
const FLAGS_MASK: u64 = ((1u64 << FLAGS_BITS) - 1) << FLAGS_SHIFT;

pub const MAX_PACKED_MATERIAL_ID: u16 = 1023;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackedCell(pub u64);

impl PackedCell {
    #[inline]
    pub fn new() -> Self {
        Self(0)
    }

    #[inline]
    pub fn material_type(self) -> u16 {
        ((self.0 & TYPE_MASK) >> TYPE_SHIFT) as u16
    }

    #[inline]
    pub fn set_material_type(&mut self, id: u16) {
        debug_assert!(id <= MAX_PACKED_MATERIAL_ID);
        self.0 = (self.0 & !TYPE_MASK) | (((id as u64) << TYPE_SHIFT) & TYPE_MASK);
    }

    #[inline]
    pub fn ctype(self) -> u16 {
        ((self.0 & CTYPE_MASK) >> CTYPE_SHIFT) as u16
    }

    #[inline]
    pub fn set_ctype(&mut self, id: u16) {
        debug_assert!(id <= MAX_PACKED_MATERIAL_ID);
        self.0 = (self.0 & !CTYPE_MASK) | (((id as u64) << CTYPE_SHIFT) & CTYPE_MASK);
    }

    #[inline]
    pub fn velocity_x(self) -> i8 {
        decode_signed_4(((self.0 & VX_MASK) >> VX_SHIFT) as u8)
    }

    #[inline]
    pub fn set_velocity_x(&mut self, vx: i8) {
        let nibble = encode_signed_4(vx);
        self.0 = (self.0 & !VX_MASK) | (((nibble as u64) << VX_SHIFT) & VX_MASK);
    }

    #[inline]
    pub fn velocity_y(self) -> i8 {
        decode_signed_4(((self.0 & VY_MASK) >> VY_SHIFT) as u8)
    }

    #[inline]
    pub fn set_velocity_y(&mut self, vy: i8) {
        let nibble = encode_signed_4(vy);
        self.0 = (self.0 & !VY_MASK) | (((nibble as u64) << VY_SHIFT) & VY_MASK);
    }

    #[inline]
    pub fn lifetime_data(self) -> u8 {
        ((self.0 & LIFETIME_MASK) >> LIFETIME_SHIFT) as u8
    }

    #[inline]
    pub fn set_lifetime_data(&mut self, lifetime: u8) {
        self.0 = (self.0 & !LIFETIME_MASK) | (((lifetime as u64) << LIFETIME_SHIFT) & LIFETIME_MASK);
    }

    #[inline]
    pub fn frame_tracker(self) -> u8 {
        ((self.0 & FRAME_MASK) >> FRAME_SHIFT) as u8
    }

    #[inline]
    pub fn set_frame_tracker(&mut self, frame: u8) {
        let v = frame & 0b11;
        self.0 = (self.0 & !FRAME_MASK) | (((v as u64) << FRAME_SHIFT) & FRAME_MASK);
    }

    #[inline]
    pub fn flags(self) -> u32 {
        ((self.0 & FLAGS_MASK) >> FLAGS_SHIFT) as u32
    }

    #[inline]
    pub fn set_flags(&mut self, flags: u32) {
        let v = flags & ((1u32 << FLAGS_BITS) - 1);
        self.0 = (self.0 & !FLAGS_MASK) | (((v as u64) << FLAGS_SHIFT) & FLAGS_MASK);
    }

    /// Compatibility adapter for current `Cell` layout.
    ///
    /// Mapping:
    /// - `type` <- `cell.material`
    /// - `ctype` <- combine `variant` (low byte) and `scorch` (high byte), clamped to u10
    /// - `vel_x` <- legacy scalar `velocity` (clamped to [-8, 7]); `vel_y = 0`
    /// - `lifetime_data` <- `cell.lifetime`
    /// - `flags` <- lower bits from `cell.flags`; `frame_tracker = 0`
    #[inline]
    pub fn from_legacy_cell(cell: Cell) -> Self {
        let mut out = Self::new();
        out.set_material_type(cell.material.min(MAX_PACKED_MATERIAL_ID));
        let ctype16 = u16::from(cell.variant) | (u16::from(cell.scorch) << 8);
        out.set_ctype(ctype16.min(MAX_PACKED_MATERIAL_ID));
        out.set_velocity_x(cell.velocity.clamp(-8, 7));
        out.set_velocity_y(0);
        out.set_lifetime_data(cell.lifetime);
        out.set_frame_tracker(0);
        out.set_flags(u32::from(cell.flags));
        out
    }

    /// Compatibility adapter to current `Cell` layout.
    ///
    /// Reverse mapping preserves existing fields where possible, but this conversion is lossy with
    /// respect to `vel_y` and high-precision expanded flags.
    #[inline]
    pub fn to_legacy_cell(self) -> Cell {
        let ctype = self.ctype();
        Cell {
            material: self.material_type(),
            flags: self.flags() as u16,
            velocity: self.velocity_x(),
            lifetime: self.lifetime_data(),
            variant: (ctype & 0x00FF) as u8,
            scorch: ((ctype >> 8) & 0x00FF) as u8,
        }
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

const _: () = assert!(std::mem::size_of::<PackedCell>() == 8);
const _: () = assert!(TYPE_BITS + CTYPE_BITS + VX_BITS + VY_BITS + LIFETIME_BITS + FRAME_BITS + FLAGS_BITS == 64);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::material;

    #[test]
    fn signed_nibble_roundtrip() {
        for v in -8..=7 {
            assert_eq!(decode_signed_4(encode_signed_4(v)), v);
        }
    }

    #[test]
    fn pack_unpack_fields() {
        let mut c = PackedCell::new();
        c.set_material_type(511);
        c.set_ctype(777);
        c.set_velocity_x(-3);
        c.set_velocity_y(6);
        c.set_lifetime_data(123);
        c.set_frame_tracker(3);
        c.set_flags(0x03FF_FFFF);

        assert_eq!(c.material_type(), 511);
        assert_eq!(c.ctype(), 777);
        assert_eq!(c.velocity_x(), -3);
        assert_eq!(c.velocity_y(), 6);
        assert_eq!(c.lifetime_data(), 123);
        assert_eq!(c.frame_tracker(), 3);
        assert_eq!(c.flags(), 0x03FF_FFFF);
    }

    #[test]
    fn legacy_adapter_roundtrip() {
        let legacy = Cell {
            material: material::LAVA,
            flags: 0b1010_0011_0101,
            velocity: -5,
            lifetime: 200,
            variant: 12,
            scorch: 2,
        };
        let packed = PackedCell::from_legacy_cell(legacy);
        let back = packed.to_legacy_cell();
        assert_eq!(back.material, legacy.material);
        assert_eq!(back.flags, legacy.flags);
        assert_eq!(back.velocity, legacy.velocity.clamp(-8, 7));
        assert_eq!(back.lifetime, legacy.lifetime);
        assert_eq!(back.variant, legacy.variant);
        assert_eq!(back.scorch, legacy.scorch);
    }
}
