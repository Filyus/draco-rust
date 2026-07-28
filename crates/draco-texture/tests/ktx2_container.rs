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

/// Hostile headers, taken from what Binomial fixed in its own reader.
///
/// Between March and July 2026 the reference reader was hardened six times
/// against malformed KTX2: an overflow in the header parser, two cases found
/// by fuzzing, a maximum texture size, validation of the slice descriptions,
/// overflow-safe range checks in `ktx2_transcoder::init`, and an oversized
/// level uncompressed length. None of them changed the format. This reader was
/// written from the specification rather than from that code, so the same
/// classes of defect could be here on their own, and each is checked below
/// against a real file with one field rewritten.
mod hostile {
    use super::*;

    /// The implementation limit on either dimension, as `ktx2.rs` sets it.
    const MAX_DIMENSION: u32 = 16384;

    /// The fixture with one little-endian 32-bit header field replaced.
    fn with_word(name: &str, offset: usize, value: u32) -> Vec<u8> {
        let mut bytes = read(name);
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        bytes
    }

    /// The fixture with one little-endian 64-bit field replaced.
    fn with_long(name: &str, offset: usize, value: u64) -> Vec<u8> {
        let mut bytes = read(name);
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        bytes
    }

    /// Where level `level`'s index entry starts.
    fn level_entry(level: usize) -> usize {
        80 + level * 24
    }

    #[test]
    fn refuses_a_texture_larger_than_the_implementation_limit() {
        // Every buffer this crate sizes is a multiple of width by height, and
        // on wasm32 - the target it actually runs on - that product wraps at
        // 32 bits rather than saturating. A file claiming four billion texels
        // costs nothing to write.
        for (field, offset) in [("pixelWidth", 20), ("pixelHeight", 24)] {
            let bytes = with_word("2d_uastc.ktx2", offset, 0x4001);
            let error = Ktx2::parse(&bytes).expect_err(field);
            assert!(
                error.to_string().contains("pixelWidth or pixelHeight"),
                "{field} of 16385 should be refused by name, got: {error}"
            );
        }
        // The limit is a limit rather than a blanket refusal: at it, the file
        // is still read, whatever its levels then turn out not to contain.
        let bytes = with_word("2d_uastc.ktx2", 20, MAX_DIMENSION);
        assert!(
            Ktx2::parse(&bytes).is_ok(),
            "{MAX_DIMENSION} is within the limit and should still parse"
        );
    }

    #[test]
    fn refuses_a_level_claiming_more_than_it_could_hold() {
        // Believed before anything is decompressed: it is what the output
        // buffer is reserved from, so an unchecked value is an allocation
        // failure rather than a rejected file.
        let bytes = with_long("2d_uastc.ktx2", level_entry(0) + 16, u64::MAX);
        let error = Ktx2::parse(&bytes).expect_err("an exabyte level");
        assert!(
            error.to_string().contains("uncompressedByteLength"),
            "the oversized length should be named, got: {error}"
        );

        // And the bound is a ceiling rather than an equality: the real value
        // is far below it, and a file is not required to be tightly packed.
        let original = read("2d_uastc.ktx2");
        let real = Ktx2::parse(&original)
            .unwrap()
            .level_byte_length(0)
            .unwrap();
        let bytes = with_long("2d_uastc.ktx2", level_entry(0) + 16, real + 1);
        assert!(
            Ktx2::parse(&bytes).is_ok(),
            "one byte over the real size is still within what the level could hold"
        );
    }

    #[test]
    fn refuses_a_level_whose_offset_and_length_wrap() {
        // The pair is checked as "does it fit in what is left" rather than as
        // "offset + length", which wraps to something small and passes.
        let mut bytes = read("2d_uastc.ktx2");
        let entry = level_entry(0);
        bytes[entry..entry + 8].copy_from_slice(&(u64::MAX - 63).to_le_bytes());
        bytes[entry + 8..entry + 16].copy_from_slice(&128u64.to_le_bytes());
        let error = Ktx2::parse(&bytes).expect_err("a wrapping level range");
        assert!(
            error.to_string().contains("level"),
            "the truncated level should be named, got: {error}"
        );
    }

    #[test]
    fn refuses_a_descriptor_that_reaches_past_the_file() {
        let bytes = with_word("2d_uastc.ktx2", 52, u32::MAX);
        Ktx2::parse(&bytes).expect_err("a descriptor longer than the file");
        let bytes = with_word("2d_uastc.ktx2", 48, u32::MAX);
        Ktx2::parse(&bytes).expect_err("a descriptor starting past the file");
    }

    #[test]
    fn refuses_global_data_that_reaches_past_the_file() {
        // Basis LZ keeps its codebooks here, and the field is two 64-bit
        // values of someone else's choosing.
        let bytes = with_long("facecap.ktx2", 72, u64::MAX);
        Ktx2::parse(&bytes).expect_err("global data longer than the file");
        let bytes = with_long("facecap.ktx2", 64, u64::MAX - 1024);
        Ktx2::parse(&bytes).expect_err("global data starting past the file");
    }

    #[test]
    fn refuses_a_level_count_the_index_cannot_hold() {
        let bytes = with_word("2d_uastc.ktx2", 40, 17);
        Ktx2::parse(&bytes).expect_err("more levels than the implementation reads");
        let bytes = with_word("2d_uastc.ktx2", 40, 0);
        Ktx2::parse(&bytes).expect_err("no levels at all");
    }
}
