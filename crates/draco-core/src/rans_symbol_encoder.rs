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
            eprintln!(
                "RUST RANS create: num_symbols={} total_freq={}",
                num_symbols, total_freq
            );
            eprintln!(
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
            eprintln!(
                "RUST RANS initial probs (before norm): {:?}",
                self.probability_table
                    .iter()
                    .map(|s| s.prob)
                    .collect::<Vec<_>>()
            );
            eprintln!(
                "RUST RANS total_rans_prob: {} vs precision: {}",
                total_rans_prob,
                Self::RANS_PRECISION
            );
        }

        if total_rans_prob != Self::RANS_PRECISION {
            let mut sorted_probabilities: Vec<usize> = (0..num_symbols).collect();
            // Use stable sort to match C++ std::stable_sort behavior
            // Rust Vec::sort_by is documented to be stable
            sorted_probabilities.sort_by(|&a, &b| {
                self.probability_table[a]
                    .prob
                    .cmp(&self.probability_table[b].prob)
            });

            if debug_cmp {
                eprintln!("RUST RANS sorted_probabilities: {:?}", sorted_probabilities);
                eprintln!("RUST RANS total_rans_prob before fix: {}", total_rans_prob);
            }

            if total_rans_prob < Self::RANS_PRECISION {
                let last = *sorted_probabilities.last().unwrap();
                self.probability_table[last].prob += Self::RANS_PRECISION - total_rans_prob;
            } else {
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

                        let new_prob = (act_rel_error_d
                            * self.probability_table[symbol_id].prob as f64)
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
        }

        let mut total_prob = 0;
        for i in 0..num_symbols {
            self.probability_table[i].cum_prob = total_prob;
            total_prob += self.probability_table[i].prob;
        }

        if debug_cmp {
            eprintln!(
                "RUST RANS probability_table (probs): {:?}",
                self.probability_table
                    .iter()
                    .map(|s| s.prob)
                    .collect::<Vec<_>>()
            );
            eprintln!(
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
        let bitstream_version =
            ((buffer.version_major() as u16) << 8) | (buffer.version_minor() as u16);
        if bitstream_version < 0x0200 {
            buffer.encode_u32(self.num_symbols as u32);
        } else {
            buffer.encode_varint(self.num_symbols as u64);
        }

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

            if prob == 0 {
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

    pub fn encode_symbol(&mut self, symbol: u32) {
        let sym = self.probability_table[symbol as usize];
        self.rans_write(sym);
    }

    pub fn end_encoding(&mut self, buffer: &mut EncoderBuffer) {
        let _len = self
            .ans
            .write_end()
            .expect("ANS state should always be valid for symbol encoding");
        let data = self.ans.data();
        let bytes_written = data.len() as u64;

        // C++ v1.x writes the byte count as a fixed u64; v2.0+ uses varint.
        let bitstream_version =
            ((buffer.version_major() as u16) << 8) | (buffer.version_minor() as u16);
        if bitstream_version < 0x0200 {
            buffer.encode_u64(bytes_written);
        } else {
            buffer.encode_varint(bytes_written);
        }
        buffer.encode_data(data);
    }

    fn rans_write(&mut self, sym: RAnsSymbol) {
        // Hot path: avoid / and % by 256 (ANS_IO_BASE) and avoid computing both /p and %p.
        let p = sym.prob;
        let renorm_bound = (Self::L_RANS_BASE / Self::RANS_PRECISION) * crate::ans::ANS_IO_BASE * p;

        let mut state = self.ans.state;
        while state >= renorm_bound {
            // ANS_IO_BASE is 256.
            self.ans.buf.push((state & 0xFF) as u8);
            state >>= 8;
        }

        // Compute quotient once; derive remainder without an extra division.
        let quot = state / p;
        let rem = state - quot * p;
        state = quot * Self::RANS_PRECISION + rem + sym.cum_prob;
        self.ans.state = state;
    }
}
