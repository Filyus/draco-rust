//! Turning a UASTC block into the ETC1 and ETC2 blocks a phone samples.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `transcode_uastc_to_etc1`, `transcode_uastc_to_etc2_eac_a8`,
//! `etc1_determine_selectors` and `apply_etc1_bias` in
//! `transcoder/basisu_transcoder.cpp`.
//!
//! Unlike the ASTC and BC7 paths, this one is not a rewrite of the same block
//! in another layout — ETC1 describes colour in a way UASTC does not, with two
//! subblocks sharing one base colour and a delta. What makes it cheap anyway
//! is that the encoder already solved it: a UASTC block carries ETC1 hints in
//! its header — the flip and differential bits, both intensity tables, a bias,
//! and for ETC2 the alpha table and multiplier — so nothing here searches for
//! an encoding. It reads the answer, then places the selectors by measuring
//! the decoded texels against the four colours that answer implies.
//!
//! Those hint bits are the ones a decode to pixels skips.

use crate::uastc::{Unpacked, MODE_HAS_ALPHA, MODE_HAS_ETC1_BIAS, MODE_SOLID_COLOR};

/// The four selector bytes of a single-colour ETC1 block, per hinted selector.
const SOLID_SELECTORS: [[u8; 4]; 4] = [
    [255, 255, 255, 255],
    [255, 255, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 255, 255],
];

/// Where each of a subblock's eight texels sits, indexed `[flip][subblock]`.
const PIXEL_COORDS: [[[(u8, u8); 8]; 2]; 2] = [
    [
        [
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3),
        ],
        [
            (2, 0),
            (2, 1),
            (2, 2),
            (2, 3),
            (3, 0),
            (3, 1),
            (3, 2),
            (3, 3),
        ],
    ],
    [
        [
            (0, 0),
            (1, 0),
            (2, 0),
            (3, 0),
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
        ],
        [
            (0, 2),
            (1, 2),
            (2, 2),
            (3, 2),
            (0, 3),
            (1, 3),
            (2, 3),
            (3, 3),
        ],
    ],
];

/// The four modifiers each ETC1 intensity table applies to a base colour.
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

/// How far each of EAC's eight steps moves from the base value.
const EAC_MODIFIERS: [[i8; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// Which step of a table is its lowest and which its highest.
const EAC_MIN_SELECTOR: usize = 3;
const EAC_MAX_SELECTOR: usize = 7;

/// The selector bits of a constant-alpha EAC block: every texel takes step 4.
const EAC_CONSTANT_SELECTORS: [u8; 6] = [0x92, 0x49, 0x24, 0x92, 0x49, 0x24];

/// The smallest and largest delta ETC1's differential mode can encode.
const DELTA_MIN: i32 = -4;
const DELTA_MAX: i32 = 3;

/// Read `count` bits at `offset`, counting from the block's last byte.
fn get_bits(bytes: &[u8; 8], offset: u32, count: u32) -> u32 {
    let byte = 7 - (offset >> 3) as usize;
    ((bytes[byte] as u32) >> (offset & 7)) & ((1 << count) - 1)
}

/// The four colours one subblock of a written ETC1 block resolves to.
///
/// Read back out of the bytes rather than kept from where they were computed:
/// the differential form clamps a delta that does not fit, so what the block
/// says is not always what was asked of it, and the selectors have to be
/// chosen against what it says.
fn block_colors(bytes: &[u8; 8], subblock: usize) -> [[u8; 3]; 4] {
    let diff = bytes[3] & 2 != 0;
    let base: [i32; 3] = if diff {
        let five = [
            get_bits(bytes, 59, 5) as i32,
            get_bits(bytes, 51, 5) as i32,
            get_bits(bytes, 43, 5) as i32,
        ];
        let mut value = five;
        if subblock != 0 {
            let deltas = [
                get_bits(bytes, 56, 3) as i32,
                get_bits(bytes, 48, 3) as i32,
                get_bits(bytes, 40, 3) as i32,
            ];
            for channel in 0..3 {
                let delta = if deltas[channel] >= 4 {
                    deltas[channel] - 8
                } else {
                    deltas[channel]
                };
                value[channel] = (five[channel] + delta).clamp(0, 31);
            }
        }
        [
            (value[0] << 3) | (value[0] >> 2),
            (value[1] << 3) | (value[1] >> 2),
            (value[2] << 3) | (value[2] >> 2),
        ]
    } else {
        let offsets = if subblock != 0 {
            [56u32, 48, 40]
        } else {
            [60, 52, 44]
        };
        let mut value = [0i32; 3];
        for channel in 0..3 {
            let four = get_bits(bytes, offsets[channel], 4) as i32;
            value[channel] = (four << 4) | four;
        }
        value
    };

    let table = INTEN_TABLES[((bytes[3] >> if subblock != 0 { 2 } else { 5 }) & 7) as usize];
    let mut colors = [[0u8; 3]; 4];
    for (color, modifier) in colors.iter_mut().zip(table.iter()) {
        for (channel, value) in color.iter_mut().zip(base.iter()) {
            *channel = (value + modifier).clamp(0, 255) as u8;
        }
    }
    colors
}

/// Nudge a subblock's base colour by the hinted bias.
///
/// The encoder found that moving one channel of one subblock by one step gives
/// a better block, and says so in five bits. Most values name a single channel
/// and direction outright; the rest are read as three base-three digits.
fn apply_bias(color: [u8; 3], bias: u32, limit: i32, subblock: usize) -> [u8; 3] {
    const DIVS: [u32; 3] = [1, 3, 9];
    let mut result = [0u8; 3];
    for channel in 0..3usize {
        let second = subblock != 0;
        // Twelve of the values move one channel of one subblock by one step.
        const SINGLE: [(u32, bool, usize, i32); 12] = [
            (2, false, 0, -1),
            (5, false, 1, -1),
            (6, false, 2, -1),
            (7, false, 0, 1),
            (11, false, 1, 1),
            (15, false, 2, 1),
            (18, true, 0, -1),
            (19, true, 1, -1),
            (20, true, 2, -1),
            (21, true, 0, 1),
            (24, true, 1, 1),
            (8, true, 2, 1),
        ];
        let delta: i32 = match SINGLE.iter().find(|(value, ..)| *value == bias) {
            Some(&(_, want_second, want_channel, step)) => {
                i32::from(second == want_second && channel == want_channel) * step
            }
            // Six move whole subblocks, and everything else is read as three
            // base-three digits, one per channel.
            None => match bias {
                10 => -2,
                27 => -i32::from(!second),
                28 => {
                    if second {
                        -1
                    } else {
                        1
                    }
                }
                29 => i32::from(second),
                30 => -i32::from(second),
                31 => i32::from(!second),
                _ => ((bias / DIVS[channel]) % 3) as i32 - 1,
            },
        };

        let mut value = color[channel] as i32;
        if value == 0 {
            value += if delta == -2 { 3 } else { delta + 1 };
        } else if value == limit {
            value += delta - 1;
        } else {
            value += delta;
            if value < 0 || value > limit {
                value = (value - delta) - delta;
            }
        }
        result[channel] = value as u8;
    }
    result
}

/// Choose each texel's selector by luma against the subblock's four colours.
fn determine_selectors(bytes: &mut [u8; 8], pixels: &[[u8; 4]; 16]) {
    const TRAN: [u8; 4] = [1, 0, 2, 3];
    let flip = bytes[3] & 1 != 0;

    let mut low = 0u16;
    let mut high = 0u16;
    for subblock in 0..2usize {
        let colors = block_colors(bytes, subblock);
        let luma: [u32; 4] = std::array::from_fn(|index| {
            colors[index][0] as u32 * 54
                + colors[index][1] as u32 * 183
                + colors[index][2] as u32 * 19
        });
        let (y01, y12, y23) = (luma[0] + luma[1], luma[1] + luma[2], luma[2] + luma[3]);

        let mut place = |texel: [u8; 4], at: u32| {
            let value = texel[0] as u32 * 108 + texel[1] as u32 * 366 + texel[2] as u32 * 38;
            let step = TRAN[(u32::from(value < y01)
                + u32::from(value < y12)
                + u32::from(value < y23)) as usize];
            low |= ((step & 1) as u16) << at;
            high |= ((step >> 1) as u16) << at;
        };

        if flip {
            let mut at = (subblock * 2) as i32;
            for y in 0..2usize {
                for x in 0..4usize {
                    place(pixels[x + (subblock * 2 + y) * 4], at as u32);
                    at += 4;
                }
                at = at + 1 - 4 * 4;
            }
        } else {
            let mut at = (subblock * 2 * 4) as u32;
            for x in 0..2usize {
                for y in 0..4usize {
                    place(pixels[subblock * 2 + x + y * 4], at);
                    at += 1;
                }
            }
        }
    }

    bytes[7] = low as u8;
    bytes[6] = (low >> 8) as u8;
    bytes[5] = high as u8;
    bytes[4] = (high >> 8) as u8;
}

/// Restate one unpacked UASTC block as an ETC1 block.
pub(crate) fn convert_etc1(unpacked: &Unpacked, pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    let hints = &unpacked.etc_hints;
    let mut bytes = [0u8; 8];

    if unpacked.mode == MODE_SOLID_COLOR {
        bytes[3] = (u8::from(hints.diff) << 1) | (hints.inten0 << 5) | (hints.inten0 << 2);
        if hints.diff {
            bytes[0] = hints.red << 3;
            bytes[1] = hints.green << 3;
            bytes[2] = hints.blue << 3;
        } else {
            bytes[0] = hints.red | (hints.red << 4);
            bytes[1] = hints.green | (hints.green << 4);
            bytes[2] = hints.blue | (hints.blue << 4);
        }
        bytes[4..8].copy_from_slice(&SOLID_SELECTORS[(hints.selector & 3) as usize]);
        return bytes;
    }

    bytes[3] = u8::from(hints.flip)
        | (u8::from(hints.diff) << 1)
        | (hints.inten0 << 5)
        | (hints.inten1 << 2);

    // Differential mode keeps five bits per channel, absolute mode four.
    let limit: i32 = if hints.diff { 31 } else { 15 };
    let mut colors = [[0u8; 3]; 2];
    for (subblock, color) in colors.iter_mut().enumerate() {
        let mut sums = [0u32; 3];
        for (x, y) in PIXEL_COORDS[usize::from(hints.flip)][subblock] {
            let texel = pixels[y as usize * 4 + x as usize];
            for (sum, channel) in sums.iter_mut().zip(texel.iter()) {
                *sum += *channel as u32;
            }
        }
        for (channel, sum) in color.iter_mut().zip(sums.iter()) {
            *channel = ((sum * limit as u32 + 1020) / (8 * 255)) as u8;
        }
        if MODE_HAS_ETC1_BIAS[unpacked.mode] {
            *color = apply_bias(*color, hints.bias as u32, limit, subblock);
        }
    }

    if hints.diff {
        for channel in 0..3usize {
            let mut delta =
                (colors[1][channel] as i32 - colors[0][channel] as i32).clamp(DELTA_MIN, DELTA_MAX);
            if delta < 0 {
                delta += 8;
            }
            bytes[channel] = ((colors[0][channel] as i32) << 3) as u8 | delta as u8;
        }
    } else {
        for channel in 0..3usize {
            bytes[channel] = colors[1][channel] | (colors[0][channel] << 4);
        }
    }

    determine_selectors(&mut bytes, pixels);
    bytes
}

/// The EAC alpha block that goes in front of the ETC1 half of ETC2 RGBA.
pub(crate) fn convert_eac_alpha(unpacked: &Unpacked, pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    let constant = |base: u8| -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0] = base;
        // Table 13 at multiplier one with every texel on the middle step
        // reproduces the base exactly.
        bytes[1] = (1 << 4) | 13;
        bytes[2..8].copy_from_slice(&EAC_CONSTANT_SELECTORS);
        bytes
    };

    if !MODE_HAS_ALPHA[unpacked.mode] || unpacked.mode == MODE_SOLID_COLOR {
        return constant(if unpacked.mode == MODE_SOLID_COLOR {
            unpacked.solid_color[3]
        } else {
            255
        });
    }

    let lowest = pixels.iter().map(|texel| texel[3]).min().unwrap_or(0);
    let highest = pixels.iter().map(|texel| texel[3]).max().unwrap_or(0);
    if lowest == highest {
        return constant(lowest);
    }

    // The encoder chose the table and multiplier; only the base and the
    // selectors are left, and the base is where the two extremes land on the
    // table's own range.
    let table = (unpacked.etc_hints.etc2 & 0xf) as usize;
    let multiplier = (unpacked.etc_hints.etc2 >> 4) as i32;
    let modifiers = EAC_MODIFIERS[table];
    let span = (modifiers[EAC_MAX_SELECTOR] - modifiers[EAC_MIN_SELECTOR]) as f32;
    let fraction = (0 - modifiers[EAC_MIN_SELECTOR] as i32) as f32 / span;
    let center = (lowest as f32 + (highest as f32 - lowest as f32) * fraction).round() as i32;

    let values: [i32; 8] =
        std::array::from_fn(|step| (center + modifiers[step] as i32 * multiplier).clamp(0, 255));

    let mut bits = 0u64;
    for index in 0..16usize {
        // The reference walks its texels transposed here, which is what puts
        // each selector at the bit EAC reads it from.
        let alpha = pixels[(index & 3) * 4 + (index >> 2)][3] as i32;
        let best = (0..8usize)
            .map(|step| ((values[step] - alpha).unsigned_abs() << 3) | step as u32)
            .min()
            .unwrap_or(0)
            & 7;
        bits |= (best as u64) << (45 - index * 3);
    }

    let mut bytes = [0u8; 8];
    bytes[0] = center as u8;
    bytes[1] = ((multiplier as u8) << 4) | (table as u8 & 15);
    for (at, byte) in bytes[2..8].iter_mut().enumerate() {
        *byte = (bits >> (40 - at * 8)) as u8;
    }
    bytes
}
