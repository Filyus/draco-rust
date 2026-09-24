//! The BC4 block layout, shared by the conversions that write it.
//!
//! Eight bytes: two endpoints and sixteen three-bit selectors. Ported from
//! `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0; the layout
//! is `dxt5a_block` in `transcoder/basisu_transcoder.cpp`.

/// One BC4 block: two endpoints and sixteen three-bit selectors.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bc4Block {
    pub(crate) low: u8,
    pub(crate) high: u8,
    pub(crate) selectors: [u8; 6],
}

impl Bc4Block {
    /// The eight bytes a GPU expects.
    pub fn to_bytes(self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0] = self.low;
        bytes[1] = self.high;
        bytes[2..8].copy_from_slice(&self.selectors);
        bytes
    }

    // Only the ETC1S conversion writes selectors one at a time; UASTC packs
    // all sixteen at once. A build without ETC1S would otherwise fail on an
    // unused method.
    #[cfg_attr(not(feature = "etc1s"), allow(dead_code))]
    pub(crate) fn set_selector(&mut self, texel: usize, value: u8) {
        let bit = texel * 3;
        let byte = bit >> 3;
        let offset = bit & 7;
        let mut window = self.selectors[byte] as u32;
        if byte < 5 {
            window |= (self.selectors[byte + 1] as u32) << 8;
        }
        window &= !(7u32 << offset);
        window |= (value as u32) << offset;
        self.selectors[byte] = window as u8;
        if byte < 5 {
            self.selectors[byte + 1] = (window >> 8) as u8;
        }
    }
}
