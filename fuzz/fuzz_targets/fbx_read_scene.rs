#![no_main]

use draco_io::{FbxDecodeLimits, FbxReadOptions, FbxScene};
use libfuzzer_sys::fuzz_target;

// The FBX reader is the only hand-rolled untrusted binary parser in the
// workspace: node records carry file-controlled lengths that feed allocations
// directly, and array payloads go through zlib inflation.
//
// `FbxDecodeLimits::fuzzing()` is deliberately far tighter than the shipped
// defaults. libFuzzer's `-rss_limit_mb` would otherwise fire on a header that
// legitimately asks for hundreds of megabytes, drowning real findings; with the
// tight limits any allocation failure that still occurs is a genuine bug.
fuzz_target!(|data: &[u8]| {
    let options = FbxReadOptions::default().with_limits(FbxDecodeLimits::fuzzing());

    // Reading the same document twice must behave identically. The per-document
    // budgets live on the reader and are reset per read; if that ever regresses,
    // the second call fails on input the first accepted.
    let first = FbxScene::from_bytes_with_options(data, options.clone());
    let second = FbxScene::from_bytes_with_options(data, options.clone());
    assert_eq!(
        first.is_ok(),
        second.is_ok(),
        "reading the same bytes twice disagreed"
    );

    // Strict mode may reject more, but it must never accept what lenient mode
    // rejects, and it must not crash on anything.
    let strict = FbxScene::from_bytes_with_options(data, FbxReadOptions::strict().with_limits(
        FbxDecodeLimits::fuzzing(),
    ));
    if strict.is_ok() {
        assert!(
            first.is_ok(),
            "strict mode accepted input that lenient mode rejected"
        );
    }
});
