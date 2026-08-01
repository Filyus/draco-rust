//! The OBJ/PLY/STL readers, given files nobody would author.
//!
//! These three parsers are not covered by any libFuzzer target - only the FBX
//! reader is - so the cases they used to mishandle are pinned here instead.
//! Every one is driven by file content: a header count, a repeated property,
//! a body that ends early.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use draco_io::ply_reader::PlyReader;

/// Counts bytes requested, so a test can assert on what a reader *reserves*
/// rather than on how long it takes to fail.
struct CountingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn ply(body: &str) -> Vec<u8> {
    body.as_bytes().to_vec()
}

#[test]
fn a_header_naming_more_colour_properties_than_a_colour_has_is_read() {
    // `color` is four channels wide and the property list is file-controlled,
    // so a header naming five - the same one twice, say - indexed past it.
    let file = ply(concat!(
        "ply\n",
        "format ascii 1.0\n",
        "element vertex 1\n",
        "property float x\nproperty float y\nproperty float z\n",
        "property uchar red\nproperty uchar green\nproperty uchar blue\n",
        "property uchar alpha\nproperty uchar red\n",
        "end_header\n",
        "0 0 0 1 2 3 4 5\n",
    ));
    // Either outcome is acceptable; indexing past the array is not.
    let _ = PlyReader::read_from_bytes(&file);
}

#[test]
fn a_declared_element_count_does_not_outrun_the_body() {
    // The counts come from the header and are unrelated to how many lines
    // follow: this 130-byte file declaring four billion elements used to spin
    // for seven seconds before reading a single vertex.
    let file = ply(concat!(
        "ply\n",
        "format ascii 1.0\n",
        "element face 4000000000\n",
        "property list uchar int vertex_indices\n",
        "element vertex 1\n",
        "property float x\nproperty float y\nproperty float z\n",
        "end_header\n",
        "3 0 1 2\n",
    ));

    let start = std::time::Instant::now();
    let _ = PlyReader::read_from_bytes(&file);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 2,
        "reading a 130-byte file took {elapsed:?}; the declared count is driving the work"
    );
}

#[test]
fn a_declared_element_count_does_not_size_the_body_that_is_missing() {
    // Measured rather than timed: reserving from a declared count is invisible
    // to a clock when the allocator hands back address space, and only shows up
    // as a hard abort under memory pressure - which is how this surfaced, by
    // taking the whole test process down when three tests ran at once.
    let file = concat!(
        "ply
",
        "format ascii 1.0
",
        "element vertex 1
",
        "property float x
property float y
property float z
",
        "element face 4000000000
",
        "property list uchar int vertex_indices
",
        "end_header
",
        "0 0 0
",
    )
    .as_bytes();

    let before = ALLOCATED.load(Ordering::Relaxed);
    let _ = PlyReader::read_from_bytes(file);
    let requested = ALLOCATED.load(Ordering::Relaxed) - before;

    assert!(
        requested < 1 << 20,
        "reading a {}-byte file declaring four billion faces requested {requested} bytes",
        file.len()
    );
}
