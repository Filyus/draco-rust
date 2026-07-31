//! Shannon entropy estimation.
//!
//! [`ShannonEntropyTracker`] incrementally tracks symbol frequencies and
//! estimates the bit cost of encoding them. The encoder uses these estimates to
//! choose between coding schemes and to size rANS tables. Port of Draco's
//! `shannon_entropy.h`.

use crate::rans_symbol_coding::approximate_rans_frequency_table_bits;

#[derive(Clone, Copy, Debug, Default)]
pub struct EntropyData {
    pub entropy_norm: f64,
    pub num_values: i32,
    pub max_symbol: i32,
    pub num_unique_symbols: i32,
}

pub struct ShannonEntropyTracker {
    entropy_data: EntropyData,
    frequencies: Vec<i32>,
}

impl Default for ShannonEntropyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ShannonEntropyTracker {
    pub fn new() -> Self {
        Self {
            entropy_data: EntropyData::default(),
            frequencies: Vec::new(),
        }
    }

    pub fn push(&mut self, symbols: &[u32]) -> EntropyData {
        self.update_symbols(symbols, true)
    }

    pub fn peek(&mut self, symbols: &[u32]) -> EntropyData {
        self.update_symbols(symbols, false)
    }

    fn update_symbols(&mut self, symbols: &[u32], push_changes: bool) -> EntropyData {
        let mut ret_data = self.entropy_data;
        ret_data.num_values += symbols.len() as i32;

        for (i, &symbol) in symbols.iter().enumerate() {
            let index = symbol as usize;

            // The table is indexed by symbol value, so covering a symbol costs
            // memory proportional to it, and a symbol is a zig-zagged residual
            // -- bounded only by `u32`. Grow only when the symbols are really
            // being added: a peek is scoring a candidate the caller may reject,
            // and a single rejected one whose residuals overflowed would
            // otherwise hold gigabytes for the rest of the encode. A symbol the
            // table does not cover has frequency zero, so while peeking its
            // count is just how often it already appeared in this same call.
            let mut frequency = 0;
            if index < self.frequencies.len() {
                frequency = self.frequencies[index];
            } else if push_changes {
                self.frequencies.resize(index + 1, 0);
            } else {
                for &earlier in &symbols[..i] {
                    if earlier == symbol {
                        frequency += 1;
                    }
                }
            }

            let mut old_symbol_entropy_norm = 0.0;
            if frequency > 1 {
                old_symbol_entropy_norm = (frequency as f64) * (frequency as f64).log2();
            } else if frequency == 0 {
                ret_data.num_unique_symbols += 1;
                if symbol as i32 > ret_data.max_symbol {
                    ret_data.max_symbol = symbol as i32;
                }
            }

            // C++ modifies frequency during loop, then reverts if peeking.
            // We do the same for efficiency (avoids cloning the entire table).
            frequency += 1;
            if index < self.frequencies.len() {
                self.frequencies[index] = frequency;
            }
            let new_symbol_entropy_norm = (frequency as f64) * (frequency as f64).log2();

            ret_data.entropy_norm += new_symbol_entropy_norm - old_symbol_entropy_norm;
        }

        if push_changes {
            self.entropy_data = ret_data;
        } else {
            // Revert frequency table changes (like C++). Symbols the table does
            // not cover were never written above, so they need no reverting.
            for &symbol in symbols {
                let index = symbol as usize;
                if index < self.frequencies.len() {
                    self.frequencies[index] -= 1;
                }
            }
        }

        ret_data
    }

    pub fn get_number_of_data_bits(&self) -> i64 {
        Self::get_number_of_data_bits_static(&self.entropy_data)
    }

    pub fn get_number_of_r_ans_table_bits(&self) -> i64 {
        Self::get_number_of_r_ans_table_bits_static(&self.entropy_data)
    }

    pub fn get_number_of_data_bits_static(entropy_data: &EntropyData) -> i64 {
        if entropy_data.num_values < 2 {
            return 0;
        }

        let num_values = entropy_data.num_values as f64;
        let bits = num_values * num_values.log2() - entropy_data.entropy_norm;
        bits.ceil() as i64
    }

    pub fn get_number_of_r_ans_table_bits_static(entropy_data: &EntropyData) -> i64 {
        approximate_rans_frequency_table_bits(
            (entropy_data.max_symbol + 1) as u32,
            entropy_data.num_unique_symbols as u32,
        ) as i64
    }
}

pub fn compute_shannon_entropy(
    symbols: &[u32],
    max_value: usize,
    out_num_unique_symbols: Option<&mut i32>,
) -> i64 {
    let mut num_unique_symbols = 0;
    let mut symbol_frequencies = vec![0; max_value + 1];

    for &symbol in symbols {
        symbol_frequencies[symbol as usize] += 1;
    }

    let mut total_bits = 0.0;
    let num_symbols_d = symbols.len() as f64;

    for &freq in &symbol_frequencies {
        if freq > 0 {
            num_unique_symbols += 1;
            total_bits += (freq as f64) * ((freq as f64) / num_symbols_d).log2();
        }
    }

    if let Some(out) = out_num_unique_symbols {
        *out = num_unique_symbols;
    }

    (-total_bits) as i64
}

pub fn compute_binary_shannon_entropy(num_values: u32, num_true_values: u32) -> f64 {
    if num_values == 0 {
        return 0.0;
    }

    if num_true_values == 0 || num_values == num_true_values {
        return 0.0;
    }

    let true_freq = (num_true_values as f64) / (num_values as f64);
    let false_freq = 1.0 - true_freq;

    -(true_freq * true_freq.log2() + false_freq * false_freq.log2())
}
