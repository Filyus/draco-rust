//! The vendored source is somebody else's, and stays that way.
//!
//! An oracle that can be edited is not an oracle. Three megabytes of C++ sit in
//! `csrc/`, and a change to any of it — a well-meant warning fix, a merge gone
//! wrong, a patch applied to make a test pass — would make this agree with
//! something that is no longer the reference, silently.
//!
//! `csrc/UPSTREAM.txt` records the revision and a hash per file. This checks
//! them. Moving to a later revision means regenerating that file deliberately,
//! which is the point.

use std::path::Path;

use basis_cpp_oracle::sha256;

#[test]
fn the_vendored_source_is_what_was_vendored() {
    let csrc = Path::new(env!("CARGO_MANIFEST_DIR")).join("csrc");
    let manifest = std::fs::read_to_string(csrc.join("UPSTREAM.txt")).expect("the manifest");

    let mut checked = 0;
    for line in manifest.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (digest, name) = line.split_once("  ").expect("a hash and a file name");
        let bytes = std::fs::read(csrc.join(name))
            .unwrap_or_else(|error| panic!("{name} is listed but missing: {error}"));
        assert_eq!(
            sha256(&bytes),
            digest,
            "{name} is not the file that was vendored"
        );
        checked += 1;
    }

    assert!(checked > 15, "only {checked} files were listed");

    // And nothing crept in unlisted, which is the other half: a file the
    // manifest does not know about is one nobody reviewed.
    for entry in std::fs::read_dir(&csrc).expect("reading csrc") {
        let entry = entry.expect("a directory entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "UPSTREAM.txt" || name == "oracle.cpp" {
            continue;
        }
        assert!(
            manifest.contains(&format!("  {name}\n")) || manifest.ends_with(&format!("  {name}")),
            "{name} is in csrc but not in UPSTREAM.txt"
        );
    }
}
