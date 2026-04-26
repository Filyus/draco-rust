//! Symbol encoding/decoding utilities for Draco compression.
//!
//! This module provides functions for encoding and decoding symbols using
//! tagged and raw schemes with rANS entropy coding.

use crate::rans_symbol_coding::compute_rans_precision_from_unique_symbols_bit_length;

#[cfg(feature = "encoder")]
use crate::rans_symbol_coding::approximate_rans_frequency_table_bits;

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::rans_symbol_decoder::RAnsSymbolDecoder;

#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
#[cfg(feature = "encoder")]
use crate::rans_symbol_encoder::RAnsSymbolEncoder;

pub struct SymbolEncodingOptions {
    pub compression_level: i32,
}

impl Default for SymbolEncodingOptions {
    fn default() -> Self {
        Self {
            compression_level: 7,
        }
    }
}

// ============================================================================
// Encoder-only functions
// ============================================================================

#[cfg(feature = "encoder")]
pub fn encode_symbols(
    symbols: &[u32],
    num_components: usize,
    options: &SymbolEncodingOptions,
    target_buffer: &mut EncoderBuffer,
) -> bool {
    if symbols.is_empty() {
        return true;
    }

    // Compute bit lengths
    let mut bit_lengths = Vec::with_capacity(symbols.len());
    let mut max_value = 0;

    for chunk in symbols.chunks(num_components) {
        let mut max_component_value = chunk[0];
        for &val in &chunk[1..] {
            if val > max_component_value {
                max_component_value = val;
            }
        }

        // C++ uses: value_msb_pos = MostSignificantBit(max_component_value);
        //           bit_lengths.push(value_msb_pos + 1);
        // MostSignificantBit returns 0-indexed position, so +1 gives bit count.
        // For max_component_value == 0, C++ uses value_msb_pos = 0, so bit_length = 1.
        let bit_length = if max_component_value > 0 {
            32 - max_component_value.leading_zeros()
        } else {
            1 // Minimum 1 bit, matching C++ behavior
        };
        if max_component_value > max_value {
            max_value = max_component_value;
        }
        bit_lengths.push(bit_length);
    }

    // Estimate bits for tagged scheme.
    let tagged_bits = compute_tagged_scheme_bits(symbols, num_components, &bit_lengths, max_value);

    let max_value_bit_length = if max_value == 0 {
        0
    } else {
        32 - max_value.leading_zeros()
    };
    const K_MAX_RAW_ENCODING_BIT_LENGTH: u32 = 18;

    // If max value can't be represented efficiently by RAW, always use TAGGED.
    // (This matches Draco's decision rule, but avoids doing unnecessary RAW
    // estimation work.)
    if max_value_bit_length > K_MAX_RAW_ENCODING_BIT_LENGTH {
        // Draco bitstream scheme ids (see C++ SymbolCodingMethod):
        //   0 = TAGGED
        //   1 = RAW
        target_buffer.encode_u8(0); // TAGGED
        encode_tagged_symbols(symbols, num_components, &bit_lengths, target_buffer)
    } else {
        // Estimate bits for raw scheme and compute symbol frequencies once.
        let (raw_bits, raw_frequencies, raw_num_unique) =
            compute_raw_scheme_bits_and_frequencies(symbols, max_value);

        if tagged_bits < raw_bits {
            target_buffer.encode_u8(0); // TAGGED
            encode_tagged_symbols(symbols, num_components, &bit_lengths, target_buffer)
        } else {
            target_buffer.encode_u8(1); // RAW
            encode_raw_symbols_with_frequencies(
                symbols,
                max_value,
                &raw_frequencies,
                raw_num_unique,
                target_buffer,
                options.compression_level,
            )
        }
    }
}

#[cfg(feature = "encoder")]
pub fn estimate_bits(symbols: &[u32], num_components: usize) -> u64 {
    if symbols.is_empty() {
        return 0;
    }

    // Compute bit lengths
    let mut bit_lengths = Vec::with_capacity(symbols.len());
    let mut max_value = 0;

    for chunk in symbols.chunks(num_components) {
        let mut max_component_value = chunk[0];
        for &val in &chunk[1..] {
            if val > max_component_value {
                max_component_value = val;
            }
        }

        // C++ uses: value_msb_pos = MostSignificantBit(max_component_value);
        //           bit_lengths.push(value_msb_pos + 1);
        // For max_component_value == 0, bit_length = 1.
        let bit_length = if max_component_value > 0 {
            32 - max_component_value.leading_zeros()
        } else {
            1 // Minimum 1 bit, matching C++ behavior
        };
        if max_component_value > max_value {
            max_value = max_component_value;
        }
        bit_lengths.push(bit_length);
    }

    let tagged_bits = compute_tagged_scheme_bits(symbols, num_components, &bit_lengths, max_value);
    let raw_bits = compute_raw_scheme_bits(symbols, max_value);

    std::cmp::min(tagged_bits, raw_bits)
}

#[cfg(feature = "encoder")]
fn compute_raw_scheme_bits(symbols: &[u32], max_value: u32) -> u64 {
    // Match Draco C++ ApproximateRawSchemeBits():
    //   data_bits = ComputeShannonEntropy(symbols, num_symbols, max_value)
    //   table_bits = ApproximateRAnsFrequencyTableBits(max_value, num_unique_symbols)
    // where ComputeShannonEntropy truncates to int64_t.

    if symbols.is_empty() {
        return 0;
    }

    let (data_bits, num_unique_symbols) = compute_shannon_entropy_bits_trunc(symbols, max_value);
    let table_bits = approximate_rans_frequency_table_bits(max_value, num_unique_symbols);
    (data_bits as u64) + table_bits
}

#[cfg(feature = "encoder")]
fn compute_raw_scheme_bits_and_frequencies(
    symbols: &[u32],
    max_value: u32,
) -> (u64, Vec<u64>, u32) {
    if symbols.is_empty() {
        return (0, Vec::new(), 0);
    }

    let mut frequencies = vec![0u64; (max_value + 1) as usize];
    for &sym in symbols {
        frequencies[sym as usize] += 1;
    }

    let num_symbols_d = symbols.len() as f64;
    let log2_num_symbols = num_symbols_d.log2();
    let mut total_bits = 0.0f64;
    let mut num_unique_symbols: u32 = 0;
    for &freq in &frequencies {
        if freq > 0 {
            num_unique_symbols += 1;
            let f = freq as f64;
            total_bits += f * (f.log2() - log2_num_symbols);
        }
    }

    let data_bits = (-total_bits) as i64;
    let table_bits = approximate_rans_frequency_table_bits(max_value, num_unique_symbols);
    (
        (data_bits as u64) + table_bits,
        frequencies,
        num_unique_symbols,
    )
}

#[cfg(feature = "encoder")]
fn compute_tagged_scheme_bits(
    _symbols: &[u32],
    num_components: usize,
    bit_lengths: &[u32],
    _max_value: u32,
) -> u64 {
    // 1. Bits for values (raw bits)
    let mut value_bits = 0;
    for &len in bit_lengths.iter() {
        value_bits += len as u64 * num_components as u64;
    }

    // 2. Bits for tags (RAns) using C++ ComputeShannonEntropy on bit lengths.
    // C++ calls ComputeShannonEntropy(bit_lengths, num_chunks, max_value=32).
    let (tag_bits, num_unique_symbols) = compute_shannon_entropy_bits_trunc(bit_lengths, 32);

    // C++ uses num_unique_symbols for BOTH params in the tagged scheme.
    let table_bits = approximate_rans_frequency_table_bits(num_unique_symbols, num_unique_symbols);

    value_bits + (tag_bits as u64) + table_bits
}

#[cfg(feature = "encoder")]
fn compute_shannon_entropy_bits_trunc(symbols: &[u32], max_value: u32) -> (i64, u32) {
    // Draco C++ ComputeShannonEntropy():
    //   total_bits += freq * log2(freq / num_symbols)
    //   return static_cast<int64_t>(-total_bits);
    // The cast truncates toward zero.

    let mut frequencies = vec![0u32; (max_value + 1) as usize];
    for &sym in symbols {
        frequencies[sym as usize] += 1;
    }

    let num_symbols_d = symbols.len() as f64;
    let log2_num_symbols = num_symbols_d.log2();
    let mut total_bits = 0.0f64;
    let mut num_unique_symbols: u32 = 0;

    for &freq in &frequencies {
        if freq > 0 {
            num_unique_symbols += 1;
            // freq * log2(freq / N) == freq * (log2(freq) - log2(N))
            total_bits += (freq as f64) * ((freq as f64).log2() - log2_num_symbols);
        }
    }

    ((-total_bits) as i64, num_unique_symbols)
}

#[cfg(feature = "encoder")]
pub fn encode_raw_symbols(
    symbols: &[u32],
    max_value: u32,
    target_buffer: &mut EncoderBuffer,
    compression_level: i32,
) -> bool {
    // num_values is known by decoder

    // Count frequencies
    let mut frequencies = vec![0u64; (max_value + 1) as usize];
    for &s in symbols {
        frequencies[s as usize] += 1;
    }

    let mut num_unique_symbols: u32 = 0;
    for &f in &frequencies {
        if f > 0 {
            num_unique_symbols += 1;
        }
    }

    encode_raw_symbols_with_frequencies(
        symbols,
        max_value,
        &frequencies,
        num_unique_symbols,
        target_buffer,
        compression_level,
    )
}

#[cfg(feature = "encoder")]
fn encode_raw_symbols_with_frequencies(
    symbols: &[u32],
    _max_value: u32,
    frequencies: &[u64],
    num_unique_symbols: u32,
    target_buffer: &mut EncoderBuffer,
    compression_level: i32,
) -> bool {
    let mut unique_symbols_bit_length: u32 = if num_unique_symbols > 0 {
        32 - num_unique_symbols.leading_zeros()
    } else {
        0
    };

    // Compression level adjustment.
    if compression_level < 4 {
        unique_symbols_bit_length = unique_symbols_bit_length.saturating_sub(2);
    } else if compression_level < 6 {
        unique_symbols_bit_length = unique_symbols_bit_length.saturating_sub(1);
    } else if compression_level > 9 {
        unique_symbols_bit_length += 2;
    } else if compression_level > 7 {
        unique_symbols_bit_length += 1;
    }

    unique_symbols_bit_length = unique_symbols_bit_length.clamp(1, 18);

    target_buffer.encode_u8(unique_symbols_bit_length as u8);

    let rans_precision_bits =
        compute_rans_precision_from_unique_symbols_bit_length(unique_symbols_bit_length);

    match rans_precision_bits {
        12 => encode_raw_symbols_internal::<12>(symbols, frequencies, target_buffer),
        13 => encode_raw_symbols_internal::<13>(symbols, frequencies, target_buffer),
        14 => encode_raw_symbols_internal::<14>(symbols, frequencies, target_buffer),
        15 => encode_raw_symbols_internal::<15>(symbols, frequencies, target_buffer),
        16 => encode_raw_symbols_internal::<16>(symbols, frequencies, target_buffer),
        17 => encode_raw_symbols_internal::<17>(symbols, frequencies, target_buffer),
        18 => encode_raw_symbols_internal::<18>(symbols, frequencies, target_buffer),
        19 => encode_raw_symbols_internal::<19>(symbols, frequencies, target_buffer),
        20 => encode_raw_symbols_internal::<20>(symbols, frequencies, target_buffer),
        _ => false,
    }
}

#[cfg(feature = "encoder")]
fn encode_raw_symbols_internal<const RANS_PRECISION_BITS: u32>(
    symbols: &[u32],
    frequencies: &[u64],
    target_buffer: &mut EncoderBuffer,
) -> bool {
    let mut encoder = RAnsSymbolEncoder::<RANS_PRECISION_BITS>::new();
    encoder.create(frequencies, frequencies.len(), target_buffer);
    encoder.start_encoding(target_buffer);

    // Reverse encoding
    for &sym in symbols.iter().rev() {
        encoder.encode_symbol(sym);
    }

    encoder.end_encoding(target_buffer);
    true
}

/*
pub fn encode_raw_symbols_no_scheme(symbols: &[u32], max_value: u32, target_buffer: &mut EncoderBuffer) -> bool {
    // ...
}
*/

#[cfg(feature = "encoder")]
#[allow(dead_code)]
fn encode_raw_symbols_typed<const PRECISION_BITS: u32>(
    symbols: &[u32],
    frequencies: &[u64],
    num_unique_symbols: usize,
    target_buffer: &mut EncoderBuffer,
) -> bool {
    let mut encoder = RAnsSymbolEncoder::<PRECISION_BITS>::new();
    if !encoder.create(frequencies, num_unique_symbols, target_buffer) {
        return false;
    }

    encoder.start_encoding(target_buffer);
    for &sym in symbols.iter().rev() {
        encoder.encode_symbol(sym);
    }
    encoder.end_encoding(target_buffer);
    true
}

#[cfg(feature = "encoder")]
fn encode_tagged_symbols(
    symbols: &[u32],
    num_components: usize,
    bit_lengths: &[u32],
    target_buffer: &mut EncoderBuffer,
) -> bool {
    // Scheme: Tagged is already written by caller

    // Encode bit lengths using RAns
    // Count frequencies of bit lengths (0..32)
    let mut frequencies = vec![0u64; 33];
    for &len in bit_lengths {
        frequencies[len as usize] += 1;
    }

    // Draco uses unique_symbols_bit_length=5 for tagged bit-length tags,
    // which corresponds to rANS precision bits = 12.
    let mut tag_encoder = RAnsSymbolEncoder::<12>::new();
    if !tag_encoder.create(&frequencies, 33, target_buffer) {
        return false;
    }

    #[cfg(feature = "debug_logs")]
    let debug_cmp = crate::debug_env_enabled("DRACO_DEBUG_CMP");
    #[cfg(not(feature = "debug_logs"))]
    let debug_cmp = false;
    if debug_cmp {
        eprintln!(
            "RUST TAGGED tag frequencies: {:?}",
            &frequencies[..15.min(frequencies.len())]
        );
    }

    // Create a separate bit buffer for raw values (C++ value_buffer)
    let mut value_buffer = EncoderBuffer::new();
    let value_bits = 32 * (symbols.len()); // safe upper bound
    value_buffer.start_bit_encoding(value_bits, false);

    tag_encoder.start_encoding(target_buffer);

    // 1. Encode bits in FORWARD order (because our BitEncoder is FIFO).
    for (i, &len) in bit_lengths.iter().enumerate() {
        let val_idx = i * num_components;
        for j in 0..num_components {
            let val = symbols[val_idx + j];
            value_buffer.encode_least_significant_bits32(len, val);
        }
    }

    // 2. Encode tags in REVERSE order (because ANS is LIFO).
    for &len in bit_lengths.iter().rev() {
        tag_encoder.encode_symbol(len);
    }

    tag_encoder.end_encoding(target_buffer);
    value_buffer.end_bit_encoding();
    target_buffer.encode_data(value_buffer.data());
    true
}

// ============================================================================
// Decoder-only functions
// ============================================================================

#[cfg(feature = "decoder")]
pub fn decode_symbols(
    num_values: usize,
    num_components: usize,
    _options: &SymbolEncodingOptions,
    in_buffer: &mut DecoderBuffer,
    symbols: &mut [u32],
) -> bool {
    if num_values == 0 {
        return true;
    }
    if num_components == 0 || symbols.len() < num_values || num_values % num_components != 0 {
        return false;
    }

    let scheme = match in_buffer.decode_u8() {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Support both the older internal ids (0/1) and the Draco ids (2/3).
    // Draco uses: 2 = TAGGED, 3 = RAW.
    match scheme {
        0 | 2 => decode_tagged_symbols(num_values, num_components, in_buffer, symbols),
        1 | 3 => decode_raw_symbols(num_values, in_buffer, symbols),
        _ => false,
    }
}

#[cfg(feature = "decoder")]
pub fn decode_raw_symbols(
    num_values: usize,
    in_buffer: &mut DecoderBuffer,
    symbols: &mut [u32],
) -> bool {
    if symbols.len() < num_values {
        return false;
    }

    // Read serialized symbol-bit-length header (written by encoder)
    let symbols_bit_length = match in_buffer.decode_u8() {
        Ok(v) => v as u32,
        Err(_) => return false,
    };
    if symbols_bit_length == 0 {
        for i in 0..num_values {
            symbols[i] = 0;
        }
        return true;
    }
    let unique_symbols_bit_length = symbols_bit_length;
    let precision_bits =
        compute_rans_precision_from_unique_symbols_bit_length(unique_symbols_bit_length);

    // Use runtime precision to avoid monomorphization bloat
    let mut decoder = RAnsSymbolDecoder::new(precision_bits);
    if !decoder.create(in_buffer) {
        return false;
    }
    if !decoder.start_decoding(in_buffer) {
        return false;
    }
    for i in 0..num_values {
        let Some(symbol) = decoder.try_decode_symbol() else {
            return false;
        };
        symbols[i] = symbol;
    }
    true
}

#[cfg(feature = "decoder")]
fn decode_tagged_symbols(
    num_values: usize,
    num_components: usize,
    in_buffer: &mut DecoderBuffer,
    symbols: &mut [u32],
) -> bool {
    if num_components == 0 || symbols.len() < num_values || num_values % num_components != 0 {
        return false;
    }

    // C++ uses RAnsSymbolDecoder<5> where 5 is unique_symbols_bit_length.
    // This maps to precision_bits = 12 via ComputeRAnsPrecisionFromUniqueSymbolsBitLength.
    let mut tag_decoder = RAnsSymbolDecoder::new(12);

    if !tag_decoder.create(in_buffer) {
        return false;
    }
    if !tag_decoder.start_decoding(in_buffer) {
        return false;
    }

    // Start bit-decoding for raw values (value_buffer)
    if in_buffer.start_bit_decoding(false).is_err() {
        return false;
    }

    let num_chunks = num_values / num_components;

    // Pre-validate that the bit stream has enough data for the worst case:
    // each chunk reads at most 32 bits × num_components.
    // The bit stream is already bounded by start_bit_decoding.

    // Process each chunk
    let mut val_idx = 0;
    for _ in 0..num_chunks {
        let Some(len) = tag_decoder.try_decode_symbol() else {
            return false;
        };
        if len == 0 || len > 32 {
            return false;
        }
        for _ in 0..num_components {
            let val = match in_buffer.decode_least_significant_bits32_fast(len) {
                Ok(v) => v,
                Err(_) => return false,
            };
            symbols[val_idx] = val;
            val_idx += 1;
        }
    }

    in_buffer.end_bit_decoding();

    true
}

#[cfg(all(test, feature = "decoder"))]
mod tests {
    use super::*;

    #[test]
    fn decode_raw_symbols_rejects_short_output() {
        let bytes = [0u8]; // zero bit length would otherwise fill the output slice.
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut symbols = [];

        assert!(!decode_raw_symbols(1, &mut buffer, &mut symbols));
    }

    #[test]
    fn decode_tagged_symbols_rejects_zero_components() {
        let mut buffer = DecoderBuffer::new(&[]);
        let mut symbols = [0u32; 1];

        assert!(!decode_tagged_symbols(1, 0, &mut buffer, &mut symbols));
    }

    #[test]
    fn decode_tagged_symbols_rejects_partial_component_chunk() {
        let mut buffer = DecoderBuffer::new(&[]);
        let mut symbols = [0u32; 5];

        assert!(!decode_tagged_symbols(5, 2, &mut buffer, &mut symbols));
    }
}
