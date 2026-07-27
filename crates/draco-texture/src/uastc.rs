//! Basis Universal UASTC LDR 4×4: block decoding to RGBA.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `unpack_uastc` in `transcoder/basisu_transcoder.cpp`, the mode tables in
//! `transcoder/basisu_transcoder_uastc.h`, and the ASTC endpoint
//! unquantization taken there from the ASTC specification.
//!
//! Where ETC1S codes a whole image as one stream, UASTC is the opposite: every
//! block is an independent 128 bits, so a level decodes on its own and there
//! are no codebooks. The complexity is inside the block instead. Nineteen
//! modes trade off between one, two and three colour regions, one or two
//! interpolation planes, and endpoint precisions from two to eight bits, and
//! the endpoints are packed with the ASTC integer sequence encoding, which
//! bundles values in base 3 or base 5 so a quantization level that is not a
//! power of two costs a fraction of a bit rather than a whole one.

/// A UASTC block is always 4×4 texels in 16 bytes.
pub const BLOCK_SIZE: usize = 16;

const TOTAL_MODES: usize = 19;
/// The mode that stores a single colour and nothing else.
const MODE_SOLID_COLOR: usize = 8;

/// Which mode each of the 128 possible leading bit patterns selects.
///
/// The mode is prefix coded, and the reference flattens that code into a table
/// indexed by the low seven bits of the block. A value of 19 or more is a
/// pattern the format does not assign, which is how a corrupt block is caught.
const HUFF_MODES: [u8; 128] = [
    11, 0, 10, 3, 11, 15, 12, 7, 11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4, 11, 16, 12, 8, 11, 18,
    10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 17, 12, 7, 11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4,
    11, 1, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 19, 12, 7, 11, 18, 10, 5, 11, 14,
    12, 9, 11, 0, 10, 4, 11, 16, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 17, 12, 7,
    11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4, 11, 1, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13,
];

/// How many bits the mode's prefix code itself occupies.
const MODE_CODE_BITS: [u8; TOTAL_MODES] = [4, 6, 5, 5, 5, 5, 5, 5, 5, 5, 3, 2, 3, 5, 5, 7, 6, 6, 4];

pub(crate) const MODE_WEIGHT_BITS: [u8; TOTAL_MODES] =
    [4, 2, 3, 2, 2, 3, 2, 2, 0, 2, 4, 2, 3, 1, 2, 4, 2, 2, 5];
pub(crate) const MODE_ENDPOINT_RANGES: [u8; TOTAL_MODES] = [
    19, 20, 8, 7, 12, 20, 18, 12, 0, 8, 13, 13, 19, 20, 20, 20, 20, 20, 11,
];
pub(crate) const MODE_SUBSETS: [u8; TOTAL_MODES] =
    [1, 1, 2, 3, 2, 1, 1, 2, 0, 2, 1, 1, 1, 1, 1, 1, 2, 1, 1];
pub(crate) const MODE_COMPONENTS: [u8; TOTAL_MODES] =
    [3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 2, 2, 2, 3];
/// Bits of encoder hints for other formats, which a decode to pixels skips.
const MODE_HINT_BITS: [u8; TOTAL_MODES] = [
    15, 15, 15, 15, 15, 15, 15, 15, 0, 23, 17, 17, 17, 23, 23, 23, 23, 23, 15,
];

/// Bits, trits and quints of each ASTC quantization range.
const BISE_RANGES: [[u8; 3]; 21] = [
    [1, 0, 0],
    [0, 1, 0],
    [2, 0, 0],
    [0, 0, 1],
    [1, 1, 0],
    [3, 0, 0],
    [1, 0, 1],
    [2, 1, 0],
    [4, 0, 0],
    [2, 0, 1],
    [3, 1, 0],
    [5, 0, 0],
    [3, 0, 1],
    [4, 1, 0],
    [6, 0, 0],
    [4, 0, 1],
    [5, 1, 0],
    [7, 0, 0],
    [5, 0, 1],
    [6, 1, 0],
    [8, 0, 0],
];

/// The `B` bit pattern and `C` multiplier each trit/quint range unquantizes with.
///
/// Straight from the ASTC specification, tables 81 and 93. A letter names a bit
/// of the packed value to copy into that position of `B`; `0` is a zero bit.
const UNQUANT_PARAMS: [(&[u8; 9], u32); 21] = [
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 0),
    (b"000000000", 204),
    (b"000000000", 0),
    (b"000000000", 113),
    (b"b000b0bb0", 93),
    (b"000000000", 0),
    (b"b0000bb00", 54),
    (b"cb000cbcb", 44),
    (b"000000000", 0),
    (b"cb0000cbc", 26),
    (b"dcb000dcb", 22),
    (b"000000000", 0),
    (b"dcb0000dc", 13),
    (b"edcb000ed", 11),
    (b"000000000", 0),
    (b"edcb0000e", 6),
    (b"fedcb000f", 5),
    (b"000000000", 0),
];

/// Interpolation weights, indexed by weight bit count.
const WEIGHTS_1: [u32; 2] = [0, 64];
const WEIGHTS_2: [u32; 4] = [0, 21, 43, 64];
const WEIGHTS_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHTS_4: [u32; 16] = [0, 4, 8, 12, 17, 21, 25, 29, 35, 39, 43, 47, 52, 56, 60, 64];
const WEIGHTS_5: [u32; 32] = [
    0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 34, 36, 38, 40, 42, 44, 46, 48, 50,
    52, 54, 56, 58, 60, 62, 64,
];

/// Which subset each texel belongs to, for the two-subset patterns.
const ASTC_BC7_PATTERNS2: [[u8; 16]; 30] = [
    [0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1],
    [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0, 0],
    [0, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1],
    [1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0],
    [1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 1],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 1, 1],
    [1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0],
    [0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 1, 1],
    [0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0],
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1],
    [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0],
    [1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0],
];

/// Which subset each texel belongs to, for the three-subset patterns.
const ASTC_BC7_PATTERNS3: [[u8; 16]; 11] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 1, 1, 2, 2],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2],
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 2],
    [1, 1, 1, 1, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0],
    [0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2, 0, 1, 1, 2],
    [0, 2, 1, 1, 0, 2, 1, 1, 0, 2, 1, 1, 0, 2, 1, 1],
    [2, 0, 0, 0, 2, 0, 0, 0, 2, 1, 1, 1, 2, 1, 1, 1],
    [2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2, 2, 0, 1, 2],
    [1, 1, 1, 1, 0, 0, 0, 0, 2, 2, 2, 2, 1, 1, 1, 1],
    [0, 0, 2, 2, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 2, 2],
];

/// Mode 7's own two-subset patterns, which ASTC and BC7 share differently.
const BC7_3_ASTC2_PATTERNS2: [[u8; 16]; 19] = [
    [0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0],
    [1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1],
    [0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0],
    [0, 0, 0, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0],
    [0, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 0],
    [1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0],
    [0, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 0],
    [1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0],
    [0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 0],
    [1, 1, 1, 1, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
];

/// First texel of each subset, whose weight is stored one bit short.
const ASTC_BC7_PATTERN2_ANCHORS: [[u8; 3]; 30] = [
    [0, 2, 0],
    [0, 3, 0],
    [1, 0, 0],
    [0, 3, 0],
    [7, 0, 0],
    [0, 2, 0],
    [3, 0, 0],
    [7, 0, 0],
    [0, 11, 0],
    [2, 0, 0],
    [0, 7, 0],
    [11, 0, 0],
    [3, 0, 0],
    [8, 0, 0],
    [0, 4, 0],
    [12, 0, 0],
    [1, 0, 0],
    [8, 0, 0],
    [0, 1, 0],
    [0, 2, 0],
    [0, 4, 0],
    [8, 0, 0],
    [1, 0, 0],
    [0, 2, 0],
    [4, 0, 0],
    [0, 1, 0],
    [4, 0, 0],
    [1, 0, 0],
    [4, 0, 0],
    [1, 0, 0],
];

const ASTC_BC7_PATTERN3_ANCHORS: [[u8; 3]; 11] = [
    [0, 8, 10],
    [8, 0, 12],
    [4, 0, 12],
    [8, 0, 4],
    [3, 0, 2],
    [0, 1, 3],
    [0, 2, 1],
    [1, 9, 0],
    [1, 2, 0],
    [4, 0, 8],
    [0, 6, 2],
];

const BC7_3_ASTC2_PATTERNS2_ANCHORS: [[u8; 3]; 19] = [
    [0, 4, 0],
    [0, 2, 0],
    [2, 0, 0],
    [0, 7, 0],
    [8, 0, 0],
    [0, 1, 0],
    [0, 3, 0],
    [0, 1, 0],
    [2, 0, 0],
    [0, 1, 0],
    [0, 8, 0],
    [2, 0, 0],
    [0, 1, 0],
    [0, 7, 0],
    [12, 0, 0],
    [2, 0, 0],
    [9, 0, 0],
    [0, 2, 0],
    [4, 0, 0],
];

const ZERO_PATTERN: [u8; 16] = [0; 16];

/// A block that does not decode.
#[derive(Debug, thiserror::Error)]
pub enum UastcError {
    /// The level data is not a whole number of blocks.
    #[error("UASTC level is {0} bytes, which is not a whole number of 16-byte blocks")]
    Truncated(usize),
    /// The block's leading bits name no mode.
    #[error("UASTC block {index} names mode {mode}, which does not exist")]
    BadMode {
        /// Which block in the level.
        index: usize,
        /// The mode value read.
        mode: u8,
    },
    /// A multi-subset mode named a partition pattern outside its table.
    #[error("UASTC block {index} names partition pattern {pattern}, which does not exist")]
    BadPattern {
        /// Which block in the level.
        index: usize,
        /// The pattern index read.
        pattern: u32,
    },
}

/// Decode a whole mip level into RGBA8, `width * height * 4` bytes.
///
/// The stored values come out as they are, with no transfer function applied,
/// whatever the file's data format descriptor says. That is what the reference
/// does on its own path to pixels, and it is what a caller wants: the sRGB
/// question belongs to whoever uploads or displays the image, and answering it
/// twice would darken the result.
pub fn decode_rgba(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, UastcError> {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    if data.len() < blocks_x * blocks_y * BLOCK_SIZE {
        return Err(UastcError::Truncated(data.len()));
    }

    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let mut texels = [[0u8; 4]; 16];
    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let index = block_y * blocks_x + block_x;
            let block = &data[index * BLOCK_SIZE..index * BLOCK_SIZE + BLOCK_SIZE];
            decode_block(block, index, &mut texels)?;

            // A block on the right or bottom edge hangs over when the image is
            // not a multiple of four, and the texels past the edge are dropped.
            let max_x = 4.min(width as usize - block_x * 4);
            let max_y = 4.min(height as usize - block_y * 4);
            for y in 0..max_y {
                let row = ((block_y * 4 + y) * width as usize + block_x * 4) * 4;
                for x in 0..max_x {
                    pixels[row + x * 4..row + x * 4 + 4].copy_from_slice(&texels[y * 4 + x]);
                }
            }
        }
    }
    Ok(pixels)
}

/// Transcode a whole mip level into ASTC 4x4 blocks, sixteen bytes each.
///
/// UASTC is a restricted profile of ASTC, so this rewrites each block into
/// ASTC's bit layout without approximating anything.
#[cfg(feature = "block-formats")]
pub fn decode_astc(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, UastcError> {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    if data.len() < blocks_x * blocks_y * BLOCK_SIZE {
        return Err(UastcError::Truncated(data.len()));
    }

    let mut blocks = vec![0u8; blocks_x * blocks_y * 16];
    for index in 0..blocks_x * blocks_y {
        let block = &data[index * BLOCK_SIZE..index * BLOCK_SIZE + BLOCK_SIZE];
        let mut unpacked = unpack_block(block, index)?;
        apply_blue_contract(&mut unpacked);
        blocks[index * 16..index * 16 + 16]
            .copy_from_slice(&crate::uastc_to_astc::convert(&unpacked));
    }
    Ok(blocks)
}

/// Transcode a whole mip level into BC7 blocks, sixteen bytes each.
///
/// BC7 is the one target UASTC reaches without loss worth naming: the format
/// was drawn up so its modes land on BC7's, so this restates each block rather
/// than searching for a nearest one.
#[cfg(feature = "block-formats")]
pub fn decode_bc7(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, UastcError> {
    let blocks_x = width.div_ceil(4) as usize;
    let blocks_y = height.div_ceil(4) as usize;
    if data.len() < blocks_x * blocks_y * BLOCK_SIZE {
        return Err(UastcError::Truncated(data.len()));
    }

    let converter = crate::uastc_to_bc7::Bc7Converter::new();
    let mut blocks = vec![0u8; blocks_x * blocks_y * 16];
    for index in 0..blocks_x * blocks_y {
        let block = &data[index * BLOCK_SIZE..index * BLOCK_SIZE + BLOCK_SIZE];
        let unpacked = unpack_block(block, index)?;
        blocks[index * 16..index * 16 + 16]
            .copy_from_slice(&converter.convert(&unpacked).to_bytes());
    }
    Ok(blocks)
}

/// Read a bit field out of the block, least significant bit first.
fn read_bits(block: &[u8], offset: &mut usize, count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    let mut value = 0u32;
    for bit in 0..count as usize {
        let at = *offset + bit;
        value |= (((block[at >> 3] >> (at & 7)) & 1) as u32) << bit;
    }
    *offset += count as usize;
    value
}

/// One block read out of its 128 bits, before anything is done with it.
///
/// The same description serves both consumers: pixels are it interpolated,
/// and a BC7 block is it restated in BC7's own terms. Keeping them one step
/// apart is what stops the second from re-reading the bits its own way.
#[derive(Debug, Clone)]
pub(crate) struct Unpacked {
    pub mode: usize,
    pub common_pattern: u32,
    pub solid_color: [u8; 4],
    /// Endpoint values as stored, still quantized to the mode's range.
    pub endpoints: Vec<u8>,
    /// One weight per texel, or two per texel for a dual-plane mode.
    pub weights: [u8; 32],
    /// Dual-plane modes: which component the second plane interpolates.
    pub component_selector: usize,
    /// Which subset each texel belongs to.
    pub partition: &'static [u8; 16],
}

/// Decode one 128-bit block into sixteen texels.
fn decode_block(block: &[u8], index: usize, texels: &mut [[u8; 4]; 16]) -> Result<(), UastcError> {
    let unpacked = unpack_block(block, index)?;
    write_unpacked(&unpacked, texels);
    Ok(())
}

/// Write one unpacked block's sixteen texels.
fn write_unpacked(unpacked: &Unpacked, texels: &mut [[u8; 4]; 16]) {
    if unpacked.mode == MODE_SOLID_COLOR {
        texels.fill(unpacked.solid_color);
        return;
    }
    write_texels(
        MODE_SUBSETS[unpacked.mode] as usize,
        MODE_COMPONENTS[unpacked.mode] as usize,
        MODE_ENDPOINT_RANGES[unpacked.mode] as usize,
        planes_of(unpacked.mode),
        unpacked.component_selector,
        MODE_WEIGHT_BITS[unpacked.mode] as u32,
        &unpacked.endpoints,
        &unpacked.weights,
        unpacked.partition,
        texels,
    );
}

/// How many interpolation planes a mode uses.
pub(crate) fn planes_of(mode: usize) -> usize {
    match mode {
        6 | 11 | 13 | 17 => 2,
        _ => 1,
    }
}

/// Read one 128-bit block into its description.
pub(crate) fn unpack_block(block: &[u8], index: usize) -> Result<Unpacked, UastcError> {
    let mode = HUFF_MODES[(block[0] & 127) as usize] as usize;
    if mode >= TOTAL_MODES {
        return Err(UastcError::BadMode {
            index,
            mode: mode as u8,
        });
    }
    let mut offset = MODE_CODE_BITS[mode] as usize;

    if mode == MODE_SOLID_COLOR {
        return Ok(Unpacked {
            mode,
            common_pattern: 0,
            solid_color: [
                read_bits(block, &mut offset, 8) as u8,
                read_bits(block, &mut offset, 8) as u8,
                read_bits(block, &mut offset, 8) as u8,
                read_bits(block, &mut offset, 8) as u8,
            ],
            endpoints: Vec::new(),
            weights: [0; 32],
            component_selector: 0,
            partition: &ZERO_PATTERN,
        });
    }

    // Hints for encoders targeting BC1 and ETC. Nothing here reads them.
    offset += MODE_HINT_BITS[mode] as usize;

    let subsets = MODE_SUBSETS[mode] as usize;
    let common_pattern = match mode {
        2 | 4 | 7 | 9 | 16 => read_bits(block, &mut offset, 5),
        3 => read_bits(block, &mut offset, 4),
        _ => 0,
    };

    let (partition, anchors): (&[u8; 16], &[u8; 3]) = match subsets {
        3 => {
            let at = common_pattern as usize;
            if at >= ASTC_BC7_PATTERNS3.len() {
                return Err(UastcError::BadPattern {
                    index,
                    pattern: common_pattern,
                });
            }
            (&ASTC_BC7_PATTERNS3[at], &ASTC_BC7_PATTERN3_ANCHORS[at])
        }
        2 if mode == 7 => {
            let at = common_pattern as usize;
            if at >= BC7_3_ASTC2_PATTERNS2.len() {
                return Err(UastcError::BadPattern {
                    index,
                    pattern: common_pattern,
                });
            }
            (
                &BC7_3_ASTC2_PATTERNS2[at],
                &BC7_3_ASTC2_PATTERNS2_ANCHORS[at],
            )
        }
        2 => {
            let at = common_pattern as usize;
            if at >= ASTC_BC7_PATTERNS2.len() {
                return Err(UastcError::BadPattern {
                    index,
                    pattern: common_pattern,
                });
            }
            (&ASTC_BC7_PATTERNS2[at], &ASTC_BC7_PATTERN2_ANCHORS[at])
        }
        _ => (&ZERO_PATTERN, &[0, 0, 0]),
    };

    // A dual-plane mode interpolates one component separately from the other
    // three, which is how it keeps a sharp alpha over a smooth colour.
    let (planes, component_selector) = match mode {
        6 | 11 | 13 => (2usize, read_bits(block, &mut offset, 2) as usize),
        17 => (2usize, 3usize),
        _ => (1usize, 0usize),
    };

    let components = MODE_COMPONENTS[mode] as usize;
    let weight_bits = MODE_WEIGHT_BITS[mode] as u32;
    let endpoint_range = MODE_ENDPOINT_RANGES[mode] as usize;
    let total_values = components * 2 * subsets;

    let endpoints = read_endpoints(block, &mut offset, total_values, endpoint_range);
    let weights = read_weights(block, offset, mode, weight_bits, planes, subsets, anchors);

    Ok(Unpacked {
        mode,
        common_pattern,
        solid_color: [0; 4],
        endpoints,
        weights,
        component_selector,
        partition,
    })
}

/// Read the endpoint values, undoing the integer sequence bundling.
///
/// A range whose level count is not a power of two spends most of each value
/// in plain bits and packs the remainder in base 3 or base 5, five or three
/// values to a bundle. The bundle is read first, then peeled one digit per
/// value — which is why this cannot be a simple bit field read.
fn read_endpoints(block: &[u8], offset: &mut usize, total_values: usize, range: usize) -> Vec<u8> {
    let bits = BISE_RANGES[range][0] as u32;
    let trits = BISE_RANGES[range][1] != 0;
    let quints = BISE_RANGES[range][2] != 0;

    let (total_bundles, bundle_size, base) = if trits {
        (total_values.div_ceil(5), 5usize, 3u32)
    } else if quints {
        (total_values.div_ceil(3), 3usize, 5u32)
    } else {
        (0, 0, 0)
    };

    let mut bundles = [0u32; 8];
    for (bundle, slot) in bundles.iter_mut().enumerate().take(total_bundles) {
        // The last bundle carries only as many digits as remain, so it is
        // written in the fewest bits that can hold them.
        let count = if bundle + 1 == total_bundles {
            let remaining = total_values - (total_bundles - 1) * bundle_size;
            if trits {
                match remaining {
                    1 => 2,
                    2 => 4,
                    3 => 5,
                    4 => 7,
                    _ => 8,
                }
            } else {
                match remaining {
                    1 => 3,
                    2 => 5,
                    _ => 7,
                }
            }
        } else if trits {
            8
        } else {
            7
        };
        *slot = read_bits(block, offset, count);
    }

    let mut values = Vec::with_capacity(total_values);
    let mut accumulator = 0u32;
    let mut remaining = 0usize;
    let mut next_bundle = 0usize;
    for _ in 0..total_values {
        let mut value = read_bits(block, offset, bits);
        if total_bundles != 0 {
            if remaining == 0 {
                accumulator = bundles[next_bundle];
                next_bundle += 1;
                remaining = bundle_size;
            }
            let digit = accumulator % base;
            accumulator /= base;
            remaining -= 1;
            value |= digit << bits;
        }
        values.push(value as u8);
    }
    values
}

/// Read the interpolation weights, one per texel per plane.
///
/// Each subset's first texel — its anchor — stores one bit fewer than the
/// rest. That is not a saving trick: fixing the anchor's high bit to zero is
/// what removes the ambiguity between swapping a subset's two endpoints and
/// inverting all of its weights.
fn read_weights(
    block: &[u8],
    mut offset: usize,
    mode: usize,
    weight_bits: u32,
    planes: usize,
    subsets: usize,
    anchors: &[u8; 3],
) -> [u8; 32] {
    let mut weights = [0u8; 32];
    if mode == 18 {
        // The one mode whose weights do not fit in 64 bits.
        for (texel, weight) in weights.iter_mut().enumerate().take(16) {
            let count = if texel == 0 {
                weight_bits - 1
            } else {
                weight_bits
            };
            *weight = read_bits(block, &mut offset, count) as u8;
        }
        return weights;
    }

    if planes == 2 {
        // Dual plane modes have one subset, and both of the first texel's
        // weights are anchors.
        weights[0] = read_bits(block, &mut offset, weight_bits - 1) as u8;
        weights[1] = read_bits(block, &mut offset, weight_bits - 1) as u8;
        for weight in weights.iter_mut().take(32).skip(2) {
            *weight = read_bits(block, &mut offset, weight_bits) as u8;
        }
        return weights;
    }

    if subsets == 1 {
        weights[0] = read_bits(block, &mut offset, weight_bits - 1) as u8;
        for weight in weights.iter_mut().take(16).skip(1) {
            *weight = read_bits(block, &mut offset, weight_bits) as u8;
        }
        return weights;
    }

    for (texel, weight) in weights.iter_mut().enumerate().take(16) {
        let is_anchor = anchors.iter().any(|anchor| *anchor as usize == texel);
        let count = if is_anchor {
            weight_bits - 1
        } else {
            weight_bits
        };
        *weight = read_bits(block, &mut offset, count) as u8;
    }
    weights
}

/// Turn any subset whose endpoints run dark-to-light the other way round.
///
/// ASTC reads a subset whose low endpoint sums higher than its high one as
/// "blue contract", a mode that rescales the colour rather than interpolating
/// it plainly. UASTC does not mean that, so before a block is written out as
/// ASTC any such subset is restated the other way: its endpoints swap and its
/// weights invert, which names the same colours without tripping the rule.
///
/// Only ASTC needs this. Decoding to pixels or to BC7 interprets the endpoints
/// directly, where the order carries no such meaning.
#[cfg(feature = "block-formats")]
pub(crate) fn apply_blue_contract(unpacked: &mut Unpacked) {
    let mode = unpacked.mode;
    let components = MODE_COMPONENTS[mode] as usize;
    if mode == MODE_SOLID_COLOR || components < 3 {
        return;
    }
    let range = MODE_ENDPOINT_RANGES[mode] as usize;
    let subsets = MODE_SUBSETS[mode] as usize;

    let mut inverted = [false; 3];
    let mut any = false;
    for (subset, flag) in inverted.iter_mut().enumerate().take(subsets) {
        let base = subset * components * 2;
        let sum = |offset: usize| -> u32 {
            (0..3)
                .map(|channel| {
                    unquantize(
                        unpacked.endpoints[base + channel * 2 + offset] as u32,
                        range,
                    )
                })
                .sum()
        };
        if sum(1) < sum(0) {
            for channel in 0..components {
                unpacked
                    .endpoints
                    .swap(base + channel * 2, base + channel * 2 + 1);
            }
            *flag = true;
            any = true;
        }
    }
    if !any {
        return;
    }

    let planes = planes_of(mode);
    let mask = (1u8 << MODE_WEIGHT_BITS[mode]) - 1;
    for texel in 0..16 {
        if !inverted[unpacked.partition[texel] as usize] {
            continue;
        }
        unpacked.weights[texel * planes] = mask - unpacked.weights[texel * planes];
        if planes == 2 {
            unpacked.weights[texel * planes + 1] = mask - unpacked.weights[texel * planes + 1];
        }
    }
}

/// The bits, trits and quints of one ASTC quantization range.
pub(crate) fn bise_range(range: usize) -> (u32, bool, bool) {
    (
        BISE_RANGES[range][0] as u32,
        BISE_RANGES[range][1] != 0,
        BISE_RANGES[range][2] != 0,
    )
}

/// Undo one endpoint value's quantization, per the ASTC specification.
pub(crate) fn unquantize(value: u32, range: usize) -> u32 {
    let bits = BISE_RANGES[range][0] as u32;
    let trits = BISE_RANGES[range][1] != 0;
    let quints = BISE_RANGES[range][2] != 0;

    if !trits && !quints {
        // A plain bit field is stretched to eight bits by repeating it.
        let mut result = 0u32;
        let mut left = 8i32;
        while left > 0 {
            let take = left.min(bits as i32);
            let mut part = value;
            if take < bits as i32 {
                part >>= bits as i32 - take;
            }
            result |= part << (left - take);
            left -= take;
        }
        return result;
    }

    let packed_bits = value & ((1 << bits) - 1);
    let digit = value >> bits;
    let (pattern, c) = UNQUANT_PARAMS[range];

    let a = if packed_bits & 1 != 0 { 511 } else { 0 };
    let mut b = 0u32;
    for letter in pattern.iter() {
        b <<= 1;
        if *letter != b'0' {
            b |= (packed_bits >> (*letter - b'a')) & 1;
        }
    }
    let mut result = digit * c + b;
    result ^= a;
    (a & 0x80) | (result >> 2)
}

/// Interpolate between two endpoint values at one weight.
fn interpolate(low: u32, high: u32, weight: u32) -> u8 {
    // Each eight-bit endpoint is widened to sixteen by repeating it, so the
    // interpolation happens at higher precision than the result needs.
    let (low, high) = ((low << 8) | low, (high << 8) | high);
    (((low * (64 - weight) + high * weight + 32) >> 6) >> 8) as u8
}

/// Build each subset's colour ramp and write the texels through it.
#[allow(clippy::too_many_arguments)]
fn write_texels(
    subsets: usize,
    components: usize,
    endpoint_range: usize,
    planes: usize,
    component_selector: usize,
    weight_bits: u32,
    endpoints: &[u8],
    weights: &[u8; 32],
    partition: &[u8; 16],
    texels: &mut [[u8; 4]; 16],
) {
    let weight_table: &[u32] = match weight_bits {
        1 => &WEIGHTS_1,
        2 => &WEIGHTS_2,
        3 => &WEIGHTS_3,
        4 => &WEIGHTS_4,
        _ => &WEIGHTS_5,
    };
    let levels = 1usize << weight_bits;
    let components = components.min(4);

    // Every colour a texel of this block can take: per subset, per weight.
    let mut ramps = [[[0u8; 4]; 32]; 3];
    for (subset, ramp) in ramps.iter_mut().enumerate().take(subsets) {
        let base = subset * components * 2;
        let mut low = [0u32; 4];
        let mut high = [0u32; 4];
        if components == 2 {
            // A two-component mode is luminance and alpha: the one value fills
            // red, green and blue.
            let luma = (
                unquantize(endpoints[base] as u32, endpoint_range),
                unquantize(endpoints[base + 1] as u32, endpoint_range),
            );
            let alpha = (
                unquantize(endpoints[base + 2] as u32, endpoint_range),
                unquantize(endpoints[base + 3] as u32, endpoint_range),
            );
            low = [luma.0, luma.0, luma.0, alpha.0];
            high = [luma.1, luma.1, luma.1, alpha.1];
        } else {
            for component in 0..components {
                low[component] = unquantize(endpoints[base + component * 2] as u32, endpoint_range);
                high[component] =
                    unquantize(endpoints[base + component * 2 + 1] as u32, endpoint_range);
            }
            for component in components..4 {
                low[component] = 255;
                high[component] = 255;
            }
        }
        for (level, color) in ramp.iter_mut().enumerate().take(levels) {
            for component in 0..4 {
                // A component the mode does not carry was set to 255 at both
                // ends above, and interpolating between them lands back on 255.
                color[component] =
                    interpolate(low[component], high[component], weight_table[level]);
            }
        }
    }

    if planes == 1 {
        for (texel, color) in texels.iter_mut().enumerate() {
            let subset = if subsets == 1 {
                0
            } else {
                partition[texel] as usize
            };
            *color = ramps[subset][weights[texel] as usize];
        }
        return;
    }

    // Dual plane: the selected component follows the second weight.
    for (texel, color) in texels.iter_mut().enumerate() {
        let first = ramps[0][weights[texel * 2] as usize];
        let second = ramps[0][weights[texel * 2 + 1] as usize];
        for component in 0..4 {
            color[component] = if component == component_selector {
                second[component]
            } else {
                first[component]
            };
        }
    }
}

/// Which mode a block names, for tools that want to know without decoding.
pub fn mode_of(block: &[u8]) -> u8 {
    HUFF_MODES[(block[0] & 127) as usize]
}
