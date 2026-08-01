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

use crate::status::{DracoError, Status};

/// How many bytes a decode may allocate per byte of input.
///
/// The bound is a heuristic, and the two numbers that place it are worth
/// recording. The largest legitimate ratio this crate is known to produce is
/// about 8,000× — the 100,000-point cloud in 171 bytes, at 12 bytes per
/// position — so 2^20 leaves better than two orders of magnitude of headroom.
/// The malformed headers pinned in `drc_edge_cases_test.rs` declare `u32::MAX`
/// counts in bodies of 11 to 19 bytes, which is a ratio above 2^31, so they
/// stay refused by a factor of about 2^11.
pub(crate) const MAX_ALLOCATED_BYTES_PER_INPUT_BYTE: usize = 1 << 20;

/// Refuses an allocation the stream is too small to be describing.
///
/// Written as a division rather than `stream_bytes * MAX`, because `usize` is
/// 32 bits on the `wasm32` target this crate ships to: there the multiplication
/// saturates for any stream over 4 KiB and the bound silently stops existing.
pub(crate) fn ensure_allocation_is_backed(requested_bytes: usize, stream_bytes: usize) -> Status {
    if requested_bytes / MAX_ALLOCATED_BYTES_PER_INPUT_BYTE > stream_bytes {
        return Err(DracoError::AllocationExceedsInput {
            requested_bytes,
            stream_bytes,
        });
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

    #[test]
    fn the_bound_survives_a_32_bit_usize() {
        // Written as a division precisely so this does not saturate away on
        // wasm32; at 4 KiB of input the multiplicative form already would.
        assert!(ensure_allocation_is_backed(usize::MAX, 4096).is_err());
    }
}
