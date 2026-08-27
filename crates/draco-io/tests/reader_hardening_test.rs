//! The OBJ/PLY/STL readers, given files nobody would author.
//!
//! The `mesh_text_readers` campaign covers all three; what it finds is pinned
//! here, where it runs on stable CI without the fuzzing toolchain. Every case
//! is driven by file content: a header count, a repeated property, a body that
//! ends early.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use draco_io::obj_reader::ObjReader;
use draco_io::ply_reader::PlyReader;

/// Counts bytes requested, so a test can assert on what a reader *reserves*
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
        // Its validity is the caller's obligation under `GlobalAlloc::alloc`,
        // and nothing here weakens it.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was produced by this allocator, which hands back
        // `System`'s pointers unchanged, and `layout` is the one it was
        // allocated with. Both are the caller's obligations under
        // `GlobalAlloc::dealloc` and both are forwarded intact.
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

/// The x/y/z of every position an OBJ reader returned.
fn obj_positions(source: &str) -> Result<Vec<[f32; 3]>, String> {
    use draco_core::geometry_attribute::GeometryAttributeType;

    let mesh = ObjReader::read_from_bytes(source.as_bytes()).map_err(|error| error.to_string())?;
    let attribute = mesh
        .named_attribute(GeometryAttributeType::Position)
        .ok_or("no position attribute")?;
    let data = attribute.buffer().data();
    Ok((0..attribute.size())
        .map(|i| {
            let offset = i * 12;
            [
                f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()),
                f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()),
                f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()),
            ]
        })
        .collect())
}

#[test]
fn an_unparsable_obj_vertex_line_is_refused_rather_than_dropped() {
    // OBJ indices are 1-based and count the file's own `v` lines, so dropping
    // one shifts every later index: `f 1 2 4` silently named vertex 5. The
    // reader returned Ok with geometry the file does not describe, where
    // upstream refuses the file.
    let well_formed = "v 0 0 0
v 1 0 0
v 5 5 5
v 0 1 0
v 9 9 9
f 1 2 4
";
    assert_eq!(
        obj_positions(well_formed).expect("a well-formed file must read"),
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    );

    for malformed in [
        // Two components where three are required.
        "v 0 0 0
v 1 0 0
v 1 2
v 0 1 0
v 9 9 9
f 1 2 4
",
        // Locale comma decimals - a common real-world malformation.
        "v 0 0 0
v 1 0 0
v 1,5 2,5 3,5
v 0 1 0
v 9 9 9
f 1 2 4
",
    ] {
        let error = obj_positions(malformed).expect_err("a bad vertex line must be refused");
        assert!(error.contains("three numbers"), "unexpected error: {error}");
    }
}

#[test]
fn obj_keywords_are_separated_by_any_whitespace() {
    // The dispatch matched on `"v "`, so a tab-delimited file - which is valid
    // OBJ - was invisible to it and decoded to an empty mesh with Ok.
    let tabbed = "v	0 0 0
v	1 0 0
v	0 1 0
f	1 2 3
";
    assert_eq!(
        obj_positions(tabbed).expect("a tab-delimited file must read"),
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
    );
}

#[test]
fn a_face_list_size_that_overflows_the_cursor_is_refused() {
    // libFuzzer reproducer (fuzz target `mesh_text_readers`): the per-face list
    // size is whatever the line says, and the check that it fits the line added
    // it to the cursor. At `usize::MAX` that sum wrapped below the cursor, so
    // the check passed and the slice that followed started after it ended.
    let file = ply(&format!(
        concat!(
            "ply
",
            "format ascii 1.0
",
            "element vertex 3
",
            "property float x
property float y
property float z
",
            "element face 1
",
            "property list uchar int vertex_indices
",
            "end_header
",
            "0 0 0
1 0 0
0 1 0
",
            "{} 0 1 2
",
        ),
        usize::MAX
    ));
    // Either outcome is acceptable; slicing past the line is not.
    let _ = PlyReader::read_from_bytes(&file);
}

#[test]
fn a_binary_face_list_reserves_from_the_body_not_from_its_count() {
    // libFuzzer reproducer (fuzz target `mesh_text_readers`): the per-face list
    // length in a binary PLY is a number in the payload, and the vector for the
    // indices was reserved from it before a single index was read. This
    // 460-byte file asked for 6,538,768,792 bytes in one allocation and then
    // failed on the missing data, which is where it should have failed first.
    let file: [u8; 460] = [
        112, 108, 121, 10, 102, 111, 114, 109, 97, 15, 0, 0, 0, 0, 0, 0, 105, 0, 32, 49, 46, 48, 9,
        10, 99, 111, 109, 109, 101, 110, 116, 9, 9, 109, 97, 110, 117, 97, 108, 32, 103, 101, 110,
        101, 114, 97, 116, 101, 100, 10, 32, 101, 108, 101, 109, 101, 110, 116, 32, 118, 101, 114,
        116, 101, 120, 9, 51, 10, 112, 114, 111, 112, 32, 32, 32, 108, 121, 10, 102, 111, 114, 109,
        97, 116, 32, 98, 105, 110, 97, 114, 121, 95, 108, 105, 116, 116, 108, 101, 95, 101, 110,
        100, 105, 97, 110, 32, 32, 102, 108, 111, 97, 116, 32, 121, 10, 112, 114, 111, 112, 101,
        114, 116, 121, 32, 102, 108, 111, 97, 116, 32, 122, 10, 112, 114, 111, 112, 101, 114, 116,
        121, 32, 102, 108, 111, 97, 116, 32, 110, 116, 54, 10, 112, 114, 111, 112, 101, 114, 116,
        121, 32, 102, 108, 111, 97, 116, 51, 50, 32, 120, 10, 112, 114, 111, 112, 101, 114, 116,
        121, 32, 102, 108, 111, 97, 116, 51, 50, 32, 121, 10, 112, 114, 111, 97, 114, 32, 114, 101,
        100, 10, 112, 114, 111, 101, 114, 116, 121, 32, 102, 108, 111, 97, 116, 51, 49, 32, 122,
        10, 101, 108, 101, 109, 101, 110, 116, 32, 102, 97, 99, 101, 32, 54, 10, 112, 114, 111,
        112, 101, 114, 116, 121, 32, 108, 105, 115, 116, 32, 105, 110, 116, 51, 50, 32, 105, 110,
        116, 51, 50, 32, 118, 101, 101, 32, 54, 100, 101, 120, 10, 101, 110, 100, 95, 104, 101, 97,
        100, 101, 114, 10, 48, 48, 49, 32, 48, 32, 48, 10, 49, 32, 48, 101, 114, 116, 121, 32, 117,
        105, 110, 116, 56, 64, 103, 114, 101, 101, 111, 112, 101, 114, 116, 121, 32, 117, 105, 219,
        116, 49, 55, 110, 32, 49, 10, 49, 32, 49, 32, 48, 102, 108, 111, 97, 116, 51, 50, 32, 122,
        10, 101, 108, 101, 109, 101, 110, 32, 102, 97, 99, 101, 32, 54, 10, 112, 114, 111, 112,
        101, 114, 116, 121, 32, 108, 117, 105, 110, 116, 56, 32, 105, 110, 116, 51, 50, 32, 118,
        101, 114, 116, 101, 120, 95, 105, 110, 100, 110, 122, 102, 97, 99, 101, 32, 54, 100, 101,
        120, 10, 101, 110, 100, 95, 104, 101, 97, 100, 101, 114, 10, 48, 48, 49, 32, 48, 32, 48,
        10, 49, 32, 48, 101, 114, 116, 121, 32, 117, 105, 110, 116, 56, 64, 103, 114, 101, 101,
        110, 10, 112, 114, 111, 112, 101, 114, 116, 10, 112, 114, 111, 112, 101, 114, 116, 121, 32,
        108, 57, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    let before = ALLOCATED.load(Ordering::Relaxed);
    let _ = PlyReader::read_from_bytes(&file);
    let requested = ALLOCATED.load(Ordering::Relaxed) - before;

    assert!(
        requested < 16 * 1024 * 1024,
        "reading a {} byte file reserved {requested} bytes",
        file.len()
    );
}
