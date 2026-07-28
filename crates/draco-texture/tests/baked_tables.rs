//! Where the baked tables came from, checked rather than remembered.
//!
//! Two of the three blobs under `src/tables/` are solved by
//! [`etc1s_to_bc1::bake_bc1_tables`] and a test in that module proves the
//! committed bytes are what it produces. The third is not solved here at all —
//! it is the reference's own `basisu_transcoder_tables_astc.inc`, transformed
//! into four bytes an entry — and it was baked by a throwaway script, which
//! left 61440 bytes in the repository that nobody could re-derive.
//!
//! The idea of keeping the generator beside the table is taken from the
//! `basisu` crate, which has `tools/gen_table.py` for the same reason. Here the
//! source is already in the tree: `tools/basis-cpp-oracle` vendors that `.inc`
//! for the oracle, under a hash, so the transformation can simply be redone and
//! compared.

use std::path::Path;

/// The blob as `etc1s_to_astc.rs` reads it: low, high, and a 16-bit error.
const SOLUTION_BYTES: usize = 4;
/// 32 base colours x 8 intensity tables x 6 selector ranges x 10 mappings.
const SOLUTIONS: usize = 32 * 8 * 6 * 10;

#[test]
fn the_astc_table_is_the_reference_s_own() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("../../tools/basis-cpp-oracle/csrc/basisu_transcoder_tables_astc.inc");
    let text = std::fs::read_to_string(&source).unwrap_or_else(|error| {
        panic!(
            "the vendored table is missing: {error}. It lives with the oracle \
             because the oracle needs it too; see tools/basis-cpp-oracle."
        )
    });

    // The file is a list of `{lo,hi,err}` initialisers and nothing else.
    let mut baked = Vec::with_capacity(SOLUTIONS * SOLUTION_BYTES);
    let mut entries = 0;
    for entry in text.split('{').skip(1) {
        let entry = entry.split('}').next().expect("a closed initialiser");
        let mut numbers = entry.split(',').map(|value| {
            value
                .trim()
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("{value:?} is not a number"))
        });
        let low = numbers.next().expect("a low endpoint");
        let high = numbers.next().expect("a high endpoint");
        let error = numbers.next().expect("an error");
        assert!(low < 48 && high < 48, "an endpoint outside [0,47]");
        assert!(error <= u32::from(u16::MAX), "an error that does not fit");

        baked.push(low as u8);
        baked.push(high as u8);
        baked.extend_from_slice(&(error as u16).to_le_bytes());
        entries += 1;
    }

    assert_eq!(
        entries, SOLUTIONS,
        "the vendored table has {entries} entries, not {SOLUTIONS}"
    );

    let committed = std::fs::read(root.join("src/tables/etc1s_to_astc.bin")).expect("the blob");
    assert_eq!(
        committed.len(),
        baked.len(),
        "the committed blob is a different size from the reference's table"
    );
    let difference = committed
        .iter()
        .zip(baked.iter())
        .position(|(a, b)| a != b)
        .map(|at| format!("first differs at byte {at}, entry {}", at / SOLUTION_BYTES));
    assert!(
        difference.is_none(),
        "the committed blob is not the reference's table: {}",
        difference.unwrap()
    );
}
