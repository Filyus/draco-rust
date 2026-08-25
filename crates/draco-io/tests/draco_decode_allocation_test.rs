//! What a Draco decode *reserves*, for headers that lie about their size.
//!
//! These belong beside the decoder in `draco-core`, and cannot live there:
//! measuring an allocation needs a `#[global_allocator]`, and that crate sets
//! `unsafe_code = "forbid"`. `draco-io` depends on the decoder, allows the
//! `unsafe` a counting allocator needs, and already pins the readers this way
//! in `reader_hardening_test.rs`.
//!
//! Asserting `is_err()` is not enough for this class and that is the whole
//! reason the file exists. A decoder that reserves gigabytes from a header
//! still returns an error afterwards -- it runs out of *data* a moment later --
//! so an error-only test passes just as happily with the bug in place. What
//! separates the two is the number of bytes asked for on the way there.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;

/// Counts bytes requested, so a test can assert on what a decode reserves
/// rather than on how long it takes to fail.
struct CountingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to `System`, which is a correct `GlobalAlloc`,
// passing the layout and pointer through unchanged. The only thing added is an
// atomic add on a counter, which allocates nothing and touches no allocator
// state, so the obligations this impl carries are exactly `System`'s and are
// discharged by delegating to it.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: `layout` is the caller's and reaches `System` unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` are the caller's, unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(new_size, Ordering::Relaxed);
        // SAFETY: `ptr`, `layout` and `new_size` are the caller's, unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

/// `DRACO` magic, version, encoder type and method.
fn draco_header(major: u8, minor: u8, encoder_type: u8, method: u8) -> Vec<u8> {
    let mut header = b"DRACO".to_vec();
    header.push(major);
    header.push(minor);
    header.push(encoder_type);
    header.push(method);
    header.extend_from_slice(&0u16.to_le_bytes()); // no flags
    header
}

/// A sequential point cloud whose header claims a billion-and-a-half points
/// and carries one quantized position attribute.
///
/// The point count is a fixed-width `u32` on this path for every version, not
/// a varint -- see `PointCloudSequentialDecoder`.
fn a_point_cloud_claiming(num_points: u32, padding: usize) -> Vec<u8> {
    let mut stream = draco_header(2, 2, 0, 0);
    stream.extend_from_slice(&num_points.to_le_bytes());
    stream.push(1); // one attributes decoder
    stream.push(1); // one attribute in it
                    // Attribute: position, float32, three components, unnormalized.
    stream.extend_from_slice(&[0, 9, 3, 0]);
    stream.push(0); // unique id
    stream.push(2); // decoder type 2: quantized integer values
    stream.push(1); // entropy-coded corrections, which is what the artifact had
    stream.resize(stream.len() + padding, 0);
    stream
}

/// A stream this small cannot be carrying a billion points, and the decoder
/// must not reserve one value per claimed point to find that out.
///
/// From the `decode_drc` fuzz campaign: a 27,911-byte artifact declaring
/// 1,768,300,085 points asked for a single 21,219,601,020-byte buffer --
/// 19.76 GiB, and the ratio budget waves it through, because the ratio scales
/// with the input and the input is the attacker's. Deferring the buffer took
/// the largest single allocation on that file to 6.8 MiB.
#[test]
fn a_quantized_point_cloud_does_not_reserve_one_value_per_claimed_point() {
    // Padding so the stream clears the allocation-ratio budget on its own; the
    // point of the test is what happens *after* that gate, not at it.
    let stream = a_point_cloud_claiming(1_768_300_085, 32 * 1024);

    let before = ALLOCATED.load(Ordering::Relaxed);
    let mut decoded = PointCloud::new();
    let result = PointCloudDecoder::new().decode(&mut DecoderBuffer::new(&stream), &mut decoded);
    let requested = ALLOCATED.load(Ordering::Relaxed) - before;

    assert!(
        result.is_err(),
        "a stream with no values in it must not decode"
    );
    // The buffer the bug reserved was 21 GB. Anything within two orders of
    // magnitude of the input is the decoder working from the data it has.
    assert!(
        requested < 64 * 1024 * 1024,
        "decode reserved {requested} bytes for a {} byte stream",
        stream.len()
    );
}

/// The same shape through the octahedral-normal arm, which sized its portable
/// buffer from the claim in the same way.
#[test]
fn an_octahedral_normal_point_cloud_does_not_reserve_one_value_per_claimed_point() {
    let mut stream = draco_header(2, 2, 0, 0);
    stream.extend_from_slice(&1_768_300_085u32.to_le_bytes());
    stream.push(1); // one attributes decoder
    stream.push(1); // one attribute in it
    stream.extend_from_slice(&[1, 9, 3, 0]); // normal, float32, three components
    stream.push(0); // unique id
    stream.push(3); // decoder type 3: octahedral normals
    stream.push(1); // entropy-coded corrections
    stream.resize(stream.len() + 32 * 1024, 0);

    let before = ALLOCATED.load(Ordering::Relaxed);
    let mut decoded = PointCloud::new();
    let result = PointCloudDecoder::new().decode(&mut DecoderBuffer::new(&stream), &mut decoded);
    let requested = ALLOCATED.load(Ordering::Relaxed) - before;

    assert!(
        result.is_err(),
        "a stream with no values in it must not decode"
    );
    assert!(
        requested < 64 * 1024 * 1024,
        "decode reserved {requested} bytes for a {} byte stream",
        stream.len()
    );
}

/// Generic attribute values are copied verbatim out of the stream, so the
/// stream is an exact bound on how many there can be.
///
/// The budget that stood in for that bound allowed a million bytes per input
/// byte: a 32 KB stream claiming 200 million points cleared it and bought
/// 2.4 GB before a byte of the values was read. Nothing here needs a ratio --
/// the slice comes first and the buffer is sized from what it returned.
///
/// The generic decoder is the mesh side's raw-value path; the point cloud
/// reaches its own `decode_raw_attribute_values`, which was already exact.
#[test]
fn a_generic_attribute_does_not_reserve_more_than_the_stream_can_carry() {
    fn append_varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    let mut stream = draco_header(2, 2, 1, 0); // mesh, sequential
    append_varint(&mut stream, 0); // zero faces, so connectivity is skipped
    append_varint(&mut stream, 200_000_000); // 2.4 GB of claimed values
    append_varint(&mut stream, 1); // one attribute decoder
    append_varint(&mut stream, 1); // one attribute in it
    stream.extend_from_slice(&[4, 9, 3, 0]); // generic, float32, three components
    append_varint(&mut stream, 0); // unique id
    stream.push(0); // generic decoder
    stream.resize(stream.len() + 32 * 1024, 0);

    let before = ALLOCATED.load(Ordering::Relaxed);
    let mut decoded = draco_core::mesh::Mesh::new();
    let result = draco_core::mesh_decoder::MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&stream), &mut decoded);
    let requested = ALLOCATED.load(Ordering::Relaxed) - before;

    assert!(
        result.is_err(),
        "a stream carrying none of the values it claims must not decode"
    );
    assert!(
        requested < 1024 * 1024,
        "decode reserved {requested} bytes for a {} byte stream",
        stream.len()
    );
}
