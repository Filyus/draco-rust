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

/// Above this many bits the raw scheme cannot represent the alphabet
/// efficiently, and the coder takes the tagged one without weighing them.
#[cfg(feature = "encoder")]
const K_MAX_RAW_ENCODING_BIT_LENGTH: u32 = 18;

#[cfg(feature = "encoder")]
pub fn encode_symbols(
    symbols: &[u32],
    num_components: usize,
    options: &SymbolEncodingOptions,
    target_buffer: &mut EncoderBuffer,
) -> Status {
    let plan = plan_symbols(symbols, num_components);
    encode_symbols_with_plan(symbols, num_components, options, &plan, target_buffer)
}

/// What the symbol coder works out about a set of symbols before it can
/// choose the scheme to write them with: how many chunks need each bit length,
/// and what each scheme would cost.
///
/// Ranking two prediction candidates and choosing the coder's own scheme ask
/// the same question of the same symbols, so the answer is worked out once:
/// the loser's plan is dropped and the winner's is handed to the coder.
///
/// The bit lengths themselves are not kept. Pricing the tagged scheme needs
/// only how many chunks have each length, and writing it -- which only the
/// tagged scheme does -- takes each one from its chunk again; a list of them
/// was a value per chunk, built twice per attribute by the prediction search
/// and dropped unread whenever the raw scheme won.
#[cfg(feature = "encoder")]
pub struct SymbolPlan {
    tag_frequencies: [u64; 33],
    max_value: u32,
    tagged_bits: u64,
    raw_bits: u64,
    raw_frequencies: Vec<u64>,
    raw_num_unique: u32,
}

#[cfg(feature = "encoder")]
impl SymbolPlan {
    /// What these symbols would cost, under whichever scheme is cheaper.
    ///
    /// This is the estimate the coder itself decides by, so ranking candidates
    /// by it ranks them the way the coder will see them.
    pub fn estimated_bits(&self) -> u64 {
        std::cmp::min(self.tagged_bits, self.raw_bits)
    }
}

/// Works out how these symbols would be coded, without coding them.
#[cfg(feature = "encoder")]
pub fn plan_symbols(symbols: &[u32], num_components: usize) -> SymbolPlan {
    if symbols.is_empty() {
        return SymbolPlan {
            tag_frequencies: [0; 33],
            max_value: 0,
            tagged_bits: 0,
            raw_bits: 0,
            raw_frequencies: Vec::new(),
            raw_num_unique: 0,
        };
    }

    let (tag_frequencies, max_value) = count_bit_lengths(symbols, num_components);
    let tagged_bits = compute_tagged_scheme_bits(num_components, &tag_frequencies);
    // RAW is not a candidate past its bit-length limit, so it gets no estimate
    // there: its histogram has one entry per value up to `max_value`, which a
    // single 32-bit symbol would make 32 GiB. Pricing it out keeps
    // `estimated_bits` equal to what the coder will actually write.
    let (raw_bits, raw_frequencies, raw_num_unique) =
        if bit_length(max_value) > K_MAX_RAW_ENCODING_BIT_LENGTH {
            (u64::MAX, Vec::new(), 0)
        } else {
            compute_raw_scheme_bits_and_frequencies(symbols, max_value)
        };

    SymbolPlan {
        tag_frequencies,
        max_value,
        tagged_bits,
        raw_bits,
        raw_frequencies,
        raw_num_unique,
    }
}

/// Writes `symbols` the way `encode_symbols` would, from a plan already built
/// for exactly these symbols.
#[cfg(feature = "encoder")]
pub fn encode_symbols_with_plan(
    symbols: &[u32],
    num_components: usize,
    options: &SymbolEncodingOptions,
    plan: &SymbolPlan,
    target_buffer: &mut EncoderBuffer,
) -> Status {
    if symbols.is_empty() {
        return Ok(());
    }

    if bit_length(plan.max_value) > K_MAX_RAW_ENCODING_BIT_LENGTH
        || plan.tagged_bits < plan.raw_bits
    {
        // Draco bitstream scheme ids (see C++ SymbolCodingMethod):
        //   0 = TAGGED
        //   1 = RAW
        target_buffer.encode_u8(0); // TAGGED
        encode_tagged_symbols(
            symbols,
            num_components,
            &plan.tag_frequencies,
            target_buffer,
        )
    } else {
        target_buffer.encode_u8(1); // RAW
        encode_raw_symbols_with_frequencies(
            symbols,
            plan.max_value,
            &plan.raw_frequencies,
            plan.raw_num_unique,
            target_buffer,
            options.compression_level,
        )
    }
}

/// Bits needed to hold `value`, zero for zero.
#[cfg(feature = "encoder")]
fn bit_length(value: u32) -> u32 {
    32 - value.leading_zeros()
}

/// The number of bits a chunk of components needs: that of its largest value.
///
/// C++ takes `MostSignificantBit(max) + 1`, so a chunk of zeros needs one bit.
#[cfg(feature = "encoder")]
fn chunk_bit_length(chunk: &[u32]) -> u32 {
    bit_length(chunk.iter().copied().max().unwrap_or(0)).max(1)
}

/// How many chunks of components need each bit length, and the largest symbol.
///
/// Counted into four tables, as `histogram` does and for its reason: the
/// lengths cluster, and one table would make each increment wait for the one
/// before it.
#[cfg(feature = "encoder")]
fn count_bit_lengths(symbols: &[u32], num_components: usize) -> ([u64; 33], u32) {
    let mut tables = [[0u64; 33]; 4];
    let mut max_value = 0;
    if num_components == 1 {
        let (quads, remainder) = symbols.as_chunks::<4>();
        for quad in quads {
            for (table, &value) in tables.iter_mut().zip(quad) {
                table[bit_length(value).max(1) as usize] += 1;
            }
            max_value = max_value.max(quad[0].max(quad[1]).max(quad[2].max(quad[3])));
        }
        for &value in remainder {
            tables[0][bit_length(value).max(1) as usize] += 1;
            max_value = max_value.max(value);
        }
    } else if num_components == 3 {
        let (triples, remainder) = symbols.as_chunks::<3>();
        for (index, triple) in triples.iter().enumerate() {
            let largest = triple[0].max(triple[1]).max(triple[2]);
            tables[index & 3][bit_length(largest).max(1) as usize] += 1;
            max_value = max_value.max(largest);
        }
        if !remainder.is_empty() {
            tables[0][chunk_bit_length(remainder) as usize] += 1;
            max_value = max_value.max(remainder.iter().copied().max().unwrap_or(0));
        }
    } else {
        for (index, chunk) in symbols.chunks(num_components).enumerate() {
            let largest = chunk.iter().copied().max().unwrap_or(0);
            tables[index & 3][bit_length(largest).max(1) as usize] += 1;
            max_value = max_value.max(largest);
        }
    }
    let mut frequencies = tables[0];
    for table in &tables[1..] {
        for (total, count) in frequencies.iter_mut().zip(table) {
            *total += count;
        }
    }
    (frequencies, max_value)
}

#[cfg(feature = "encoder")]
fn compute_raw_scheme_bits_and_frequencies(
    symbols: &[u32],
    max_value: u32,
) -> (u64, Vec<u64>, u32) {
    if symbols.is_empty() {
        return (0, Vec::new(), 0);
    }

    let frequencies: Vec<u64> = histogram(symbols, max_value);

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
fn compute_tagged_scheme_bits(num_components: usize, tag_frequencies: &[u64; 33]) -> u64 {
    // 1. Bits for values (raw bits)
    let mut value_bits = 0;
    for (len, &count) in tag_frequencies.iter().enumerate() {
        value_bits += len as u64 * num_components as u64 * count;
    }

    // 2. Bits for tags (RAns) using C++ ComputeShannonEntropy on bit lengths.
    // C++ calls ComputeShannonEntropy(bit_lengths, num_chunks, max_value=32),
    // which is the entropy of exactly these counts.
    let (tag_bits, num_unique_symbols) = shannon_entropy_bits_trunc(tag_frequencies);

    // C++ uses num_unique_symbols for BOTH params in the tagged scheme.
    let table_bits = approximate_rans_frequency_table_bits(num_unique_symbols, num_unique_symbols);

    value_bits + (tag_bits as u64) + table_bits
}

/// Counts how often each symbol occurs.
///
/// A histogram is a scatter into one small table, so a run of equal or nearby
/// symbols makes each increment wait for the one before it to leave the store
/// buffer. Counting into four independent tables and adding them up breaks that
/// chain: the four increments in flight are to four different tables by
/// construction, whatever the symbols are. The tables are only worth their
/// cache footprint while they are small - a 16-bit position attribute reaches
/// one of 2^17 symbols - so a wide alphabet keeps the single table, where the
/// chain is rare and the misses are what cost.
///
/// The counter type is the caller's, so counting straight into the width the
/// caller needs costs no pass over the alphabet to widen it afterwards. That
/// pass is not free: an attribute with few symbols over a wide alphabet walks
/// far more table than it does data.
#[cfg(feature = "encoder")]
fn histogram<T>(symbols: &[u32], max_value: u32) -> Vec<T>
where
    T: Copy + Default + std::ops::Add<Output = T> + std::ops::AddAssign + From<u8>,
{
    let len = max_value as usize + 1;
    let one = T::from(1u8);

    /// Four tables of this many counters still sit inside a 64 KiB L1.
    const INTERLEAVED_MAX_LEN: usize = 1 << 12;

    if len > INTERLEAVED_MAX_LEN {
        let mut frequencies = vec![T::default(); len];
        for &sym in symbols {
            frequencies[sym as usize] += one;
        }
        return frequencies;
    }

    let mut tables = vec![T::default(); len * 4];
    let (first, rest) = tables.split_at_mut(len);
    let (second, rest) = rest.split_at_mut(len);
    let (third, fourth) = rest.split_at_mut(len);

    let (quads, remainder) = symbols.as_chunks::<4>();
    for quad in quads {
        first[quad[0] as usize] += one;
        second[quad[1] as usize] += one;
        third[quad[2] as usize] += one;
        fourth[quad[3] as usize] += one;
    }
    for &sym in remainder {
        first[sym as usize] += one;
    }

    for index in 0..len {
        first[index] += second[index] + third[index] + fourth[index];
    }
    tables.truncate(len);
    tables
}

#[cfg(feature = "encoder")]
fn shannon_entropy_bits_trunc(frequencies: &[u64]) -> (i64, u32) {
    // Draco C++ ComputeShannonEntropy():
    //   total_bits += freq * log2(freq / num_symbols)
    //   return static_cast<int64_t>(-total_bits);
    // The cast truncates toward zero.

    let num_symbols_d = frequencies.iter().sum::<u64>() as f64;
    let log2_num_symbols = num_symbols_d.log2();
    let mut total_bits = 0.0f64;
    let mut num_unique_symbols: u32 = 0;

    for &freq in frequencies {
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
    let frequencies: Vec<u64> = histogram(symbols, max_value);

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
    frequencies: &[u64; 33],
    target_buffer: &mut EncoderBuffer,
) -> Status {
    // Scheme: Tagged is already written by caller

    // Encode bit lengths using RAns, from how often each (0..32) occurs.
    // Draco uses unique_symbols_bit_length=5 for tagged bit-length tags,
    // which corresponds to rANS precision bits = 12.
    let mut tag_encoder = RAnsSymbolEncoder::<12>::new();
    if !tag_encoder.create(frequencies, 33, target_buffer) {
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

    // The raw values go in a separate bit sequence (C++ value_buffer), whose
    // size the counts already give.
    let value_bits: u64 = frequencies
        .iter()
        .enumerate()
        .map(|(len, &count)| len as u64 * count * num_components as u64)
        .sum();
    let mut values = BitPacker::with_capacity(value_bits.div_ceil(8) as usize + 8);

    let num_chunks = symbols.len().div_ceil(num_components);
    tag_encoder.start_encoding_with_capacity(
        target_buffer,
        num_chunks.saturating_mul(2).saturating_add(4),
    );

    // 1. Encode bits in FORWARD order (because our BitEncoder is FIFO).
    for chunk in symbols.chunks(num_components) {
        let len = chunk_bit_length(chunk);
        for &val in chunk {
            values.put(len, val);
        }
    }

    // 2. Encode tags in REVERSE order (because ANS is LIFO).
    for chunk in symbols.chunks(num_components).rev() {
        tag_encoder.encode_symbol(chunk_bit_length(chunk));
    }

    tag_encoder.end_encoding(target_buffer);
    target_buffer.encode_data(&values.finish());
    Ok(())
}

/// Bits packed least significant first into bytes, the layout
/// `EncoderBuffer::encode_least_significant_bits32` writes, built in a 64-bit
/// accumulator and stored four bytes at a time.
///
/// The buffer's own writer ORs each value into bytes it zero-filled up front,
/// one byte per step of a loop, and is sized for 32 bits a value whatever the
/// values need: a quarter of a gigabyte zeroed for a scan's colours that
/// packed into a tenth of it.
#[cfg(feature = "encoder")]
struct BitPacker {
    bytes: Vec<u8>,
    pending: u64,
    pending_bits: u32,
}

#[cfg(feature = "encoder")]
impl BitPacker {
    fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
            pending: 0,
            pending_bits: 0,
        }
    }

    /// Appends the low `nbits` bits of `value`, `nbits` in `1..=32`.
    fn put(&mut self, nbits: u32, value: u32) {
        let value = u64::from(value) & ((1u64 << nbits) - 1);
        // Below 32 bits pending before and at most 32 added: inside 64.
        self.pending |= value << self.pending_bits;
        self.pending_bits += nbits;
        if self.pending_bits >= 32 {
            self.bytes
                .extend_from_slice(&(self.pending as u32).to_le_bytes());
            self.pending >>= 32;
            self.pending_bits -= 32;
        }
    }

    /// The packed bytes, the last one padded with zeros.
    fn finish(mut self) -> Vec<u8> {
        let tail = self.pending_bits.div_ceil(8) as usize;
        self.bytes
            .extend_from_slice(&self.pending.to_le_bytes()[..tail]);
        self.bytes
    }
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

/// Steps `in_buffer` over the symbols [`decode_symbols`] would have read,
/// without reading them.
///
/// `Ok(true)` when it did. `Ok(false)` when this stream cannot be stepped over
/// without decoding it -- the tagged scheme, whose coded bytes end where its
/// last value's bits do and nowhere the stream says -- or when it is not one the
/// decoder would accept; the buffer's position is then unspecified and the
/// caller goes back and decodes. Only the raw scheme is stepped over: its
/// frequency table is read and the rANS state initialised, which walks the
/// position past the coded bytes, and no symbol is drawn from them.
#[cfg(feature = "point_cloud_decode")]
pub(crate) fn skip_symbols(
    num_values: usize,
    num_components: usize,
    in_buffer: &mut DecoderBuffer,
) -> Result<bool, DracoError> {
    if num_values == 0 {
        return Ok(true);
    }
    if num_components == 0 || !num_values.is_multiple_of(num_components) {
        return Ok(false);
    }
    // Draco uses: 0 = TAGGED, 1 = RAW.
    match in_buffer.decode_u8() {
        Ok(1) => {}
        _ => return Ok(false),
    }
    let Ok(symbols_bit_length) = in_buffer.decode_u8() else {
        return Ok(false);
    };
    let symbols_bit_length = u32::from(symbols_bit_length);
    if !(1..=18).contains(&symbols_bit_length) {
        return Ok(false);
    }
    let mut decoder = RAnsSymbolDecoder::new(
        compute_rans_precision_from_unique_symbols_bit_length(symbols_bit_length),
    );
    Ok(decoder.create(in_buffer) && decoder.start_decoding(in_buffer))
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
/// Sixty-four per byte rather than one per byte because the byte was both
/// wrong and slow: entropy coding beats one symbol per byte routinely, so real
/// streams reserved a fraction of what they needed and paid for the
/// reallocations -- 598 to 698 us on a 10,000-point decode. Eight per byte (a
/// bit per symbol) fell short on the seeded ribbon at speed 5, whose
/// strip-regular corrections code at 2.7 symbols per *bit*; thirty-two fell
/// short on the same ribbon at speed 0, which reaches 4.4 per bit. At
/// sixty-four per byte every corpus payload at every speed reserves once
/// (speed 0 is the densest coding a Draco encoder produces), a 9 KB stream
/// claiming two billion symbols still reserves 2.4 MB rather than 8 GB, and
/// the hostile budget (256 bytes of u32 per input byte) stays under the
/// corner table's accepted 576. No ratio covers every stream -- a degenerate
/// symbol distribution makes symbols-per-byte unbounded -- so this stays a
/// measured dial.
#[cfg(feature = "decoder")]
fn reserve_within_input(symbols: &mut Vec<u32>, num_values: usize, in_buffer: &DecoderBuffer) {
    symbols.reserve(num_values.min(in_buffer.remaining_size().saturating_mul(64)));
}

#[cfg(feature = "decoder")]
pub fn decode_raw_symbols(
    num_values: usize,
    in_buffer: &mut DecoderBuffer,
    symbols: &mut Vec<u32>,
) -> Status {
    let mut decoder = open_raw_symbols(num_values, in_buffer)?;

    // Growth is capped at `num_values` the same way the corner table caps at
    // the declared face count: the target is only reached after decoding a
    // capacity's worth of real symbols, so it never exceeds doubling of
    // proven content, and a truthful count lands the buffer exactly at its
    // final size instead of overshooting by up to 2x -- on a mesh whose
    // symbols outgrow the initial input-bounded reserve, the doubling copies
    // alone moved 700 KB per decode.
    let mut index = 0;
    while index < num_values {
        if symbols.len() == symbols.capacity() {
            let doubled = symbols.capacity().saturating_mul(2).max(symbols.len() + 1);
            let target = doubled.min(num_values.max(symbols.len() + 1));
            symbols
                .try_reserve_exact(target - symbols.len())
                .map_err(|_| {
                    DracoError::general(format!("Failed to allocate {target} raw symbols"))
                })?;
        }
        let chunk_end = num_values.min(index + (symbols.capacity() - symbols.len()));
        // One call per chunk rather than per symbol: the run loop hoists both
        // tables, the input and the coder state out of the loop, which it can
        // only do over a span it owns. The chunk is already sized to the spare
        // capacity, so this fills without reallocating.
        if !decoder.decode_run(symbols, chunk_end - index) {
            return Err(raw_run_outlived_its_input(num_values));
        }
        index = chunk_end;
    }
    Ok(())
}

#[cfg(feature = "decoder")]
fn raw_run_outlived_its_input(num_values: usize) -> DracoError {
    DracoError::new(
        crate::status::ErrorKind::AllocationExceedsInput,
        format!("the stream declared {num_values} symbols, more than its coded bytes carry"),
    )
}

/// Two attributes' raw symbols, decoded side by side -- see
/// [`decode_run_pair`](crate::rans_symbol_decoder::decode_run_pair) for why
/// that is faster than one after the other. `buffer` stands at the first
/// stream's scheme byte, as [`decode_symbols`] would find it, and is left past
/// its symbols; the second stream's scheme byte is at `second.0` in the same
/// data, with `second.1` values of `second.2` components.
///
/// The second stream is read through a buffer of its own, opened there after
/// the first stream has been charged for, and what it charges is then
/// `buffer`'s: the two draw on one budget, in the order a decode one after the
/// other would.
///
/// `None` where either stream is not one this takes -- the tagged scheme, no
/// values, a count that is not whole entries -- or where anything goes wrong at
/// all, a run its coded bytes do not back included. The caller then puts the
/// budget back as it was and decodes each the ordinary way, which is what
/// reports the error if there is one: so this changes when the symbols are
/// decoded, never what comes of a stream.
#[cfg(feature = "point_cloud_decode")]
pub(crate) fn decode_raw_symbol_pair(
    buffer: &mut DecoderBuffer,
    first: (usize, usize),
    second: (usize, usize, usize),
) -> Option<(Vec<u32>, Vec<u32>)> {
    fn open<'a>(
        buffer: &mut DecoderBuffer<'a>,
        num_values: usize,
        num_components: usize,
    ) -> Option<(RAnsSymbolDecoder<'a>, Vec<u32>)> {
        if num_values == 0
            || num_components == 0
            || !num_values.is_multiple_of(num_components)
            || buffer.decode_u8().ok()? != 1
        {
            return None;
        }
        let decoder = open_raw_symbols(num_values, buffer).ok()?;
        // The count is backed now: what the payload cannot account for was
        // just charged to the budget, so reserving for it is what the budget
        // has agreed to.
        let mut symbols = Vec::new();
        symbols.try_reserve_exact(num_values).ok()?;
        Some((decoder, symbols))
    }
    let (count_a, count_b) = (first.0, second.1);
    let (mut decoder_a, mut symbols_a) = open(buffer, first.0, first.1)?;
    let mut other = buffer.fork_at(second.0).ok()?;
    let (mut decoder_b, mut symbols_b) = open(&mut other, second.1, second.2)?;
    buffer.adopt_budget(&other);
    let both = count_a.min(count_b);
    let (backed_a, backed_b) = crate::rans_symbol_decoder::decode_run_pair(
        &mut decoder_a,
        &mut symbols_a,
        &mut decoder_b,
        &mut symbols_b,
        both,
    );
    let rest_a = decoder_a.decode_run(&mut symbols_a, count_a - both);
    let rest_b = decoder_b.decode_run(&mut symbols_b, count_b - both);
    (backed_a && backed_b && rest_a && rest_b).then_some((symbols_a, symbols_b))
}

/// Reads a raw symbol stream's header and frequency table, starts its rANS
/// coder, and charges the budget for the part of `num_values` the payload
/// cannot account for. `in_buffer` is left past the stream.
#[cfg(feature = "decoder")]
fn open_raw_symbols<'a>(
    num_values: usize,
    in_buffer: &mut DecoderBuffer<'a>,
) -> Result<RAnsSymbolDecoder<'a>, DracoError> {
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
    // Taken before `start_decoding` walks past the coded bytes, so the
    // difference below is the payload this run has to work from.
    let before_payload = in_buffer.remaining_size();
    if !decoder.start_decoding(in_buffer) {
        return Err(DracoError::general(
            "Failed to start rANS decoding of the raw symbols",
        ));
    }
    let payload_bytes = before_payload.saturating_sub(in_buffer.remaining_size());
    // Only the part of the count the payload cannot plausibly account for is
    // charged, so a real stream charges nothing and a runaway is bounded.
    //
    // Neither the payload nor the coder gives an exact bound here. rANS spends
    // well under a bit on a near-certain symbol, and its state does not have
    // to fall out of range once the bytes are spent: a near-deterministic
    // alphabet keeps producing symbols from state alone indefinitely, which is
    // how a small stream asked for two billion values and got them, in nine
    // seconds. An alphabet of *one* is the extreme -- no payload at all, so
    // nothing in the stream says how far the run goes -- and a constant
    // attribute reaches it legitimately, at any count. So the unbacked part is
    // drawn first from the values the caller's limits admitted for the
    // attributes, which a constant attribute's run is, and only what exceeds
    // them reaches the budget; see `DecoderBuffer::charge_unbacked`.
    //
    // Sixty-four symbols per payload byte is the same measured dial the
    // reserve below uses: the densest coding a Draco encoder produces is 4.4
    // symbols per *bit* on the seeded ribbon at speed 0, which is 35 per byte.
    let backed_by_payload = payload_bytes.saturating_mul(64);
    if num_values > backed_by_payload {
        in_buffer.charge_unbacked(num_values - backed_by_payload, size_of::<u32>())?;
    }
    Ok(decoder)
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

    /// The packer writes the bytes the buffer's own bit writer does, for
    /// every width, values carrying bits above their width included.
    #[test]
    fn the_bit_packer_writes_what_the_buffer_writer_does() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut writes = Vec::new();
        for _ in 0..5000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            writes.push((1 + (seed % 32) as u32, (seed >> 32) as u32));
        }
        for count in [0, 1, 7, 5000] {
            let mut buffer = EncoderBuffer::new();
            buffer.start_bit_encoding(32 * count, false);
            let mut packer = BitPacker::with_capacity(0);
            for &(nbits, value) in &writes[..count] {
                buffer.encode_least_significant_bits32(nbits, value);
                packer.put(nbits, value);
            }
            buffer.end_bit_encoding();
            assert_eq!(packer.finish(), buffer.data(), "{count} writes");
        }
    }

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

    /// A count larger than the coded bytes can back is refused, not filled.
    ///
    /// The rANS coder does not run out: once its payload is spent the state can
    /// no longer renormalize and every further symbol is a function of the
    /// state alone, so a declared count is a promise the decoder used to keep
    /// no matter what -- 134 million symbols out of a 226-byte file, two and a
    /// half seconds and 86 MB of them, in the `decode_drc` campaign that found
    /// this.
    ///
    /// The count is not bounded against the input size here, and cannot be:
    /// the sibling test above encodes 50,000 symbols into 82 bytes, so any
    /// symbols-per-byte constant that admits it admits this too. What
    /// separates them is the coded payload running out, which is what the
    /// decoder now reports.
    #[test]
    fn a_symbol_count_the_coded_bytes_cannot_back_is_refused() {
        // Two symbols, one rare: the raw scheme, as in the sibling test.
        let symbols: Vec<u32> = (0..50_000u32).map(|i| u32::from(i % 997 == 0)).collect();
        let options = SymbolEncodingOptions::default();

        let mut target = EncoderBuffer::new();
        encode_symbols(&symbols, 1, &options, &mut target).unwrap();
        let data = target.data().to_vec();

        // The stream is intact and its own count decodes.
        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        decode_symbols(symbols.len(), 1, &options, &mut source, &mut out).unwrap();
        assert_eq!(out, symbols);

        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        let error = decode_symbols(50_000_000, 1, &options, &mut source, &mut out)
            .expect_err("a count the payload cannot back decoded anyway");
        assert_eq!(
            error.kind(),
            crate::status::ErrorKind::AllocationExceedsInput
        );
        assert!(
            out.len() < 50_000_000,
            "the sink was filled to the declared count before failing"
        );
    }

    /// A count the coded bytes cannot account for is charged to the budget,
    /// not filled.
    ///
    /// The sibling above pins the case where the coder runs out of state. This
    /// is the case where it does not: a near-deterministic alphabet keeps
    /// producing symbols from state alone after its payload is spent, so
    /// nothing inside the coder ever objects. Before the charge, this decoded
    /// two billion values out of a hundred-odd bytes and took nine seconds
    /// doing it.
    ///
    /// The bound cannot be exact -- rANS spends well under a bit on a
    /// near-certain symbol -- so what is charged is only the part of the count
    /// the payload cannot plausibly back, at the same sixty-four symbols per
    /// byte the reserve uses.
    #[test]
    fn a_symbol_count_the_payload_cannot_account_for_is_refused() {
        let symbols = vec![7u32; 4_000];
        let options = SymbolEncodingOptions::default();

        let mut target = EncoderBuffer::new();
        encode_symbols(&symbols, 1, &options, &mut target).unwrap();
        let data = target.data().to_vec();

        // Its own count decodes and charges nothing.
        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        decode_symbols(symbols.len(), 1, &options, &mut source, &mut out).unwrap();
        assert_eq!(out, symbols);

        let mut source = DecoderBuffer::new(&data);
        let mut out = Vec::new();
        let error = decode_symbols(2_000_000_000, 1, &options, &mut source, &mut out)
            .expect_err("a count past the ceiling was filled anyway");
        assert_eq!(
            error.kind(),
            crate::status::ErrorKind::AllocationExceedsInput
        );
        assert!(
            out.len() < 2_000_000_000,
            "the sink was filled to the declared count before failing"
        );
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

    /// Planning is sized by the scheme the coder can use, not by the largest
    /// symbol: RAW's histogram has an entry per value, so pricing it for a
    /// symbol past its limit would allocate in proportion to that symbol --
    /// 32 GiB for one near `u32::MAX`. `1 << 24` keeps a regression at 128 MiB.
    #[test]
    fn a_plan_past_the_raw_limit_builds_no_histogram() {
        let symbols = [0u32, 1 << 24, 7];
        let plan = plan_symbols(&symbols, 1);
        assert!(
            plan.raw_frequencies.is_empty(),
            "planned a {}-entry RAW histogram for a scheme the coder cannot pick",
            plan.raw_frequencies.len()
        );
        assert_eq!(plan.estimated_bits(), plan.tagged_bits);

        let options = SymbolEncodingOptions::default();
        let mut planned = EncoderBuffer::new();
        encode_symbols_with_plan(&symbols, 1, &options, &plan, &mut planned).unwrap();
        let mut direct = EncoderBuffer::new();
        encode_symbols(&symbols, 1, &options, &mut direct).unwrap();
        assert_eq!(planned.data(), direct.data());
    }
}
