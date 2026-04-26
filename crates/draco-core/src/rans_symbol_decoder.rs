use crate::ans::AnsDecoder;
use crate::decoder_buffer::DecoderBuffer;
use crate::rans_symbol_coding::RAnsSymbol;

/// RAnsSymbolDecoder with runtime precision to avoid monomorphization bloat.
/// Instead of const generics, we store the precision bits at runtime.
/// Performance is preserved by storing `rans_precision_bits` and using bit
/// operations (shift/mask) instead of division/modulo.
pub struct RAnsSymbolDecoder<'a> {
    pub ans: AnsDecoder<'a>,
    probability_table: Vec<RAnsSymbol>,
    lut: Vec<u32>,
    num_symbols: usize,
    rans_precision_bits: u32, // Store bits for shift operations
    rans_precision_mask: u32, // (1 << bits) - 1 for fast modulo
    rans_precision: u32,
    l_rans_base: u32,
}

impl<'a> RAnsSymbolDecoder<'a> {
    pub fn new(rans_precision_bits: u32) -> Self {
        let rans_precision = 1u32 << rans_precision_bits;
        let l_rans_base = rans_precision * 4;
        Self {
            ans: AnsDecoder::new(&[]),
            probability_table: Vec::new(),
            lut: Vec::new(),
            num_symbols: 0,
            rans_precision_bits,
            rans_precision_mask: rans_precision - 1,
            rans_precision,
            l_rans_base,
        }
    }

    pub fn create(&mut self, buffer: &mut DecoderBuffer) -> bool {
        if !self.decode_table(buffer) {
            return false;
        }
        true
    }

    fn decode_table(&mut self, buffer: &mut DecoderBuffer) -> bool {
        let _start_pos = buffer.position();
        let bitstream_version =
            ((buffer.version_major() as u16) << 8) | (buffer.version_minor() as u16);
        let num_symbols = if bitstream_version < 0x0200 {
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            {
                return false;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            match buffer.decode_u32() {
                Ok(v) => v as usize,
                Err(_) => return false,
            }
        } else {
            match buffer.decode_varint() {
                Ok(v) => v as usize,
                Err(_) => return false,
            }
        };
        self.num_symbols = num_symbols;
        if num_symbols == 0 {
            return true;
        }

        self.probability_table
            .resize(num_symbols, RAnsSymbol::default());

        // NOTE: C++ only early-returns for num_symbols == 0.
        // For num_symbols == 1, it still reads the probability table byte.
        // We must do the same to stay in sync with the buffer!

        let mut i = 0;
        while i < num_symbols {
            let b = match buffer.decode_u8() {
                Ok(v) => v,
                Err(_) => return false,
            };

            let mode = b & 3;
            if mode == 3 {
                // Zero frequency offset
                let offset = (b >> 2) as usize;
                for j in 0..=offset {
                    if i + j >= num_symbols {
                        return false;
                    }
                    self.probability_table[i + j].prob = 0;
                }
                i += offset;
            } else {
                let num_extra_bytes = mode as usize;
                let mut prob = (b >> 2) as u32;
                for b_idx in 0..num_extra_bytes {
                    let extra = match buffer.decode_u8() {
                        Ok(v) => v,
                        Err(_) => return false,
                    };
                    prob |= (extra as u32) << (8 * (b_idx + 1) - 2);
                }
                self.probability_table[i].prob = prob;
            }
            i += 1;
        }

        // Compute cumulative probabilities and LUT
        self.lut.resize(self.rans_precision as usize, 0);
        let mut cum_prob: u32 = 0;
        for i in 0..num_symbols {
            let prob = self.probability_table[i].prob;
            self.probability_table[i].cum_prob = cum_prob;
            // Bounds check: ensure we don't write past the LUT
            let end_idx = cum_prob.saturating_add(prob);
            if end_idx > self.rans_precision {
                // Malformed probability table - probabilities exceed precision
                return false;
            }
            for j in 0..prob {
                self.lut[(cum_prob + j) as usize] = i as u32;
            }
            cum_prob = end_idx;
        }

        if cum_prob != self.rans_precision {
            return false;
        }
        true
    }

    pub fn start_decoding(&mut self, buffer: &mut DecoderBuffer<'a>) -> bool {
        // Draco advances the buffer past the encoded rANS data regardless of the
        // number of symbols (the encoded size prefix is always present).
        // C++: v < 2.0 uses fixed u64, v >= 2.0 uses varint u64.
        let bitstream_version =
            ((buffer.version_major() as u16) << 8) | (buffer.version_minor() as u16);
        let bytes_to_read = if bitstream_version < 0x0200 {
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            {
                return false;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            match buffer.decode::<u64>() {
                Ok(v) => v as usize,
                Err(_) => return false,
            }
        } else {
            match buffer.decode_varint() {
                Ok(v) => v as usize,
                Err(_) => return false,
            }
        };
        if self.num_symbols <= 1 {
            // Still need to advance the buffer past the encoded bytes.
            if buffer.try_advance(bytes_to_read).is_err() {
                return false;
            }
            return true;
        }
        let data = buffer.remaining_data();
        if data.len() < bytes_to_read {
            return false;
        }

        let rans_data = &data[..bytes_to_read];
        self.ans = AnsDecoder::new(rans_data);
        if !self.ans.read_init(self.l_rans_base) {
            return false;
        }

        if buffer.try_advance(bytes_to_read).is_err() {
            return false;
        }
        true
    }

    #[inline(always)]
    pub fn decode_symbol(&mut self) -> u32 {
        self.try_decode_symbol().unwrap_or(0)
    }

    #[inline(always)]
    pub fn try_decode_symbol(&mut self) -> Option<u32> {
        if self.num_symbols <= 1 {
            return Some(0);
        }
        // Match Draco C++ (ans.h) rans_read(): normalize first, then use
        // bit operations for division/modulo by rans_precision (power of two).
        // Using shift/mask is equivalent to div/mod but much faster.
        self.ans.read_normalize();
        let quo = self.ans.state >> self.rans_precision_bits; // Fast division
        let rem = self.ans.state & self.rans_precision_mask; // Fast modulo
        let symbol_id = *self.lut.get(rem as usize)?;
        let sym = self.probability_table.get(symbol_id as usize)?;
        let state_base = quo.checked_mul(sym.prob)?;
        let state_offset = rem.checked_sub(sym.cum_prob)?;
        self.ans.state = state_base.checked_add(state_offset)?;
        Some(symbol_id)
    }
}

#[cfg(test)]
mod tests {
    use super::RAnsSymbolDecoder;
    use crate::rans_symbol_coding::RAnsSymbol;

    #[test]
    fn try_decode_symbol_rejects_invalid_lut_symbol_id() {
        let mut decoder = RAnsSymbolDecoder::new(1);
        decoder.num_symbols = 2;
        decoder.lut = vec![99, 99];
        decoder.probability_table = vec![RAnsSymbol::default(); 2];
        decoder.ans.state = decoder.l_rans_base;

        assert_eq!(decoder.try_decode_symbol(), None);
    }

    #[test]
    fn try_decode_symbol_rejects_inconsistent_cumulative_probability() {
        let mut decoder = RAnsSymbolDecoder::new(1);
        decoder.num_symbols = 2;
        decoder.lut = vec![0, 0];
        decoder.probability_table = vec![
            RAnsSymbol {
                prob: 1,
                cum_prob: 1,
            },
            RAnsSymbol::default(),
        ];
        decoder.ans.state = decoder.l_rans_base;

        assert_eq!(decoder.try_decode_symbol(), None);
    }
}
