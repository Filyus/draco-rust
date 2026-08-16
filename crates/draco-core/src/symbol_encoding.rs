//! Symbol encoding/decoding utilities for Draco compression.
//!
//! This module provides functions for encoding and decoding symbols using
//! tagged and raw schemes with rANS entropy coding.

use crate::rans_symbol_coding::compute_rans_precision_from_unique_symbols_bit_length;
use crate::status::{DracoError, Status};

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
) -> Status {
    if symbols.is_empty() {
        return Ok(());
    }

    // Compute bit lengths
    let mut bit_lengths = Vec::with_capacity(symbols.len().div_ceil(num_components));
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
    let mut bit_lengths = Vec::with_capacity(symbols.len().div_ceil(num_components));
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
) -> Status {
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
) -> Status {
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
        other => Err(DracoError::general(format!(
            "rANS precision {other} bits has no encoder: the table covers 12..=20"
        ))),
    }
}

#[cfg(feature = "encoder")]
fn encode_raw_symbols_internal<const RANS_PRECISION_BITS: u32>(
    symbols: &[u32],
    frequencies: &[u64],
    target_buffer: &mut EncoderBuffer,
) -> Status {
    let mut encoder = RAnsSymbolEncoder::<RANS_PRECISION_BITS>::new();
    encoder.create(frequencies, frequencies.len(), target_buffer);
    encoder.start_encoding_with_capacity(
        target_buffer,
        symbols.len().saturating_mul(2).saturating_add(4),
    );

    // Reverse encoding
    for &sym in symbols.iter().rev() {
        encoder.encode_symbol(sym);
    }

    encoder.end_encoding(target_buffer);
    Ok(())
}

/*
pub fn encode_raw_symbols_no_scheme(symbols: &[u32], max_value: u32, target_buffer: &mut EncoderBuffer) -> bool {
    // ...
}
*/

#[cfg(feature = "encoder")]
fn encode_tagged_symbols(
    symbols: &[u32],
    num_components: usize,
    bit_lengths: &[u32],
    target_buffer: &mut EncoderBuffer,
) -> Status {
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
        return Err(DracoError::general(
            "Failed to build the rANS frequency table for the tagged bit lengths",
        ));
    }

    #[cfg(feature = "debug_logs")]
    let debug_cmp = crate::debug_env_enabled("DRACO_DEBUG_CMP");
    #[cfg(not(feature = "debug_logs"))]
    let debug_cmp = false;
    if debug_cmp {
        debug_log!(
            "RUST TAGGED tag frequencies: {:?}",
            &frequencies[..15.min(frequencies.len())]
        );
    }

    // Create a separate bit buffer for raw values (C++ value_buffer)
    let mut value_buffer = EncoderBuffer::new();
    let value_bits = 32 * (symbols.len()); // safe upper bound
    value_buffer.start_bit_encoding(value_bits, false);

    tag_encoder.start_encoding_with_capacity(
        target_buffer,
        bit_lengths.len().saturating_mul(2).saturating_add(4),
    );

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
    Ok(())
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
    symbols: &mut Vec<u32>,
) -> Status {
    symbols.clear();
    if num_values == 0 {
        return Ok(());
    }
    if num_components == 0 {
        return Err(DracoError::invalid_parameter(
            "Symbol decode needs at least one component",
        ));
    }
    if !num_values.is_multiple_of(num_components) {
        return Err(DracoError::invalid_parameter(format!(
            "Symbol count {num_values} is not a multiple of the {num_components} components it is read into"
        )));
    }
    reserve_within_input(symbols, num_values, in_buffer);

    let scheme = in_buffer
        .decode_u8()
        .map_err(|_| DracoError::buffer("Buffer ran out reading the symbol coding scheme"))?;

    // Draco uses: 0 = TAGGED, 1 = RAW.
    match scheme {
        0 => decode_tagged_symbols(num_values, num_components, in_buffer, symbols),
        1 => decode_raw_symbols(num_values, in_buffer, symbols),
        other => Err(DracoError::unsupported_feature(format!(
            "Unknown symbol coding scheme {other}: Draco defines 0 (tagged) and 1 (raw)"
        ))),
    }
}

/// Reserves for what the stream could plausibly produce, not for what it says.
///
/// The declared count is a ceiling to decode up to, never a size to allocate:
/// a nine-byte header naming two billion symbols must not reserve for them
/// before the stream has produced one. So the starting capacity is bounded by
/// the input -- one symbol per *bit* of what remains, the same bound
/// `MeshEdgebreakerDecoder` already uses for its symbol run -- and anything
/// past that arrives through `push`, whose growth is backed by symbols that
/// were actually decoded.
///
/// A bit per symbol rather than a byte per symbol because the byte was both
/// wrong and slow: entropy coding beats one symbol per byte routinely, so real
/// streams reserved a fraction of what they needed and paid for the
/// reallocations -- 598 to 698 us on a 10,000-point decode. At eight per byte a
/// real stream reserves once, and a 9 KB stream claiming two billion symbols
/// still reserves 289 KB rather than 8 GB.
#[cfg(feature = "decoder")]
fn reserve_within_input(symbols: &mut Vec<u32>, num_values: usize, in_buffer: &DecoderBuffer) {
    symbols.reserve(num_values.min(in_buffer.remaining_size().saturating_mul(8)));
}

#[cfg(feature = "decoder")]
pub fn decode_raw_symbols(
    num_values: usize,
    in_buffer: &mut DecoderBuffer,
    symbols: &mut Vec<u32>,
) -> Status {
    // Read serialized symbol-bit-length header (written by encoder)
    let symbols_bit_length = in_buffer
        .decode_u8()
        .map_err(|_| DracoError::buffer("Buffer ran out reading the raw symbol bit length"))?
        as u32;
    if !(1..=18).contains(&symbols_bit_length) {
        return Err(DracoError::general(format!(
            "Raw symbol bit length {symbols_bit_length} outside the supported range 1..=18"
        )));
    }
    let unique_symbols_bit_length = symbols_bit_length;
    let precision_bits =
        compute_rans_precision_from_unique_symbols_bit_length(unique_symbols_bit_length);

    // Use runtime precision to avoid monomorphization bloat
    let mut decoder = RAnsSymbolDecoder::new(precision_bits);
    if !decoder.create(in_buffer) {
        return Err(DracoError::general(
            "Failed to read the raw scheme's rANS frequency table",
        ));
    }
    if !decoder.start_decoding(in_buffer) {
        return Err(DracoError::general(
            "Failed to start rANS decoding of the raw symbols",
        ));
    }
    for index in 0..num_values {
        let Some(symbol) = decoder.try_decode_symbol() else {
            return Err(DracoError::general(format!(
                "Raw symbol stream ended after {index} of {num_values} symbols"
            )));
        };
        symbols.push(symbol);
    }
    Ok(())
}

#[cfg(feature = "decoder")]
fn decode_tagged_symbols(
    num_values: usize,
    num_components: usize,
    in_buffer: &mut DecoderBuffer,
    symbols: &mut Vec<u32>,
) -> Status {
    if num_components == 0 || !num_values.is_multiple_of(num_components) {
        return Err(DracoError::invalid_parameter(format!(
            "Tagged symbol count {num_values} is not a multiple of the {num_components} components it is read into"
        )));
    }

    // C++ uses RAnsSymbolDecoder<5> where 5 is unique_symbols_bit_length.
    // This maps to precision_bits = 12 via ComputeRAnsPrecisionFromUniqueSymbolsBitLength.
    let mut tag_decoder = RAnsSymbolDecoder::new(12);

    if !tag_decoder.create(in_buffer) {
        return Err(DracoError::general(
            "Failed to read the tagged scheme's rANS frequency table",
        ));
    }
    if !tag_decoder.start_decoding(in_buffer) {
        return Err(DracoError::general(
            "Failed to start rANS decoding of the tagged symbol tags",
        ));
    }

    // Start bit-decoding for raw values (value_buffer)
    in_buffer
        .start_bit_decoding(false)
        .map_err(|_| DracoError::buffer("Buffer ran out starting the tagged value bit stream"))?;

    let num_chunks = num_values / num_components;

    // Pre-validate that the bit stream has enough data for the worst case:
    // each chunk reads at most 32 bits × num_components.
    // The bit stream is already bounded by start_bit_decoding.

    // Process each chunk
    for chunk in 0..num_chunks {
        let Some(len) = tag_decoder.try_decode_symbol() else {
            return Err(DracoError::general(format!(
                "Tag stream ended after {chunk} of {num_chunks} chunks"
            )));
        };
        if len == 0 || len > 32 {
            return Err(DracoError::general(format!(
                "Tagged value width {len} outside the supported range 1..=32"
            )));
        }
        for _ in 0..num_components {
            let val = in_buffer
                .decode_least_significant_bits32_fast(len)
                .map_err(|_| {
                    DracoError::buffer(format!(
                        "Value bit stream ran out reading {len} bits in chunk {chunk} of {num_chunks}"
                    ))
                })?;
            symbols.push(val);
        }
    }

    in_buffer.end_bit_decoding();

    Ok(())
}

#[cfg(all(test, feature = "decoder"))]
mod tests {
    use super::*;

    #[test]
    fn decode_raw_symbols_rejects_short_output() {
        let bytes = [0u8]; // A zero bit length would otherwise fill the sink.
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut symbols = Vec::new();

        assert!(decode_raw_symbols(1, &mut buffer, &mut symbols).is_err());
    }

    #[test]
    fn decode_symbols_rejects_non_draco_scheme_ids() {
        let bytes = [2u8];
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut symbols = Vec::new();
        let options = SymbolEncodingOptions::default();

        // The id is what the refusal is about, so the refusal names it.
        let err = decode_symbols(1, 1, &options, &mut buffer, &mut symbols).unwrap_err();
        assert_eq!(err.kind(), crate::status::ErrorKind::UnsupportedFeature);
        assert!(err.message().contains('2'), "{err}");
    }

    #[test]
    fn decode_raw_symbols_rejects_zero_bit_length() {
        let bytes = [0u8];
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut symbols: Vec<u32> = Vec::new();

        let err = decode_raw_symbols(1, &mut buffer, &mut symbols).unwrap_err();
        assert!(err.message().contains("1..=18"), "{err}");
    }

    #[test]
    fn decode_raw_symbols_rejects_bit_length_above_draco_limit() {
        let bytes = [19u8];
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut symbols: Vec<u32> = Vec::new();

        let err = decode_raw_symbols(1, &mut buffer, &mut symbols).unwrap_err();
        assert!(err.message().contains("19"), "{err}");
    }

    #[test]
    fn decode_tagged_symbols_rejects_zero_components() {
        let mut buffer = DecoderBuffer::new(&[]);
        let mut symbols: Vec<u32> = Vec::new();

        assert!(decode_tagged_symbols(1, 0, &mut buffer, &mut symbols).is_err());
    }

    #[test]
    fn decode_tagged_symbols_rejects_partial_component_chunk() {
        let mut buffer = DecoderBuffer::new(&[]);
        let mut symbols: Vec<u32> = Vec::new();

        // The count and the component width both appear: which pair failed to
        // divide is the whole content of the refusal.
        let err = decode_tagged_symbols(5, 2, &mut buffer, &mut symbols).unwrap_err();
        assert!(
            err.message().contains('5') && err.message().contains('2'),
            "{err}"
        );
    }
}

#[cfg(all(test, feature = "encoder", feature = "decoder"))]
mod roundtrip_tests {
    use super::*;

    /// The decode grows its sink; this is the case that needs it to.
    ///
    /// `reserve_within_input` starts at eight symbols per remaining input byte,
    /// which is deliberately below what a compressible stream carries. Highly
    /// repetitive symbols cost a fraction of a bit each, so this run holds far
    /// more values than the reserve, and only decodes in full if `push` is
    /// allowed to grow past it.
    ///
    /// The refusal tests cannot catch a broken growth path -- they assert that
    /// oversized counts are rejected, which a decoder that never grows also
    /// does. Falsified by making the raw scheme stop at `capacity()`: this
    /// fails and nothing else does.
    ///
    /// Only the raw scheme needs it. Tagged spends at least one bit per value
    /// plus a tag, so eight values per byte is its ceiling and the reserve is
    /// always enough -- stopping *its* push at `capacity()` changes nothing,
    /// which is the reason there is one case here rather than two.
    #[test]
    fn a_run_longer_than_the_reserve_decodes_in_full() {
        // Two values, one rare: compressible enough that the byte count lands
        // far under the symbol count, and still entropy coded rather than raw.
        let symbols: Vec<u32> = (0..50_000u32).map(|i| u32::from(i % 997 == 0)).collect();
        let options = SymbolEncodingOptions::default();

        let mut target = EncoderBuffer::new();
        encode_symbols(&symbols, 1, &options, &mut target).unwrap();
        let data = target.data().to_vec();

        let reserve = data.len() * 8;
        assert!(
            symbols.len() > reserve,
            "{} symbols in {} bytes reserves {reserve}: not past the initial              allowance, so this no longer tests growth",
            symbols.len(),
            data.len()
        );

        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        decode_symbols(symbols.len(), 1, &options, &mut source, &mut out).unwrap();
        assert_eq!(out, symbols, "the sink stopped short of the symbol count");
    }

    /// A symbol whose top bit is set forces the tagged scheme's per-chunk
    /// bit length to 32 (`max_value_bit_length` past 18 always selects
    /// TAGGED). `DecoderBuffer::decode_least_significant_bits32_fast` used to
    /// compute `1u32 << nbits` for that width, which panics in a debug build
    /// and silently returns 0 in release; this is unrelated to quantization
    /// bit counts and reproduces the same way with no attribute involved.
    #[test]
    fn tagged_scheme_round_trips_a_full_width_symbol() {
        let symbols = [u32::MAX];
        let options = SymbolEncodingOptions::default();

        let mut target = EncoderBuffer::new();
        encode_symbols(&symbols, 1, &options, &mut target).unwrap();

        let data = target.data().to_vec();
        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        decode_symbols(1, 1, &options, &mut source, &mut out).unwrap();
        assert_eq!(out, [u32::MAX]);
    }

    #[test]
    fn tagged_scheme_round_trips_mixed_width_symbols() {
        let symbols = [0u32, 1, u32::MAX, 1 << 30, u32::MAX - 1];
        let options = SymbolEncodingOptions::default();

        let mut target = EncoderBuffer::new();
        encode_symbols(&symbols, 1, &options, &mut target).unwrap();

        let data = target.data().to_vec();
        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        decode_symbols(5, 1, &options, &mut source, &mut out).unwrap();
        assert_eq!(out, symbols);
    }
}
