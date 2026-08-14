//! Multi-symbol rANS encoder.
//!
//! [`RAnsSymbolEncoder`] builds a probability table from symbol frequencies and
//! rANS-encodes symbols at a compile-time precision (`RANS_PRECISION_BITS`).
//! Encode-side counterpart of `RAnsSymbolDecoder`. Port of Draco's
//! `rans_symbol_encoder.h`.

use crate::ans::AnsCoder;
use crate::encoder_buffer::EncoderBuffer;
use crate::rans_symbol_coding::RAnsSymbol;

pub struct RAnsSymbolEncoder<const RANS_PRECISION_BITS: u32> {
    pub ans: AnsCoder,
    probability_table: Vec<RAnsSymbol>,
    num_symbols: usize,
}

impl<const RANS_PRECISION_BITS: u32> Default for RAnsSymbolEncoder<RANS_PRECISION_BITS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const RANS_PRECISION_BITS: u32> RAnsSymbolEncoder<RANS_PRECISION_BITS> {
    const RANS_PRECISION: u32 = 1 << RANS_PRECISION_BITS;
    const L_RANS_BASE: u32 = Self::RANS_PRECISION * 4;

    pub fn new() -> Self {
        Self {
            ans: AnsCoder::new(),
            probability_table: Vec::new(),
            num_symbols: 0,
        }
    }

    pub fn create(
        &mut self,
        frequencies: &[u64],
        num_symbols: usize,
        buffer: &mut EncoderBuffer,
    ) -> bool {
        // Compute the total of the input frequencies.
        let mut total_freq: u64 = 0;
        let mut max_valid_symbol = 0;
        for (i, &freq) in frequencies.iter().enumerate().take(num_symbols) {
            total_freq += freq;
            if freq > 0 {
                max_valid_symbol = i;
            }
        }

        let num_symbols = max_valid_symbol + 1;
        self.num_symbols = num_symbols;
        self.probability_table
            .resize(num_symbols, RAnsSymbol::default());
        #[cfg(feature = "debug_logs")]
        let debug_cmp = crate::debug_env_enabled("DRACO_DEBUG_CMP");
        #[cfg(not(feature = "debug_logs"))]
        let debug_cmp = false;

        if debug_cmp {
            debug_log!(
                "RUST RANS create: num_symbols={} total_freq={}",
                num_symbols,
                total_freq
            );
            debug_log!(
                "RUST RANS frequencies: {:?}",
                &frequencies[..num_symbols.min(frequencies.len())]
            );
        }

        if total_freq == 0 {
            return false;
        }

        let total_freq_d = total_freq as f64;
        let rans_precision_d = Self::RANS_PRECISION as f64;

        let mut total_rans_prob: u32 = 0;
        for i in 0..num_symbols {
            let freq = frequencies[i];
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            self.probability_table[i].prob = rans_prob;
            total_rans_prob += rans_prob;
        }

        if debug_cmp {
            debug_log!(
                "RUST RANS initial probs (before norm): {:?}",
                self.probability_table
                    .iter()
                    .map(|s| s.prob)
                    .collect::<Vec<_>>()
            );
            debug_log!(
                "RUST RANS total_rans_prob: {} vs precision: {}",
                total_rans_prob,
                Self::RANS_PRECISION
            );
        }

        if total_rans_prob < Self::RANS_PRECISION {
            let mut largest_probability = 0;
            for i in 1..num_symbols {
                if self.probability_table[i].prob
                    >= self.probability_table[largest_probability].prob
                {
                    largest_probability = i;
                }
            }

            if debug_cmp {
                debug_log!(
                    "RUST RANS largest_probability: {} prob={}",
                    largest_probability,
                    self.probability_table[largest_probability].prob
                );
                debug_log!("RUST RANS total_rans_prob before fix: {}", total_rans_prob);
            }

            self.probability_table[largest_probability].prob +=
                Self::RANS_PRECISION - total_rans_prob;
        } else if total_rans_prob > Self::RANS_PRECISION {
            let mut sorted_probabilities: Vec<usize> = (0..num_symbols).collect();
            // Use stable sort to match C++ std::stable_sort behavior
            // Rust Vec::sort_by is documented to be stable
            sorted_probabilities.sort_by(|&a, &b| {
                self.probability_table[a]
                    .prob
                    .cmp(&self.probability_table[b].prob)
            });

            if debug_cmp {
                debug_log!("RUST RANS sorted_probabilities: {:?}", sorted_probabilities);
                debug_log!("RUST RANS total_rans_prob before fix: {}", total_rans_prob);
            }

            let mut error = total_rans_prob as i32 - Self::RANS_PRECISION as i32;
            while error > 0 {
                let act_total_prob_d = total_rans_prob as f64;
                let act_rel_error_d = rans_precision_d / act_total_prob_d;

                for j in (1..num_symbols).rev() {
                    let symbol_id = sorted_probabilities[j];
                    if self.probability_table[symbol_id].prob <= 1 {
                        if j == num_symbols - 1 {
                            return false;
                        }
                        break;
                    }

                    let new_prob = (act_rel_error_d * self.probability_table[symbol_id].prob as f64)
                        .floor() as i32;
                    let mut fix = self.probability_table[symbol_id].prob as i32 - new_prob;
                    if fix == 0 {
                        fix = 1;
                    }
                    if fix >= self.probability_table[symbol_id].prob as i32 {
                        fix = self.probability_table[symbol_id].prob as i32 - 1;
                    }
                    if fix > error {
                        fix = error;
                    }

                    self.probability_table[symbol_id].prob -= fix as u32;
                    total_rans_prob -= fix as u32;
                    error -= fix;
                    if total_rans_prob == Self::RANS_PRECISION {
                        break;
                    }
                }
            }
        }

        let mut total_prob = 0;
        for i in 0..num_symbols {
            self.probability_table[i].cum_prob = total_prob;
            total_prob += self.probability_table[i].prob;
        }

        if debug_cmp {
            debug_log!(
                "RUST RANS probability_table (probs): {:?}",
                self.probability_table
                    .iter()
                    .map(|s| s.prob)
                    .collect::<Vec<_>>()
            );
            debug_log!(
                "RUST RANS probability_table (cums): {:?}",
                self.probability_table
                    .iter()
                    .map(|s| s.cum_prob)
                    .collect::<Vec<_>>()
            );
        }

        if total_prob != Self::RANS_PRECISION {
            return false;
        }

        self.encode_table(buffer)
    }

    fn encode_table(&self, buffer: &mut EncoderBuffer) -> bool {
        // C++ v1.x writes num_symbols as u32; v2.0+ uses varint.
        let bitstream_version = buffer.bitstream_version();
        if bitstream_version < 0x0200 {
            buffer.encode_u32(self.num_symbols as u32);
        } else {
            buffer.encode_varint(self.num_symbols as u64);
        }

        // A run of zero-probability symbols is written as one byte carrying the
        // run length, tagged with token 3. That token used to mean "three extra
        // probability bytes" -- a width no probability reaches, which is why it
        // could be repurposed -- and the repurposing landed in Draco 0.10.0,
        // whose bitstream is 1.2. So a 1.2 stream may carry the run, and a 1.1
        // stream may not: 0.9.1 reads token 3 as a byte count and desynchronises
        // on the whole table.
        //
        // The asymmetry is why nothing noticed. An old stream never contains
        // token 3, so every later decoder reads one; only the reverse direction
        // breaks, and the reverse direction is exactly what writing 1.1 is.
        let pre_zero_run_table = bitstream_version != 0 && bitstream_version < 0x0102;

        let mut i = 0;
        while i < self.num_symbols {
            let prob = self.probability_table[i].prob;
            let mut num_extra_bytes = 0;
            if prob >= (1 << 6) {
                num_extra_bytes += 1;
                if prob >= (1 << 14) {
                    num_extra_bytes += 1;
                    if prob >= (1 << 22) {
                        return false;
                    }
                }
            }

            if prob == 0 && !pre_zero_run_table {
                let mut offset = 0;
                while offset < (1 << 6) - 1 {
                    if i + offset + 1 >= self.num_symbols {
                        break;
                    }
                    let next_prob = self.probability_table[i + offset + 1].prob;
                    if next_prob > 0 {
                        break;
                    }
                    offset += 1;
                }
                buffer.encode_u8(((offset as u8) << 2) | 3);
                i += offset;
            } else {
                buffer.encode_u8(((prob as u8) << 2) | (num_extra_bytes & 3));
                for b in 0..num_extra_bytes {
                    buffer.encode_u8((prob >> (8 * (b + 1) - 2)) as u8);
                }
            }
            i += 1;
        }
        true
    }

    pub fn start_encoding(&mut self, _buffer: &mut EncoderBuffer) {
        self.ans.write_init(Self::L_RANS_BASE);
    }

    /// Starts rANS encoding and reserves space for the expected output bytes.
    pub(crate) fn start_encoding_with_capacity(
        &mut self,
        _buffer: &mut EncoderBuffer,
        byte_capacity: usize,
    ) {
        self.ans
            .write_init_with_capacity(Self::L_RANS_BASE, byte_capacity);
    }

    pub fn encode_symbol(&mut self, symbol: u32) {
        let sym = self.probability_table[symbol as usize];
        self.rans_write(sym);
    }

    pub fn end_encoding(&mut self, buffer: &mut EncoderBuffer) {
        let _len = self
            .ans
            .write_end(true)
            .expect("ANS state should always be valid for symbol encoding");
        let data = self.ans.data();
        let bytes_written = data.len() as u64;

        // C++ v1.x writes the byte count as a fixed u64; v2.0+ uses varint.
        let bitstream_version = buffer.bitstream_version();
        if bitstream_version < 0x0200 {
            buffer.encode_u64(bytes_written);
        } else {
            buffer.encode_varint(bytes_written);
        }
        buffer.encode_data(data);
    }

    fn rans_write(&mut self, sym: RAnsSymbol) {
        // Hot path: the renormalization loop's divide and modulo are by
        // ANS_IO_BASE (256), a constant, so they are a shift and a mask.
        let p = sym.prob;
        let renorm_bound = (Self::L_RANS_BASE / Self::RANS_PRECISION) * crate::ans::ANS_IO_BASE * p;

        let mut state = self.ans.state;
        while state >= renorm_bound {
            // ANS_IO_BASE is 256.
            self.ans.buf.push((state & 0xFF) as u8);
            state >>= 8;
        }

        // `p` is a runtime probability, so this division is a real one -- but
        // ask for the remainder directly rather than deriving it as
        // `state - quot * p`. Both results come out of the same hardware
        // `div`, and spelling the subtraction by hand adds a multiply and a
        // subtract to every symbol: measured at 2.1% of encode, two builds per
        // condition with disjoint clusters. See TRICKS.md.
        let quot = state / p;
        let rem = state % p;
        state = quot * Self::RANS_PRECISION + rem + sym.cum_prob;
        self.ans.state = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run of zero-probability symbols is written one byte each below 1.2.
    ///
    /// Token 3 in the low two bits means "this byte carries a run length of
    /// zero-probability symbols", and it means that only from Draco 0.10.0,
    /// whose bitstream is 1.2. Before that it meant "three extra probability
    /// bytes" -- a width no probability reaches, which is why it could be taken
    /// over. So 0.9.1 reads a run byte as a length prefix and desynchronises on
    /// the rest of the table, and a 1.1 stream must not contain one.
    ///
    /// The asymmetry hid this: an old stream never contains token 3, so every
    /// later decoder reads one. Only writing the old version breaks, and that
    /// direction had no reader in the test suite until draco_decoder 0.9.1 was
    /// pointed at it.
    fn table_bytes(major: u8, minor: u8) -> Vec<u8> {
        // Gaps between the used symbols are what produce zero-probability runs.
        let mut frequencies = vec![0u64; 64];
        for symbol in [0usize, 17, 40, 63] {
            frequencies[symbol] = 64;
        }
        let mut buffer = EncoderBuffer::new();
        buffer.set_version(major, minor);
        let mut encoder = RAnsSymbolEncoder::<12>::new();
        assert!(
            encoder.create(&frequencies, frequencies.len(), &mut buffer),
            "v{major}.{minor}: create"
        );
        buffer.data().to_vec()
    }

    fn carries_zero_run_token(bytes: &[u8]) -> bool {
        // Past the u32 symbol count that pre-2.0 writes.
        bytes[4..].iter().any(|byte| byte & 3 == 3)
    }

    #[test]
    fn the_zero_run_token_is_written_only_from_1_2() {
        assert!(
            !carries_zero_run_token(&table_bytes(1, 1)),
            "1.1 must not carry the run token"
        );
        assert!(
            carries_zero_run_token(&table_bytes(1, 2)),
            "1.2 is expected to use the run token, or this test proves nothing"
        );
    }
}
