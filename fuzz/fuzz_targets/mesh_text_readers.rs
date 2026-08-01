#![no_main]

//! The OBJ, PLY and STL readers.
//!
//! Until this target they were the workspace's only untrusted parsers with no
//! coverage-guided fuzzing at all - `fbx_read_scene` covered FBX, and these
//! three were reached only by the fixtures their own unit tests carry. An audit
//! found a panic, a seven-second spin and a 48 GB reservation in the PLY reader
//! alone, all from counts printed in a header, which is exactly what a campaign
//! is for.
//!
//! One target for the three because they take the same shape of input - a
//! `&[u8]` that is a whole file - and libFuzzer explores them from one corpus:
//! a mutation that turns a PLY header into something the OBJ reader will chew
//! on is a mutation worth having. Each reader sees every input, so the corpus
//! is shared rather than partitioned by extension.
//!
//! The oracle is oracle 1 only: reading must end in a `Mesh` or an
//! `io::Error`, never a panic, a hang or an unbounded allocation. There is no
//! round-trip claim here - these are readers, and what a file "should" decode
//! to is not something the fuzzer knows.

use draco_io::obj_reader::ObjReader;
use draco_io::ply_reader::PlyReader;
use draco_io::stl_reader::StlReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Every reader sees every input. A file that one of them recognises is
    // usually nonsense to the other two, which is the point: their error paths
    // get as much coverage as their success paths.
    let _ = ObjReader::read_from_bytes(data);
    let _ = PlyReader::read_from_bytes(data);
    let _ = StlReader::read_from_bytes(data);
});
