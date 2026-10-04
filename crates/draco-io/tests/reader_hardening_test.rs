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
use draco_io::stl_reader::StlReader;

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

/// A file that ends before its header says it does is refused, not read as the
/// smaller file it happens to hold. Each case is a complete file with one part
/// cut short, beside the complete file itself, which reads; and a face of two
/// corners, which is complete and only has no triangle in it, still reads.
#[test]
fn a_truncated_ascii_ply_is_refused() {
    let file = |header_tail: &str, body: &str| {
        format!(
            "ply\nformat ascii 1.0\nelement vertex 3\n\
             property float x\nproperty float y\nproperty float z\n{header_tail}end_header\n{body}"
        )
    };
    let face = "element face 1\nproperty list uchar int vertex_indices\n";
    let flagged_face =
        "element face 1\nproperty list uchar int vertex_indices\nproperty uchar flags\n";
    let vertices = "0 0 0\n1 0 0\n0 1 0\n";

    let reads = [
        ("complete", file(face, &format!("{vertices}3 0 1 2\n"))),
        ("two-corner face", file(face, &format!("{vertices}2 0 1\n"))),
    ];
    let refused = [
        ("vertex block", file("", "0 0 0\n1 0 0\n")),
        ("face block", file(face, vertices)),
        ("vertex line", file(face, "0 0 0\n1 0\n0 1 0\n3 0 1 2\n")),
        ("face list", file(face, &format!("{vertices}3 0 1\n"))),
        (
            "scalar after a face list",
            file(flagged_face, &format!("{vertices}3 0 1 2\n")),
        ),
    ];
    let mut failures = Vec::new();
    for (case, text) in &reads {
        if let Err(e) = PlyReader::read_from_bytes(text.as_bytes()) {
            failures.push(format!("{case}: refused: {e}"));
        }
    }
    for (case, text) in &refused {
        if let Ok(mesh) = PlyReader::read_from_bytes(text.as_bytes()) {
            failures.push(format!(
                "{case}: read {} points and {} faces",
                mesh.num_points(),
                mesh.num_faces()
            ));
        }
    }

    // A list declared last on a vertex can claim more values than the line
    // holds; nothing after it is left to find the line short.
    let listed = |line: &str| {
        format!(
            "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\n\
             property float z\nproperty list uchar float extra\nend_header\n{line}\n"
        )
    };
    if let Err(e) = PlyReader::read_from_bytes(listed("0 0 0 2 1 2").as_bytes()) {
        failures.push(format!("vertex list: refused: {e}"));
    }
    if PlyReader::read_from_bytes(listed("0 0 0 2 1").as_bytes()).is_ok() {
        failures.push("vertex list: a list one value short read".into());
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Inputs from upstream's own recent reader fixes that this crate already
/// refuses, held here so a refactor cannot quietly start accepting them: a
/// binary PLY face list cut off mid-index, and OBJ faces naming vertices that
/// do not exist, counting forward and back.
#[test]
fn inputs_upstream_now_refuses_stay_refused() {
    let mut binary = b"ply\nformat binary_little_endian 1.0\nelement vertex 3\n\
property float x\nproperty float y\nproperty float z\n\
element face 1\nproperty list uchar int vertex_indices\nend_header\n"
        .to_vec();
    binary.extend_from_slice(&[0; 36]);
    binary.extend_from_slice(&[3, 0, 0, 0, 0, 1, 0, 0, 0]); // three indices promised, two given

    let mut failures = Vec::new();
    if PlyReader::read_from_bytes(&binary).is_ok() {
        failures.push("binary PLY with a truncated face list");
    }
    if ObjReader::read_from_bytes(b"f 1 2 3\n").is_ok() {
        failures.push("OBJ face with no vertices");
    }
    if ObjReader::read_from_bytes(b"v 0 0 0\nf -2 1 1\n").is_ok() {
        failures.push("OBJ face counting back past the first vertex");
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Input too short to hold a binary STL's header is refused unless it is
/// text: an empty file and 83 zero bytes read as an ASCII file without
/// facets. A short ASCII file still reads, empty or not.
#[test]
fn input_shorter_than_a_binary_stl_header_is_refused_unless_it_is_text() {
    let mut failures = Vec::new();
    for (case, bytes) in [("empty", &[][..]), ("83 zero bytes", &[0u8; 83][..])] {
        if let Ok(mesh) = StlReader::read_from_bytes(bytes) {
            failures.push(format!("{case}: read {} faces", mesh.num_faces()));
        }
    }
    if let Err(e) = StlReader::read_from_bytes(b"solid t\nendsolid t\n") {
        failures.push(format!("short ASCII file: refused: {e}"));
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
