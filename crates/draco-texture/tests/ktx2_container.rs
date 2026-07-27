//! What each fixture in `testdata/ktx2` is, read straight out of its header.
//!
//! The facts asserted here — format, dimensions, level count, supercompression
//! — were taken from the reference Basis transcoder reading the same files, not
//! from this crate reading them, so the two are independent.
//!
//! The per-level sizes are the part that matters most. KTX2 stores its level
//! *data* smallest-first while the level *index* runs base-first, and a reader
//! that confuses the two still produces a plausible-looking texture with its
//! mips inside out. For the formats whose level size follows from the
//! dimensions, the size is therefore computed rather than recorded.

use std::path::{Path, PathBuf};

use draco_texture::ktx2::{Ktx2, Ktx2Format, Supercompression};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/ktx2")
        .join(name)
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap_or_else(|error| panic!("reading {name}: {error}"))
}

/// Blocks needed to cover one mip level of a 4×4-block format.
fn blocks_4x4(width: u32, height: u32) -> u64 {
    (width.div_ceil(4) as u64) * (height.div_ceil(4) as u64)
}

#[test]
fn reads_etc1s_files() {
    for (name, width, height, levels, has_alpha) in [
        ("facecap.ktx2", 1024, 1024, 11, false),
        ("2d_etc1s.ktx2", 512, 512, 10, false),
        ("sample_etc1s.ktx2", 1024, 1024, 11, true),
    ] {
        let bytes = read(name);
        let file = Ktx2::parse(&bytes).unwrap_or_else(|error| panic!("parsing {name}: {error}"));

        assert_eq!(
            file.format(),
            Ktx2Format::Etc1s { has_alpha },
            "{name} format"
        );
        assert_eq!(
            (file.width(), file.height()),
            (width, height),
            "{name} dimensions"
        );
        assert_eq!(file.level_count(), levels, "{name} level count");
        assert_eq!(
            file.supercompression(),
            Supercompression::BasisLz,
            "{name} supercompression"
        );
        assert_eq!(file.face_count(), 1, "{name} face count");
        assert_eq!(file.layer_count(), 1, "{name} layer count");
        // Basis LZ keeps its codebooks out of the levels, so there has to be
        // global data and the levels are handed back still encoded.
        assert!(!file.global_data().is_empty(), "{name} has no global data");

        // An ETC1S level is entropy coded, so its size is not derivable - but
        // it can never grow as the image shrinks. Reading the index backwards
        // turns this the other way up.
        let mut previous = u64::MAX;
        for level in 0..file.level_count() {
            let size = file.level_bytes(level).unwrap().len() as u64;
            assert!(
                size <= previous,
                "{name} level {level} is larger than the level above it"
            );
            previous = size;
        }
    }
}

#[test]
fn reads_uastc_files() {
    for (name, width, height, levels, has_alpha) in [
        ("2d_uastc.ktx2", 512, 512, 10, false),
        ("sample_uastc_zstd.ktx2", 1000, 1392, 1, true),
    ] {
        let bytes = read(name);
        let file = Ktx2::parse(&bytes).unwrap_or_else(|error| panic!("parsing {name}: {error}"));

        assert_eq!(
            file.format(),
            Ktx2Format::UastcLdr4x4 { has_alpha },
            "{name} format"
        );
        assert_eq!(
            (file.width(), file.height()),
            (width, height),
            "{name} dimensions"
        );
        assert_eq!(file.level_count(), levels, "{name} level count");
        assert_eq!(
            file.supercompression(),
            Supercompression::Zstd,
            "{name} supercompression"
        );

        // UASTC is a fixed 16 bytes per 4×4 block, so every level's size after
        // decompression follows from its dimensions alone.
        for level in 0..file.level_count() {
            let (level_width, level_height) = file.level_dimensions(level);
            let expected = blocks_4x4(level_width, level_height) * 16;
            assert_eq!(
                file.level_byte_length(level),
                Some(expected),
                "{name} level {level} size"
            );
            assert_eq!(
                file.level_bytes(level).unwrap().len() as u64,
                expected,
                "{name} level {level} decompressed to the wrong size"
            );
        }
    }
}

#[test]
fn reads_a_plain_vk_format_file() {
    // Not something `KHR_texture_basisu` may point at, and read anyway: saying
    // "this is R8G8B8A8, which is not Basis Universal" is a better answer for
    // a caller than refusing the file as unreadable.
    let bytes = read("2d_rgba8.ktx2");
    let file = Ktx2::parse(&bytes).unwrap();

    assert!(
        matches!(file.format(), Ktx2Format::Plain { vk_format: 43, .. }),
        "expected a plain vkFormat, got {:?}",
        file.format()
    );
    assert_eq!((file.width(), file.height()), (512, 512));
    assert_eq!(file.level_count(), 10);
    assert_eq!(file.supercompression(), Supercompression::Zstd);

    for level in 0..file.level_count() {
        let (width, height) = file.level_dimensions(level);
        let expected = u64::from(width) * u64::from(height) * 4;
        assert_eq!(
            file.level_byte_length(level),
            Some(expected),
            "level {level} size"
        );
        assert_eq!(
            file.level_bytes(level).unwrap().len() as u64,
            expected,
            "level {level} bytes"
        );
    }
}

#[test]
fn reads_the_orientation_key() {
    let bytes = read("facecap.ktx2");
    let file = Ktx2::parse(&bytes).unwrap();

    // "rd" is right-then-down: the first row is the top one, which is the
    // origin glTF expects and the reason no flip is needed on upload.
    assert_eq!(file.orientation(), Some("rd"));
    assert!(file.key_value("KTXwriter").is_some());
}

#[test]
fn rejects_files_that_are_not_ktx2() {
    assert!(
        Ktx2::parse(&[0u8; 200]).is_err(),
        "a run of zeroes is not a KTX2 file"
    );
    assert!(
        Ktx2::parse(&[]).is_err(),
        "no bytes at all is not a KTX2 file"
    );

    // Truncation has to be caught by the range checks rather than by panicking
    // on a slice, which is the whole point of writing them that way.
    let bytes = read("2d_etc1s.ktx2");
    for length in [100, 200, 1000, bytes.len() - 1] {
        let _ = Ktx2::parse(&bytes[..length]).map(|file| {
            for level in 0..file.level_count() {
                let _ = file.level_bytes(level);
            }
        });
    }
}
