//! Reading a KTX2 file down to the bytes of each mip level.
//!
//! Ported from the Khronos KTX2 specification and the reference reader in
//! `BinomialLLC/basis_universal` (`transcoder/basisu_transcoder.cpp`,
//! `ktx2_transcoder::init`), revision `9bebe16` of 2026-07-22, Apache-2.0 —
//! the same licence as this repository. The structure layout, the validation
//! rules and the data format descriptor offsets follow that reader; the code
//! is written from it rather than bound to it.
//!
//! What this module answers is "what is in this file and where", not "what
//! do the pixels look like": a level of an ETC1S or UASTC file comes back
//! still encoded, because turning it into pixels is a codec's job and each
//! codec is its own module.

use std::borrow::Cow;

/// The 12 bytes every KTX2 file starts with.
pub const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Byte size of the fixed header, after which the level index begins.
const HEADER_SIZE: usize = 80;
/// Byte size of one level index entry: offset, length, uncompressed length.
const LEVEL_INDEX_SIZE: usize = 24;

/// Implementation limit on the mip level count, as the reference reader sets it.
const MAX_LEVEL_COUNT: u32 = 16;
/// Implementation limit on the array layer count, as the reference reader sets it.
const MAX_LAYER_COUNT: u32 = 65535;
/// Implementation limit on either dimension, as the reference reader sets it.
///
/// Nothing in the format says a texture cannot be four billion texels wide, and
/// a file claiming that costs nothing to write. What it costs to read is the
/// point: every buffer this crate sizes is some multiple of width by height,
/// and on the 32-bit target it actually runs on that multiplication wraps
/// rather than saturating. A ceiling here is what keeps the arithmetic below
/// well inside `u32` on any target.
const MAX_DIMENSION: u32 = 16384;
/// The most any format read here can spend on one texel.
///
/// Only used to bound a declared uncompressed length before it is believed. A
/// block format spends at most sixteen bytes per 4x4 block, so this is loose
/// by a factor of sixteen against the formats that matter - being right is not
/// the job, being finite is.
const MAX_BYTES_PER_TEXEL: u64 = 16;

/// The most bytes one level could hold, whatever format it turns out to be.
///
/// Not a size but a ceiling, and deliberately a loose one: it exists so that a
/// declared uncompressed length can be refused before anything is reserved
/// from it. Every term is bounded by a check above, so the product cannot
/// overflow.
fn level_ceiling(width: u32, height: u32, level: u32, layers: u32, faces: u32) -> u64 {
    let width = (width >> level).max(1) as u64;
    let height = (height >> level).max(1) as u64;
    width * height * MAX_BYTES_PER_TEXEL * layers.max(1) as u64 * faces.max(1) as u64
}

// Data format descriptor colour models. The two this crate can transcode plus
// the plain-ASTC one, which is named so the error can say what it found.
const COLOR_MODEL_ETC1S: u32 = 163;
const COLOR_MODEL_UASTC_LDR_4X4: u32 = 166;

// Channel ids of a UASTC sample, which is where its alpha claim comes from.
const CHANNEL_UASTC_RGBA: u32 = 3;
const CHANNEL_UASTC_RRRG: u32 = 5;

const TRANSFER_LINEAR: u32 = 1;
const TRANSFER_SRGB: u32 = 2;

/// Everything that can stop a KTX2 file from being read.
#[derive(Debug, thiserror::Error)]
pub enum Ktx2Error {
    /// The first twelve bytes are not the KTX2 identifier.
    #[error("not a KTX2 file: the identifier is missing")]
    Identifier,
    /// A structure reaches past the end of the data.
    #[error("KTX2 file is truncated: {0} lies outside the data")]
    Truncated(&'static str),
    /// A header field holds a value the format does not allow.
    #[error("invalid KTX2 {field}: {value}")]
    Invalid {
        /// Which field disagreed with the format.
        field: &'static str,
        /// What it held.
        value: u64,
    },
    /// The file is well formed but holds something this crate does not read.
    #[error("unsupported KTX2 file: {0}")]
    Unsupported(String),
    /// A supercompressed level did not decompress.
    #[error("KTX2 level {level} failed to decompress: {reason}")]
    Decompress {
        /// Which mip level.
        level: u32,
        /// What the decompressor said.
        reason: String,
    },
}

/// How the level data is packed on top of its own encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Supercompression {
    /// Level data is stored as-is.
    None,
    /// Basis LZ: the ETC1S codebooks and slices live in the global data.
    BasisLz,
    /// Zstandard, which is what `toktx --zstd` writes around UASTC.
    Zstd,
}

/// What the level data is encoded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ktx2Format {
    /// Basis Universal ETC1S, always paired with Basis LZ supercompression.
    Etc1s {
        /// Whether a second slice carries alpha.
        has_alpha: bool,
    },
    /// Basis Universal UASTC LDR, 4×4 blocks.
    UastcLdr4x4 {
        /// Whether the block's alpha channel is meaningful.
        has_alpha: bool,
    },
    /// A concrete `vkFormat`: ordinary or block-compressed pixels.
    ///
    /// Read, but not transcoded. `KHR_texture_basisu` requires Basis
    /// Universal payloads, so a glTF texture is never legitimately one of
    /// these — naming it is how the caller can say so instead of failing with
    /// "unsupported".
    Plain {
        /// The Vulkan format the header names.
        vk_format: u32,
        /// The colour model the data format descriptor names.
        color_model: u32,
    },
}

/// One entry of the level index.
#[derive(Debug, Clone, Copy)]
struct LevelIndex {
    byte_offset: u64,
    byte_length: u64,
    uncompressed_byte_length: u64,
}

/// One key/value pair out of the key/value data.
#[derive(Debug, Clone)]
pub struct KeyValue {
    /// The key, which the format defines as UTF-8 and NUL-terminated.
    pub key: String,
    /// The value bytes, with the key's terminator and the padding removed.
    pub value: Vec<u8>,
}

/// A parsed KTX2 file, borrowing the bytes it was parsed from.
#[derive(Debug)]
pub struct Ktx2<'a> {
    data: &'a [u8],
    format: Ktx2Format,
    supercompression: Supercompression,
    pixel_width: u32,
    pixel_height: u32,
    layer_count: u32,
    face_count: u32,
    levels: Vec<LevelIndex>,
    key_values: Vec<KeyValue>,
    global_data: (usize, usize),
    srgb: bool,
}

impl<'a> Ktx2<'a> {
    /// Read the header, the level index, the format descriptor and the key values.
    ///
    /// Level data is not touched here: a file is described before any of it is
    /// decompressed, so a caller can decide what to do with a format it does
    /// not want without paying for the levels.
    pub fn parse(data: &'a [u8]) -> Result<Self, Ktx2Error> {
        if data.len() <= HEADER_SIZE {
            return Err(Ktx2Error::Truncated("the header"));
        }
        if data[..12] != IDENTIFIER {
            return Err(Ktx2Error::Identifier);
        }

        let word = |index: usize| u32::from_le_bytes(data[index..index + 4].try_into().unwrap());
        let long = |index: usize| u64::from_le_bytes(data[index..index + 8].try_into().unwrap());

        let vk_format = word(12);
        let type_size = word(16);
        let pixel_width = word(20);
        let pixel_height = word(24);
        let pixel_depth = word(28);
        let layer_count = word(32);
        let face_count = word(36);
        let level_count = word(40);
        let scheme = word(44);
        let dfd_offset = word(48) as usize;
        let dfd_length = word(52) as usize;
        let sgd_offset = long(64);
        let sgd_length = long(72);

        // 3.3: "When format is VK_FORMAT_UNDEFINED, typeSize must equal 1", and
        // every format this crate reads leaves it at 1 regardless.
        if type_size != 1 {
            return Err(Ktx2Error::Invalid {
                field: "typeSize",
                value: type_size.into(),
            });
        }
        // 1D and 3D textures are outside both this crate and Basis itself.
        if pixel_width < 1 || pixel_height < 1 || pixel_depth > 0 {
            return Err(Ktx2Error::Unsupported(
                "only 2D and cubemap textures are read".into(),
            ));
        }
        if pixel_width > MAX_DIMENSION || pixel_height > MAX_DIMENSION {
            return Err(Ktx2Error::Invalid {
                field: "pixelWidth or pixelHeight",
                value: pixel_width.max(pixel_height).into(),
            });
        }
        if face_count != 1 && face_count != 6 {
            return Err(Ktx2Error::Invalid {
                field: "faceCount",
                value: face_count.into(),
            });
        }
        if face_count > 1 && pixel_width != pixel_height {
            return Err(Ktx2Error::Invalid {
                field: "cubemap pixelWidth",
                value: pixel_width.into(),
            });
        }
        if !(1..=MAX_LEVEL_COUNT).contains(&level_count) {
            return Err(Ktx2Error::Invalid {
                field: "levelCount",
                value: level_count.into(),
            });
        }
        if layer_count > MAX_LAYER_COUNT {
            return Err(Ktx2Error::Invalid {
                field: "layerCount",
                value: layer_count.into(),
            });
        }

        let supercompression = match scheme {
            0 => Supercompression::None,
            1 => Supercompression::BasisLz,
            2 => Supercompression::Zstd,
            other => {
                return Err(Ktx2Error::Unsupported(format!(
                    "supercompression scheme {other} is not read"
                )))
            }
        };

        // Offsets and lengths here are attacker-controlled 64-bit values, so
        // every range check is written as "does it fit in what is left" rather
        // than as "offset + length", which can wrap and pass.
        let in_range = |offset: u64, length: u64| -> bool {
            offset >= HEADER_SIZE as u64
                && offset <= data.len() as u64
                && length <= data.len() as u64 - offset
        };

        if supercompression == Supercompression::BasisLz && !in_range(sgd_offset, sgd_length) {
            return Err(Ktx2Error::Truncated("the supercompression global data"));
        }

        let index_size = level_count as usize * LEVEL_INDEX_SIZE;
        if HEADER_SIZE + index_size > data.len() {
            return Err(Ktx2Error::Truncated("the level index"));
        }
        let mut levels = Vec::with_capacity(level_count as usize);
        for level in 0..level_count as usize {
            let base = HEADER_SIZE + level * LEVEL_INDEX_SIZE;
            let entry = LevelIndex {
                byte_offset: long(base),
                byte_length: long(base + 8),
                uncompressed_byte_length: long(base + 16),
            };
            if entry.byte_length == 0 || !in_range(entry.byte_offset, entry.byte_length) {
                return Err(Ktx2Error::Truncated("a level"));
            }
            // Basis LZ carries its own sizes, so the field must be zero there;
            // Zstd needs it, because it is the size to allocate.
            match supercompression {
                Supercompression::BasisLz if entry.uncompressed_byte_length != 0 => {
                    return Err(Ktx2Error::Invalid {
                        field: "level uncompressedByteLength",
                        value: entry.uncompressed_byte_length,
                    })
                }
                // 3.9.4: with no supercompression the two lengths describe the
                // same bytes and must agree. The reference asserts on it; this
                // reader did not, which the parity test found by writing a
                // file this reader would have read and the reference would not.
                Supercompression::None if entry.uncompressed_byte_length != entry.byte_length => {
                    return Err(Ktx2Error::Invalid {
                        field: "level uncompressedByteLength",
                        value: entry.uncompressed_byte_length,
                    })
                }
                Supercompression::Zstd if entry.uncompressed_byte_length == 0 => {
                    return Err(Ktx2Error::Invalid {
                        field: "level uncompressedByteLength",
                        value: 0,
                    })
                }
                // This one is believed before anything is decompressed - it is
                // what the output buffer is reserved from - so a file claiming
                // an exabyte would be an allocation failure rather than a
                // rejected file. The dimensions are already bounded above, so
                // what the level could possibly hold is finite and known.
                Supercompression::Zstd
                    if entry.uncompressed_byte_length
                        > level_ceiling(
                            pixel_width,
                            pixel_height,
                            level as u32,
                            layer_count,
                            face_count,
                        ) =>
                {
                    return Err(Ktx2Error::Invalid {
                        field: "level uncompressedByteLength",
                        value: entry.uncompressed_byte_length,
                    })
                }
                _ => {}
            }
            levels.push(entry);
        }

        // A descriptor is a fixed 28-byte block plus one 16-byte sample per
        // channel, so anything shorter cannot name a colour model at all. The
        // reference reader additionally insists on 44 or 60 bytes, but that is
        // a property of files Basis itself wrote: an ordinary R8G8B8A8 file
        // has four samples and a 92-byte descriptor, and refusing to read its
        // header would be refusing to say what it is.
        // The key/value data is the one section this crate reads opportunistically
        // rather than structurally - orientation, and whatever else a writer left
        // - so its range was never checked. It has to be: a length past the end
        // of the file is a malformed file, and reading it as if it were absent
        // means accepting one the reference refuses and then disagreeing with it
        // about what the rest of the file says. The differential gate found
        // exactly that.
        let kvd_offset = word(56) as u64;
        let kvd_length = word(60) as u64;
        if kvd_length != 0 && !in_range(kvd_offset, kvd_length) {
            return Err(Ktx2Error::Truncated("the key/value data"));
        }

        if dfd_length < 28 {
            return Err(Ktx2Error::Unsupported(format!(
                "data format descriptor of {dfd_length} bytes"
            )));
        }
        if !in_range(dfd_offset as u64, dfd_length as u64) {
            return Err(Ktx2Error::Truncated("the data format descriptor"));
        }
        if word(dfd_offset) as usize != dfd_length {
            return Err(Ktx2Error::Invalid {
                field: "dfdTotalSize",
                value: word(dfd_offset).into(),
            });
        }

        let descriptor_bits = word(dfd_offset + 12);
        let color_model = descriptor_bits & 255;
        let transfer_function = (descriptor_bits >> 16) & 255;
        let channel0 = (word(dfd_offset + 28) >> 24) & 15;

        if transfer_function != TRANSFER_LINEAR && transfer_function != TRANSFER_SRGB {
            return Err(Ktx2Error::Invalid {
                field: "transfer function",
                value: transfer_function.into(),
            });
        }

        let format = match (vk_format, color_model) {
            // 3.10.2: "Whether the image has 1 or 2 slices can be determined
            // from the DFD's sample count" - and with ETC1S's one fixed sample
            // layout, the sample count is the descriptor's length.
            (0, COLOR_MODEL_ETC1S) if dfd_length == 44 || dfd_length == 60 => Ktx2Format::Etc1s {
                has_alpha: dfd_length == 60,
            },
            (0, COLOR_MODEL_ETC1S) => {
                return Err(Ktx2Error::Unsupported(format!(
                    "ETC1S with a data format descriptor of {dfd_length} bytes"
                )))
            }
            (0, COLOR_MODEL_UASTC_LDR_4X4) => Ktx2Format::UastcLdr4x4 {
                has_alpha: channel0 == CHANNEL_UASTC_RGBA || channel0 == CHANNEL_UASTC_RRRG,
            },
            // A Basis colour model with a concrete vkFormat, or the reverse, is
            // a contradiction rather than a format we happen not to read.
            (_, COLOR_MODEL_ETC1S) | (_, COLOR_MODEL_UASTC_LDR_4X4) => {
                return Err(Ktx2Error::Invalid {
                    field: "vkFormat",
                    value: vk_format.into(),
                })
            }
            (0, other) => {
                return Err(Ktx2Error::Unsupported(format!(
                    "colour model {other} with no vkFormat"
                )))
            }
            _ => Ktx2Format::Plain {
                vk_format,
                color_model,
            },
        };

        let key_values = read_key_values(data, word(56) as usize, word(60) as usize);

        Ok(Ktx2 {
            data,
            format,
            supercompression,
            pixel_width,
            pixel_height,
            layer_count,
            face_count,
            levels,
            key_values,
            global_data: (sgd_offset as usize, sgd_length as usize),
            srgb: transfer_function == TRANSFER_SRGB,
        })
    }

    /// What the level data is encoded as.
    pub fn format(&self) -> Ktx2Format {
        self.format
    }

    /// How the level data is packed on top of that encoding.
    pub fn supercompression(&self) -> Supercompression {
        self.supercompression
    }

    /// Width of mip level 0.
    pub fn width(&self) -> u32 {
        self.pixel_width
    }

    /// Height of mip level 0.
    pub fn height(&self) -> u32 {
        self.pixel_height
    }

    /// How many mip levels the file stores, always at least one.
    pub fn level_count(&self) -> u32 {
        self.levels.len() as u32
    }

    /// How many array layers, counting a non-array texture as one.
    pub fn layer_count(&self) -> u32 {
        self.layer_count.max(1)
    }

    /// How many cubemap faces: one, or six.
    pub fn face_count(&self) -> u32 {
        self.face_count
    }

    /// Whether the transfer function is sRGB rather than linear.
    pub fn is_srgb(&self) -> bool {
        self.srgb
    }

    /// Pixel dimensions of one mip level, never smaller than one texel.
    pub fn level_dimensions(&self, level: u32) -> (u32, u32) {
        (
            (self.pixel_width >> level).max(1),
            (self.pixel_height >> level).max(1),
        )
    }

    /// The supercompression global data, which for Basis LZ holds the codebooks.
    pub fn global_data(&self) -> &'a [u8] {
        let (offset, length) = self.global_data;
        &self.data[offset..offset + length]
    }

    /// Every key/value pair the file carries, in file order.
    pub fn key_values(&self) -> &[KeyValue] {
        &self.key_values
    }

    /// The value stored under one key, if the file has it.
    pub fn key_value(&self, key: &str) -> Option<&[u8]> {
        self.key_values
            .iter()
            .find(|pair| pair.key == key)
            .map(|pair| pair.value.as_slice())
    }

    /// `KTXorientation`, which says where the first row of pixels belongs.
    ///
    /// glTF wants the origin at the top left, which is what `rd` means and
    /// what every writer in practice produces. A file that says otherwise has
    /// to be flipped by whoever uploads it, so the raw value is handed over
    /// rather than interpreted here.
    pub fn orientation(&self) -> Option<&str> {
        let value = self.key_value("KTXorientation")?;
        let end = value
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(value.len());
        std::str::from_utf8(&value[..end]).ok()
    }

    /// The bytes of one mip level, with supercompression undone.
    ///
    /// Borrowed when the level is stored as-is, which is every ETC1S and every
    /// uncompressed file: only Zstd has to allocate. Basis LZ is *not* undone
    /// here, because it is not a wrapper around the level — the codebooks live
    /// in the global data and only the ETC1S decoder can use them.
    pub fn level_bytes(&self, level: u32) -> Result<Cow<'a, [u8]>, Ktx2Error> {
        let entry = self.levels.get(level as usize).ok_or(Ktx2Error::Invalid {
            field: "level index",
            value: level.into(),
        })?;
        let start = entry.byte_offset as usize;
        let raw = &self.data[start..start + entry.byte_length as usize];
        match self.supercompression {
            Supercompression::None | Supercompression::BasisLz => Ok(Cow::Borrowed(raw)),
            Supercompression::Zstd => {
                let expected = entry.uncompressed_byte_length as usize;
                let mut out = Vec::with_capacity(expected);
                ruzstd::FrameDecoder::new()
                    .decode_all_to_vec(raw, &mut out)
                    .map_err(|error| Ktx2Error::Decompress {
                        level,
                        reason: error.to_string(),
                    })?;
                if out.len() != expected {
                    return Err(Ktx2Error::Decompress {
                        level,
                        reason: format!("expected {expected} bytes, decompressed {}", out.len()),
                    });
                }
                Ok(Cow::Owned(out))
            }
        }
    }

    /// How many bytes level `level` occupies once supercompression is undone.
    ///
    /// Known from the index alone for Zstd, and equal to the stored length
    /// otherwise, so a caller can size a buffer without decompressing.
    pub fn level_byte_length(&self, level: u32) -> Option<u64> {
        let entry = self.levels.get(level as usize)?;
        Some(match self.supercompression {
            Supercompression::Zstd => entry.uncompressed_byte_length,
            _ => entry.byte_length,
        })
    }
}

/// Read the key/value data, skipping anything malformed rather than failing.
///
/// Nothing downstream depends on these: they carry orientation and writer
/// provenance, and a file whose key/value block is damaged still has perfectly
/// good pixels. The reference reader treats them the same way.
fn read_key_values(data: &[u8], offset: usize, length: usize) -> Vec<KeyValue> {
    let mut pairs = Vec::new();
    let Some(end) = offset.checked_add(length).filter(|end| *end <= data.len()) else {
        return pairs;
    };
    let mut cursor = offset;
    while cursor + 4 <= end {
        let size = u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 4;
        if size == 0 || size > end - cursor {
            break;
        }
        let entry = &data[cursor..cursor + size];
        if let Some(split) = entry.iter().position(|byte| *byte == 0) {
            if let Ok(key) = std::str::from_utf8(&entry[..split]) {
                pairs.push(KeyValue {
                    key: key.to_string(),
                    value: entry[split + 1..].to_vec(),
                });
            }
        }
        // Each pair is padded so the next length starts on a four-byte boundary.
        cursor += (size + 3) & !3;
    }
    pairs
}
