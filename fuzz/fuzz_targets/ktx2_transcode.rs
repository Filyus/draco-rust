#![no_main]

use draco_texture::ktx2::Ktx2;
use draco_texture::transcode::{Target, Transcoder};
use libfuzzer_sys::fuzz_target;

/// Every target either Basis codec can be asked for.
const TARGETS: [Target; 7] = [
    Target::Rgba8,
    Target::Bc1,
    Target::Bc3,
    Target::Bc7,
    Target::Etc1,
    Target::Etc2,
    Target::Astc,
];

/// How large an image this target will actually decode.
///
/// The reader's own limit is 16384 either way, which is the right limit for a
/// texture and the wrong one for a fuzzing campaign: a header well inside it
/// can legitimately ask for a gigabyte, `-rss_limit_mb` fires on that, and real
/// findings drown in the noise. Parsing is exercised at any size; only decoding
/// is held to this, so that an allocation failure past it is a genuine bug.
const MAX_TEXELS: u64 = 1 << 20;

// KTX2 is untrusted binary with the same shape as the FBX reader's: a header of
// file-controlled offsets and lengths, a supercompression payload that expands,
// and block data indexed arithmetically from dimensions the file states. The
// byte-exact gates cannot see any of that - they compare decoded output against
// a reference, which requires the file to be valid in the first place.
fuzz_target!(|data: &[u8]| {
    let Ok(file) = Ktx2::parse(data) else {
        return;
    };

    // Parsing the same bytes twice must reach the same verdict. Nothing here
    // keeps state between calls, and if that ever stops being true this is
    // where it shows.
    assert!(
        Ktx2::parse(data).is_ok(),
        "parsing the same bytes twice disagreed"
    );

    let texels = (file.width() as u64)
        * (file.height() as u64)
        * (file.layer_count() as u64)
        * (file.face_count() as u64);
    if texels > MAX_TEXELS {
        return;
    }

    let Ok(transcoder) = Transcoder::new(&file) else {
        return;
    };

    // Every level into every target. A target the file's codec cannot reach is
    // a named error rather than a panic, which is itself part of what this
    // checks.
    for level in 0..file.level_count() {
        for target in TARGETS {
            let _ = transcoder.decode(&file, level, 0, 0, target);
        }
    }
});
