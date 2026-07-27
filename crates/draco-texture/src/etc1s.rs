//! Basis Universal ETC1S: codebooks, slice decoding, and ETC1 colour.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `basisu_lowlevel_etc1s_transcoder::decode_palettes`, `decode_tables` and
//! `transcode_slice` in `transcoder/basisu_transcoder.cpp`, and the ETC1
//! reconstruction in `decoder_etc_block`.
//!
//! An ETC1S image is not stored block by block. Two codebooks — endpoint
//! colours and 4×4 selector patterns — live once per file in the
//! supercompression global data, and each mip level is a stream of indices
//! into them, predicted from the blocks to the left and above. That is why a
//! level cannot be decoded on its own and why the whole file has to be opened
//! before any one image can be.

use crate::huffman::{BitReader, HuffmanError, HuffmanTable};

/// The ETC1 intensity modifier table, indexed by table then selector.
const INTEN_TABLES: [[i32; 4]; 8] = [
    [-8, -2, 2, 8],
    [-17, -5, 5, 17],
    [-29, -9, 9, 29],
    [-42, -13, 13, 42],
    [-60, -18, 18, 60],
    [-80, -24, 24, 80],
    [-106, -33, 33, 106],
    [-183, -47, 47, 183],
];

/// Which delta model an endpoint channel uses, by its previous value.
const COLOR5_PAL0_PREV_HI: u8 = 9;
const COLOR5_PAL1_PREV_HI: u8 = 21;

/// One symbol per combination of four two-bit predictors, plus the repeat code.
const ENDPOINT_PRED_TOTAL_SYMBOLS: u32 = (4 * 4 * 4 * 4) + 1;
const ENDPOINT_PRED_REPEAT_LAST_SYMBOL: u32 = ENDPOINT_PRED_TOTAL_SYMBOLS - 1;
const ENDPOINT_PRED_MIN_REPEAT_COUNT: u32 = 3;
const ENDPOINT_PRED_COUNT_VLC_BITS: u32 = 4;

const SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH: u32 = 3;
const SELECTOR_HISTORY_BUF_RLE_COUNT_BITS: u32 = 6;
const SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL: u32 = 1 << SELECTOR_HISTORY_BUF_RLE_COUNT_BITS;

/// Anything that can stop an ETC1S image from decoding.
#[derive(Debug, thiserror::Error)]
pub enum Etc1sError {
    /// The entropy coding is damaged.
    #[error("ETC1S entropy coding: {0}")]
    Huffman(#[from] HuffmanError),
    /// The global data is too small for what its header claims.
    #[error("ETC1S global data is truncated")]
    Truncated,
    /// A header field holds a value the format does not allow.
    #[error("invalid ETC1S global data: {0}")]
    Invalid(&'static str),
    /// A stream index points outside its codebook.
    #[error("ETC1S stream references {kind} {index}, but only {count} exist")]
    OutOfRange {
        /// Which codebook.
        kind: &'static str,
        /// The index the stream asked for.
        index: u32,
        /// How many entries the codebook has.
        count: u32,
    },
    /// A block predicted from a neighbour that does not exist.
    #[error("ETC1S block at {x},{y} predicts from a neighbour outside the image")]
    BadPrediction {
        /// Block column.
        x: u32,
        /// Block row.
        y: u32,
    },
    /// The file uses a feature this decoder does not implement.
    #[error("unsupported ETC1S file: {0}")]
    Unsupported(&'static str),
}

/// One endpoint codebook entry: a 5:5:5 base colour and an intensity table.
#[derive(Debug, Clone, Copy, Default)]
struct Endpoint {
    color5: [u8; 3],
    inten5: u8,
}

/// One selector codebook entry: four rows of four two-bit selectors.
#[derive(Debug, Clone, Copy, Default)]
struct Selector {
    rows: [u8; 4],
}

/// Where one image's two slices live inside its mip level.
#[derive(Debug, Clone, Copy)]
pub struct ImageDesc {
    /// Byte offset of the colour slice, relative to the level.
    pub rgb_offset: u32,
    /// Byte length of the colour slice.
    pub rgb_length: u32,
    /// Byte offset of the alpha slice, relative to the level.
    pub alpha_offset: u32,
    /// Byte length of the alpha slice, zero when the file has no alpha.
    pub alpha_length: u32,
}

/// Byte size of the ETC1S global data header.
const GLOBAL_HEADER_SIZE: usize = 20;
/// Byte size of one per-image slice descriptor.
const IMAGE_DESC_SIZE: usize = 20;

/// Everything an ETC1S file shares across its images.
pub struct Etc1sDecoder {
    endpoints: Vec<Endpoint>,
    selectors: Vec<Selector>,
    image_descs: Vec<ImageDesc>,
    endpoint_pred_model: HuffmanTable,
    delta_endpoint_model: HuffmanTable,
    selector_model: HuffmanTable,
    selector_history_rle_model: HuffmanTable,
    selector_history_size: u32,
}

impl Etc1sDecoder {
    /// Read the codebooks and the per-image slice table out of the global data.
    ///
    /// `image_count` is levels × layers × faces, which is how many slice
    /// descriptors the file wrote.
    pub fn new(global_data: &[u8], image_count: usize) -> Result<Self, Etc1sError> {
        if global_data.len() < GLOBAL_HEADER_SIZE {
            return Err(Etc1sError::Truncated);
        }
        let short = |at: usize| u16::from_le_bytes(global_data[at..at + 2].try_into().unwrap());
        let word = |at: usize| u32::from_le_bytes(global_data[at..at + 4].try_into().unwrap());

        let endpoint_count = short(0) as usize;
        let selector_count = short(2) as usize;
        let endpoints_length = word(4) as usize;
        let selectors_length = word(8) as usize;
        let tables_length = word(12) as usize;

        if endpoint_count == 0 || selector_count == 0 {
            return Err(Etc1sError::Invalid("codebook is empty"));
        }
        if endpoints_length == 0 || selectors_length == 0 || tables_length == 0 {
            return Err(Etc1sError::Invalid("a codebook section is empty"));
        }

        let descs_end = GLOBAL_HEADER_SIZE + IMAGE_DESC_SIZE * image_count;
        let endpoints_end = descs_end
            .checked_add(endpoints_length)
            .ok_or(Etc1sError::Truncated)?;
        let selectors_end = endpoints_end
            .checked_add(selectors_length)
            .ok_or(Etc1sError::Truncated)?;
        let tables_end = selectors_end
            .checked_add(tables_length)
            .ok_or(Etc1sError::Truncated)?;
        if tables_end > global_data.len() {
            return Err(Etc1sError::Truncated);
        }

        let image_descs = (0..image_count)
            .map(|index| {
                let base = GLOBAL_HEADER_SIZE + index * IMAGE_DESC_SIZE;
                // The first word is per-image flags, which only video uses.
                ImageDesc {
                    rgb_offset: word(base + 4),
                    rgb_length: word(base + 8),
                    alpha_offset: word(base + 12),
                    alpha_length: word(base + 16),
                }
            })
            .collect();

        let mut decoder = Etc1sDecoder {
            endpoints: Vec::new(),
            selectors: Vec::new(),
            image_descs,
            endpoint_pred_model: HuffmanTable::default(),
            delta_endpoint_model: HuffmanTable::default(),
            selector_model: HuffmanTable::default(),
            selector_history_rle_model: HuffmanTable::default(),
            selector_history_size: 0,
        };
        decoder.decode_tables(&global_data[selectors_end..tables_end])?;
        decoder.decode_endpoints(&global_data[descs_end..endpoints_end], endpoint_count)?;
        decoder.decode_selectors(&global_data[endpoints_end..selectors_end], selector_count)?;
        Ok(decoder)
    }

    /// Where image `index` keeps its slices.
    pub fn image_desc(&self, index: usize) -> Option<ImageDesc> {
        self.image_descs.get(index).copied()
    }

    /// The four Huffman models and the history buffer size the slices use.
    fn decode_tables(&mut self, data: &[u8]) -> Result<(), Etc1sError> {
        let mut reader = BitReader::new(data);
        self.endpoint_pred_model = reader.read_huffman_table()?;
        self.delta_endpoint_model = reader.read_huffman_table()?;
        self.selector_model = reader.read_huffman_table()?;
        self.selector_history_rle_model = reader.read_huffman_table()?;
        if !self.endpoint_pred_model.is_valid()
            || !self.delta_endpoint_model.is_valid()
            || !self.selector_model.is_valid()
            || !self.selector_history_rle_model.is_valid()
        {
            return Err(Etc1sError::Invalid("a slice model is empty"));
        }
        self.selector_history_size = reader.get_bits(13);
        if self.selector_history_size == 0 {
            return Err(Etc1sError::Invalid("selector history buffer size is zero"));
        }
        Ok(())
    }

    /// Endpoints, each coded as a delta from the one before it.
    ///
    /// Which of the three colour models codes a channel depends on that
    /// channel's *previous* value, so the models stay matched to the range the
    /// delta is likely to fall in.
    fn decode_endpoints(&mut self, data: &[u8], count: usize) -> Result<(), Etc1sError> {
        let mut reader = BitReader::new(data);
        let color_model0 = reader.read_huffman_table()?;
        let color_model1 = reader.read_huffman_table()?;
        let color_model2 = reader.read_huffman_table()?;
        let inten_model = reader.read_huffman_table()?;
        if !color_model0.is_valid()
            || !color_model1.is_valid()
            || !color_model2.is_valid()
            || !inten_model.is_valid()
        {
            return Err(Etc1sError::Invalid("an endpoint model is empty"));
        }

        let grayscale = reader.get_bits(1) != 0;
        let mut previous_color = [16u8; 3];
        let mut previous_inten = 0u32;

        self.endpoints = Vec::with_capacity(count);
        for _ in 0..count {
            let inten_delta = reader.decode(&inten_model)?;
            let inten5 = ((inten_delta + previous_inten) & 7) as u8;
            previous_inten = inten5 as u32;

            let mut color5 = [0u8; 3];
            for channel in 0..if grayscale { 1 } else { 3 } {
                let delta = if previous_color[channel] <= COLOR5_PAL0_PREV_HI {
                    reader.decode(&color_model0)?
                } else if previous_color[channel] <= COLOR5_PAL1_PREV_HI {
                    reader.decode(&color_model1)?
                } else {
                    reader.decode(&color_model2)?
                };
                let value = ((previous_color[channel] as u32 + delta) & 31) as u8;
                color5[channel] = value;
                previous_color[channel] = value;
            }
            if grayscale {
                color5[1] = color5[0];
                color5[2] = color5[0];
            }
            self.endpoints.push(Endpoint { color5, inten5 });
        }
        Ok(())
    }

    /// Selector patterns, either raw or as a byte-wise xor from the one before.
    fn decode_selectors(&mut self, data: &[u8], count: usize) -> Result<(), Etc1sError> {
        let mut reader = BitReader::new(data);
        if reader.get_bits(1) == 1 {
            return Err(Etc1sError::Unsupported("global selector codebooks"));
        }
        if reader.get_bits(1) == 1 {
            return Err(Etc1sError::Unsupported("hybrid global selector codebooks"));
        }
        let raw = reader.get_bits(1) == 1;

        self.selectors = Vec::with_capacity(count);
        if raw {
            for _ in 0..count {
                let mut rows = [0u8; 4];
                for row in rows.iter_mut() {
                    *row = reader.get_bits(8) as u8;
                }
                self.selectors.push(Selector { rows });
            }
            return Ok(());
        }

        let delta_model = reader.read_huffman_table()?;
        if count > 1 && !delta_model.is_valid() {
            return Err(Etc1sError::Invalid("the selector delta model is empty"));
        }
        let mut previous = [0u8; 4];
        for index in 0..count {
            let mut rows = [0u8; 4];
            for (row, previous_row) in rows.iter_mut().zip(previous.iter_mut()) {
                let byte = if index == 0 {
                    reader.get_bits(8) as u8
                } else {
                    (reader.decode(&delta_model)? as u8) ^ *previous_row
                };
                *previous_row = byte;
                *row = byte;
            }
            self.selectors.push(Selector { rows });
        }
        Ok(())
    }

    /// Decode one image into RGBA8, `width * height * 4` bytes in raster order.
    ///
    /// The colour and the alpha halves are separate slices decoded by the same
    /// machinery: alpha is an ETC1S image in its own right whose green channel
    /// carries the value.
    pub fn decode_rgba(
        &self,
        level_data: &[u8],
        desc: ImageDesc,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, Etc1sError> {
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        let slice = |offset: u32, length: u32| -> Result<&[u8], Etc1sError> {
            let start = offset as usize;
            let end = start
                .checked_add(length as usize)
                .ok_or(Etc1sError::Truncated)?;
            level_data.get(start..end).ok_or(Etc1sError::Truncated)
        };

        // Alpha first, then colour: the colour pass leaves the alpha byte
        // alone when there is an alpha slice, and writes 255 when there is not.
        if desc.alpha_length != 0 {
            let data = slice(desc.alpha_offset, desc.alpha_length)?;
            self.walk_blocks(
                data,
                width,
                height,
                block_writer(width, height, &mut pixels, SliceKind::Alpha),
            )?;
        }
        let data = slice(desc.rgb_offset, desc.rgb_length)?;
        let kind = if desc.alpha_length != 0 {
            SliceKind::Color { opaque: false }
        } else {
            SliceKind::Color { opaque: true }
        };
        self.walk_blocks(
            data,
            width,
            height,
            block_writer(width, height, &mut pixels, kind),
        )?;
        Ok(pixels)
    }

    /// Decode one image's colour slice into BC1 blocks, eight bytes each.
    ///
    /// Blocks in raster order, which is the layout `compressedTexImage2D`
    /// wants. Alpha is not part of BC1: a file that has an alpha slice needs
    /// BC3, which pairs these blocks with an alpha block of its own.
    ///
    /// `three_color` is passed through to the block conversion, where it says
    /// whether BC1's punch-through mode may be used.
    #[cfg(feature = "block-formats")]
    pub fn decode_bc1(
        &self,
        level_data: &[u8],
        desc: ImageDesc,
        width: u32,
        height: u32,
        three_color: bool,
    ) -> Result<Vec<u8>, Etc1sError> {
        let converter = crate::etc1s_to_bc1::Bc1Converter::new();
        let blocks_x = width.div_ceil(4) as usize;
        let mut blocks = vec![0u8; blocks_x * height.div_ceil(4) as usize * 8];
        let start = desc.rgb_offset as usize;
        let end = start
            .checked_add(desc.rgb_length as usize)
            .ok_or(Etc1sError::Truncated)?;
        let data = level_data.get(start..end).ok_or(Etc1sError::Truncated)?;

        self.walk_blocks(
            data,
            width,
            height,
            |block_x, block_y, color5, inten5, selectors| {
                let at = (block_y as usize * blocks_x + block_x as usize) * 8;
                let block = converter.convert(color5, inten5, selectors, three_color);
                blocks[at..at + 8].copy_from_slice(&block.to_bytes());
            },
        )?;
        Ok(blocks)
    }

    /// Walk one slice's blocks, resolving each block's two codebook indices.
    ///
    /// What to do with a resolved block is the caller's: the same walk feeds
    /// pixels, BC1 blocks and anything added later. Only the walk is subtle -
    /// the prediction state, the selector history, the run lengths - and it
    /// exists once so a new output format cannot get it subtly different.
    fn walk_blocks(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        mut visit: impl FnMut(u32, u32, [u8; 3], u8, [u8; 4]),
    ) -> Result<(), Etc1sError> {
        let blocks_x = width.div_ceil(4);
        let blocks_y = height.div_ceil(4);
        let total_blocks = blocks_x * blocks_y;
        let endpoint_count = self.endpoints.len() as u32;
        let selector_count = self.selectors.len() as u32;

        let mut reader = BitReader::new(data);
        let mut history = MoveToFront::new(self.selector_history_size as usize);
        let mut selector_rle_count = 0u32;

        // Two rows of per-column state: the row being written and the one above
        // it, which is where the "upper" and "upper left" predictions read from.
        let mut preds = [
            vec![BlockPred::default(); blocks_x as usize],
            vec![BlockPred::default(); blocks_x as usize],
        ];
        let mut pred_bits = 0u32;
        let mut previous_pred_symbol = 0u32;
        let mut pred_repeat_count = 0u32;
        let mut previous_endpoint_index = 0u32;

        let history_first_symbol = selector_count;
        let history_rle_symbol = self.selector_history_size + selector_count;

        for block_y in 0..blocks_y {
            let current = (block_y & 1) as usize;
            for block_x in 0..blocks_x {
                // One symbol carries the predictors for a 2×2 group of blocks:
                // the low four bits for this row, the high four for the next.
                if block_x & 1 == 0 {
                    if block_y & 1 == 0 {
                        if pred_repeat_count > 0 {
                            pred_repeat_count -= 1;
                            pred_bits = previous_pred_symbol;
                        } else {
                            pred_bits = reader.decode(&self.endpoint_pred_model)?;
                            if pred_bits == ENDPOINT_PRED_REPEAT_LAST_SYMBOL {
                                pred_repeat_count = reader.decode_vlc(ENDPOINT_PRED_COUNT_VLC_BITS)
                                    + ENDPOINT_PRED_MIN_REPEAT_COUNT
                                    - 1;
                                pred_bits = previous_pred_symbol;
                            } else {
                                previous_pred_symbol = pred_bits;
                            }
                        }
                        preds[current ^ 1][block_x as usize].bits = (pred_bits >> 4) as u8;
                    } else {
                        pred_bits = preds[current][block_x as usize].bits as u32;
                    }
                }

                let prediction = pred_bits & 3;
                pred_bits >>= 2;

                let endpoint_index = match prediction {
                    // Left.
                    0 => {
                        if block_x == 0 {
                            return Err(Etc1sError::BadPrediction {
                                x: block_x,
                                y: block_y,
                            });
                        }
                        previous_endpoint_index
                    }
                    // Upper.
                    1 => {
                        if block_y == 0 {
                            return Err(Etc1sError::BadPrediction {
                                x: block_x,
                                y: block_y,
                            });
                        }
                        preds[current ^ 1][block_x as usize].endpoint_index as u32
                    }
                    // Upper left.
                    2 => {
                        if block_x == 0 || block_y == 0 {
                            return Err(Etc1sError::BadPrediction {
                                x: block_x,
                                y: block_y,
                            });
                        }
                        preds[current ^ 1][block_x as usize - 1].endpoint_index as u32
                    }
                    // Coded as a delta from the block before, wrapping once.
                    _ => {
                        let delta = reader.decode(&self.delta_endpoint_model)?;
                        let index = delta.wrapping_add(previous_endpoint_index);
                        if index >= endpoint_count {
                            index.wrapping_sub(endpoint_count)
                        } else {
                            index
                        }
                    }
                };
                preds[current][block_x as usize].endpoint_index = endpoint_index as u16;
                previous_endpoint_index = endpoint_index;

                // Selector indices past the codebook name a slot in the recent
                // history instead, which is what makes repeated patterns cheap.
                let selector_symbol = if selector_rle_count > 0 {
                    selector_rle_count -= 1;
                    selector_count
                } else {
                    let symbol = reader.decode(&self.selector_model)?;
                    if symbol == history_rle_symbol {
                        let run = reader.decode(&self.selector_history_rle_model)?;
                        selector_rle_count = if run == SELECTOR_HISTORY_BUF_RLE_COUNT_TOTAL - 1 {
                            reader.decode_vlc(7) + SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH
                        } else {
                            run + SELECTOR_HISTORY_BUF_RLE_COUNT_THRESH
                        };
                        if selector_rle_count > total_blocks {
                            return Err(Etc1sError::Invalid(
                                "selector run is longer than the image",
                            ));
                        }
                        selector_rle_count -= 1;
                        selector_count
                    } else {
                        symbol
                    }
                };

                let selector_index = if selector_symbol >= history_first_symbol {
                    let slot = (selector_symbol - history_first_symbol) as usize;
                    if slot >= history.len() {
                        return Err(Etc1sError::OutOfRange {
                            kind: "selector history slot",
                            index: slot as u32,
                            count: history.len() as u32,
                        });
                    }
                    let index = history.get(slot);
                    history.use_slot(slot);
                    index
                } else {
                    history.add(selector_symbol);
                    selector_symbol
                };

                if endpoint_index >= endpoint_count {
                    return Err(Etc1sError::OutOfRange {
                        kind: "endpoint",
                        index: endpoint_index,
                        count: endpoint_count,
                    });
                }
                if selector_index >= selector_count {
                    return Err(Etc1sError::OutOfRange {
                        kind: "selector",
                        index: selector_index,
                        count: selector_count,
                    });
                }

                let endpoint = self.endpoints[endpoint_index as usize];
                let selector = self.selectors[selector_index as usize];
                visit(
                    block_x,
                    block_y,
                    endpoint.color5,
                    endpoint.inten5,
                    selector.rows,
                );
            }
        }
        Ok(())
    }
}

/// Which half of the image a slice fills in.
#[derive(Debug, Clone, Copy)]
enum SliceKind {
    /// Colour. `opaque` writes 255 into alpha; otherwise alpha is left alone
    /// because a preceding alpha slice already wrote it.
    Color {
        /// Whether this slice owns the alpha byte.
        opaque: bool,
    },
    /// Alpha, taken from the block's green channel.
    Alpha,
}

/// Per-column state carried between the two most recent block rows.
#[derive(Debug, Clone, Copy, Default)]
struct BlockPred {
    bits: u8,
    endpoint_index: u16,
}

/// The reference's approximate move-to-front list.
///
/// Not a true move-to-front: using a slot swaps it with the one at half its
/// index, and adding overwrites in a rotating window over the upper half. The
/// approximation is part of the format, not an optimisation — the encoder made
/// the same one.
struct MoveToFront {
    values: Vec<u32>,
    rover: usize,
}

impl MoveToFront {
    fn new(size: usize) -> Self {
        MoveToFront {
            values: vec![0; size],
            rover: size / 2,
        }
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn get(&self, index: usize) -> u32 {
        self.values[index]
    }

    fn add(&mut self, value: u32) {
        self.values[self.rover] = value;
        self.rover += 1;
        if self.rover == self.values.len() {
            self.rover = self.values.len() / 2;
        }
    }

    fn use_slot(&mut self, index: usize) {
        if index != 0 {
            self.values.swap(index / 2, index);
        }
    }
}

/// A visitor that expands each ETC1S block into up to sixteen pixels.
///
/// The last block of a row or column hangs off the edge when the image is not
/// a multiple of four, and those pixels are simply not written.
fn block_writer(
    width: u32,
    height: u32,
    pixels: &mut [u8],
    kind: SliceKind,
) -> impl FnMut(u32, u32, [u8; 3], u8, [u8; 4]) + '_ {
    move |block_x, block_y, color5, inten5, selectors| {
        let max_x = 4.min(width.saturating_sub(block_x * 4));
        let max_y = 4.min(height.saturating_sub(block_y * 4));
        let colors = block_colors5(color5, inten5);
        for y in 0..max_y {
            let row = selectors[y as usize];
            let base = (((block_y * 4 + y) * width) + block_x * 4) as usize * 4;
            for x in 0..max_x {
                let color = colors[((row >> (x * 2)) & 3) as usize];
                let at = base + x as usize * 4;
                match kind {
                    SliceKind::Color { opaque } => {
                        pixels[at] = color[0];
                        pixels[at + 1] = color[1];
                        pixels[at + 2] = color[2];
                        if opaque {
                            pixels[at + 3] = 255;
                        }
                    }
                    // Alpha rides in the green channel of its own ETC1S image.
                    SliceKind::Alpha => pixels[at + 3] = color[1],
                }
            }
        }
    }
}

/// The four colours an ETC1S block can use: its base colour plus each modifier.
pub(crate) fn block_colors5(color5: [u8; 3], inten5: u8) -> [[u8; 3]; 4] {
    // 5 bits per channel expanded to 8 by repeating the top bits, which is what
    // ETC1 specifies rather than a multiply.
    let base = [
        ((color5[0] << 3) | (color5[0] >> 2)) as i32,
        ((color5[1] << 3) | (color5[1] >> 2)) as i32,
        ((color5[2] << 3) | (color5[2] >> 2)) as i32,
    ];
    let table = INTEN_TABLES[(inten5 & 7) as usize];
    let mut colors = [[0u8; 3]; 4];
    for (color, modifier) in colors.iter_mut().zip(table.iter()) {
        for (channel, value) in color.iter_mut().zip(base.iter()) {
            *channel = (value + modifier).clamp(0, 255) as u8;
        }
    }
    colors
}

/// The one colour a given selector picks out of the four.
pub(crate) fn block_color5(color5: [u8; 3], inten5: u8, selector: u8) -> [u8; 3] {
    block_colors5(color5, inten5)[(selector & 3) as usize]
}
