//! Dynamic integer-point KD-tree.
//!
//! The spatial-partitioning core behind KD-tree point-cloud coding: builds and
//! traverses a KD-tree over integer point coordinates ([`PointDVector`] backs
//! the point storage), emitting and consuming the per-node splits the attribute
//! coders entropy-code. Port of Draco's `dynamic_integer_points_kd_tree_*`.

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::status::DracoError;
#[cfg(feature = "decoder")]
use crate::direct_bit_decoder::DirectBitDecoder;
#[cfg(feature = "encoder")]
use crate::direct_bit_encoder::DirectBitEncoder;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
#[cfg(feature = "decoder")]
use crate::folded_bit32_coder::FoldedBit32Decoder;
#[cfg(feature = "encoder")]
use crate::folded_bit32_coder::FoldedBit32Encoder;
#[cfg(feature = "decoder")]
use crate::rans_bit_decoder::RAnsBitDecoder;
#[cfg(feature = "encoder")]
use crate::rans_bit_encoder::RAnsBitEncoder;

fn most_significant_bit(value: u32) -> u32 {
    debug_assert!(value > 0);
    31 - value.leading_zeros()
}

fn increment_mod(v: u32, m: u32) -> u32 {
    let next = v + 1;
    if next >= m {
        0
    } else {
        next
    }
}

#[derive(Clone)]
pub struct PointDVector {
    data: Vec<u32>,
    num_points: usize,
    dimension: usize,
}

impl PointDVector {
    pub fn new(num_points: usize, dimension: usize) -> Self {
        Self {
            data: vec![0; num_points * dimension],
            num_points,
            dimension,
        }
    }

    pub fn num_points(&self) -> usize {
        self.num_points
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn point(&self, index: usize) -> &[u32] {
        let start = index * self.dimension;
        &self.data[start..start + self.dimension]
    }

    pub fn point_mut(&mut self, index: usize) -> &mut [u32] {
        let start = index * self.dimension;
        &mut self.data[start..start + self.dimension]
    }

    pub fn as_slice(&self) -> &[u32] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u32] {
        &mut self.data
    }

    pub fn swap_points(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        let dim = self.dimension;
        for i in 0..dim {
            self.data.swap(a * dim + i, b * dim + i);
        }
    }

    /// Partitions points in `[begin, end)` by `point[axis] < value`.
    /// Returns split index such that `[begin, split)` are `< value`.
    ///
    /// Which permutation this leaves is part of the bitstream, not an internal
    /// detail: a node holding one or two points writes their remaining bits in
    /// the order the partition left them, so two partitions that agree on the
    /// split index but not on the order produce different files carrying the
    /// same points. Upstream calls `std::partition`, whose permutation the
    /// standard does not specify -- but MSVC's STL and libstdc++ both implement
    /// the same classic two-ended scan, reproduced here: skip the leading
    /// elements that already belong, skip the trailing ones that do not, swap
    /// that pair, repeat.
    pub fn partition(&mut self, begin: usize, end: usize, axis: usize, value: u32) -> usize {
        let mut first = begin;
        let mut last = end;
        loop {
            loop {
                if first == last {
                    return first;
                }
                if self.point(first)[axis] >= value {
                    break;
                }
                first += 1;
            }
            loop {
                // `last > first >= 0` here, so this cannot underflow.
                last -= 1;
                if first == last {
                    return first;
                }
                if self.point(last)[axis] < value {
                    break;
                }
            }
            self.swap_points(first, last);
            first += 1;
        }
    }
}

#[cfg(feature = "encoder")]
enum NumbersEncoder {
    Direct(DirectBitEncoder),
    RAns(RAnsBitEncoder),
    Folded(FoldedBit32Encoder),
}

#[cfg(feature = "encoder")]
impl NumbersEncoder {
    fn start_encoding(&mut self) {
        match self {
            NumbersEncoder::Direct(e) => e.start_encoding(),
            NumbersEncoder::RAns(e) => e.start_encoding(),
            NumbersEncoder::Folded(e) => e.start_encoding(),
        }
    }

    fn encode_least_significant_bits32(&mut self, nbits: u32, value: u32) {
        match self {
            NumbersEncoder::Direct(e) => e.encode_least_significant_bits32(nbits, value),
            NumbersEncoder::RAns(e) => e.encode_least_significant_bits32(nbits, value),
            NumbersEncoder::Folded(e) => e.encode_least_significant_bits32(nbits, value),
        }
    }

    fn end_encoding(&mut self, target_buffer: &mut EncoderBuffer) {
        match self {
            NumbersEncoder::Direct(e) => e.end_encoding(target_buffer),
            NumbersEncoder::RAns(e) => e.end_encoding(target_buffer),
            NumbersEncoder::Folded(e) => e.end_encoding(target_buffer),
        }
    }
}

#[cfg(feature = "encoder")]
pub struct DynamicIntegerPointsKdTreeEncoder {
    compression_level: u8,
    bit_length: u32,
    dimension: u32,
    deviations: Vec<u32>,
    num_remaining_bits: Vec<u32>,
    axes: Vec<u32>,
    base_stack: Vec<u32>,
    levels_stack: Vec<u32>,
    numbers_encoder: NumbersEncoder,
    remaining_bits_encoder: DirectBitEncoder,
    axis_encoder: DirectBitEncoder,
    half_encoder: DirectBitEncoder,
}

#[cfg(feature = "encoder")]
impl DynamicIntegerPointsKdTreeEncoder {
    pub fn new(compression_level: u8, dimension: u32) -> Self {
        assert!(compression_level <= 6);
        let stack_len = (32 * dimension + 1) as usize;

        let numbers_encoder = match compression_level {
            0 | 1 => NumbersEncoder::Direct(DirectBitEncoder::new()),
            2 | 3 => NumbersEncoder::RAns(RAnsBitEncoder::new()),
            4..=6 => NumbersEncoder::Folded(FoldedBit32Encoder::new()),
            _ => unreachable!(),
        };

        Self {
            compression_level,
            bit_length: 0,
            dimension,
            deviations: vec![0; dimension as usize],
            num_remaining_bits: vec![0; dimension as usize],
            axes: vec![0; dimension as usize],
            base_stack: vec![0; stack_len * dimension as usize],
            levels_stack: vec![0; stack_len * dimension as usize],
            numbers_encoder,
            remaining_bits_encoder: DirectBitEncoder::new(),
            axis_encoder: DirectBitEncoder::new(),
            half_encoder: DirectBitEncoder::new(),
        }
    }

    pub fn encode_points(
        &mut self,
        points: &mut PointDVector,
        bit_length: u32,
        buffer: &mut EncoderBuffer,
    ) {
        self.bit_length = bit_length;
        buffer.encode_u32(self.bit_length);
        buffer.encode_u32(points.num_points() as u32);
        if points.num_points() == 0 {
            return;
        }

        self.numbers_encoder.start_encoding();
        self.remaining_bits_encoder.start_encoding();
        self.axis_encoder.start_encoding();
        self.half_encoder.start_encoding();

        self.encode_internal(points);

        self.numbers_encoder.end_encoding(buffer);
        self.remaining_bits_encoder.end_encoding(buffer);
        self.axis_encoder.end_encoding(buffer);
        self.half_encoder.end_encoding(buffer);
    }

    fn get_and_encode_axis(
        &mut self,
        points: &PointDVector,
        begin: usize,
        end: usize,
        old_base: &[u32],
        levels: &[u32],
        last_axis: u32,
    ) -> u32 {
        if self.compression_level != 6 {
            return increment_mod(last_axis, self.dimension);
        }

        let size = (end - begin) as u32;
        debug_assert!(size != 0);

        let mut best_axis = 0u32;
        if size < 64 {
            for axis in 1..self.dimension {
                if levels[best_axis as usize] > levels[axis as usize] {
                    best_axis = axis;
                }
            }
        } else {
            for i in 0..self.dimension as usize {
                self.deviations[i] = 0;
                self.num_remaining_bits[i] = self.bit_length - levels[i];
                if self.num_remaining_bits[i] > 0 {
                    let split = old_base[i] + (1u32 << (self.num_remaining_bits[i] - 1));
                    let mut cnt = 0u32;
                    for p in begin..end {
                        if points.point(p)[i] < split {
                            cnt += 1;
                        }
                    }
                    let other = size - cnt;
                    self.deviations[i] = if other > cnt { other } else { cnt };
                }
            }

            let mut max_value = 0u32;
            best_axis = 0;
            for i in 0..self.dimension as usize {
                if self.num_remaining_bits[i] != 0 && self.deviations[i] > max_value {
                    max_value = self.deviations[i];
                    best_axis = i as u32;
                }
            }
            self.axis_encoder
                .encode_least_significant_bits32(4, best_axis);
        }

        best_axis
    }

    fn encode_number(&mut self, nbits: u32, value: u32) {
        self.numbers_encoder
            .encode_least_significant_bits32(nbits, value);
    }

    fn encode_internal(&mut self, points: &mut PointDVector) {
        #[derive(Clone, Copy)]
        struct Status {
            begin: usize,
            end: usize,
            last_axis: u32,
            stack_pos: usize,
        }

        let dimension = self.dimension as usize;
        self.base_stack[0..dimension].fill(0);
        self.levels_stack[0..dimension].fill(0);
        let mut old_base = vec![0; dimension];
        let mut levels = vec![0; dimension];

        let mut stack: Vec<Status> = Vec::new();
        stack.push(Status {
            begin: 0,
            end: points.num_points(),
            last_axis: 0,
            stack_pos: 0,
        });

        while let Some(status) = stack.pop() {
            let begin = status.begin;
            let end = status.end;
            let last_axis = status.last_axis;
            let stack_pos = status.stack_pos;

            let row_start = stack_pos * dimension;
            old_base.copy_from_slice(&self.base_stack[row_start..row_start + dimension]);
            levels.copy_from_slice(&self.levels_stack[row_start..row_start + dimension]);

            let axis = self.get_and_encode_axis(points, begin, end, &old_base, &levels, last_axis);
            let level = levels[axis as usize];
            let num_remaining_points = (end - begin) as u32;

            if (self.bit_length - level) == 0 {
                continue;
            }

            if num_remaining_points <= 2 {
                self.axes[0] = axis;
                for i in 1..self.dimension as usize {
                    self.axes[i] = increment_mod(self.axes[i - 1], self.dimension);
                }
                for p in begin..end {
                    let point = points.point(p);
                    for j in 0..self.dimension as usize {
                        let num_bits = self.bit_length - levels[self.axes[j] as usize];
                        if num_bits != 0 {
                            self.remaining_bits_encoder.encode_least_significant_bits32(
                                num_bits,
                                point[self.axes[j] as usize],
                            );
                        }
                    }
                }
                continue;
            }

            let num_remaining_bits = self.bit_length - level;
            let modifier = 1u32 << (num_remaining_bits - 1);
            let child_start = (stack_pos + 1) * dimension;
            self.base_stack[child_start..child_start + dimension].copy_from_slice(&old_base);
            self.base_stack[child_start + axis as usize] += modifier;
            let new_base_axis_value = self.base_stack[child_start + axis as usize];

            let split = points.partition(begin, end, axis as usize, new_base_axis_value);

            let required_bits = most_significant_bit(num_remaining_points);
            let first_half = (split - begin) as u32;
            let second_half = (end - split) as u32;
            let left = first_half < second_half;

            if first_half != second_half {
                self.half_encoder.encode_bit(left);
            }

            if left {
                self.encode_number(required_bits, num_remaining_points / 2 - first_half);
            } else {
                self.encode_number(required_bits, num_remaining_points / 2 - second_half);
            }

            levels[axis as usize] += 1;
            self.levels_stack[row_start..row_start + dimension].copy_from_slice(&levels);
            self.levels_stack[child_start..child_start + dimension].copy_from_slice(&levels);

            if split != begin {
                stack.push(Status {
                    begin,
                    end: split,
                    last_axis: axis,
                    stack_pos,
                });
            }
            if split != end {
                stack.push(Status {
                    begin: split,
                    end,
                    last_axis: axis,
                    stack_pos: stack_pos + 1,
                });
            }
        }
    }
}

#[cfg(feature = "decoder")]
enum NumbersDecoder<'a> {
    Direct(DirectBitDecoder),
    RAns(RAnsBitDecoder<'a>),
    Folded(FoldedBit32Decoder<'a>),
}

#[cfg(feature = "decoder")]
impl<'a> NumbersDecoder<'a> {
    fn start_decoding(&mut self, buffer: &mut DecoderBuffer<'a>) -> bool {
        match self {
            NumbersDecoder::Direct(d) => d.start_decoding(buffer),
            NumbersDecoder::RAns(d) => d.start_decoding(buffer),
            NumbersDecoder::Folded(d) => d.start_decoding(buffer),
        }
    }

    fn decode_least_significant_bits32(&mut self, nbits: u32, value: &mut u32) -> bool {
        match self {
            NumbersDecoder::Direct(d) => d.decode_least_significant_bits32(nbits, value),
            NumbersDecoder::RAns(d) => d.decode_least_significant_bits32(nbits as i32, value),
            NumbersDecoder::Folded(d) => d.decode_least_significant_bits32(nbits, value),
        }
    }

    fn end_decoding(&mut self) {
        match self {
            NumbersDecoder::Direct(d) => d.end_decoding(),
            NumbersDecoder::RAns(d) => d.end_decoding(),
            NumbersDecoder::Folded(d) => d.end_decoding(),
        }
    }
}

#[cfg(feature = "decoder")]
pub struct DynamicIntegerPointsKdTreeDecoder<'a> {
    compression_level: u8,
    bit_length: u32,
    num_points: u32,
    num_decoded_points: u32,
    dimension: u32,
    p: Vec<u32>,
    axes: Vec<u32>,
    base_stack: Vec<u32>,
    levels_stack: Vec<u32>,
    numbers_decoder: NumbersDecoder<'a>,
    remaining_bits_decoder: DirectBitDecoder,
    axis_decoder: DirectBitDecoder,
    half_decoder: DirectBitDecoder,
}

#[cfg(feature = "decoder")]
impl<'a> DynamicIntegerPointsKdTreeDecoder<'a> {
    pub fn new(compression_level: u8, dimension: u32) -> Self {
        assert!(compression_level <= 6);
        let stack_len = (32 * dimension + 1) as usize;
        let numbers_decoder = match compression_level {
            0 | 1 => NumbersDecoder::Direct(DirectBitDecoder::new()),
            2 | 3 => NumbersDecoder::RAns(RAnsBitDecoder::new()),
            4..=6 => NumbersDecoder::Folded(FoldedBit32Decoder::new()),
            _ => unreachable!(),
        };
        Self {
            compression_level,
            bit_length: 0,
            num_points: 0,
            num_decoded_points: 0,
            dimension,
            p: vec![0; dimension as usize],
            axes: vec![0; dimension as usize],
            base_stack: vec![0; stack_len * dimension as usize],
            levels_stack: vec![0; stack_len * dimension as usize],
            numbers_decoder,
            remaining_bits_decoder: DirectBitDecoder::new(),
            axis_decoder: DirectBitDecoder::new(),
            half_decoder: DirectBitDecoder::new(),
        }
    }

    pub fn num_decoded_points(&self) -> u32 {
        self.num_decoded_points
    }

    pub fn decode_points(
        &mut self,
        buffer: &mut DecoderBuffer<'a>,
        oit_max_points: u32,
    ) -> Result<Vec<u32>, DracoError> {
        self.bit_length = buffer
            .decode_u32()
            .map_err(|_| DracoError::buffer("Buffer ran out reading the KD-tree bit length"))?;
        if self.bit_length > 32 {
            return Err(DracoError::general(format!(
                "KD-tree bit length {} above the 32 a u32 coordinate holds",
                self.bit_length
            )));
        }
        self.num_points = buffer
            .decode_u32()
            .map_err(|_| DracoError::buffer("Buffer ran out reading the KD-tree point count"))?;
        if self.num_points == 0 {
            self.num_decoded_points = 0;
            return Ok(Vec::new());
        }
        if self.num_points > oit_max_points {
            return Err(DracoError::general(format!(
                "KD-tree declares {} points against the {oit_max_points} the header allows",
                self.num_points
            )));
        }

        self.num_decoded_points = 0;

        for (name, started) in [
            ("numbers", self.numbers_decoder.start_decoding(buffer)),
            (
                "remaining bits",
                self.remaining_bits_decoder.start_decoding(buffer),
            ),
            ("axis", self.axis_decoder.start_decoding(buffer)),
            ("half", self.half_decoder.start_decoding(buffer)),
        ] {
            if !started {
                return Err(DracoError::general(format!(
                    "Failed to start the KD-tree's {name} decoder"
                )));
            }
        }

        let out_len = (self.num_points as usize)
            .checked_mul(self.dimension as usize)
            .ok_or_else(|| {
                DracoError::general("KD-tree point count times dimension overflows a usize")
            })?;
        let mut out: Vec<u32> = Vec::new();
        // Reserved against the input, not against the count the stream declares:
        // `decode_internal` appends, so everything past this arrives on points
        // that were actually decoded. Eight values per remaining byte is the
        // same allowance the symbol decoders take, and it leaves a header
        // naming millions of points in a few kilobytes reserving kilobytes.
        let reserve = out_len.min(buffer.remaining_size().saturating_mul(8));
        out.try_reserve(reserve)
            .map_err(|_| DracoError::allocation_exceeds_input(reserve * 4, buffer.size()))?;
        if !self.decode_internal(self.num_points, &mut out) {
            return Err(DracoError::general(format!(
                "KD-tree traversal failed after {} of {} points",
                self.num_decoded_points, self.num_points
            )));
        }

        self.numbers_decoder.end_decoding();
        self.remaining_bits_decoder.end_decoding();
        self.axis_decoder.end_decoding();
        self.half_decoder.end_decoding();

        Ok(out)
    }

    fn get_axis(
        &mut self,
        num_remaining_points: u32,
        levels: &[u32],
        last_axis: u32,
    ) -> Option<u32> {
        if self.compression_level != 6 {
            return Some(increment_mod(last_axis, self.dimension));
        }

        let mut best_axis = 0u32;
        if num_remaining_points < 64 {
            for axis in 1..self.dimension {
                if levels[best_axis as usize] > levels[axis as usize] {
                    best_axis = axis;
                }
            }
        } else {
            let mut v = 0u32;
            if !self.axis_decoder.decode_least_significant_bits32(4, &mut v) {
                return None;
            }
            best_axis = v;
        }
        Some(best_axis)
    }

    fn decode_number(&mut self, nbits: u32, value: &mut u32) -> bool {
        self.numbers_decoder
            .decode_least_significant_bits32(nbits, value)
    }

    fn decode_internal(&mut self, num_points: u32, out: &mut Vec<u32>) -> bool {
        #[derive(Clone, Copy)]
        struct Status {
            num_remaining_points: u32,
            last_axis: u32,
            stack_pos: usize,
        }

        let dimension = self.dimension as usize;
        self.base_stack[0..dimension].fill(0);
        self.levels_stack[0..dimension].fill(0);
        let mut old_base = vec![0; dimension];
        let mut levels = vec![0; dimension];

        let mut stack: Vec<Status> = Vec::new();
        stack.push(Status {
            num_remaining_points: num_points,
            last_axis: 0,
            stack_pos: 0,
        });

        while let Some(status) = stack.pop() {
            let num_remaining_points = status.num_remaining_points;
            let last_axis = status.last_axis;
            let stack_pos = status.stack_pos;

            let row_start = stack_pos * dimension;
            old_base.copy_from_slice(&self.base_stack[row_start..row_start + dimension]);
            levels.copy_from_slice(&self.levels_stack[row_start..row_start + dimension]);

            if num_remaining_points > num_points {
                return false;
            }

            let Some(axis) = self.get_axis(num_remaining_points, &levels, last_axis) else {
                return false;
            };
            if axis >= self.dimension {
                return false;
            }

            let level = levels[axis as usize];

            if (self.bit_length - level) == 0 {
                for _ in 0..num_remaining_points {
                    out.extend_from_slice(&old_base);
                    self.num_decoded_points += 1;
                }
                continue;
            }

            if num_remaining_points <= 2 {
                self.axes[0] = axis;
                for i in 1..self.dimension as usize {
                    self.axes[i] = increment_mod(self.axes[i - 1], self.dimension);
                }

                for _ in 0..num_remaining_points {
                    for j in 0..self.dimension as usize {
                        self.p[self.axes[j] as usize] = 0;
                        let num_bits = self.bit_length - levels[self.axes[j] as usize];
                        if num_bits != 0 {
                            let ok = self.remaining_bits_decoder.decode_least_significant_bits32(
                                num_bits,
                                &mut self.p[self.axes[j] as usize],
                            );
                            if !ok {
                                return false;
                            }
                        }
                        self.p[self.axes[j] as usize] |= old_base[self.axes[j] as usize];
                    }
                    out.extend_from_slice(&self.p);
                    self.num_decoded_points += 1;
                }
                continue;
            }

            if self.num_decoded_points > self.num_points {
                return false;
            }

            let num_remaining_bits = self.bit_length - level;
            let modifier = 1u32 << (num_remaining_bits - 1);
            let child_start = (stack_pos + 1) * dimension;
            self.base_stack[child_start..child_start + dimension].copy_from_slice(&old_base);
            self.base_stack[child_start + axis as usize] += modifier;

            let incoming_bits = most_significant_bit(num_remaining_points);
            let mut number = 0u32;
            if !self.decode_number(incoming_bits, &mut number) {
                return false;
            }

            let mut first_half = num_remaining_points / 2;
            if first_half < number {
                return false;
            }
            first_half -= number;
            let mut second_half = num_remaining_points - first_half;

            if first_half != second_half {
                // The loop count comes from the stream, so a tree that claims
                // more splits than the half bits cover used to read zeros past
                // the end and keep building. `DirectBitDecoder` reports the
                // exhaustion exactly; refuse rather than invent the swap.
                let Some(keep_order) = self.half_decoder.decode_next_bit() else {
                    return false;
                };
                if !keep_order {
                    std::mem::swap(&mut first_half, &mut second_half);
                }
            }

            levels[axis as usize] += 1;
            self.levels_stack[row_start..row_start + dimension].copy_from_slice(&levels);
            self.levels_stack[child_start..child_start + dimension].copy_from_slice(&levels);

            if first_half != 0 {
                stack.push(Status {
                    num_remaining_points: first_half,
                    last_axis: axis,
                    stack_pos,
                });
            }
            if second_half != 0 {
                stack.push(Status {
                    num_remaining_points: second_half,
                    last_axis: axis,
                    stack_pos: stack_pos + 1,
                });
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_axis_rejects_truncated_axis_stream() {
        let mut decoder = DynamicIntegerPointsKdTreeDecoder::new(6, 3);
        let levels = [0, 0, 0];

        assert_eq!(decoder.get_axis(64, &levels, 0), None);
    }
}
