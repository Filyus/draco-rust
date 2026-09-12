//! The EAC R11 block layout, shared by the conversions that write it.
//!
//! Eight bytes: a base value, a table and multiplier, and sixteen three-bit
//! selectors packed big-endian. Ported from `BinomialLLC/basis_universal`,
//! revision `9bebe16`, Apache-2.0; the layout is `eac_block` in
//! `transcoder/basisu_transcoder.cpp`.

/// The selector pattern that reads as the middle step on every texel.
///
/// Three bits per texel, all fours, which is how EAC says "base plus the
/// table's fourth modifier" — the reference writes this byte pattern for
/// constant blocks rather than packing a zero.
const ALL_FOURS: [u8; 6] = [0x92, 0x49, 0x24, 0x92, 0x49, 0x24];

/// One EAC R11 block: a base value, a table and multiplier, and sixteen
/// three-bit selectors.
#[derive(Debug, Clone, Copy, Default)]
pub struct EacR11Block {
    pub(crate) base: u8,
    pub(crate) table: u8,
    pub(crate) multiplier: u8,
    pub(crate) selectors: [u8; 6],
}

impl EacR11Block {
    /// Take the eight bytes a GPU expects back apart again, so one target
    /// can reuse another's output.
    pub(crate) fn from_bytes(bytes: [u8; 8]) -> Self {
        EacR11Block {
            base: bytes[0],
            table: bytes[1] & 15,
            multiplier: bytes[1] >> 4,
            selectors: bytes[2..8].try_into().unwrap(),
        }
    }

    /// The eight bytes a GPU expects.
    pub fn to_bytes(self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0] = self.base;
        bytes[1] = (self.multiplier << 4) | (self.table & 15);
        bytes[2..8].copy_from_slice(&self.selectors);
        bytes
    }

    pub(crate) fn set_selector_bits(&mut self, bits: u64) {
        // The 48-bit selector field is laid out big-endian: the texel that
        // sits at bit 45 lands in the top three bits of the first byte.
        self.selectors.copy_from_slice(&bits.to_be_bytes()[2..8]);
    }

    /// A block that reads back `value` on every texel.
    ///
    /// The ETC1S conversion reaches a constant through table 13 and
    /// multiplier 1, which puts the middle step on the base exactly; the
    /// reference's block packer writes multiplier 0 for the same read-back,
    /// and the two are not the same bytes. [`EacR11Block::solid`] is that one.
    pub fn constant(value: u8) -> Self {
        EacR11Block {
            base: value,
            table: 13,
            multiplier: 1,
            selectors: ALL_FOURS,
        }
    }

    /// A constant block as the reference's packer writes it: table 13 and
    /// multiplier 0, which also lands every step on the base.
    pub(crate) fn solid(value: u8) -> Self {
        EacR11Block {
            base: value,
            table: 13,
            multiplier: 0,
            selectors: ALL_FOURS,
        }
    }
}
