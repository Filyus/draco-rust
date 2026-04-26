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

        for &symbol in symbols {
            let symbol = symbol as usize;
            if self.frequencies.len() <= symbol {
                self.frequencies.resize(symbol + 1, 0);
            }

            let mut old_symbol_entropy_norm = 0.0;
            let frequency = self.frequencies[symbol];

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
            self.frequencies[symbol] += 1;
            let new_frequency = self.frequencies[symbol];
            let new_symbol_entropy_norm = (new_frequency as f64) * (new_frequency as f64).log2();

            ret_data.entropy_norm += new_symbol_entropy_norm - old_symbol_entropy_norm;
        }

        if push_changes {
            self.entropy_data = ret_data;
        } else {
            // Revert frequency table changes (like C++)
            for &symbol in symbols {
                self.frequencies[symbol as usize] -= 1;
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
