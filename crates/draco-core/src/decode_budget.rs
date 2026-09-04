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
//! What replaced it is a **backstop**, and the word is load-bearing: what
//! actually keeps these paths bounded is that every buffer on them grows as its
//! data arrives or is sized after the step that produced it, so a legitimate
//! decode charges almost nothing here.
//!
//! Almost, and the exception is worth naming because it is the one shape with
//! no honest bound at all: an entropy-coded run over an alphabet of *one*
//! carries no payload, so nothing in the stream says how far it goes and the
//! declared count is all there is. A constant attribute reaches it -- one
//! tracked fixture charges 192 bytes that way -- and a 170-byte stream naming
//! 6.8 billion values reached it for 27 GB.
//! `corpus::no_tracked_fixture_spends_the_budget` reads every fixture the
//! repository carries and holds the charge to a rounding error against the
//! ceiling.
//!
//! What the backstop catches is the residue: a reservation made against a
//! count with no data behind it. It is bounded two ways, because either alone
//! has a failure mode the other does not:
//!
//! - [`MAX_ALLOCATED_BYTES_PER_INPUT_BYTE`], a ratio against the stream. It
//!   refuses a nine-byte header naming gigabytes, which no absolute ceiling
//!   set high enough to be safe would catch.
//! - [`MAX_UNBACKED_BYTES`], an absolute ceiling. A ratio grants a longer input
//!   a larger allocation, and the input is the attacker's: a 32 KB stream buys
//!   21 GB and clears the ratio with room to spare. That is not a constant to
//!   be tuned — halve it and a stream twice as long buys the same.
//!
//! The counter is **cumulative** over one decode, held on [`DecoderBuffer`],
//! which is the stream. Checked per allocation the same budget is granted
//! again at every site, and the number of sites is the attacker's to pick.
//!
//! For a declared count of *symbols* neither instrument is right, so
//! [`ensure_symbols_are_backed`] bounds those at one bit each. A 26 KB stream
//! naming a billion faces is the case the `decode_drc` fuzz target found.
//!
//! [`DecoderBuffer`]: crate::decoder_buffer::DecoderBuffer

use crate::status::{DracoError, Status};

/// How many bytes a decode may reserve, against a count nothing backs, per
/// byte of input.
///
/// Legitimate files do not reach this — they charge nothing — so what places
/// it is the malformed side: the KD-tree header pinned in
/// `drc_edge_cases_test.rs` asks for 51 GB from 19 bytes, a ratio near 2^31,
/// and is refused by about 2,500×.
///
/// It is deliberately loose, because it is the wrong shape for the job and
/// tightening it does not fix that: the ratio's purpose is the small-stream
/// case, and [`MAX_UNBACKED_BYTES`] is what stops a long one.
///
/// Fallible allocation is **not** a substitute, which is the reason this exists
/// at all. Measured on Windows: `Vec::<u32>::try_reserve_exact` for 51 GB
/// *succeeds*, in 0.165 s, and the decoder then spends 14.5 s writing into what
/// it was given. Asking the allocator politely does not bound anything when the
/// allocator says yes.
///
/// It is not waiting on an API fix, whatever this comment used to say.
/// `DirectBitDecoder::decode_next_bit` already reports an exhausted buffer;
/// `RAnsBitDecoder::decode_next_bit` cannot and, by its own reasoning, never
/// will — the rABS tail legitimately draws from state alone in 26% of reads on
/// correct files, and upstream has no check there either. What bounds those
/// loops is a structural per-call-site count, audited under
/// `rans-over-read-call-site-bounds` in `hardening_status.yaml`.
pub(crate) const MAX_ALLOCATED_BYTES_PER_INPUT_BYTE: usize = 1 << 20;

/// The most a decode may reserve for buffers nothing in the stream backs,
/// whatever its size.
///
/// A ratio cannot bound this on its own and the reason is measured: for
/// geometry whose values are all equal, the stream length is independent of
/// the point count, so the legitimate bytes-per-input-byte ratio has no upper
/// bound. Six million points encode to 58 bytes here; a hundred million would
/// encode to 58 bytes as well. Whatever constant the ratio carries, a large
/// enough legitimate file walks past it -- and one did, which is the
/// interoperability bug this module was created to remove, reappearing at a
/// larger count.
///
/// An absolute ceiling has the opposite failure mode, and this one can be
/// placed honestly because of what the counter now counts. Every buffer on the
/// decode paths grows as its data arrives or is sized after the step that
/// produced it, so a legitimate decode charges **nothing at all** -- held to
/// that by `corpus::no_tracked_fixture_spends_the_budget` over every fixture
/// the repository carries, and by its sibling over the compressible extreme.
/// The ceiling therefore does not cap geometry, which `SECURITY.md` promises
/// it never will; it caps reservations made against a claim and nothing else.
///
/// One path can still reach it from a real file: raw corrections declaring
/// zero bytes each, where "every correction is zero" is read from the header
/// and no data is read at all. This crate never writes that -- its encoder
/// always emits entropy-coded corrections -- and 256 MiB covers 64 million
/// such values, well past any attribute whose corrections are uniformly zero.
/// The 19.76 GiB the `decode_drc` campaign found is refused by 83x.
pub(crate) const MAX_UNBACKED_BYTES: usize = 256 << 20;

/// Refuses an allocation the stream is too small to be describing, or one past
/// the absolute ceiling on reservations nothing backs.
///
/// The ratio is written as a division rather than `stream_bytes * MAX`,
/// because `usize` is 32 bits on the `wasm32` target this crate ships to:
/// there the multiplication saturates for any stream over 4 KiB and the bound
/// silently stops existing.
pub(crate) fn ensure_allocation_is_backed(requested_bytes: usize, stream_bytes: usize) -> Status {
    if requested_bytes > MAX_UNBACKED_BYTES
        || requested_bytes / MAX_ALLOCATED_BYTES_PER_INPUT_BYTE > stream_bytes
    {
        return Err(DracoError::allocation_exceeds_input(
            requested_bytes,
            stream_bytes,
        ));
    }
    Ok(())
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
/// Sub-bit symbols are not just theoretically possible, they are measured:
/// the seeded ribbon's attribute corrections entropy-code at 2.7 symbols per
/// bit of their own stream (PERFORMANCE-LOG.md, the dhat round). This gate
/// survives that fact only because of its denominator -- it is applied once,
/// on the sequential-connectivity path, against the *whole* stream, which
/// also carries every attribute. Do not reuse it against the narrow
/// `remaining_size` of a single symbol stream: a real file would be refused,
/// which is exactly the interoperability bug this module replaced. What makes
/// the trade acceptable where it stands is the alternative: without it a
/// nine-byte header names a multi-gigabyte reservation, which is the thing
/// this module exists to prevent, and upstream has shipped a bound 24 times
/// stricter for years without an interoperability complaint.
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

#[cfg(all(test, feature = "decoder"))]
mod corpus {
    /// No file this crate is meant to read spends any of the budget.
    ///
    /// That is the property, not a coincidence: every buffer on the decode
    /// paths either grows as the data arrives or is sized after the step that
    /// produced it, so the backstop is reached only by a reservation nothing
    /// backs. A charge appearing here means a header-driven allocation came
    /// back -- which is worth failing over even when the ratio would have
    /// covered it, because the ratio is what breaks first on a legitimate
    /// file (see the sibling test).
    #[test]
    fn no_tracked_fixture_spends_the_budget() {
        use crate::decoder_buffer::DecoderBuffer;
        use std::path::PathBuf;

        fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "drc") {
                    out.push(path);
                }
            }
        }

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata");
        let mut files = Vec::new();
        walk(&root, &mut files);
        files.sort();
        assert!(
            files.len() > 50,
            "found only {} .drc fixtures: the walk is wrong, not the corpus",
            files.len()
        );

        let mut decoded_ok = 0;
        for path in &files {
            let bytes = std::fs::read(path).expect("read fixture");
            if bytes.is_empty() {
                continue;
            }
            for as_mesh in [true, false] {
                let mut buffer = DecoderBuffer::new(&bytes);
                let ok = if as_mesh {
                    let mut mesh = crate::mesh::Mesh::new();
                    crate::mesh_decoder::MeshDecoder::new()
                        .decode(&mut buffer, &mut mesh)
                        .is_ok()
                } else {
                    let mut point_cloud = crate::point_cloud::PointCloud::new();
                    crate::point_cloud_decoder::PointCloudDecoder::new()
                        .decode(&mut buffer, &mut point_cloud)
                        .is_ok()
                };
                if !ok {
                    continue;
                }
                decoded_ok += 1;
                // Not zero, and the exception is exact: a run over an alphabet
                // of one carries no payload, so the count is genuinely unbacked
                // and gets charged. `cube_att.obj.edgebreaker.cl10.2.2.drc` has
                // a constant attribute and charges 192 bytes for it. What the
                // corpus still has to say is that such a charge stays a rounding
                // error against the ceiling -- a fixture spending megabytes here
                // would mean something other than a constant attribute had found
                // its way onto this path.
                const ROUNDING_ERROR: usize = super::MAX_UNBACKED_BYTES / 1024;
                assert!(
                    buffer.spent() < ROUNDING_ERROR,
                    "{} charged {} bytes against the budget, past the {ROUNDING_ERROR} a                      constant attribute can explain",
                    path.display(),
                    buffer.spent()
                );
            }
        }
        assert!(decoded_ok > 50, "only {decoded_ok} fixtures decoded");
    }

    /// The tracked fixtures are all under a kilobyte, so they say nothing
    /// about the extreme. This is the extreme, and it is the reason the
    /// charges above had to go: geometry whose values are all equal
    /// entropy-codes to a size independent of how many there are, so the
    /// legitimate bytes-per-input-byte ratio has no upper bound at all.
    ///
    /// Measured on this shape, six million points in a 58-byte stream: the
    /// charge was `72,000,000`, the ratio `1,241,379x`, and
    /// [`MAX_ALLOCATED_BYTES_PER_INPUT_BYTE`] refused it -- a file this crate
    /// had just written, which is the interoperability bug this module was
    /// created to remove, reappearing at a larger point count. Nothing is
    /// charged for it now, so nothing refuses it.
    #[test]
    #[cfg(feature = "encoder")]
    fn a_stream_that_decodes_far_larger_than_itself_is_not_refused() {
        use crate::decoder_buffer::DecoderBuffer;
        use crate::draco_types::DataType;
        use crate::encoder_buffer::EncoderBuffer;
        use crate::encoder_options::EncoderOptions;
        use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
        use crate::point_cloud_encoder::PointCloudEncoder;

        const NUM_POINTS: usize = 6_000_000;

        let mut point_cloud = crate::point_cloud::PointCloud::new();
        point_cloud.set_num_points(NUM_POINTS);
        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            NUM_POINTS,
        );
        point_cloud.add_attribute(position);

        let mut options = EncoderOptions::new();
        options.set_attribute_int(0, "quantization_bits", 8);
        let mut encoded = EncoderBuffer::new();
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(point_cloud);
        encoder.encode(&options, &mut encoded).expect("encode");
        let stream = encoded.data().to_vec();
        assert!(
            stream.len() < 1024,
            "the point of this test is a stream far smaller than what it decodes to, got {}",
            stream.len()
        );

        let mut buffer = DecoderBuffer::new(&stream);
        let mut decoded = crate::point_cloud::PointCloud::new();
        crate::point_cloud_decoder::PointCloudDecoder::new()
            .decode(&mut buffer, &mut decoded)
            .expect("a stream this crate wrote must decode back");
        assert_eq!(decoded.num_points(), NUM_POINTS);
        assert_eq!(buffer.spent(), 0, "a legitimate file spent the backstop");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_backs_an_allocation_within_the_ratio() {
        assert!(ensure_allocation_is_backed(MAX_ALLOCATED_BYTES_PER_INPUT_BYTE, 1).is_ok());
        // The case the old guard refused: 100,000 positions from 171 bytes.
        assert!(ensure_allocation_is_backed(100_000 * 12, 171).is_ok());
    }

    /// The ceiling is absolute: a stream long enough to buy 19.76 GiB through
    /// the ratio -- which is exactly what the `decode_drc` campaign supplied --
    /// does not buy it.
    #[test]
    fn a_long_stream_does_not_buy_an_unbounded_reservation() {
        assert!(ensure_allocation_is_backed(MAX_UNBACKED_BYTES, usize::MAX).is_ok());
        assert!(ensure_allocation_is_backed(MAX_UNBACKED_BYTES + 1, usize::MAX).is_err());
        assert!(ensure_allocation_is_backed(21_219_601_020, 27_911).is_err());
    }

    #[test]
    fn a_tiny_stream_does_not_back_a_huge_allocation() {
        // The shape the pinned malformed headers have: u32::MAX values from a
        // body of a dozen bytes.
        assert!(ensure_allocation_is_backed(u32::MAX as usize * 12, 12).is_err());
    }

    /// The shape a 26 KB fuzz artifact had: a mesh header naming 1,095,910,464
    /// faces, whose 3,287,731,392 indices reserve 13 GB. One bit per symbol
    /// refuses it on the count; the byte budget refuses the reservation too,
    /// now that its ceiling is absolute -- no stream length buys 13 GB.
    #[test]
    fn a_symbol_count_beyond_one_bit_each_is_refused() {
        assert!(ensure_symbols_are_backed(3_287_731_392, 26_386).is_err());
        // The ratio on its own is what let it through, and still would.
        assert!(ensure_allocation_is_backed(3_287_731_392 * 4, usize::MAX).is_err());
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
