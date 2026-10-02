//! Multi-symbol rANS decoder.
//!
//! [`RAnsSymbolDecoder`] reconstructs symbols from a probability table and a
//! lookup table built during initialization. Precision is stored at runtime
//! (rather than as a const generic) to avoid monomorphization bloat while
//! keeping shift/mask-based decoding. Port of Draco's `rans_symbol_decoder.h`.

use crate::ans::AnsDecoder;
use crate::decoder_buffer::DecoderBuffer;
use crate::rans_symbol_coding::RAnsSymbol;

/// The symbol that owns each slot of `[0, rans_precision)`.
///
/// The decoder reads one slot per symbol, and which one is the low bits of the
/// coder state -- uniform over the whole table, whatever the symbol
/// probabilities. So the table's footprint is what decides where those reads
/// land: a 16-bit position attribute codes at 18 or 19 bits of precision, a
/// million-byte table in `u32` and half that in `u16`, and every symbol waits
/// on the read. An alphabet of at most 2^16 symbols takes the narrow form,
/// which is every alphabet this crate's encoder writes for a quantized
/// attribute up to 16 bits.
enum Slots {
    Narrow(Vec<u16>),
    Wide(Vec<u32>),
}

impl Slots {
    fn len(&self) -> usize {
        match self {
            Slots::Narrow(slots) => slots.len(),
            Slots::Wide(slots) => slots.len(),
        }
    }

    /// Bytes one slot takes.
    fn slot_bytes(&self) -> usize {
        match self {
            Slots::Narrow(_) => 2,
            Slots::Wide(_) => 4,
        }
    }

    fn get(&self, slot: usize) -> Option<u32> {
        match self {
            Slots::Narrow(slots) => slots.get(slot).map(|&id| u32::from(id)),
            Slots::Wide(slots) => slots.get(slot).copied(),
        }
    }
}

/// The id of the symbol that owns each slot, for a table whose probabilities
/// add up to `precision`. `T` holds every id below `table.len()`, which is what
/// the caller chose it by.
fn fill_slots<T: Copy + Default + TryFrom<usize>>(
    table: &[RAnsSymbol],
    precision: usize,
) -> Vec<T> {
    let mut slots = Vec::with_capacity(precision);
    for (id, sym) in table.iter().enumerate() {
        let id = T::try_from(id).unwrap_or_default();
        slots.resize(slots.len() + sym.prob as usize, id);
    }
    slots
}

/// How many buckets `run_buckets` summarizes a fine slot table in: 32 KB of
/// entries, which stays in L1. Measured on a scan's position stream at 18 bits
/// of precision, 4096 beat 8192 and 16384.
const BUCKETS: usize = 1 << 12;

/// RAnsSymbolDecoder with runtime precision to avoid monomorphization bloat.
/// Instead of const generics, we store the precision bits at runtime.
/// Performance is preserved by storing `rans_precision_bits` and using bit
/// operations (shift/mask) instead of division/modulo.
pub struct RAnsSymbolDecoder<'a> {
    pub ans: AnsDecoder<'a>,
    probability_table: Vec<RAnsSymbol>,
    lut: Slots,
    /// Per slot, what decoding it does to the state, as `prob | (slot -
    /// cum_prob) << 16`: the probability and the offset into it of the symbol
    /// that owns the slot. Built by `decode_run` for a run long enough to pay
    /// for it, and only while both halves fit in 16 bits; empty otherwise.
    steps: Vec<u32>,
    /// For a table too fine for `steps`, one entry per `BUCKETS`-th of the slot
    /// range: the first symbol whose slots reach into it and, when that symbol
    /// owns the whole bucket, its probability and cumulative probability too.
    /// See `run_buckets`. Built by `decode_run` like `steps`; empty otherwise.
    buckets: Vec<u64>,
    /// Whether `build_buckets` has run for this table, kept or not.
    buckets_tried: bool,
    num_symbols: usize,
    /// `probability_table.len() - 1`, the table having been padded to a power
    /// of two. Masking a symbol id with it makes the lookup provably in
    /// bounds, so the run loop indexes the table without a check and without
    /// `unsafe`; the padding entries are unreachable through a LUT this
    /// decoder built.
    table_mask: u32,
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
            lut: Slots::Narrow(Vec::new()),
            steps: Vec::new(),
            buckets: Vec::new(),
            buckets_tried: false,
            num_symbols: 0,
            table_mask: 0,
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
        self.steps.clear();
        self.buckets.clear();
        self.buckets_tried = false;
        let bitstream_version = buffer.bitstream_version();
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

        // Each probability-table entry consumes at least one input byte while it
        // is decoded below, and a single byte can cover at most 64 entries (a
        // zero-frequency run encodes up to 63 extra symbols). A count beyond that
        // bound cannot be backed by the remaining input, so reject it before
        // resizing instead of allocating gigabytes for a malformed varint. This
        // is a relative input-consistency check on a cold path, not a fixed cap.
        if num_symbols > buffer.remaining_size().saturating_mul(64) {
            return false;
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
        let mut cum_prob: u32 = 0;
        for sym in &mut self.probability_table[..num_symbols] {
            sym.cum_prob = cum_prob;
            cum_prob = cum_prob.saturating_add(sym.prob);
            if cum_prob > self.rans_precision {
                // Malformed probability table - probabilities exceed precision
                return false;
            }
        }

        if cum_prob != self.rans_precision {
            return false;
        }
        let table = &self.probability_table[..num_symbols];
        let precision = self.rans_precision as usize;
        self.lut = if num_symbols <= 1 << 16 {
            Slots::Narrow(fill_slots(table, precision))
        } else {
            Slots::Wide(fill_slots(table, precision))
        };

        // Pad the table to a power of two so `decode_run` can mask instead of
        // check. The entries added here carry a zero probability and cover no
        // LUT slot, so reaching one would already mean the LUT was built by
        // something other than the loop above.
        let padded = num_symbols.next_power_of_two();
        self.probability_table.resize(padded, RAnsSymbol::default());
        self.table_mask = (padded - 1) as u32;
        true
    }

    /// How many distinct symbols the table holds. One or none is the case with
    /// no rANS state at all: the encoder wrote no payload, so the run is that
    /// symbol repeated and nothing in the stream bounds how many times.
    pub fn num_symbols(&self) -> usize {
        self.num_symbols
    }

    pub fn start_decoding(&mut self, buffer: &mut DecoderBuffer<'a>) -> bool {
        // Draco advances the buffer past the encoded rANS data regardless of the
        // number of symbols (the encoded size prefix is always present).
        // C++: v < 2.0 uses fixed u64, v >= 2.0 uses varint u64.
        let bitstream_version = buffer.bitstream_version();
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
        // Multi-symbol rANS may use the 4-byte (0xC0) final-state encoding.
        if !self.ans.read_init(self.l_rans_base, true) {
            return false;
        }

        if buffer.try_advance(bytes_to_read).is_err() {
            return false;
        }
        true
    }

    /// Decodes `count` symbols onto the end of `out`.
    ///
    /// The per-symbol form below is what the tagged scheme needs, where each
    /// symbol is interleaved with reads from a second bit stream. The raw
    /// scheme decodes a run of them against nothing else, and this is that
    /// run: the two tables, the input and the coder state all become locals,
    /// so the loop carries no reload and no check the tables have not already
    /// proved. The symbols are written where they land in `out`, which a run
    /// into a slice could only do after the slice had been filled once with
    /// zeros to exist.
    ///
    /// The arithmetic wraps by construction rather than by hope. `state` stays
    /// below `l_rans_base * 256`, so `quo` is under `256 * 4` and `quo * prob`
    /// under `2^30`; `rem` lands inside the LUT slot owned by its own symbol,
    /// so `rem - cum_prob` is the offset within that symbol's range and cannot
    /// go negative. Both hold for any table `decode_table` accepted, which is
    /// the only way one is built.
    /// Returns whether every symbol came out of the coded bytes. A `false` says
    /// the run outlived its input: `start_decoding` gave the coder a payload of
    /// exactly the length the stream declared, and once that is spent the state
    /// can no longer renormalize, so each further slot is a function of the
    /// state alone and carries no information from the file. The loop below
    /// would otherwise fill a caller-declared count with those -- 134 million
    /// of them out of 226 bytes in the case that put this check here. The
    /// alternative, bounding the count against the input size, does not exist:
    /// rANS spends well under a bit on a near-certain symbol, and this crate's
    /// own encoder writes 50,000 symbols into 82 bytes.
    pub fn decode_run(&mut self, out: &mut Vec<u32>, count: usize) -> bool {
        // A single-symbol alphabet carries no rANS state at all -- the encoder
        // wrote nothing and `start_decoding` initialized nothing -- so the run
        // is that symbol repeated.
        if self.num_symbols <= 1 {
            out.resize(out.len() + count, 0);
            return true;
        }
        if !self.prepare_run(count) {
            out.resize(out.len() + count, 0);
            return false;
        }
        // Buckets cost a write each, a few thousand of them, for a table whose
        // slots are past what steps can hold.
        if self.steps.is_empty() && !self.buckets_tried && count >= BUCKETS {
            self.buckets_tried = true;
            self.build_buckets();
        }
        // The slot table is moved out for the length of the run so the loops
        // can borrow it next to the coder state they update.
        let lut = std::mem::replace(&mut self.lut, Slots::Narrow(Vec::new()));
        let form = (self.steps.is_empty(), self.buckets.is_empty());
        let backed = match (&lut, form) {
            (Slots::Narrow(slots), (false, _)) => self.run_steps(slots, out, count),
            (Slots::Wide(slots), (false, _)) => self.run_steps(slots, out, count),
            (Slots::Narrow(slots), (true, false)) => self.run_buckets(slots, out, count),
            (Slots::Wide(slots), (true, false)) => self.run_buckets(slots, out, count),
            (Slots::Narrow(slots), (true, true)) => self.run_table(slots, out, count),
            (Slots::Wide(slots), (true, true)) => self.run_table(slots, out, count),
        };
        self.lut = lut;
        backed
    }

    /// Builds `buckets` for a table finer than `BUCKETS` slots, or leaves it
    /// empty where an entry cannot hold what it needs -- a symbol id past 21
    /// bits; a probability or a cumulative one is at most 2^20, inside 21 --
    /// or where the slot table alone is the faster way.
    ///
    /// A bucket one symbol owns is the step in one read from L1. A bucket
    /// several symbols share sends the run to the slot table after all, and
    /// the branch between the two, taken by data, costs a pipeline flush each
    /// time it is mispredicted. So buckets pay where the reads they save
    /// outweigh the flushes, which turns on what a slot-table read costs --
    /// where the table sits, a property of the processor -- and on the shape
    /// of the stream: at the same share, a flat run of symbols each a bucket
    /// or two wide makes the branch a coin toss, where a peak owning many
    /// buckets beside a flat tail does not.
    ///
    /// Neither is something the decoder can know or ask, so the rule is the
    /// one whose worst case is smallest across the processors measured (Zen 3,
    /// 4 and 5, Ice Lake, Neoverse N2, Apple M1), over owned shares from none
    /// to all and over smooth, flat and peaked streams:
    ///
    /// - A table of 2^18 slots, 512 KiB, is read from L2 on most of them, and
    ///   the buckets pay from about two thirds owned. Below that a flat stream
    ///   loses up to 46% to them (Zen 5), a smooth one gains up to 15% (Zen 3).
    /// - Every larger table keeps them whatever the share: on a smooth stream
    ///   with a third owned that costs up to 17% (Zen 5), and the share rule it
    ///   replaces cost up to 72% at 70% owned (Neoverse N2). A stream with
    ///   none owned decodes the same either way.
    fn build_buckets(&mut self) {
        /// The slot table past which the buckets are kept whatever the share.
        const ALWAYS_PAST_BYTES: usize = 1 << 19;
        let precision = self.rans_precision as usize;
        if precision <= BUCKETS || self.num_symbols >= 1 << 21 {
            return;
        }
        let width = precision / BUCKETS;
        let table = &self.probability_table;
        let mut buckets = Vec::with_capacity(BUCKETS);
        for bucket in 0..BUCKETS {
            let (Some(first), Some(last)) = (
                self.lut.get(bucket * width),
                self.lut.get((bucket + 1) * width - 1),
            ) else {
                return;
            };
            let sym = table[(first & self.table_mask) as usize];
            buckets.push(if first == last {
                1 << 63
                    | u64::from(first) << 42
                    | u64::from(sym.prob) << 21
                    | u64::from(sym.cum_prob)
            } else {
                0
            });
        }
        let owned = buckets.iter().filter(|&&entry| entry >> 63 != 0).count();
        if self.lut.len() * self.lut.slot_bytes() > ALWAYS_PAST_BYTES || owned * 3 >= BUCKETS * 2 {
            self.buckets = buckets;
        }
    }

    /// The run against `buckets`: the low bits of the state pick a bucket, a
    /// table that sits in L1 where the slot table it summarizes does not. A
    /// bucket one symbol owns -- most of the probability mass, since a likely
    /// symbol owns many buckets whole -- gives the step in that one read; a
    /// bucket several symbols share is read through `slots` as `run_table`
    /// reads every symbol.
    ///
    /// A scan's position codes at 18-20 bits over tens of thousands of
    /// symbols, so its slot table is half a megabyte or more, and the state
    /// picks a slot in it uniformly: the read missed the near caches on most
    /// symbols, where the bucket read does on few.
    fn run_buckets<T: Copy + Into<u32>>(
        &mut self,
        slots: &[T],
        out: &mut Vec<u32>,
        count: usize,
    ) -> bool {
        const LOW: u64 = (1 << 21) - 1;
        let precision = self.rans_precision as usize;
        let slots = &slots[..precision];
        let buckets = &self.buckets[..BUCKETS];
        let table = &self.probability_table[..];
        let table_mask = self.table_mask;
        let mask = self.rans_precision_mask;
        let bits = self.rans_precision_bits;
        let shift = bits - BUCKETS.trailing_zeros();
        let l_base = self.ans.l_base;
        let buf = self.ans.buf;
        let mut offset = self.ans.buf_offset.min(buf.len());
        let mut state = self.ans.state;

        let mut backed = true;
        out.extend((0..count).map(|_| {
            while state < l_base && offset > 0 {
                offset -= 1;
                state = (state << 8) | buf[offset] as u32;
            }
            backed &= state >= l_base;
            let quo = state >> bits;
            let rem = state & mask;
            // `rem >> shift` is below `BUCKETS`, the table's length, and `rem`
            // below `slots.len() == mask + 1`.
            let entry = buckets[(rem >> shift) as usize];
            let (symbol_id, prob, cum_prob) = if entry >> 63 != 0 {
                (
                    ((entry >> 42) & LOW) as u32,
                    (entry >> 21 & LOW) as u32,
                    (entry & LOW) as u32,
                )
            } else {
                let symbol_id: u32 = slots[rem as usize].into();
                let sym = table[(symbol_id & table_mask) as usize];
                (symbol_id, sym.prob, sym.cum_prob)
            };
            state = quo
                .wrapping_mul(prob)
                .wrapping_add(rem.wrapping_sub(cum_prob));
            symbol_id
        }));

        self.ans.buf_offset = offset;
        self.ans.state = state;
        backed
    }

    /// Whether a run of `count` can go through [`decode_run_pair`]'s loop:
    /// steps built (or buildable, and worth it) over a narrow slot table.
    #[cfg(feature = "point_cloud_decode")]
    fn ready_for_pair(&mut self, count: usize) -> bool {
        self.num_symbols > 1
            && self.prepare_run(count)
            && !self.steps.is_empty()
            && matches!(self.lut, Slots::Narrow(_))
    }

    /// Readies a run of `count` symbols: `false` where the table is not one a
    /// run can read from, and otherwise `steps` built if a run this long
    /// repays them. A table costs one write per slot, and a step saves the run
    /// a dependent read per symbol; half a table's worth of symbols is past
    /// where the two meet.
    fn prepare_run(&mut self, count: usize) -> bool {
        let precision = self.rans_precision as usize;
        if self.lut.len() < precision || self.probability_table.is_empty() {
            return false;
        }
        if self.steps.is_empty() && count >= precision / 2 {
            self.build_steps();
        }
        true
    }

    /// Builds `steps`, or leaves it empty where a step does not fit in 32 bits:
    /// precision past 16 bits, or one symbol holding all of it.
    fn build_steps(&mut self) {
        let precision = self.rans_precision as usize;
        if precision > 1 << 16 {
            return;
        }
        let table = &self.probability_table[..self.num_symbols];
        if table.iter().any(|sym| sym.prob > 0xFFFF) {
            return;
        }
        let mut steps = Vec::with_capacity(precision);
        for sym in table {
            steps.extend((0..sym.prob).map(|offset| sym.prob | offset << 16));
        }
        self.steps = steps;
    }

    /// The run against `steps`: one read gives the state its next value, where
    /// the table form reads the slot's symbol id and then the symbol's entry,
    /// the second read waiting on the first. The id is still read, but nothing
    /// waits on it.
    fn run_steps<T: Copy + Into<u32>>(
        &mut self,
        slots: &[T],
        out: &mut Vec<u32>,
        count: usize,
    ) -> bool {
        let precision = self.rans_precision as usize;
        let slots = &slots[..precision];
        let steps = &self.steps[..precision];
        let mask = self.rans_precision_mask;
        let bits = self.rans_precision_bits;
        let l_base = self.ans.l_base;
        let buf = self.ans.buf;
        let mut offset = self.ans.buf_offset.min(buf.len());
        let mut state = self.ans.state;

        let mut backed = true;
        out.extend((0..count).map(|_| {
            while state < l_base && offset > 0 {
                offset -= 1;
                state = (state << 8) | buf[offset] as u32;
            }
            backed &= state >= l_base;
            let quo = state >> bits;
            let rem = (state & mask) as usize;
            // `rem <= mask` and both tables are `mask + 1` long.
            let step = steps[rem];
            state = quo.wrapping_mul(step & 0xFFFF).wrapping_add(step >> 16);
            slots[rem].into()
        }));

        self.ans.buf_offset = offset;
        self.ans.state = state;
        backed
    }

    fn run_table<T: Copy + Into<u32>>(
        &mut self,
        slots: &[T],
        out: &mut Vec<u32>,
        count: usize,
    ) -> bool {
        let precision = self.rans_precision as usize;
        let slots = &slots[..precision];
        let table = &self.probability_table[..];
        let table_mask = self.table_mask;
        let mask = self.rans_precision_mask;
        let bits = self.rans_precision_bits;
        let l_base = self.ans.l_base;
        let buf = self.ans.buf;
        let mut offset = self.ans.buf_offset.min(buf.len());
        let mut state = self.ans.state;

        // Set once the state could not be refilled, never cleared: from that
        // slot on the run is drawing on nothing. One predicated compare per
        // symbol, off the dependency chain the loop is actually waiting on.
        let mut backed = true;
        out.extend((0..count).map(|_| {
            while state < l_base && offset > 0 {
                offset -= 1;
                state = (state << 8) | buf[offset] as u32;
            }
            backed &= state >= l_base;
            let quo = state >> bits;
            let rem = state & mask;
            // `rem <= mask` and `slots.len() == mask + 1`, so this indexes in
            // bounds; the masked table index below does the same for the id.
            let symbol_id: u32 = slots[rem as usize].into();
            let sym = table[(symbol_id & table_mask) as usize];
            state = quo
                .wrapping_mul(sym.prob)
                .wrapping_add(rem.wrapping_sub(sym.cum_prob));
            symbol_id
        }));

        self.ans.buf_offset = offset;
        self.ans.state = state;
        backed
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
        let symbol_id = self.lut.get(rem as usize)?;
        let sym = self.probability_table.get(symbol_id as usize)?;
        let state_base = quo.checked_mul(sym.prob)?;
        let state_offset = rem.checked_sub(sym.cum_prob)?;
        self.ans.state = state_base.checked_add(state_offset)?;
        Some(symbol_id)
    }
}

/// Decodes `count` symbols from each of two independent streams onto the ends
/// of `out_a` and `out_b`, in one loop, and returns whether each run was
/// backed by its coded bytes, as [`RAnsSymbolDecoder::decode_run`] does.
///
/// One stream's symbols form a single chain -- each state is computed from the
/// one before -- and a decode waits on it at every symbol. Two streams are two
/// chains, and in one loop the core works on both at once: the rANS stage of a
/// splat decode takes about half the time it does stream by stream. The
/// refill is branchless here (below), which is what lets the two chains
/// overlap: with a branch, a misprediction on either stalls both.
///
/// Streams that do not suit the loop -- no steps, a wide slot table, a single
/// symbol -- are decoded one after the other instead; the symbols are the
/// same either way.
#[cfg(feature = "point_cloud_decode")]
pub(crate) fn decode_run_pair(
    a: &mut RAnsSymbolDecoder<'_>,
    out_a: &mut Vec<u32>,
    b: &mut RAnsSymbolDecoder<'_>,
    out_b: &mut Vec<u32>,
    count: usize,
) -> (bool, bool) {
    if !(a.ready_for_pair(count) && b.ready_for_pair(count)) {
        return (a.decode_run(out_a, count), b.decode_run(out_b, count));
    }
    let (state_a, state_b) = {
        let mut lane_a = Lane::new(a);
        let mut lane_b = Lane::new(b);
        out_b.reserve(count);
        out_a.extend((0..count).map(|_| {
            out_b.push(lane_b.next());
            lane_a.next()
        }));
        (lane_a.end(), lane_b.end())
    };
    (a.ans.state, a.ans.buf_offset) = (state_a.0, state_a.1);
    (b.ans.state, b.ans.buf_offset) = (state_b.0, state_b.1);
    (state_a.2, state_b.2)
}

/// One stream of [`decode_run_pair`], its tables and coder state as locals.
#[cfg(feature = "point_cloud_decode")]
struct Lane<'t> {
    slots: &'t [u16],
    steps: &'t [u32],
    buf: &'t [u8],
    mask: u32,
    bits: u32,
    l_base: u32,
    offset: usize,
    state: u32,
    backed: bool,
}

#[cfg(feature = "point_cloud_decode")]
impl<'t> Lane<'t> {
    /// For a decoder `ready_for_pair` accepted.
    fn new(decoder: &'t RAnsSymbolDecoder<'_>) -> Self {
        let precision = decoder.rans_precision as usize;
        let Slots::Narrow(slots) = &decoder.lut else {
            unreachable!("ready_for_pair accepts narrow tables only")
        };
        Self {
            slots: &slots[..precision],
            steps: &decoder.steps[..precision],
            buf: decoder.ans.buf,
            mask: decoder.rans_precision_mask,
            bits: decoder.rans_precision_bits,
            l_base: decoder.ans.l_base,
            offset: decoder.ans.buf_offset.min(decoder.ans.buf.len()),
            state: decoder.ans.state,
            backed: true,
        }
    }

    /// The next symbol, the run loop of `run_steps` with its refill made
    /// branchless while four bytes remain.
    ///
    /// The bytes a refill takes are the fewest that bring the state to
    /// `l_base = 2^(bits + 2)` or above. A state about to be refilled lies in
    /// `[4, 2^(bits + 10))`: it was in `[l_base, 256 * l_base)` before the last
    /// symbol, which `read_init` checks for the first, and a symbol's decode
    /// takes it to at least four times its probability, which is at least one
    /// for any slot `decode_table` gave a symbol. So the count is
    /// `ceil((bits + 3 - bit_length) / 8)`, which is `(leading_zeros + bits -
    /// 22) >> 3` -- at most 3, so four bytes always cover it -- and those
    /// bytes, taken most significant first, are the low end of a little-endian
    /// read ending at `offset`. Past the last four bytes the loop below takes
    /// over, and with it the bookkeeping for a run that outlives its input.
    #[inline(always)]
    fn next(&mut self) -> u32 {
        let mut state = self.state;
        let mut offset = self.offset;
        if offset >= 4 {
            let shift = (state.leading_zeros() + self.bits).wrapping_sub(22) & 0x18;
            let word: [u8; 4] = self.buf[offset - 4..offset]
                .try_into()
                .expect("a four-byte range");
            let word = u64::from(u32::from_le_bytes(word));
            state = (state << shift) | (word >> (32 - shift)) as u32;
            offset -= (shift >> 3) as usize;
        }
        while state < self.l_base && offset > 0 {
            offset -= 1;
            state = (state << 8) | self.buf[offset] as u32;
        }
        self.backed &= state >= self.l_base;
        let quo = state >> self.bits;
        let rem = (state & self.mask) as usize;
        let step = self.steps[rem];
        self.state = quo.wrapping_mul(step & 0xFFFF).wrapping_add(step >> 16);
        self.offset = offset;
        u32::from(self.slots[rem])
    }

    fn end(self) -> (u32, usize, bool) {
        (self.state, self.offset, self.backed)
    }
}

#[cfg(test)]
mod tests {
    use super::{RAnsSymbolDecoder, Slots};
    use crate::rans_symbol_coding::RAnsSymbol;

    /// Every form `decode_run` takes reads what the per-symbol path reads, and
    /// leaves the coder where it does. The per-symbol path is the one that
    /// follows upstream's `rans_read` line for line.
    ///
    /// The alphabets reach each form: 12 and 16 bits of precision with a slot
    /// table narrow enough for steps, 20 bits -- too fine for steps, so buckets
    /// -- over a narrow table and over one too wide for `u16`. A run cut into
    /// pieces shorter than the bucket count and half a table builds neither,
    /// so it is the table form throughout.
    #[cfg(feature = "encoder")]
    #[test]
    fn every_run_form_reads_what_the_per_symbol_path_reads() {
        use crate::decoder_buffer::DecoderBuffer;
        use crate::encoder_buffer::EncoderBuffer;
        use crate::rans_symbol_coding::compute_rans_precision_from_unique_symbols_bit_length;
        use crate::symbol_encoding::encode_raw_symbols;

        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        // (alphabet, count, skew, narrow slot table, steps, buckets): 12 and 16
        // bits with steps; 18 bits with the mass on a hundred-odd symbols, a
        // geometric draw (skew 0) like a scan's corrections, where the buckets
        // are kept, and with it spread over the alphabet, half owned, where
        // they are declined; 19 and 20 bits with a long tail, past 512 KiB of
        // slots, where they are kept whatever the share -- 41% owned at 19 --
        // over a narrow table and over one too wide for `u16`.
        for (alphabet, count, skew, narrow, steps, buckets) in [
            (40u32, 20_000usize, 4, true, true, false),
            (1_500, 100_000, 4, true, true, false),
            (3_000, 300_000, 0, true, false, true),
            (3_000, 300_000, 4, true, false, false),
            (6_000, 300_000, 4, true, false, true),
            (60_000, 150_000, 4, true, false, true),
            (70_000, 150_000, 4, false, false, true),
        ] {
            // Every symbol once, then a draw that favours the small ones the way
            // prediction corrections do.
            let symbols: Vec<u32> = (0..count)
                .map(|i| {
                    if i < alphabet as usize {
                        return i as u32;
                    }
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let unit = (seed >> 11) as f64 / (1u64 << 53) as f64;
                    if skew == 0 {
                        ((-(unit.max(1e-12)).ln() * 30.0) as u32).min(alphabet - 1)
                    } else {
                        (unit.powi(skew) * f64::from(alphabet)) as u32
                    }
                })
                .collect();
            let mut target = EncoderBuffer::new();
            encode_raw_symbols(&symbols, alphabet - 1, &mut target, 7).unwrap();
            let data = target.data().to_vec();
            let decoder = || {
                let mut buffer = DecoderBuffer::new(&data);
                let bits = buffer.decode_u8().unwrap() as u32;
                let mut decoder = RAnsSymbolDecoder::new(
                    compute_rans_precision_from_unique_symbols_bit_length(bits),
                );
                assert!(decoder.create(&mut buffer) && decoder.start_decoding(&mut buffer));
                decoder
            };

            let mut reference = decoder();
            let expected: Vec<u32> = (0..count)
                .map(|_| reference.try_decode_symbol().unwrap())
                .collect();
            assert_eq!(
                expected, symbols,
                "{alphabet} symbols: the reference itself"
            );

            let mut whole = decoder();
            let mut out = Vec::new();
            assert!(whole.decode_run(&mut out, count));
            assert_eq!(matches!(whole.lut, Slots::Narrow(_)), narrow, "{alphabet}");
            assert_eq!(!whole.steps.is_empty(), steps, "{alphabet}");
            assert_eq!(!whole.buckets.is_empty(), buckets, "{alphabet}");

            let mut pieces = decoder();
            let mut cut = Vec::new();
            while cut.len() < count {
                let piece = 997.min(count - cut.len());
                assert!(pieces.decode_run(&mut cut, piece));
            }
            assert!(pieces.steps.is_empty() && pieces.buckets.is_empty());

            for (form, decoded, coder) in [("whole", &out, &whole), ("pieces", &cut, &pieces)] {
                assert_eq!(decoded, &expected, "{alphabet} symbols, {form}");
                assert_eq!(
                    (coder.ans.state, coder.ans.buf_offset),
                    (reference.ans.state, reference.ans.buf_offset),
                    "{alphabet} symbols, {form}: the coder ended elsewhere"
                );
            }
        }
    }

    /// Two streams decoded in one loop come out as each does alone: the
    /// symbols, whether the run was backed by its bytes, and where the coder
    /// stopped. Alphabets that suit the loop and ones that send it back to one
    /// stream at a time, and runs that ask for more symbols than were coded,
    /// so that the tail past the last four bytes and an exhausted input are
    /// both reached.
    #[cfg(all(feature = "encoder", feature = "point_cloud_decode"))]
    #[test]
    fn a_pair_of_runs_reads_what_each_run_reads_alone() {
        use super::decode_run_pair;
        use crate::decoder_buffer::DecoderBuffer;
        use crate::encoder_buffer::EncoderBuffer;
        use crate::rans_symbol_coding::compute_rans_precision_from_unique_symbols_bit_length;
        use crate::symbol_encoding::encode_raw_symbols;

        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut stream = |alphabet: u32, count: usize| {
            let symbols: Vec<u32> = (0..count)
                .map(|i| {
                    if i < alphabet as usize {
                        return i as u32;
                    }
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    let unit = (seed >> 11) as f64 / (1u64 << 53) as f64;
                    (unit.powi(3) * f64::from(alphabet)) as u32
                })
                .collect();
            let mut target = EncoderBuffer::new();
            encode_raw_symbols(&symbols, alphabet - 1, &mut target, 7).unwrap();
            target.data().to_vec()
        };
        fn open(data: &[u8]) -> RAnsSymbolDecoder<'_> {
            let mut buffer = DecoderBuffer::new(data);
            let bits = buffer.decode_u8().unwrap() as u32;
            let mut decoder =
                RAnsSymbolDecoder::new(compute_rans_precision_from_unique_symbols_bit_length(bits));
            assert!(decoder.create(&mut buffer) && decoder.start_decoding(&mut buffer));
            decoder
        }
        for (alphabet_a, alphabet_b, coded, asked) in [
            (40u32, 64u32, 30_000usize, 30_000usize),
            (40, 1_500, 30_000, 30_000),
            (2, 40, 30_000, 30_000),
            (40, 60_000, 70_000, 70_000),
            (40, 100, 30_000, 31_000),
        ] {
            let (data_a, data_b) = (stream(alphabet_a, coded), stream(alphabet_b, coded));
            let alone = |data: &[u8]| {
                let mut decoder = open(data);
                let mut out = Vec::new();
                let backed = decoder.decode_run(&mut out, asked);
                (out, backed, decoder.ans.state, decoder.ans.buf_offset)
            };
            let (expected_a, expected_b) = (alone(&data_a), alone(&data_b));
            let (mut a, mut b) = (open(&data_a), open(&data_b));
            let (mut out_a, mut out_b) = (vec![7], vec![9]);
            let (backed_a, backed_b) =
                decode_run_pair(&mut a, &mut out_a, &mut b, &mut out_b, asked);
            let label = format!("{alphabet_a} and {alphabet_b} symbols, {asked} of {coded}");
            assert_eq!(
                (out_a[0], out_b[0]),
                (7, 9),
                "{label}: what was there is kept"
            );
            assert_eq!(
                (&out_a[1..], backed_a, a.ans.state, a.ans.buf_offset),
                (&expected_a.0[..], expected_a.1, expected_a.2, expected_a.3),
                "{label}: the first stream"
            );
            assert_eq!(
                (&out_b[1..], backed_b, b.ans.state, b.ans.buf_offset),
                (&expected_b.0[..], expected_b.1, expected_b.2, expected_b.3),
                "{label}: the second stream"
            );
        }
    }

    #[test]
    fn try_decode_symbol_rejects_invalid_lut_symbol_id() {
        let mut decoder = RAnsSymbolDecoder::new(1);
        decoder.num_symbols = 2;
        decoder.lut = Slots::Wide(vec![99, 99]);
        decoder.probability_table = vec![RAnsSymbol::default(); 2];
        decoder.ans.state = decoder.l_rans_base;

        assert_eq!(decoder.try_decode_symbol(), None);
    }

    #[test]
    fn try_decode_symbol_rejects_inconsistent_cumulative_probability() {
        let mut decoder = RAnsSymbolDecoder::new(1);
        decoder.num_symbols = 2;
        decoder.lut = Slots::Wide(vec![0, 0]);
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
