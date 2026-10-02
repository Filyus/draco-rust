//! The order a point cloud's points are written in.
//!
//! A point cloud's point order carries no meaning, and the sequential coder
//! writes every attribute as the difference between one point and the one
//! before it, so the order decides how large those differences are. Two ways of
//! choosing it live here:
//!
//! * [`curve`] strings the points along a Hilbert curve laid over the
//!   positions. Cheap, and purely spatial: neighbours in space are neighbours
//!   in the stream.
//! * [`search`] looks for the order that makes the stream small. It starts
//!   from the curve and repairs it where the other attributes disagree with
//!   the positions, because in a Gaussian splat the positions are 5% of the
//!   bytes and the other 56 numbers a point carries are the rest. It can also
//!   decide that the order it was handed is already better than anything it
//!   would write, and leave it alone.
//!
//! # What is minimized, and what decides
//!
//! The refinement minimizes the estimated bits of a step: `log2(1 + |delta|)`
//! summed over the position and over the attribute columns, each in the units
//! the encode quantizes it to. The coder spends about 1.8 to 2.9 times that on
//! a column, close enough to flat across columns that weighting them by what
//! they really cost bought under a third of a percent. Position goes in as 16
//! bits, however many the grid has, and an attribute as at most 8, so a column
//! quantized finer than that is read at its top eight bits.
//!
//! Which order is written is not decided on that estimate. It prices a step by
//! its size, and the coder by how steps are distributed: a step that is always
//! the same costs the coder next to nothing and the estimate its logarithm,
//! and a column that does not follow its neighbours, which the coder writes as
//! it is, the estimate still prices by its steps. The decision is taken on
//! `coder_bits` instead, the coder's own plan of each order's residuals.
//!
//! # How
//!
//! Only the columns that cost the most are counted (`Effort::columns`): they
//! carry most of the gain, and the cost of the search is proportional to how
//! many are. The path is cut into blocks of [`BLOCK`] points with their end
//! points held fixed, so a block is a function of itself alone and the result
//! is the same on any number of threads. Within a block each start looks at
//! the `window` points after it, takes the one whose reversal of the stretch in
//! between saves the most, and looks again. The candidates of one start are
//! priced together, one lane each, over columns stored apart so a lane is a
//! byte beside its neighbour: plain loops over fixed-size arrays that the
//! compiler turns into vector code, with no `unsafe` anywhere.

use crate::draco_types::DataType;
use crate::encoder_options::EncoderOptions;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{AttributeValueIndex, PointIndex};
use crate::parallel;
use crate::point_cloud::PointCloud;
use crate::sequential_attribute_encoder::{
    select_sequential_encoder, SequentialAttributeEncoderType,
};
use crate::symbol_encoding::plan_symbols_with_threads;

/// Points to a block of the path. A block is cache-sized and independent of its
/// neighbours, which is what lets blocks run on separate threads and is also
/// faster on one.
const BLOCK: usize = 8192;

/// Every how many steps the estimates are taken. Odd, so a regular pattern in
/// the input cannot line up with the sample.
const SAMPLE_STRIDE: usize = 17;

/// An order is written only if it undercuts, by the coder's price, by at least
/// this every order it would replace: the input, and for a refinement also the
/// curve it was refined from. The price is the coder's plan on a sample, which
/// ranks orders the way the stream does to within this; a refinement that
/// saves a terrestrial room scan 3% passes it, one that saves 0.3% is not worth
/// the risk of a sample that misjudged.
const WORTH_KEEPING_BELOW: f64 = 0.99;

/// One block in this many is refined first, as a trial: if what it comes to,
/// scaled to the whole, is not under both the input and the curve by
/// [`WORTH_KEEPING_BELOW`], the rest is not worth doing.
/// What a given effort makes of a given cloud is not something a threshold can
/// say, so the trial measures it. A cloud of fewer than four blocks has its
/// every block tried, and the trial is the whole refinement.
const TRIAL_EVERY: usize = 16;

/// How many values of an order `coder_bits` prices, over every component of
/// every attribute: each is two reads that miss the cache, so a larger cloud,
/// or one with more attributes, is sampled more sparsely rather than more.
const PRICED_VALUES: usize = 1 << 21;

/// The fewest steps a stride leaves, where an order has them: a histogram of
/// fewer residuals than this stops ranking orders as the whole would.
const PRICED_STEPS_AT_LEAST: usize = 1 << 15;

/// The stride that prices about [`PRICED_VALUES`] of `len` steps of
/// `components` values each, and never fewer than [`PRICED_STEPS_AT_LEAST`]
/// steps.
fn pricing_stride(len: usize, components: usize) -> usize {
    (len * components.max(1) / PRICED_VALUES).clamp(1, (len / PRICED_STEPS_AT_LEAST).max(1))
}

/// Positions are priced on this many bits an axis at most.
const POSITION_BITS: u32 = 16;

/// How hard `search` works, from the encoding speed.
#[derive(Clone, Copy)]
struct Effort {
    /// Attribute columns counted, the dearest first.
    columns: usize,
    /// Candidates a start looks at.
    window: usize,
    passes: usize,
}

/// `None` is the curve alone, with nothing refined.
fn effort(speed: i32) -> Option<Effort> {
    let (columns, window, passes) = match speed {
        9.. => return None,
        7..=8 => (8, 8, 1),
        5..=6 => (16, 16, 2),
        3..=4 => (32, 32, 2),
        _ => (64, 64, 2),
    };
    Some(Effort {
        columns,
        window,
        passes,
    })
}

// ---------------------------------------------------------------------------
// The curve
// ---------------------------------------------------------------------------

/// How finely the curve resolves each axis, from how finely the positions will
/// be stored.
///
/// The grid is not a free parameter. Coarser than the quantization and
/// distinct points share a cell, where their order is whatever the sort left
/// them in rather than anything spatial: at ten bits an axis that was 86% of
/// the points of a million-point splat, seven to a cell. Finer than the
/// quantization and the order sorts by differences the encode then discards,
/// which measurably buys nothing.
///
/// What a fixed grid costs depends entirely on how crowded its cells get, so
/// it is worth 5% of that splat and 1% of two interiors whose points fill
/// their bounding box. Following the quantization is never the worse of the
/// two, which is the reason to do it; the size of the win is the scene's.
///
/// Twenty-one bits an axis is the ceiling either way, being what still
/// interleaves into a `u64` key.
pub(crate) fn curve_axis_bits(options: &EncoderOptions, att_id: i32) -> u32 {
    const MAX_AXIS_BITS: i32 = 21;
    let quantization = options.get_attribute_int(att_id, "quantization_bits", -1);
    if quantization <= 0 {
        // Nothing quantizes the positions, so they reach the decoder with
        // every bit they arrived with and there is no coarser grid to match.
        return MAX_AXIS_BITS as u32;
    }
    quantization.min(MAX_AXIS_BITS) as u32
}

/// 24 states x 8 octants: the low three bits are the octant's index along the
/// curve, the rest is the next state. The automaton of a 3D Hilbert curve, one
/// level a step.
const HILBERT_STEP: [u8; 192] = build_hilbert_step();

/// The same automaton two levels a step: 24 states x 64 pairs of octants, the
/// low six bits the pair's two indices along the curve and the rest the state
/// after them. Built from the table above, so the two cannot disagree.
const HILBERT_PAIR: [u16; 1536] = build_hilbert_pair();

const fn build_hilbert_step() -> [u8; 192] {
    let mut table = [0u8; 192];
    let mut state = 0usize;
    while state < 24 {
        let c = (state & 7) as u32;
        let n = (state / 8) as u32;
        let mut m = 0u32;
        while m < 8 {
            let gray = rotate_right_3(c ^ m, n);
            let index = gray_to_integer_3(gray);
            let without_high_bit = gray & 0b011;
            let next_rotation = if without_high_bit == 0 {
                1
            } else if (without_high_bit & 1) != 0 {
                2
            } else {
                3
            };
            let transform = if index == 0 {
                0
            } else {
                let low_bit = index & 0u32.wrapping_sub(index);
                gray ^ (low_bit | 1)
            };
            let next_c = c ^ rotate_left_3(transform, n);
            let next_n = (n + next_rotation) % 3;
            let next_state = next_n * 8 + next_c;
            table[state * 8 + m as usize] = ((next_state as u8) << 3) | (index as u8);
            m += 1;
        }
        state += 1;
    }
    table
}

const fn build_hilbert_pair() -> [u16; 1536] {
    let mut table = [0u16; 1536];
    let mut state = 0usize;
    while state < 24 {
        let mut m = 0u32;
        while m < 64 {
            let (x, y, z) = ((m >> 4) & 3, (m >> 2) & 3, m & 3);
            let mut next_state = state;
            let mut out = 0u32;
            let mut bit = 2u32;
            while bit > 0 {
                bit -= 1;
                let octant = (((x >> bit) & 1) << 2) | (((y >> bit) & 1) << 1) | ((z >> bit) & 1);
                let entry = HILBERT_STEP[next_state * 8 + octant as usize];
                out = (out << 3) | (entry & 7) as u32;
                next_state = (entry >> 3) as usize;
            }
            table[state * 64 + m as usize] = ((next_state as u16) << 6) | (out as u16);
            m += 1;
        }
        state += 1;
    }
    table
}

const fn rotate_left_3(value: u32, shift: u32) -> u32 {
    match shift {
        0 => value & 7,
        1 => ((value << 1) | (value >> 2)) & 7,
        _ => ((value << 2) | (value >> 1)) & 7,
    }
}

const fn rotate_right_3(value: u32, shift: u32) -> u32 {
    match shift {
        0 => value & 7,
        1 => ((value >> 1) | (value << 2)) & 7,
        _ => ((value >> 2) | (value << 1)) & 7,
    }
}

const fn gray_to_integer_3(mut gray: u32) -> u32 {
    gray ^= gray >> 1;
    gray ^= gray >> 2;
    gray & 7
}

/// The index of `(x, y, z)` along a 3D Hilbert curve of `bits` levels.
fn hilbert_key(x: u32, y: u32, z: u32, bits: u32) -> u64 {
    let mut key = 0u64;
    let mut state = 0usize;
    let mut shift = bits;
    while shift >= 2 {
        shift -= 2;
        let m = (((x >> shift) & 3) << 4) | (((y >> shift) & 3) << 2) | ((z >> shift) & 3);
        let entry = HILBERT_PAIR[state * 64 + m as usize];
        key = (key << 6) | u64::from(entry & 0x3f);
        state = (entry >> 6) as usize;
    }
    if shift == 1 {
        let m = ((x & 1) << 2) | ((y & 1) << 1) | (z & 1);
        let entry = HILBERT_STEP[state * 8 + m as usize];
        key = (key << 3) | u64::from(entry & 7);
    }
    key
}

/// The positions on a grid of `axis_bits` an axis, one column an axis.
struct Grid {
    axis_bits: u32,
    cells: [Vec<u32>; 3],
}

impl Grid {
    fn len(&self) -> usize {
        self.cells[0].len()
    }

    /// The cell an axis falls in, on the 16-bit grid the estimate prices.
    fn price_cells(&self, threads: usize) -> [Vec<u16>; 3] {
        let shift = self.axis_bits.saturating_sub(POSITION_BITS);
        let mut axes = parallel::map(3, threads, |axis| -> Vec<u16> {
            self.cells[axis]
                .iter()
                .map(|&c| (c >> shift) as u16)
                .collect()
        })
        .into_iter();
        std::array::from_fn(|_| axes.next().expect("one per axis"))
    }

    /// Point indices along the Hilbert curve; points sharing a cell keep the
    /// order they came in.
    ///
    /// The order is that of the pairs `(key, index)`, which are all distinct,
    /// so there is exactly one and every way of sorting arrives at it. This
    /// one deals the indices into buckets by the top bits of their keys and
    /// sorts the buckets side by side: the encode waits on this whole sort
    /// before anything else can start, and one sort of millions of pairs is
    /// the part of it no other thread could help with.
    fn hilbert_order(&self, threads: usize) -> Vec<u32> {
        let bits = self.axis_bits;
        let mut keys = vec![0u64; self.len()];
        parallel::for_each_chunk_mut(&mut keys, parallel::PIECE, threads, |chunk, keys| {
            for (offset, key) in keys.iter_mut().enumerate() {
                let p = chunk * parallel::PIECE + offset;
                *key = hilbert_key(self.cells[0][p], self.cells[1][p], self.cells[2][p], bits);
            }
        });

        let key_bits = 3 * bits;
        let bucket_bits = key_bits.min(HILBERT_BUCKET_BITS);
        let shift = key_bits - bucket_bits;
        let mut starts = vec![0usize; (1 << bucket_bits) + 1];
        for &key in &keys {
            starts[(key >> shift) as usize + 1] += 1;
        }
        for bucket in 1..starts.len() {
            starts[bucket] += starts[bucket - 1];
        }
        let mut next = starts.clone();
        let mut order = vec![0u32; keys.len()];
        for (p, &key) in keys.iter().enumerate() {
            let slot = &mut next[(key >> shift) as usize];
            order[*slot] = p as u32;
            *slot += 1;
        }

        let mut buckets = Vec::with_capacity(starts.len() - 1);
        let mut rest = order.as_mut_slice();
        for bucket in starts.windows(2) {
            let (piece, tail) = rest.split_at_mut(bucket[1] - bucket[0]);
            if piece.len() > 1 {
                buckets.push(piece);
            }
            rest = tail;
        }
        parallel::for_each_piece_mut(buckets, threads, |bucket| {
            let mut keyed: Vec<(u64, u32)> =
                bucket.iter().map(|&p| (keys[p as usize], p)).collect();
            keyed.sort_unstable();
            for (slot, (_, p)) in bucket.iter_mut().zip(keyed) {
                *slot = p;
            }
        });
        order
    }
}

/// How many top bits of a Hilbert key choose its bucket in `hilbert_order`:
/// enough buckets that the occupied ones spread over every thread even when a
/// scan's points sit in a few percent of the cube, few enough that dealing
/// into them stays inside the cache.
const HILBERT_BUCKET_BITS: u32 = 12;

/// The values of one component of an attribute, one per point, as `f32`.
///
/// `None` for a type this does not read. A cloud read from a file maps points
/// to values one to one and is read straight off its buffer; one whose equal
/// values were merged is read by its map.
fn column_values(attribute: &PointAttribute, component: usize, count: usize) -> Option<Vec<f32>> {
    let data = attribute.buffer().data();
    let at = value_offsets(attribute, component, count)?;
    macro_rules! read {
        ($t:ty, $n:expr) => {
            (0..count)
                .map(|p| {
                    let o = at(p);
                    <$t>::from_le_bytes(data[o..o + $n].try_into().unwrap()) as f32
                })
                .collect()
        };
    }
    Some(match attribute.data_type() {
        DataType::Float32 => read!(f32, 4),
        DataType::Float64 => read!(f64, 8),
        DataType::Uint8 => (0..count).map(|p| f32::from(data[at(p)])).collect(),
        DataType::Int8 => (0..count).map(|p| f32::from(data[at(p)] as i8)).collect(),
        DataType::Uint16 => read!(u16, 2),
        DataType::Int16 => read!(i16, 2),
        DataType::Uint32 => read!(u32, 4),
        DataType::Int32 => read!(i32, 4),
        DataType::Uint64 => read!(u64, 8),
        DataType::Int64 => read!(i64, 8),
        DataType::Bool | DataType::Invalid => return None,
    })
}

/// The values of one component, one per point, exactly: what the encoder
/// predicts from, for the types it predicts. `None` for the types it copies
/// as they are, where the order a point is written in costs nothing.
fn exact_values(attribute: &PointAttribute, component: usize, count: usize) -> Option<Vec<f64>> {
    let data = attribute.buffer().data();
    let at = value_offsets(attribute, component, count)?;
    macro_rules! read {
        ($t:ty, $n:expr) => {
            (0..count)
                .map(|p| {
                    let o = at(p);
                    f64::from(<$t>::from_le_bytes(data[o..o + $n].try_into().unwrap()))
                })
                .collect()
        };
    }
    Some(match attribute.data_type() {
        DataType::Float32 => read!(f32, 4),
        DataType::Uint8 => (0..count).map(|p| f64::from(data[at(p)])).collect(),
        DataType::Int8 => (0..count).map(|p| f64::from(data[at(p)] as i8)).collect(),
        DataType::Uint16 => read!(u16, 2),
        DataType::Int16 => read!(i16, 2),
        DataType::Uint32 => read!(u32, 4),
        DataType::Int32 => read!(i32, 4),
        _ => return None,
    })
}

/// Where each point's value of `component` starts in `attribute`'s buffer, or
/// `None` if any of them would read past it.
fn value_offsets<'a>(
    attribute: &'a PointAttribute,
    component: usize,
    count: usize,
) -> Option<impl Fn(usize) -> usize + 'a> {
    let len = attribute.buffer().data().len();
    let stride = attribute.byte_stride() as usize;
    let size = attribute.data_type().byte_length();
    if stride == 0 || size == 0 || (component + 1) * size > stride {
        return None;
    }
    let identity = len / stride == count;
    let at = move |point: usize| -> usize {
        let value = if identity {
            point
        } else {
            let AttributeValueIndex(index) = attribute.mapped_index(PointIndex(point as u32));
            index as usize
        };
        value * stride + component * size
    };
    if (0..count).any(|p| at(p) + size > len) {
        return None;
    }
    Some(at)
}

/// The position attribute and its points on the curve's grid, or `None` where
/// there is nothing to lay a curve over: no position attribute, fewer than
/// three components, a type this cannot read, or a coordinate that is not
/// finite and so has no place on any grid. Returning the caller's order
/// unchanged is the only honest answer there; an order derived from values that
/// were not the positions would be worse than none.
fn grid(pc: &PointCloud, options: &EncoderOptions, threads: usize) -> Option<(i32, Grid)> {
    let att_id = (0..pc.num_attributes())
        .find(|id| pc.attribute(*id).attribute_type() == GeometryAttributeType::Position)?;
    let attribute = pc.attribute(att_id);
    if attribute.num_components() < 3 {
        return None;
    }
    let count = pc.num_points();
    let axis_bits = curve_axis_bits(options, att_id);
    let levels = ((1u64 << axis_bits) - 1) as f64;
    let axes = parallel::map(3, threads, |axis| -> Option<Vec<u32>> {
        let values = column_values(attribute, axis, count)?;
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in &values {
            if !v.is_finite() {
                return None;
            }
            low = low.min(v);
            high = high.max(v);
        }
        let (low, span) = (f64::from(low), f64::from(high) - f64::from(low));
        Some(
            values
                .iter()
                .map(|&v| {
                    let normalized = if span > 0.0 {
                        (f64::from(v) - low) / span
                    } else {
                        0.0
                    };
                    (normalized * levels) as u32
                })
                .collect(),
        )
    });
    let mut axes = axes.into_iter();
    let cells = [axes.next()??, axes.next()??, axes.next()??];
    Some((att_id, Grid { axis_bits, cells }))
}

/// The points in Hilbert order, or `None` where no curve can be laid.
pub(crate) fn curve(pc: &PointCloud, options: &EncoderOptions) -> Option<Vec<PointIndex>> {
    let threads = parallel::resolve(options.get_threads());
    let (_, grid) = grid(pc, options, threads)?;
    Some(
        grid.hilbert_order(threads)
            .into_iter()
            .map(PointIndex)
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// The estimate
// ---------------------------------------------------------------------------

/// `log2(1 + d)` in quarters of a bit, rounded down: the exponent and the two
/// top mantissa bits of the float `1 + d`, which is the whole computation. It is
/// the logarithm exactly at the powers of two and linear between them, so it
/// never exceeds the logarithm and falls short of it by under 1.4 quarter-bits.
/// `1 + d` is exact as a float up to 2^24, which no cell or byte reaches; past
/// it, on a fine column, rounding can lift a value to the next quarter at most.
#[inline(always)]
fn quarter_bits(d: u32) -> u16 {
    ((d.saturating_add(1) as f32).to_bits() >> 21) as u16 - 508
}

/// One attribute component quantized to bytes: `((v - low) * scale + 0.5)`.
#[derive(Clone, Copy)]
struct Quantizer {
    low: f32,
    scale: f32,
}

impl Quantizer {
    #[inline(always)]
    fn byte(&self, v: f32) -> u8 {
        ((v - self.low) * self.scale + 0.5) as u8
    }
}

/// One attribute component on the grid the encoder predicts it on, where that
/// is finer than a byte: an integer's own values, or a float's quantization.
#[derive(Clone, Copy)]
struct Fine {
    low: f64,
    scale: f64,
}

impl Fine {
    #[inline(always)]
    fn level(&self, v: f64) -> u32 {
        ((v - self.low) * self.scale + 0.5) as u32
    }

    /// `exact` on this grid, one level a point.
    fn levels(&self, exact: &[f64]) -> Vec<u32> {
        exact.iter().map(|&v| self.level(v)).collect()
    }
}

/// One attribute component, with what it costs along the curve in
/// quarter-bits over the sampled steps: what ranks the columns the refinement
/// counts.
///
/// The refinement compares points on `quantizer`'s bytes, which keeps its
/// lanes narrow. Where the encoder predicts on a finer grid, the bytes cap
/// what breaking a column's order can cost at eight bits a step when it can
/// cost thirty: a lidar's time, in ticks along its scan, steps by a few ticks
/// and costs a few bits; out of scan order it costs most of its range. So the
/// cost here is on that finer grid where there is one; only the refinement
/// itself works on the bytes.
struct Column {
    attribute: i32,
    component: usize,
    quantizer: Quantizer,
    cost_curve: u64,
}

/// The attribute components that vary and that the encoder predicts, with what
/// each costs along `curve_order`. Constant ones are left out:
/// stepping across them is free, and 3DGS files carry three (`nx ny nz`, all
/// zero). So are the ones the encoder copies as they are -- a `f64`, a 64-bit
/// integer, an unquantized float -- whose cost no order changes.
fn measure_columns(
    pc: &PointCloud,
    options: &EncoderOptions,
    curve_order: &[u32],
    threads: usize,
) -> Vec<Column> {
    let count = pc.num_points();
    let position = (0..pc.num_attributes())
        .find(|id| pc.attribute(*id).attribute_type() == GeometryAttributeType::Position);
    let components: Vec<(i32, usize)> = (0..pc.num_attributes())
        .filter(|&id| Some(id) != position)
        .flat_map(|id| (0..pc.attribute(id).num_components() as usize).map(move |c| (id, c)))
        .collect();
    let measured = parallel::map(components.len(), threads, |index| -> Option<Column> {
        let (id, component) = components[index];
        let attribute = pc.attribute(id);
        let quantization_bits = options.get_attribute_int(id, "quantization_bits", -1);
        let coder = select_sequential_encoder(attribute, quantization_bits);
        if coder == SequentialAttributeEncoderType::Generic {
            return None;
        }
        let values = column_values(attribute, component, count)?;
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in &values {
            low = low.min(v);
            high = high.max(v);
        }
        let span = high - low;
        if !span.is_finite() || span <= 1e-12 {
            return None;
        }
        let bits = if attribute.data_type() == DataType::Float32 {
            match options.get_attribute_int(id, "quantization_bits", -1) {
                b if b > 0 => (b as u32).min(8),
                _ => 8,
            }
        } else {
            8
        };
        let levels = ((1u32 << bits) - 1) as f32;
        let quantizer = Quantizer {
            low,
            scale: levels / span,
        };
        let fine_levels = fine_grid(attribute, component, count, coder, quantization_bits)
            .map(|(fine, exact)| fine.levels(&exact));
        let cost_curve = (0..count.saturating_sub(1))
            .step_by(SAMPLE_STRIDE)
            .map(|i| {
                let (a, b) = (curve_order[i] as usize, curve_order[i + 1] as usize);
                let step = match &fine_levels {
                    Some(levels) => levels[a].abs_diff(levels[b]),
                    None => u32::from(
                        quantizer
                            .byte(values[a])
                            .abs_diff(quantizer.byte(values[b])),
                    ),
                };
                u64::from(quarter_bits(step))
            })
            .sum();
        Some(Column {
            attribute: id,
            component,
            quantizer,
            cost_curve,
        })
    });
    measured.into_iter().flatten().collect()
}

/// The grid the encoder predicts `component` on and the exact values, where
/// that grid is finer than the byte a column is refined on; `None` where a byte
/// already resolves it -- a byte attribute, a float of at most eight bits, a
/// normal, which is coded on its own octahedral grid.
fn fine_grid(
    attribute: &PointAttribute,
    component: usize,
    count: usize,
    coder: SequentialAttributeEncoderType,
    quantization_bits: i32,
) -> Option<(Fine, Vec<f64>)> {
    let finer = match coder {
        SequentialAttributeEncoderType::Quantization => quantization_bits > 8,
        SequentialAttributeEncoderType::Integer => attribute.data_type().byte_length() > 1,
        _ => false,
    };
    if !finer {
        return None;
    }
    let exact = exact_values(attribute, component, count)?;
    let (low, high) = exact
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &v| {
            (low.min(v), high.max(v))
        });
    let span = high - low;
    let scale = if coder == SequentialAttributeEncoderType::Quantization {
        ((1u64 << quantization_bits.min(30)) - 1) as f64 / span
    } else if span > 255.0 {
        1.0
    } else {
        return None;
    };
    // A level is a `u32`: an `i32` attribute's widest span is exactly its top.
    (span > 0.0 && scale.is_finite() && span * scale <= f64::from(u32::MAX))
        .then_some((Fine { low, scale }, exact))
}

// ---------------------------------------------------------------------------
// The coder's price
// ---------------------------------------------------------------------------

/// One attribute the way the sequential coder predicts it: its components'
/// values, point by point, as an integer's own or a float's on its
/// quantization grid, and the bounds the wrap transform takes from them.
///
/// A normal is coded on an octahedral grid of two components; it is read here
/// as its three, quantized like any float, which prices its steps closely
/// enough to rank orders by.
struct Coded<'a> {
    attribute: &'a PointAttribute,
    components: usize,
    /// Whether point `p`'s value is the `p`-th of the buffer.
    identity: bool,
    /// Per component, what is subtracted before scaling; `None` for an
    /// integer, which is coded as it is.
    lows: Option<Vec<f64>>,
    scale: f64,
    low: i32,
    high: i32,
}

/// The value of `data_type` at `at` in `data`, for the types `exact_values`
/// reads; zero for any other, which `coded_attributes` never hands it.
#[inline(always)]
fn read_exact(data: &[u8], data_type: DataType, at: usize) -> f64 {
    let bytes = |n: usize| &data[at..at + n];
    match data_type {
        DataType::Float32 => f64::from(f32::from_le_bytes(bytes(4).try_into().unwrap())),
        DataType::Uint8 => f64::from(data[at]),
        DataType::Int8 => f64::from(data[at] as i8),
        DataType::Uint16 => f64::from(u16::from_le_bytes(bytes(2).try_into().unwrap())),
        DataType::Int16 => f64::from(i16::from_le_bytes(bytes(2).try_into().unwrap())),
        DataType::Uint32 => f64::from(u32::from_le_bytes(bytes(4).try_into().unwrap())),
        DataType::Int32 => f64::from(i32::from_le_bytes(bytes(4).try_into().unwrap())),
        _ => 0.0,
    }
}

impl Coded<'_> {
    fn value(&self, point: usize, component: usize) -> i32 {
        let attribute = self.attribute;
        let index = if self.identity {
            point
        } else {
            let AttributeValueIndex(index) = attribute.mapped_index(PointIndex(point as u32));
            index as usize
        };
        let data_type = attribute.data_type();
        let at = index * attribute.byte_stride() as usize + component * data_type.byte_length();
        let v = read_exact(attribute.buffer().data(), data_type, at);
        match &self.lows {
            None => v as i64 as i32,
            Some(lows) => ((v - lows[component]) * self.scale + 0.5).floor() as i32,
        }
    }
}

/// The attributes whose cost an order changes, as the coder sees them. An
/// attribute the coder copies as it is, or one this cannot read, is left out.
fn coded_attributes<'a>(
    pc: &'a PointCloud,
    options: &EncoderOptions,
    threads: usize,
) -> Vec<Coded<'a>> {
    let count = pc.num_points();
    let inner = (threads / (pc.num_attributes() as usize).max(1)).max(1);
    let read = parallel::map(
        pc.num_attributes() as usize,
        threads,
        |id| -> Option<Coded> {
            let id = id as i32;
            let attribute = pc.attribute(id);
            let quantization_bits = options.get_attribute_int(id, "quantization_bits", -1);
            let coder = select_sequential_encoder(attribute, quantization_bits);
            let components = attribute.num_components() as usize;
            if coder == SequentialAttributeEncoderType::Generic || components == 0 {
                return None;
            }
            // The types `exact_values` reads, each point's value inside the buffer
            // as `value_offsets` checks: what `Coded::value` then reads unchecked.
            let data_type = attribute.data_type();
            if !matches!(
                data_type,
                DataType::Float32
                    | DataType::Uint8
                    | DataType::Int8
                    | DataType::Uint16
                    | DataType::Int16
                    | DataType::Uint32
                    | DataType::Int32
            ) {
                return None;
            }
            let data = attribute.buffer().data();
            let bounds: Vec<(f64, f64)> = (0..components)
                .map(|c| {
                    let at = value_offsets(attribute, c, count)?;
                    let pieces = parallel::map(count.div_ceil(parallel::PIECE), inner, |k| {
                        let end = ((k + 1) * parallel::PIECE).min(count);
                        (k * parallel::PIECE..end).fold(
                            (f64::INFINITY, f64::NEG_INFINITY),
                            |(l, h), p| {
                                let v = read_exact(data, data_type, at(p));
                                (l.min(v), h.max(v))
                            },
                        )
                    });
                    Some(
                        pieces
                            .into_iter()
                            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), (pl, ph)| {
                                (l.min(pl), h.max(ph))
                            }),
                    )
                })
                .collect::<Option<_>>()?;
            let identity =
                attribute.buffer().data().len() / attribute.byte_stride() as usize == count;
            if coder == SequentialAttributeEncoderType::Integer {
                let (low, high) = bounds
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), &(cl, ch)| {
                        (l.min(cl), h.max(ch))
                    });
                return Some(Coded {
                    attribute,
                    components,
                    identity,
                    lows: None,
                    scale: 1.0,
                    low: low as i64 as i32,
                    high: high as i64 as i32,
                });
            }
            // One range for every component, as the quantization transform takes it.
            let range = bounds.iter().map(|&(l, h)| h - l).fold(0.0, f64::max);
            if !range.is_finite() {
                return None;
            }
            let scale = if range > 0.0 {
                ((1u64 << quantization_bits.clamp(1, 30)) - 1) as f64 / range
            } else {
                0.0
            };
            let high = bounds
                .iter()
                .map(|&(l, h)| ((h - l) * scale + 0.5).floor() as i32)
                .max()
                .unwrap_or(0);
            Some(Coded {
                attribute,
                components,
                identity,
                lows: Some(bounds.iter().map(|&(l, _)| l).collect()),
                scale,
                low: 0,
                high,
            })
        },
    );
    read.into_iter().flatten().collect()
}

/// What the sequential coder would write for the predicted attributes of the
/// points of each of `orders`, taken in that order, in bits.
///
/// Each attribute is priced the way its coder chooses how to write it: the
/// wrap-transformed difference from the point before, zigzagged, planned by
/// the symbol coder -- which takes the cheaper of its tagged and raw schemes,
/// raw only up to 18 bits a symbol -- and, where the prediction search is on,
/// the cheaper of that and the values planned as they are, which no order
/// changes. So a step that is always the same costs next to nothing, as it
/// does in the stream, and an attribute that does not follow its neighbours
/// costs the same in every order.
///
/// The orders are of the same points, all or some, and the price is of those
/// points in each. It is taken over every `stride`-th step, the same steps of
/// every order, so the prices compare with one another and not with the
/// stream; the values planned as they are, the same in every order, are
/// planned once.
fn coder_bits(
    attributes: &[Coded],
    search: bool,
    orders: &[&[u32]],
    stride: usize,
    threads: usize,
) -> Vec<u64> {
    // The attributes side by side, each on what threads they leave: a splat
    // has dozens, each a sampled pass of reads scattered over its buffer, and
    // a scan has one or two, each worth splitting.
    let inner = (threads / attributes.len().max(1)).max(1);
    let priced = parallel::map(attributes.len(), threads, |a| {
        price_attribute(&attributes[a], search, orders, stride, inner)
    });
    let mut bits = vec![0u64; orders.len()];
    for attribute in priced {
        for (total, cost) in bits.iter_mut().zip(attribute) {
            *total += cost;
        }
    }
    bits
}

/// `coder_bits` for one attribute.
fn price_attribute(
    coded: &Coded,
    search: bool,
    orders: &[&[u32]],
    stride: usize,
    threads: usize,
) -> Vec<u64> {
    let mut bits = vec![0u64; orders.len()];
    {
        let components = coded.components;
        let (low, high) = (coded.low, coded.high);
        let span = 1 + i64::from(high) - i64::from(low);
        let max_correction = (span / 2 - i64::from(span % 2 == 0)) as i32;
        let min_correction = -((span / 2) as i32);
        let zigzag = |c: i32| ((c << 1) ^ (c >> 31)) as u32;
        let wrap = |d: i32| {
            if d < min_correction {
                d.wrapping_add(span as i32)
            } else if d > max_correction {
                d.wrapping_sub(span as i32)
            } else {
                d
            }
        };
        let mut plain: Option<u64> = None;
        for (k, order) in orders.iter().enumerate() {
            let steps = order.len().div_ceil(stride);
            let mut symbols = vec![0u32; steps * components];
            parallel::for_each_chunk_mut(
                &mut symbols,
                parallel::PIECE * components,
                threads,
                |chunk, out| {
                    let first = chunk * parallel::PIECE;
                    for (i, entry) in out.chunks_mut(components).enumerate() {
                        let at = (first + i) * stride;
                        let p = order[at] as usize;
                        for (c, symbol) in entry.iter_mut().enumerate() {
                            let predicted = if at == 0 {
                                low
                            } else {
                                coded.value(order[at - 1] as usize, c)
                            };
                            *symbol = zigzag(wrap(coded.value(p, c).wrapping_sub(predicted)));
                        }
                    }
                },
            );
            let mut cost =
                plan_symbols_with_threads(&symbols, components, threads).estimated_bits();
            if search {
                let plain = *plain.get_or_insert_with(|| {
                    for (i, entry) in symbols.chunks_mut(components).enumerate() {
                        let p = order[i * stride] as usize;
                        for (c, symbol) in entry.iter_mut().enumerate() {
                            *symbol = zigzag(coded.value(p, c));
                        }
                    }
                    plan_symbols_with_threads(&symbols, components, threads).estimated_bits()
                });
                cost = cost.min(plain);
            }
            bits[k] += cost;
        }
    }
    bits
}

// ---------------------------------------------------------------------------
// The refinement
// ---------------------------------------------------------------------------

/// How far past the path the columns are padded, so a window of any lane count
/// can be loaded as one slice wherever it starts.
const LANE_PAD: usize = 72;

/// One block of the path with each column stored on its own, so the
/// candidates of a start are consecutive bytes of every column.
struct Lanes {
    n: usize,
    cols: Vec<Vec<u8>>,
    pos: [Vec<u16>; 3],
    ids: Vec<u32>,
    /// `edges[i]` is the cost of the step from point `i` to point `i + 1`.
    edges: Vec<u16>,
}

/// The cost from `anchor` to each of the `L` points starting at `first`.
#[inline(always)]
fn lane_costs<const L: usize>(
    cols: &[Vec<u8>],
    pos: &[Vec<u16>; 3],
    anchor: usize,
    first: usize,
) -> [u16; L] {
    let mut total = [0u16; L];
    for axis in pos {
        let a = axis[anchor];
        let w: &[u16; L] = axis[first..first + L].try_into().unwrap();
        for l in 0..L {
            total[l] += quarter_bits(u32::from(a.abs_diff(w[l])));
        }
    }
    for column in cols {
        let a = column[anchor];
        let w: &[u8; L] = column[first..first + L].try_into().unwrap();
        for l in 0..L {
            total[l] += quarter_bits(u32::from(a.abs_diff(w[l])));
        }
    }
    total
}

/// Reverses `buf[i..i + m]` for `m` in `2..=16` as one 128-bit byte swap, a
/// shift and a blend. The column is padded, so sixteen bytes from `i` exist.
#[inline(always)]
fn reverse_short(buf: &mut [u8], i: usize, m: usize) {
    let chunk: &mut [u8; 16] = (&mut buf[i..i + 16]).try_into().unwrap();
    let x = u128::from_le_bytes(*chunk);
    let reversed = x.swap_bytes() >> (8 * (16 - m));
    let mask = if m == 16 {
        u128::MAX
    } else {
        (1u128 << (8 * m)) - 1
    };
    *chunk = ((x & !mask) | (reversed & mask)).to_le_bytes();
}

impl Lanes {
    /// The block `ids` of the path, gathered column by column from the
    /// file-order columns so the reads stay inside one column's memory.
    fn gather(columns: &[Vec<u8>], cells: &[Vec<u16>; 3], ids: &[u32]) -> Self {
        let n = ids.len();
        let cols = columns
            .iter()
            .map(|column| {
                let mut v: Vec<u8> = ids.iter().map(|&p| column[p as usize]).collect();
                v.resize(n + LANE_PAD, 0);
                v
            })
            .collect();
        let pos = std::array::from_fn(|axis| {
            let mut v: Vec<u16> = ids.iter().map(|&p| cells[axis][p as usize]).collect();
            v.resize(n + LANE_PAD, 0);
            v
        });
        let mut lanes = Lanes {
            n,
            cols,
            pos,
            ids: ids.to_vec(),
            edges: vec![0; n + LANE_PAD],
        };
        for i in 0..n.saturating_sub(1) {
            lanes.edges[i] = lanes.edge(i, i + 1);
        }
        lanes
    }

    fn edge(&self, a: usize, b: usize) -> u16 {
        let mut sum = 0u16;
        for axis in &self.pos {
            sum += quarter_bits(u32::from(axis[a].abs_diff(axis[b])));
        }
        for column in &self.cols {
            sum += quarter_bits(u32::from(column[a].abs_diff(column[b])));
        }
        sum
    }

    /// 2-opt with `L` candidate ends a start: take the end whose reversal of
    /// the stretch saves the most, and look again until none saves anything.
    /// Both end points of the block stay where they are.
    fn refine<const L: usize>(&mut self, passes: usize) {
        let n = self.n;
        if n < 4 {
            return;
        }
        for _ in 0..passes {
            let mut moved = false;
            for i in 1..n - 1 {
                loop {
                    let last = (i + L).min(n - 2);
                    if i + 1 > last {
                        break;
                    }
                    let ac = lane_costs::<L>(&self.cols, &self.pos, i - 1, i + 1);
                    let bd = lane_costs::<L>(&self.cols, &self.pos, i, i + 2);
                    let old: &[u16; L] = self.edges[i + 1..i + 1 + L].try_into().unwrap();
                    let before = i32::from(self.edges[i - 1]);
                    let mut best_gain = 0i32;
                    let mut best_lane = usize::MAX;
                    for l in 0..L {
                        let gain = before + i32::from(old[l]) - i32::from(ac[l]) - i32::from(bd[l]);
                        if i + 1 + l <= last && gain > best_gain {
                            best_gain = gain;
                            best_lane = l;
                        }
                    }
                    if best_lane == usize::MAX {
                        break;
                    }
                    let j = i + 1 + best_lane;
                    let m = j - i + 1;
                    for column in &mut self.cols {
                        if m <= 16 {
                            reverse_short(column, i, m);
                        } else {
                            column[i..=j].reverse();
                        }
                    }
                    for axis in &mut self.pos {
                        axis[i..=j].reverse();
                    }
                    self.ids[i..=j].reverse();
                    self.edges[i..j].reverse();
                    self.edges[i - 1] = ac[best_lane];
                    self.edges[j] = bd[best_lane];
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }
}

fn refine_block(lanes: &mut Lanes, window: usize, passes: usize) {
    match window {
        0..=8 => lanes.refine::<8>(passes),
        9..=16 => lanes.refine::<16>(passes),
        17..=32 => lanes.refine::<32>(passes),
        _ => lanes.refine::<64>(passes),
    }
}

// ---------------------------------------------------------------------------
// The search
// ---------------------------------------------------------------------------

/// The order to write `pc`'s points in so the stream comes out small, or `None`
/// to leave them in the order they came.
///
/// `None` is an answer and not a failure: it is what comes back when the input
/// order already beats what the search would write, when there is no position
/// to sort by, and when the cloud is too small to be worth reordering.
pub(crate) fn search(pc: &PointCloud, options: &EncoderOptions) -> Option<Vec<PointIndex>> {
    let count = pc.num_points();
    if count < 64 {
        return None;
    }
    let threads = parallel::resolve(options.get_threads());
    let (_, grid) = grid(pc, options, threads)?;
    let cells = grid.price_cells(threads);
    let curve_order = grid.hilbert_order(threads);
    let columns = measure_columns(pc, options, &curve_order, threads);

    // Which order is written is decided on what the coder would write for
    // each; the estimate below only steers the refinement.
    let coded = coded_attributes(pc, options, threads);
    let components: usize = coded.iter().map(|c| c.components).sum();
    let prediction_search = options.prediction_search();
    let identity: Vec<u32> = (0..count as u32).collect();
    let whole = coder_bits(
        &coded,
        prediction_search,
        &[&identity, &curve_order],
        pricing_stride(count, components),
        threads,
    );
    let (bits_input, bits_curve) = (whole[0] as f64, whole[1] as f64);
    let curve_wins = bits_curve <= WORTH_KEEPING_BELOW * bits_input;
    let unrefined = |curve_order: Vec<u32>| {
        curve_wins.then(|| curve_order.into_iter().map(PointIndex).collect())
    };

    let Some(effort) = effort(options.get_encoding_speed()) else {
        return unrefined(curve_order);
    };

    // The dearest columns along the curve are the ones worth counting.
    let mut ranked: Vec<usize> = (0..columns.len()).collect();
    ranked.sort_by(|&a, &b| {
        columns[b]
            .cost_curve
            .cmp(&columns[a].cost_curve)
            .then(a.cmp(&b))
    });
    ranked.truncate(effort.columns);
    let chosen: Vec<&Column> = ranked.iter().map(|&k| &columns[k]).collect();
    let quantized: Vec<Vec<u8>> = parallel::map(chosen.len(), threads, |k| {
        let column = chosen[k];
        let values = column_values(pc.attribute(column.attribute), column.component, count)
            .expect("a column that was measured can be read");
        values.iter().map(|&v| column.quantizer.byte(v)).collect()
    });
    // What a refinement has to undercut: both orders it would replace.
    let to_beat = WORTH_KEEPING_BELOW * bits_input.min(bits_curve);

    let refine = |ids: &mut [u32]| {
        if ids.len() < 4 {
            return;
        }
        let mut lanes = Lanes::gather(&quantized, &cells, ids);
        refine_block(&mut lanes, effort.window, effort.passes);
        ids.copy_from_slice(&lanes.ids);
    };
    let every = (count.div_ceil(BLOCK) / 4).clamp(1, TRIAL_EVERY);
    let mut order = curve_order.clone();
    parallel::for_each_chunk_mut(&mut order, BLOCK, threads, |block, ids| {
        if block % every == 0 {
            refine(ids);
        }
    });
    // A refinement moves points only inside their blocks, so the trial blocks
    // of the curve and of the refinement hold the same points: the two are
    // priced on exactly those, and what the trial saves is scaled to the whole.
    let trial_points = |path: &[u32]| -> Vec<u32> {
        path.chunks(BLOCK)
            .step_by(every)
            .flatten()
            .copied()
            .collect()
    };
    let (trial_curve, trial_refined) = (trial_points(&curve_order), trial_points(&order));
    let trial = coder_bits(
        &coded,
        prediction_search,
        &[&trial_curve, &trial_refined],
        pricing_stride(trial_curve.len(), components),
        threads,
    );
    if trial[1] as f64 / trial[0].max(1) as f64 * bits_curve > to_beat {
        return unrefined(curve_order);
    }
    parallel::for_each_chunk_mut(&mut order, BLOCK, threads, |block, ids| {
        if block % every != 0 {
            refine(ids);
        }
    });

    // The check after, over the whole result: the blocks that were not in the
    // trial might not have behaved like the ones that were.
    let stride = pricing_stride(count, components);
    if coder_bits(&coded, prediction_search, &[&order], stride, threads)[0] as f64 > to_beat {
        return unrefined(curve_order);
    }
    Some(order.into_iter().map(PointIndex).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry_attribute::PointAttribute;

    /// The curve is a curve: it visits every cell once, and each step is to a
    /// face neighbour. Both halves matter -- a broken state table can still
    /// produce a bijection while jumping across the grid, which is exactly the
    /// property the order is meant to have. Odd and even depths, because the
    /// walk takes two levels at a time and finishes on one when the depth is
    /// odd.
    #[test]
    fn the_hilbert_key_walks_every_cell_once_without_jumping() {
        for bits in [1u32, 2, 3, 4, 5] {
            let side = 1u32 << bits;
            let cells = (side * side * side) as usize;
            let mut position_of = vec![None; cells];
            for x in 0..side {
                for y in 0..side {
                    for z in 0..side {
                        let key = hilbert_key(x, y, z, bits) as usize;
                        assert!(key < cells, "key {key} outside the grid at {bits} bits");
                        assert!(
                            position_of[key].replace((x, y, z)).is_none(),
                            "two cells share key {key} at {bits} bits"
                        );
                    }
                }
            }
            let mut previous = position_of[0].expect("cell 0 is visited");
            for (key, cell) in position_of.iter().enumerate().skip(1) {
                let cell = cell.expect("every cell is visited");
                let step = previous.0.abs_diff(cell.0)
                    + previous.1.abs_diff(cell.1)
                    + previous.2.abs_diff(cell.2);
                assert_eq!(step, 1, "step {key} jumps at {bits} bits");
                previous = cell;
            }
        }
    }

    /// The bucketed sort is the plain one: every index in the order of its
    /// `(key, index)` pair, on one thread and on several. Clustered points and
    /// shared cells are what the buckets must not disturb, so the grid has a
    /// dense cluster with repeats beside a sparse scatter, and depths both
    /// above and below the bucket count's.
    #[test]
    fn the_hilbert_order_is_the_sorted_order_of_its_keys() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut draw = |range: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % u64::from(range)) as u32
        };
        for axis_bits in [3u32, 10, 16] {
            let side = 1u32 << axis_bits;
            let cells: [Vec<u32>; 3] = {
                let mut cells = [Vec::new(), Vec::new(), Vec::new()];
                for point in 0..50_000 {
                    for axis in &mut cells {
                        let value = if point % 3 == 0 {
                            draw(side)
                        } else {
                            side / 2 + draw(side.min(16))
                        };
                        axis.push(value.min(side - 1));
                    }
                }
                cells
            };
            let grid = Grid { axis_bits, cells };
            let mut expected: Vec<(u64, u32)> = (0..grid.len())
                .map(|p| {
                    let key = hilbert_key(
                        grid.cells[0][p],
                        grid.cells[1][p],
                        grid.cells[2][p],
                        axis_bits,
                    );
                    (key, p as u32)
                })
                .collect();
            expected.sort_unstable();
            let expected: Vec<u32> = expected.into_iter().map(|(_, p)| p).collect();
            for threads in [1, 4] {
                assert_eq!(
                    grid.hilbert_order(threads),
                    expected,
                    "{axis_bits} bits, {threads} threads"
                );
            }
        }
    }

    #[test]
    fn the_grid_follows_the_positions_quantization() {
        let mut options = EncoderOptions::new();
        for bits in [4, 8, 14, 16, 21] {
            options.set_attribute_int(0, "quantization_bits", bits);
            assert_eq!(curve_axis_bits(&options, 0), bits as u32);
        }

        // Past what a u64 key can hold, the grid stops rather than wrapping.
        options.set_attribute_int(0, "quantization_bits", 30);
        assert_eq!(curve_axis_bits(&options, 0), 21);

        // Unquantized positions keep every bit they arrived with, so the
        // finest grid is the one that matches them.
        let options = EncoderOptions::new();
        assert_eq!(curve_axis_bits(&options, 0), 21);
    }

    #[test]
    fn a_quarter_bit_is_a_quarter_of_a_bit_of_the_logarithm() {
        // d = 0 costs nothing, d = 1 one bit, d = 3 two, d = 255 eight.
        assert_eq!(quarter_bits(0), 0);
        assert_eq!(quarter_bits(1), 4);
        assert_eq!(quarter_bits(3), 8);
        assert_eq!(quarter_bits(255), 32);
        assert_eq!(quarter_bits(65535), 64);
        for d in 0..70000u32 {
            let exact = (1.0 + f64::from(d)).log2() * 4.0;
            let got = f64::from(quarter_bits(d));
            // Linear between powers of two where the logarithm bends, so never
            // above it and under it by at most the bend plus the rounding.
            assert!(
                got <= exact + 1e-9 && exact - got < 1.4,
                "d = {d}: {got} against {exact}"
            );
        }
    }

    // -- clouds ------------------------------------------------------------

    struct Xorshift(u64);

    impl Xorshift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A cloud of `positions` with `extras` single-component float attributes.
    fn cloud(positions: &[[f32; 3]], extras: &[Vec<f32>]) -> (PointCloud, EncoderOptions) {
        let mut pc = PointCloud::new();
        pc.set_num_points(positions.len());
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            positions.len(),
        );
        for (p, position) in positions.iter().enumerate() {
            for (axis, value) in position.iter().enumerate() {
                attribute
                    .buffer_mut()
                    .write((p * 3 + axis) * 4, &value.to_le_bytes());
            }
        }
        pc.add_attribute(attribute);
        let mut options = EncoderOptions::new();
        options.set_attribute_int(0, "quantization_bits", 14);
        for (k, values) in extras.iter().enumerate() {
            let mut attribute = PointAttribute::new();
            attribute.init(
                GeometryAttributeType::Generic,
                1,
                DataType::Float32,
                false,
                positions.len(),
            );
            for (p, value) in values.iter().enumerate() {
                attribute.buffer_mut().write(p * 4, &value.to_le_bytes());
            }
            pc.add_attribute(attribute);
            options.set_attribute_int(k as i32 + 1, "quantization_bits", 8);
        }
        (pc, options)
    }

    /// Points on a helix, their attributes smooth along it: a file order that
    /// is already coherent, as a scan's is.
    fn helix(points: usize) -> (Vec<[f32; 3]>, Vec<Vec<f32>>) {
        let positions: Vec<[f32; 3]> = (0..points)
            .map(|i| {
                let t = i as f32 / points as f32 * 60.0;
                [t.cos() * 10.0, t.sin() * 10.0, t * 0.2]
            })
            .collect();
        let extras = (0..6)
            .map(|k| {
                (0..points)
                    .map(|i| ((i as f32 / points as f32) * (3.0 + k as f32)).sin())
                    .collect()
            })
            .collect();
        (positions, extras)
    }

    fn shuffled(
        positions: &[[f32; 3]],
        extras: &[Vec<f32>],
        seed: u64,
    ) -> (Vec<[f32; 3]>, Vec<Vec<f32>>) {
        let mut rng = Xorshift(seed);
        let mut order: Vec<usize> = (0..positions.len()).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.next() as usize % (i + 1));
        }
        (
            order.iter().map(|&i| positions[i]).collect(),
            extras
                .iter()
                .map(|e| order.iter().map(|&i| e[i]).collect())
                .collect(),
        )
    }

    fn is_a_permutation(order: &[PointIndex], count: usize) -> bool {
        let mut seen = vec![false; count];
        order.len() == count
            && order.iter().all(|p| {
                let first = !seen[p.0 as usize];
                seen[p.0 as usize] = true;
                first
            })
    }

    #[test]
    fn an_order_that_is_already_coherent_is_left_alone() {
        let (positions, extras) = helix(40_000);
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        assert!(search(&pc, &options).is_none());
    }

    #[test]
    fn the_same_points_in_no_order_are_reordered_into_a_permutation() {
        let (positions, extras) = helix(40_000);
        let (positions, extras) = shuffled(&positions, &extras, 7);
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        let order = search(&pc, &options).expect("a shuffled cloud is worth reordering");
        assert!(is_a_permutation(&order, positions.len()));
    }

    /// The estimate the search minimizes, over every step, on the position and
    /// every extra: what a reorder is for.
    fn path_estimate(positions: &[[f32; 3]], extras: &[Vec<f32>], order: &[PointIndex]) -> u64 {
        let quantize = |values: &[f32]| -> Vec<u8> {
            let low = values.iter().copied().fold(f32::INFINITY, f32::min);
            let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            values
                .iter()
                .map(|&v| ((v - low) / (high - low) * 255.0 + 0.5) as u8)
                .collect()
        };
        let columns: Vec<Vec<u8>> = extras.iter().map(|e| quantize(e)).collect();
        let axes: Vec<Vec<u16>> = (0..3)
            .map(|axis| {
                let values: Vec<f32> = positions.iter().map(|p| p[axis]).collect();
                let low = values.iter().copied().fold(f32::INFINITY, f32::min);
                let high = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                values
                    .iter()
                    .map(|&v| ((v - low) / (high - low) * 65535.0) as u16)
                    .collect()
            })
            .collect();
        order
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0].0 as usize, w[1].0 as usize);
                let position: u64 = axes
                    .iter()
                    .map(|x| u64::from(quarter_bits(u32::from(x[a].abs_diff(x[b])))))
                    .sum();
                let rest: u64 = columns
                    .iter()
                    .map(|c| u64::from(quarter_bits(u32::from(c[a].abs_diff(c[b])))))
                    .sum();
                position + rest
            })
            .sum()
    }

    #[test]
    fn a_reorder_is_cheaper_by_the_estimate_than_the_order_it_replaced() {
        let (positions, extras) = helix(40_000);
        let (positions, extras) = shuffled(&positions, &extras, 11);
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        let order = search(&pc, &options).expect("reordered");
        let identity: Vec<PointIndex> = (0..positions.len() as u32).map(PointIndex).collect();
        let before = path_estimate(&positions, &extras, &identity);
        let after = path_estimate(&positions, &extras, &order);
        assert!(
            after * 2 < before,
            "{after} against {before}: a shuffle should lose more than half"
        );
    }

    #[test]
    fn the_order_does_not_depend_on_the_number_of_threads() {
        let (positions, extras) = helix(70_000);
        let (positions, extras) = shuffled(&positions, &extras, 3);
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        options.set_threads(1);
        let single = search(&pc, &options).expect("reordered");
        for threads in [2, 5, 16] {
            options.set_threads(threads);
            assert_eq!(
                search(&pc, &options).expect("reordered"),
                single,
                "{threads} threads"
            );
        }
    }

    #[test]
    fn a_slower_encoding_speed_looks_harder_and_finds_no_worse() {
        let (positions, extras) = helix(30_000);
        let (positions, extras) = shuffled(&positions, &extras, 5);
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        let mut previous = u64::MAX;
        for speed in [9, 7, 5, 3, 0] {
            options.set_global_int("encoding_speed", speed);
            let order = search(&pc, &options).expect("reordered");
            let estimate = path_estimate(&positions, &extras, &order);
            assert!(
                estimate <= previous.saturating_add(previous / 50),
                "speed {speed}: {estimate} after {previous}"
            );
            previous = estimate;
        }
    }

    /// Every point of a lattice four times over: the curve already puts each
    /// point next to its copies, and what refining it finds after that is a
    /// fraction of a percent in estimate, which the coder turns into a larger
    /// stream. The refinement has to beat the curve as well as the input, so
    /// the curve is what is written.
    #[test]
    fn a_refinement_that_barely_beats_the_curve_leaves_the_curve() {
        let side = 30;
        let positions: Vec<[f32; 3]> = (0..side * side * side * 4)
            .map(|i| {
                let cell = i / 4;
                [
                    (cell % side) as f32,
                    (cell / side % side) as f32,
                    (cell / side / side) as f32,
                ]
            })
            .collect();
        let (positions, extras) = shuffled(&positions, &[], 7);
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        assert_eq!(search(&pc, &options), curve(&pc, &options));
    }

    /// A scanner's time as integer ticks, a thousand a point over twenty
    /// million: on bytes every step along the scan rounded to nothing, so
    /// breaking the scan could only ever cost the eight bits a byte has. Priced
    /// on the ticks it costs what the coder pays. A `f64` beside it is copied
    /// as it is whatever the order, and counts for nothing.
    #[test]
    fn a_column_finer_than_a_byte_is_priced_at_its_own_resolution() {
        let count = 20_000;
        let positions: Vec<[f32; 3]> = (0..count).map(|i| [i as f32, 0.0, 0.0]).collect();
        let (mut pc, options) = cloud(&positions, &[]);
        let mut ticks = PointAttribute::new();
        ticks.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Int32,
            false,
            count,
        );
        let mut seconds = PointAttribute::new();
        seconds.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Float64,
            false,
            count,
        );
        for p in 0..count {
            ticks
                .buffer_mut()
                .write(p * 4, &(p as i32 * 1_000).to_le_bytes());
            seconds
                .buffer_mut()
                .write(p * 8, &(p as f64 * 1e-3).to_le_bytes());
        }
        pc.add_attribute(ticks);
        pc.add_attribute(seconds);
        let shuffled: Vec<u32> = (0..count as u32)
            .map(|i| i * 7_919 % count as u32)
            .collect();
        let identity: Vec<u32> = (0..count as u32).collect();
        let along_scan = measure_columns(&pc, &options, &identity, 1);
        let along_shuffle = measure_columns(&pc, &options, &shuffled, 1);
        assert_eq!(along_scan.len(), 1, "only the ticks are predicted");
        let steps = (count - 1).div_ceil(SAMPLE_STRIDE) as u64;
        assert_eq!(
            along_scan[0].cost_curve,
            steps * u64::from(quarter_bits(1_000))
        );
        assert!(along_shuffle[0].cost_curve > 2 * along_scan[0].cost_curve);
    }

    /// A scan on a half-metre grid, row after row, its height periodic along a
    /// row and its time a near-constant thousand ticks a point. In scan order
    /// the time steps by the same few values, which the coder writes for next
    /// to nothing and an estimate by step sizes prices at ten bits a step; on
    /// that estimate a reorder looked worth it and wrote three times the
    /// stream. On the coder's own price the scan order stays.
    #[test]
    fn a_scan_whose_steps_repeat_is_left_in_scan_order() {
        let (rows, per_row) = (200, 300);
        let count = rows * per_row;
        let positions: Vec<[f32; 3]> = (0..count)
            .map(|i| {
                let (row, column) = (i / per_row, i % per_row);
                [
                    column as f32 * 0.5,
                    row as f32 * 0.5,
                    (column % 12) as f32 * 0.25,
                ]
            })
            .collect();
        let (mut pc, mut options) = cloud(&positions, &[]);
        let mut ticks = PointAttribute::new();
        ticks.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Int32,
            false,
            count,
        );
        for p in 0..count {
            let time = p as i32 * 1_000 + (p % 3) as i32;
            ticks.buffer_mut().write(p * 4, &time.to_le_bytes());
        }
        pc.add_attribute(ticks);
        options.set_point_order_search(true);
        assert!(search(&pc, &options).is_none());
    }

    /// Points spread evenly through a cube, each carrying values that owe
    /// nothing to its neighbours. Refining the curve lowers the steps of
    /// those values, but the coder writes them as they are whatever the order,
    /// with the prediction search on, so the refinement buys nothing the
    /// curve had not already bought and the curve is written.
    #[test]
    fn values_that_do_not_follow_their_neighbours_leave_the_curve_unrefined() {
        let mut rng = Xorshift(17);
        let mut unit = move || (rng.next() >> 11) as f32 / (1u64 << 53) as f32;
        let count = 40_000;
        let positions: Vec<[f32; 3]> = (0..count).map(|_| [unit(), unit(), unit()]).collect();
        let extras: Vec<Vec<f32>> = (0..4)
            .map(|_| (0..count).map(|_| unit()).collect())
            .collect();
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        options.set_prediction_search(true);
        assert_eq!(search(&pc, &options), curve(&pc, &options));
    }

    #[test]
    fn a_cloud_with_nothing_to_sort_by_or_nothing_worth_sorting_is_declined() {
        let (pc, mut options) = cloud(&[[0.0, 0.0, 0.0]; 10], &[]);
        options.set_point_order_search(true);
        assert!(search(&pc, &options).is_none(), "ten points");
        let mut empty = PointCloud::new();
        empty.set_num_points(1000);
        assert!(search(&empty, &options).is_none(), "no position");
    }

    #[test]
    fn a_cloud_with_a_coordinate_that_is_not_finite_is_declined() {
        let (mut positions, extras) = helix(5_000);
        positions[100][1] = f32::NAN;
        let (pc, mut options) = cloud(&positions, &extras);
        options.set_point_order_search(true);
        assert!(search(&pc, &options).is_none());
        assert!(curve(&pc, &options).is_none());
    }
}
