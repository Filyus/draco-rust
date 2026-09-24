//! The bit reader and canonical Huffman decoder Basis Universal streams use.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `transcoder/basisu_transcoder_internal.h`, classes `bitwise_decoder` and
//! `huffman_decoding_table`, and the constants in `transcoder/basisu.h`.
//!
//! The bit order is the part that has to be exact. Bits leave the stream least
//! significant first, and a Huffman code is stored bit-reversed, so reading one
//! bit at a time and shifting it in from the right rebuilds the code in its
//! original order. That canonical walk is what defines a symbol here, and it
//! is what [`BitReader::decode`] falls back to.
//!
//! The common case goes through a table instead, the reference's
//! `cHuffmanFastLookupBits` arrangement and DEFLATE's before it: the next
//! `FAST_BITS` bits of the stream, read as they arrive, index the symbol and
//! its length directly, because a code reversed into stream order is exactly
//! those low bits. Only a code longer than the table, or a pattern no code
//! owns, takes the walk -- and the walk has consumed nothing when it starts, so
//! both paths read the same symbol from the same bits.

/// The stream carries at most 2^14 symbols per table.
const MAX_SYMS_LOG2: u32 = 14;
const MAX_SYMS: u32 = 1 << MAX_SYMS_LOG2;
/// Longest code a symbol may be given.
const MAX_CODE_SIZE: usize = 16;

/// Alphabet of the table that codes the symbol code lengths themselves.
const TOTAL_CODELENGTH_CODES: usize = 21;
const SMALL_ZERO_RUN_CODE: u32 = 17;
const BIG_ZERO_RUN_CODE: u32 = 18;
const SMALL_REPEAT_CODE: u32 = 19;
const SMALL_ZERO_RUN_SIZE_MIN: u32 = 3;
const SMALL_ZERO_RUN_EXTRA_BITS: u32 = 3;
const BIG_ZERO_RUN_SIZE_MIN: u32 = 11;
const BIG_ZERO_RUN_EXTRA_BITS: u32 = 7;
const SMALL_REPEAT_SIZE_MIN: u32 = 3;
const SMALL_REPEAT_EXTRA_BITS: u32 = 2;
const BIG_REPEAT_SIZE_MIN: u32 = 7;
const BIG_REPEAT_EXTRA_BITS: u32 = 7;

/// The order the code length codes are written in.
const SORTED_CODELENGTH_CODES: [usize; TOTAL_CODELENGTH_CODES] = [
    17, 18, 19, 20, 0, 8, 7, 9, 6, 0xA, 5, 0xB, 4, 0xC, 3, 0xD, 2, 0xE, 1, 0xF, 0x10,
];

/// A malformed entropy-coded stream.
#[derive(Debug, thiserror::Error)]
pub enum HuffmanError {
    /// The code lengths do not describe a prefix code.
    #[error("Huffman code lengths do not form a prefix code")]
    NotPrefixFree,
    /// A table header is out of range.
    #[error("invalid Huffman table header")]
    BadHeader,
    /// A code was read that no symbol owns.
    #[error("Huffman code does not decode to a symbol")]
    BadCode,
    /// A symbol was read from a table that holds none.
    #[error("read a symbol from an empty Huffman table")]
    EmptyTable,
}

/// How many stream bits the lookup table resolves at once: the reference's
/// `cHuffmanFastLookupBits`.
const FAST_BITS: u32 = 10;
const FAST_SIZE: usize = 1 << FAST_BITS;

/// A decoded Huffman table: which symbol each code stands for.
#[derive(Debug)]
pub struct HuffmanTable {
    /// Codes of each length, in the order the symbols were assigned them.
    symbols_by_length: Vec<Vec<u16>>,
    /// First code value of each length.
    first_code: [u32; MAX_CODE_SIZE + 2],
    used_symbols: u32,
    /// Indexed by the next [`FAST_BITS`] stream bits: `symbol << 16 | length`
    /// for a code that fits, zero where the canonical walk has to decide -- a
    /// longer code, or a pattern no code owns, which only a one-symbol table
    /// leaves. A length is never zero, so zero is free to mean that.
    fast: Box<[u32; FAST_SIZE]>,
}

impl Default for HuffmanTable {
    fn default() -> Self {
        HuffmanTable {
            symbols_by_length: Vec::new(),
            first_code: [0; MAX_CODE_SIZE + 2],
            used_symbols: 0,
            fast: Box::new([0; FAST_SIZE]),
        }
    }
}

impl HuffmanTable {
    /// Build the canonical assignment the reference builds, from code lengths.
    ///
    /// Symbols are handed codes in index order, shortest length first, which is
    /// what makes the assignment reproducible from the lengths alone.
    pub fn new(code_sizes: &[u8]) -> Result<Self, HuffmanError> {
        let mut table = HuffmanTable {
            symbols_by_length: vec![Vec::new(); MAX_CODE_SIZE + 1],
            ..HuffmanTable::default()
        };
        if code_sizes.is_empty() {
            return Ok(table);
        }

        let mut counts = [0u32; MAX_CODE_SIZE + 1];
        for size in code_sizes {
            if *size as usize > MAX_CODE_SIZE {
                return Err(HuffmanError::NotPrefixFree);
            }
            counts[*size as usize] += 1;
        }
        table.used_symbols = code_sizes.iter().filter(|size| **size != 0).count() as u32;

        // Each step doubles the code space and spends what this length uses, so
        // after the last length a complete code has consumed exactly all of it.
        // The reference carries the same sum out to a 31-bit internal limit,
        // where the extra steps only double; measured at 16 the total is
        // therefore 2^17. One symbol on its own is the allowed exception - a
        // table with a single entry cannot be complete.
        let mut total: u64 = 0;
        for (length, count) in counts.iter().enumerate().skip(1) {
            total = (total + *count as u64) << 1;
            table.first_code[length + 1] = total as u32;
        }
        if total != (1u64 << (MAX_CODE_SIZE + 1)) && table.used_symbols != 1 {
            return Err(HuffmanError::NotPrefixFree);
        }

        // Codes of one length run consecutively in symbol order, so the symbol
        // for a code is its distance from that length's first code.
        for (symbol, size) in code_sizes.iter().enumerate() {
            let length = *size as usize;
            if length == 0 {
                continue;
            }
            table.symbols_by_length[length].push(symbol as u16);
        }

        // The same assignment again, spread over the lookup table. A code of
        // `length` bits arrives low bit first as its own reversal, so every
        // FAST_BITS-bit pattern whose low `length` bits are that reversal
        // decodes to it: one entry every `2^length` slots.
        let mut next_code = table.first_code;
        for (symbol, size) in code_sizes.iter().enumerate() {
            let length = *size as u32;
            if length == 0 {
                continue;
            }
            let code = next_code[length as usize];
            next_code[length as usize] += 1;
            if length > FAST_BITS {
                continue;
            }
            let entry = ((symbol as u32) << 16) | length;
            let mut slot = (code.reverse_bits() >> (32 - length)) as usize;
            while slot < FAST_SIZE {
                table.fast[slot] = entry;
                slot += 1 << length;
            }
        }
        Ok(table)
    }

    /// Whether the table holds any symbol at all.
    pub fn is_valid(&self) -> bool {
        self.used_symbols != 0
    }
}

/// Bits the staging buffer holds.
const STAGE_BITS: u32 = 64;

/// Reads bits least significant first, the way Basis writes them.
///
/// A 64-bit stage refilled by one unaligned eight-byte load, so that a refill
/// serves several symbols and costs no loop away from the end of the data.
pub struct BitReader<'a> {
    data: &'a [u8],
    /// The next byte to pull into `bits`.
    position: usize,
    /// Bits pulled and not yet consumed, in the low `count` positions.
    bits: u64,
    count: u32,
}

impl<'a> BitReader<'a> {
    /// Start reading at the front of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            position: 0,
            bits: 0,
            count: 0,
        }
    }

    /// Top the stage up to at least 57 bits.
    ///
    /// Away from the end this is one load: whatever is staged sits in the low
    /// `count` bits, the load shifted up by `count` lands against it, and the
    /// whole bytes that fitted are `(63 - count) / 8`, so the byte that only
    /// partly fitted is read again next time. What that leaves above `count`
    /// is the stream's own continuation from `position`, the same bits the next
    /// refill ORs into the same places, so the overlap is idempotent.
    ///
    /// Bits past the end read as zero, exactly as the reference's do. This is
    /// not laxness: the reference relies on it, because the last symbol of a
    /// stream can need more bits than the final byte holds.
    #[inline(always)]
    fn refill(&mut self) {
        if self.count > STAGE_BITS - 8 {
            return;
        }
        if let Some(chunk) = self.data.get(self.position..self.position + 8) {
            let word = u64::from_le_bytes([
                chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
            ]);
            self.bits |= word << self.count;
            self.position += ((STAGE_BITS - 1 - self.count) / 8) as usize;
            self.count = (self.count & 7) | (STAGE_BITS - 8);
            return;
        }
        // Fewer than eight bytes left: a byte at a time, and zeros past the end.
        while self.count <= STAGE_BITS - 8 {
            if let Some(byte) = self.data.get(self.position) {
                self.bits |= u64::from(*byte) << self.count;
                self.position += 1;
            }
            self.count += 8;
        }
    }

    /// Drop `count` bits already staged.
    #[inline(always)]
    fn consume(&mut self, count: u32) {
        self.bits >>= count;
        self.count -= count;
    }

    /// Read and consume `count` bits, up to 32.
    pub fn get_bits(&mut self, count: u32) -> u32 {
        if count == 0 {
            return 0;
        }
        if self.count < count {
            self.refill();
        }
        let value = (self.bits & ((1u64 << count) - 1)) as u32;
        self.consume(count);
        value
    }

    /// The chunked variable-length integer Basis uses for run lengths.
    pub fn decode_vlc(&mut self, chunk_bits: u32) -> u32 {
        let chunk_size = 1u32 << chunk_bits;
        let chunk_mask = chunk_size - 1;
        let mut value = 0u32;
        let mut offset = 0u32;
        loop {
            let chunk = self.get_bits(chunk_bits + 1);
            value |= (chunk & chunk_mask) << offset;
            offset += chunk_bits;
            if (chunk & chunk_size) == 0 || offset >= 32 {
                break;
            }
        }
        value
    }

    /// Read one symbol.
    pub fn decode(&mut self, table: &HuffmanTable) -> Result<u32, HuffmanError> {
        if !table.is_valid() {
            return Err(HuffmanError::EmptyTable);
        }
        if self.count < FAST_BITS {
            self.refill();
        }
        let entry = table.fast[(self.bits as usize) & (FAST_SIZE - 1)];
        if entry != 0 {
            self.consume(entry & 0xFFFF);
            return Ok(entry >> 16);
        }
        self.decode_canonical(table)
    }

    /// Read one symbol by walking the canonical code a bit at a time.
    ///
    /// What the lookup table in [`Self::decode`] stands in for, and what it
    /// hands back to for a code longer than the table or a pattern no code
    /// owns. Nothing has been consumed when it starts.
    fn decode_canonical(&mut self, table: &HuffmanTable) -> Result<u32, HuffmanError> {
        // A code is stored reversed, so taking bits in stream order and
        // shifting them in from the right rebuilds it as it was assigned.
        let mut code = 0u32;
        for length in 1..=MAX_CODE_SIZE {
            code = (code << 1) | self.get_bits(1);
            let symbols = &table.symbols_by_length[length];
            if symbols.is_empty() {
                continue;
            }
            let first = table.first_code[length];
            if code >= first && (code - first) < symbols.len() as u32 {
                return Ok(symbols[(code - first) as usize] as u32);
            }
        }
        Err(HuffmanError::BadCode)
    }

    /// Read a table written by the reference's `read_huffman_table`.
    ///
    /// The lengths are themselves Huffman coded, with run codes for repeated
    /// and for absent symbols — most tables here are sparse.
    pub fn read_huffman_table(&mut self) -> Result<HuffmanTable, HuffmanError> {
        let total_used_syms = self.get_bits(MAX_SYMS_LOG2);
        if total_used_syms == 0 {
            return Ok(HuffmanTable::default());
        }
        if total_used_syms > MAX_SYMS {
            return Err(HuffmanError::BadHeader);
        }

        let mut codelength_sizes = [0u8; TOTAL_CODELENGTH_CODES];
        let num_codelength_codes = self.get_bits(5) as usize;
        if !(1..=TOTAL_CODELENGTH_CODES).contains(&num_codelength_codes) {
            return Err(HuffmanError::BadHeader);
        }
        for slot in SORTED_CODELENGTH_CODES.iter().take(num_codelength_codes) {
            codelength_sizes[*slot] = self.get_bits(3) as u8;
        }
        let codelength_table = HuffmanTable::new(&codelength_sizes)?;
        if !codelength_table.is_valid() {
            return Err(HuffmanError::BadHeader);
        }

        let total = total_used_syms as usize;
        let mut code_sizes = vec![0u8; total];
        let mut cursor = 0usize;
        while cursor < total {
            let code = self.decode(&codelength_table)?;
            if code <= 16 {
                code_sizes[cursor] = code as u8;
                cursor += 1;
            } else if code == SMALL_ZERO_RUN_CODE {
                cursor +=
                    (self.get_bits(SMALL_ZERO_RUN_EXTRA_BITS) + SMALL_ZERO_RUN_SIZE_MIN) as usize;
            } else if code == BIG_ZERO_RUN_CODE {
                cursor += (self.get_bits(BIG_ZERO_RUN_EXTRA_BITS) + BIG_ZERO_RUN_SIZE_MIN) as usize;
            } else {
                if cursor == 0 {
                    return Err(HuffmanError::BadHeader);
                }
                let mut run = if code == SMALL_REPEAT_CODE {
                    self.get_bits(SMALL_REPEAT_EXTRA_BITS) + SMALL_REPEAT_SIZE_MIN
                } else {
                    self.get_bits(BIG_REPEAT_EXTRA_BITS) + BIG_REPEAT_SIZE_MIN
                };
                let previous = code_sizes[cursor - 1];
                if previous == 0 {
                    return Err(HuffmanError::BadHeader);
                }
                while run > 0 {
                    if cursor >= total {
                        return Err(HuffmanError::BadHeader);
                    }
                    code_sizes[cursor] = previous;
                    cursor += 1;
                    run -= 1;
                }
            }
        }
        if cursor != total {
            return Err(HuffmanError::BadHeader);
        }
        HuffmanTable::new(&code_sizes)
    }
}
