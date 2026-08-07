//! What a decode is allowed to allocate, relative to the stream it is reading.
//!
//! A Draco header states counts — points, faces, attribute values — and every
//! buffer the decoder sizes comes from one of them. Those counts are as large
//! as the file says, so something has to stand between a nine-byte header and a
//! multi-gigabyte reservation.
//!
//! What used to stand there was a guard asserting the stream carries at least
//! one bit per point or face. That premise is false, and provably so: geometry
//! whose attribute values are all equal entropy-codes to a size independent of
//! the count, so this crate writes 100,000 points into 171 bytes and its own
//! decoder then refused to read the file back. C++ Draco produces such files
//! too, which made it an interoperability bug rather than a conservative bound.
//!
//! This replaces it with a ratio: a decode may allocate up to
//! [`MAX_ALLOCATED_BYTES_PER_INPUT_BYTE`] bytes for every byte of the stream.
//! It scales with the input instead of capping the geometry, which is what
//! `SECURITY.md` promises — Draco is for large meshes, and a fixed ceiling
//! would break legitimate ones.
//!
//! The ratio is not the whole of it, and the missing half is the reason this
//! paragraph exists. A ratio bounds bytes allocated per byte read, so a longer
//! input buys a larger allocation — and the input is the attacker's to choose.
//! For a declared count of *symbols* that is the wrong instrument entirely, so
//! [`ensure_symbols_are_backed`] bounds those at one bit each. The two are not
//! interchangeable: a 26 KB stream naming a billion faces clears the ratio and
//! fails the bit bound, which is exactly the case the `decode_drc` fuzz target
//! found after the ratio replaced the count check.

use crate::status::{DracoError, Status};

/// How many bytes a decode may allocate per byte of input.
///
/// It is a heuristic, so the numbers that place it are measured rather than
/// guessed:
///
/// - The largest legitimate ratio this crate is known to produce is about
///   7,000× — 100,000 positions, 1.2 MB of values, from a 171-byte stream — so
///   2^20 leaves better than two orders of magnitude of headroom.
/// - The malformed KD-tree header pinned in `drc_edge_cases_test.rs` asks for
///   51 GB from 19 bytes, a ratio near 2^31, so it is refused by about 2,500×.
///
/// Fallible allocation is **not** a substitute, which is the reason this exists
/// at all. Measured on Windows: `Vec::<u32>::try_reserve_exact` for 51 GB
/// *succeeds*, in 0.165 s, and the decoder then spends 14.5 s writing into what
/// it was given. Asking the allocator politely does not bound anything when the
/// allocator says yes.
///
/// It is also a stopgap for something deeper. The KD-tree loop cannot stop when
/// the data runs out, because `decode_next_bit` reports an exhausted buffer and
/// a zero bit the same way — see the `decode-error-api-cleanup` risk. Once that
/// is fixed the loop bounds itself, and this ratio can be revisited.
pub(crate) const MAX_ALLOCATED_BYTES_PER_INPUT_BYTE: usize = 1 << 20;

/// Refuses an allocation the stream is too small to be describing.
///
/// Written as a division rather than `stream_bytes * MAX`, because `usize` is
/// 32 bits on the `wasm32` target this crate ships to: there the multiplication
/// saturates for any stream over 4 KiB and the bound silently stops existing.
pub(crate) fn ensure_allocation_is_backed(requested_bytes: usize, stream_bytes: usize) -> Status {
    if requested_bytes / MAX_ALLOCATED_BYTES_PER_INPUT_BYTE > stream_bytes {
        return Err(DracoError::allocation_exceeds_input(
            requested_bytes,
            stream_bytes,
        ));
    }
    Ok(())
}

/// The same, for a count of `element_size`-byte elements.
pub(crate) fn ensure_elements_are_backed(
    count: usize,
    element_size: usize,
    stream_bytes: usize,
) -> Status {
    let requested_bytes = count.saturating_mul(element_size);
    ensure_allocation_is_backed(requested_bytes, stream_bytes)
}

/// Refuses a symbol count the remaining stream cannot be carrying, at one bit
/// per symbol.
///
/// The ratio above is the right instrument for attribute *values* and the wrong
/// one for a *symbol count*, and the difference is why this exists.
///
/// A ratio is a bound on bytes allocated per byte read, so a larger input buys
/// a proportionally larger allocation — which is exactly what an attacker
/// supplies. A 26 KB stream declaring 1,095,910,464 faces asks for 13 GB of
/// indices and passes the ratio with room to spare, because 13 GB over 2^20 is
/// 12 KB and the stream is twice that. Lowering the constant does not fix the
/// shape: whatever it is, a stream twice as long buys twice as much.
///
/// One bit per symbol is a real floor for the *count*, not for the values. It
/// is also far more permissive than upstream, which refuses `num_faces >
/// remaining_size() / 3` — three bytes per face against this one bit, a factor
/// of 24 — so a file this accepts and upstream refuses still decodes here,
/// which is the direction this crate errs in deliberately.
///
/// In theory a degenerate rANS distribution can carry a symbol in less than a
/// bit, so this is a bound on the plausible rather than the possible. What
/// makes that trade acceptable is the alternative: without it a nine-byte
/// header names a multi-gigabyte reservation, which is the thing this module
/// exists to prevent, and upstream has shipped a bound 24 times stricter for
/// years without an interoperability complaint.
pub(crate) fn ensure_symbols_are_backed(count: usize, stream_bytes: usize) -> Status {
    if count > stream_bytes.saturating_mul(8) {
        return Err(DracoError::new(
            crate::status::ErrorKind::AllocationExceedsInput,
            format!(
                "declared {count} symbols, more than the {stream_bytes} byte stream \
                 could carry at one bit each"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_backs_an_allocation_within_the_ratio() {
        assert!(ensure_allocation_is_backed(MAX_ALLOCATED_BYTES_PER_INPUT_BYTE, 1).is_ok());
        // The case the old guard refused: 100,000 positions from 171 bytes.
        assert!(ensure_elements_are_backed(100_000, 12, 171).is_ok());
    }

    #[test]
    fn a_tiny_stream_does_not_back_a_huge_allocation() {
        // The shape the pinned malformed headers have: u32::MAX values from a
        // body of a dozen bytes.
        assert!(ensure_elements_are_backed(u32::MAX as usize, 12, 12).is_err());
    }

    /// The shape a 26 KB fuzz artifact had: a mesh header naming 1,095,910,464
    /// faces, whose 3,287,731,392 indices reserve 13 GB. The ratio accepts it,
    /// because 13 GB over 2^20 is 12 KB and the stream is 26; one bit per
    /// symbol does not.
    #[test]
    fn a_symbol_count_beyond_one_bit_each_is_refused() {
        assert!(ensure_symbols_are_backed(3_287_731_392, 26_386).is_err());
        // The ratio on its own is what let it through, and still would.
        assert!(ensure_elements_are_backed(3_287_731_392, 4, 26_386).is_ok());
    }

    /// Eight symbols per byte is the boundary itself, and it is inclusive: a
    /// stream that carries exactly one bit per symbol is not malformed.
    #[test]
    fn a_symbol_count_of_exactly_one_bit_each_is_accepted() {
        assert!(ensure_symbols_are_backed(8, 1).is_ok());
        assert!(ensure_symbols_are_backed(9, 1).is_err());
    }

    #[test]
    fn the_bound_survives_a_32_bit_usize() {
        // Written as a division precisely so this does not saturate away on
        // wasm32; at 4 KiB of input the multiplicative form already would.
        assert!(ensure_allocation_is_backed(usize::MAX, 4096).is_err());
    }
}
