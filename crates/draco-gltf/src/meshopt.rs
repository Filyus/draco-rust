//! Decoders for the `EXT_meshopt_compression` bitstreams.
//!
//! The vertex, index and filter codecs are a port of the reference decoders in
//! meshoptimizer (`src/vertexcodec.cpp`, `src/indexcodec.cpp`,
//! `src/vertexfilter.cpp`), Copyright (c) 2016-2025 Arseny Kapoulkine, used
//! under the MIT license. The bitstream is fixed by the glTF extension, so the
//! port follows the reference control flow closely; every read is bounds
//! checked instead of relying on the C decoder's pre-validated cursors.

use crate::gltf_error::{GltfError, Result};

/// Encoded layout of one `EXT_meshopt_compression` buffer view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshoptMode {
    /// Vertex attribute stream, `count` elements of `stride` bytes.
    Attributes,
    /// Triangle index stream restricted to triangle lists.
    Triangles,
    /// Index stream with no connectivity assumptions.
    Indices,
}

/// Reversible transform applied to a decoded attribute stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshoptFilter {
    /// Attribute data is used as decoded.
    #[default]
    None,
    /// Octahedral unit vectors, 4 or 8 byte stride.
    Octahedral,
    /// Quaternions stored with the largest component dropped, 8 byte stride.
    Quaternion,
    /// Floats stored as shared-exponent mantissa pairs.
    Exponential,
    /// RGBA stored as luma / chroma with the scale folded into alpha, 4 or 8
    /// byte stride.
    Color,
}

impl MeshoptMode {
    /// Parses the extension's `mode` string.
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "ATTRIBUTES" => Ok(Self::Attributes),
            "TRIANGLES" => Ok(Self::Triangles),
            "INDICES" => Ok(Self::Indices),
            other => Err(GltfError::Unsupported(format!(
                "EXT_meshopt_compression mode {other}"
            ))),
        }
    }
}

impl MeshoptFilter {
    /// Parses the extension's `filter` string; absent means [`MeshoptFilter::None`].
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "NONE" => Ok(Self::None),
            "OCTAHEDRAL" => Ok(Self::Octahedral),
            "QUATERNION" => Ok(Self::Quaternion),
            "EXPONENTIAL" => Ok(Self::Exponential),
            "COLOR" => Ok(Self::Color),
            other => Err(GltfError::Unsupported(format!(
                "EXT_meshopt_compression filter {other}"
            ))),
        }
    }
}

/// Decodes one compressed buffer view into its `count * stride` bytes.
///
/// `destination` must be exactly the decoded size; `source` is the compressed
/// range named by the extension object.
pub fn decode_buffer_view(
    destination: &mut [u8],
    source: &[u8],
    mode: MeshoptMode,
    filter: MeshoptFilter,
    count: usize,
    stride: usize,
) -> Result<()> {
    let expected = count
        .checked_mul(stride)
        .ok_or_else(|| invalid("buffer view size overflow"))?;
    if destination.len() != expected {
        return Err(invalid(
            "buffer view byteLength does not match count times byteStride",
        ));
    }
    match mode {
        MeshoptMode::Attributes => {
            decode_vertex_buffer(destination, count, stride, source)?;
            apply_filter(destination, filter, count, stride)
        }
        MeshoptMode::Triangles => {
            if filter != MeshoptFilter::None {
                return Err(invalid("index streams cannot carry a filter"));
            }
            decode_index_buffer(destination, count, stride, source)
        }
        MeshoptMode::Indices => {
            if filter != MeshoptFilter::None {
                return Err(invalid("index streams cannot carry a filter"));
            }
            decode_index_sequence(destination, count, stride, source)
        }
    }
}

fn invalid(message: &str) -> GltfError {
    GltfError::InvalidGltf(format!("EXT_meshopt_compression: {message}"))
}

const VERTEX_HEADER: u8 = 0xa0;
const MAX_VERTEX_VERSION: u8 = 1;
const VERTEX_BLOCK_SIZE_BYTES: usize = 8192;
const VERTEX_BLOCK_MAX_SIZE: usize = 256;
const BYTE_GROUP_SIZE: usize = 16;
const BYTE_GROUP_DECODE_LIMIT: usize = 24;
const TAIL_MIN_SIZE_V0: usize = 32;
const TAIL_MIN_SIZE_V1: usize = 24;
const BITS_V0: [u32; 4] = [0, 2, 4, 8];
const BITS_V1: [u32; 5] = [0, 1, 2, 4, 8];

fn vertex_block_size(vertex_size: usize) -> usize {
    let result = (VERTEX_BLOCK_SIZE_BYTES / vertex_size) & !(BYTE_GROUP_SIZE - 1);
    result.min(VERTEX_BLOCK_MAX_SIZE)
}

/// Decodes a vertex attribute stream of `count` elements of `stride` bytes.
pub fn decode_vertex_buffer(
    destination: &mut [u8],
    count: usize,
    stride: usize,
    source: &[u8],
) -> Result<()> {
    if stride == 0 || stride > 256 || !stride.is_multiple_of(4) {
        return Err(invalid(
            "attribute byteStride must be 4..=256 and a multiple of 4",
        ));
    }
    let header = *source
        .first()
        .ok_or_else(|| invalid("empty vertex stream"))?;
    if header & 0xf0 != VERTEX_HEADER {
        return Err(invalid("vertex stream header is invalid"));
    }
    let version = header & 0x0f;
    if version > MAX_VERTEX_VERSION {
        return Err(GltfError::Unsupported(format!(
            "EXT_meshopt_compression vertex codec version {version}"
        )));
    }
    let mut pos = 1usize;

    let tail_size = stride + if version == 0 { 0 } else { stride / 4 };
    let tail_min = if version == 0 {
        TAIL_MIN_SIZE_V0
    } else {
        TAIL_MIN_SIZE_V1
    };
    let tail_padded = tail_size.max(tail_min);
    if source.len() - pos < tail_padded {
        return Err(invalid("vertex stream is truncated"));
    }
    let tail = source.len() - tail_size;

    let mut last_vertex = [0u8; 256];
    last_vertex[..stride].copy_from_slice(&source[tail..tail + stride]);
    let channels = &source[tail + stride..tail + tail_size];
    // Only the XOR mode uses the high nibble for a rotation.
    if channels
        .iter()
        .any(|&channel| channel > 1 && channel & 15 != 2)
    {
        return Err(invalid("vertex stream channel type is invalid"));
    }

    let block_capacity = vertex_block_size(stride);
    let mut scratch = vec![0u8; VERTEX_BLOCK_MAX_SIZE * 4];
    let mut offset = 0usize;
    while offset < count {
        let block = block_capacity.min(count - offset);
        let start = offset * stride;
        pos = decode_vertex_block(
            source,
            pos,
            &mut destination[start..start + block * stride],
            block,
            stride,
            &mut last_vertex,
            channels,
            version,
            &mut scratch,
        )?;
        offset += block;
    }

    if source.len() - pos != tail_padded {
        return Err(invalid("vertex stream has trailing data"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_vertex_block(
    source: &[u8],
    mut pos: usize,
    destination: &mut [u8],
    count: usize,
    stride: usize,
    last_vertex: &mut [u8; 256],
    channels: &[u8],
    version: u8,
    scratch: &mut [u8],
) -> Result<usize> {
    debug_assert!(count > 0 && count <= VERTEX_BLOCK_MAX_SIZE);
    let count_aligned = (count + BYTE_GROUP_SIZE - 1) & !(BYTE_GROUP_SIZE - 1);

    let control_size = if version == 0 { 0 } else { stride / 4 };
    if source.len() - pos < control_size {
        return Err(invalid("vertex block control bytes are truncated"));
    }
    let control = source[pos..pos + control_size].to_vec();
    pos += control_size;

    for k in (0..stride).step_by(4) {
        let control_byte = if version == 0 { 0 } else { control[k / 4] };
        for j in 0..4usize {
            let plane = j * count;
            match (control_byte >> (j * 2)) & 3 {
                3 => {
                    if source.len() - pos < count {
                        return Err(invalid("vertex block literal plane is truncated"));
                    }
                    scratch[plane..plane + count].copy_from_slice(&source[pos..pos + count]);
                    pos += count;
                }
                2 => scratch[plane..plane + count].fill(0),
                control => {
                    let bits: &[u32] = if version == 0 {
                        &BITS_V0
                    } else {
                        &BITS_V1[control as usize..]
                    };
                    pos = decode_bytes(
                        source,
                        pos,
                        &mut scratch[plane..plane + count_aligned],
                        bits,
                    )?;
                }
            }
        }

        let channel = if version == 0 { 0 } else { channels[k / 4] };
        match channel {
            0 => decode_deltas(
                scratch,
                destination,
                count,
                stride,
                last_vertex,
                k,
                1,
                false,
                0,
            ),
            1 => decode_deltas(
                scratch,
                destination,
                count,
                stride,
                last_vertex,
                k,
                2,
                false,
                0,
            ),
            _ => {
                // The channel table has already been validated.
                let rotation = (32 - u32::from(channel >> 4)) & 31;
                decode_deltas(
                    scratch,
                    destination,
                    count,
                    stride,
                    last_vertex,
                    k,
                    4,
                    true,
                    rotation,
                )
            }
        }
    }

    last_vertex[..stride].copy_from_slice(&destination[stride * (count - 1)..stride * count]);
    Ok(pos)
}

/// Reconstructs one four-byte channel group from its transposed byte planes.
#[allow(clippy::too_many_arguments)]
fn decode_deltas(
    scratch: &[u8],
    destination: &mut [u8],
    count: usize,
    stride: usize,
    last_vertex: &[u8; 256],
    k: usize,
    size: usize,
    xor: bool,
    rotation: u32,
) {
    let mask = if size == 4 {
        u32::MAX
    } else {
        (1u32 << (8 * size)) - 1
    };
    let mut plane = 0usize;
    for sub in (0..4).step_by(size) {
        let mut previous = 0u32;
        for byte in 0..size {
            previous |= u32::from(last_vertex[k + sub + byte]) << (8 * byte);
        }
        let mut offset = k + sub;
        for i in 0..count {
            let mut value = 0u32;
            for byte in 0..size {
                value |= u32::from(scratch[plane + i + count * byte]) << (8 * byte);
            }
            value = if xor {
                (value.rotate_left(rotation) ^ previous) & mask
            } else {
                unzigzag(value).wrapping_add(previous) & mask
            };
            for byte in 0..size {
                destination[offset + byte] = (value >> (8 * byte)) as u8;
            }
            previous = value;
            offset += stride;
        }
        plane += count * size;
    }
}

fn unzigzag(value: u32) -> u32 {
    (0u32.wrapping_sub(value & 1)) ^ (value >> 1)
}

fn decode_bytes(
    source: &[u8],
    mut pos: usize,
    destination: &mut [u8],
    bits: &[u32],
) -> Result<usize> {
    debug_assert!(destination.len().is_multiple_of(BYTE_GROUP_SIZE));
    let header_size = (destination.len() / BYTE_GROUP_SIZE).div_ceil(4);
    if source.len() - pos < header_size {
        return Err(invalid("byte group header is truncated"));
    }
    let header = pos;
    pos += header_size;

    for (group, chunk) in destination.chunks_mut(BYTE_GROUP_SIZE).enumerate() {
        if source.len() - pos < BYTE_GROUP_DECODE_LIMIT {
            return Err(invalid("byte group data is truncated"));
        }
        let selector = (source[header + group / 4] >> ((group % 4) * 2)) & 3;
        pos = decode_bytes_group(source, pos, chunk, bits[selector as usize]);
    }
    Ok(pos)
}

/// Expands one 16 byte group; the caller guarantees the worst-case 24 bytes.
fn decode_bytes_group(source: &[u8], pos: usize, destination: &mut [u8], bits: u32) -> usize {
    match bits {
        0 => {
            destination.fill(0);
            pos
        }
        8 => {
            destination.copy_from_slice(&source[pos..pos + BYTE_GROUP_SIZE]);
            pos + BYTE_GROUP_SIZE
        }
        bits => {
            let per_byte = 8 / bits as usize;
            let control_bytes = BYTE_GROUP_SIZE / per_byte;
            let sentinel = (1u8 << bits) - 1;
            let mut extra = pos + control_bytes;
            for group in 0..control_bytes {
                let mut byte = source[pos + group];
                if bits == 1 {
                    // 1-bit groups store their values in reverse bit order.
                    byte = byte.reverse_bits();
                }
                for slot in 0..per_byte {
                    let encoded = byte >> (8 - bits);
                    byte <<= bits;
                    destination[group * per_byte + slot] = if encoded == sentinel {
                        let value = source[extra];
                        extra += 1;
                        value
                    } else {
                        encoded
                    };
                }
            }
            extra
        }
    }
}

const INDEX_HEADER: u8 = 0xe0;
const SEQUENCE_HEADER: u8 = 0xd0;
const MAX_INDEX_VERSION: u8 = 1;

/// Decodes a triangle-list index stream of `count` indices of `size` bytes.
pub fn decode_index_buffer(
    destination: &mut [u8],
    count: usize,
    size: usize,
    source: &[u8],
) -> Result<()> {
    if !count.is_multiple_of(3) {
        return Err(invalid("triangle index count is not a multiple of 3"));
    }
    if size != 2 && size != 4 {
        return Err(invalid("index byteStride must be 2 or 4"));
    }
    if source.len() < 1 + count / 3 + 16 {
        return Err(invalid("index stream is truncated"));
    }
    if source[0] & 0xf0 != INDEX_HEADER {
        return Err(invalid("index stream header is invalid"));
    }
    let version = source[0] & 0x0f;
    if version > MAX_INDEX_VERSION {
        return Err(GltfError::Unsupported(format!(
            "EXT_meshopt_compression index codec version {version}"
        )));
    }

    let mut edge_fifo = IndexFifo::new();
    let mut vertex_fifo = IndexFifo::new();
    let mut next = 0u32;
    let mut last = 0u32;
    let fec_max = if version >= 1 { 13 } else { 15 };

    let code_end = 1 + count / 3;
    let mut data = code_end;
    let safe_end = source.len() - 16;
    let table = &source[safe_end..];
    if table[14..] != [0, 0]
        || table[..14]
            .iter()
            .any(|&entry| entry >> 4 == 15 || entry & 15 == 15)
    {
        return Err(invalid("index stream codeaux table is invalid"));
    }
    let mut written = 0usize;

    for code in 1..code_end {
        let code_tri = source[code];
        if code_tri < 0xf0 {
            let fe = usize::from(code_tri >> 4);
            let [a, b] = edge_fifo.read(1 + fe)?;
            let c;

            let fec = i32::from(code_tri & 15);
            if fec < fec_max {
                c = if fec == 0 {
                    next
                } else {
                    vertex_fifo.read(1 + fec as usize)?
                };
                let first = usize::from(fec == 0);
                next += u32::from(fec == 0);
                vertex_fifo.push(c, first);
            } else {
                if data > safe_end {
                    return Err(invalid("index stream data is truncated"));
                }
                c = if fec != 15 {
                    last.wrapping_add((fec * 2 - 27) as u32)
                } else {
                    decode_index(source, &mut data, last)?
                };
                last = c;
                vertex_fifo.push(c, 1);
            }

            edge_fifo.push([c, b], 1);
            edge_fifo.push([a, c], 1);
            write_triangle(destination, &mut written, size, a, b, c);
        } else if code_tri < 0xfe {
            let code_aux = table[usize::from(code_tri & 15)];
            let feb = usize::from(code_aux >> 4);
            let fec = usize::from(code_aux & 15);

            let a = next;
            next += 1;

            let b = if feb == 0 {
                next
            } else {
                vertex_fifo.read(feb)?
            };
            let feb0 = usize::from(feb == 0);
            next += feb0 as u32;

            let c = if fec == 0 {
                next
            } else {
                vertex_fifo.read(fec)?
            };
            let fec0 = usize::from(fec == 0);
            next += fec0 as u32;

            write_triangle(destination, &mut written, size, a, b, c);

            vertex_fifo.push(a, 1);
            vertex_fifo.push(b, feb0);
            vertex_fifo.push(c, fec0);

            edge_fifo.push([b, a], 1);
            edge_fifo.push([c, b], 1);
            edge_fifo.push([a, c], 1);
        } else {
            if data > safe_end {
                return Err(invalid("index stream data is truncated"));
            }
            let code_aux = source[data];
            data += 1;

            let fea = if code_tri == 0xfe { 0usize } else { 15 };
            let feb = usize::from(code_aux >> 4);
            let fec = usize::from(code_aux & 15);

            // A codeaux of 0 outside the table is the encoder's index reset.
            if code_aux == 0 {
                next = 0;
            }

            let mut read_index = |code| match code {
                0 => {
                    let index = next;
                    next += 1;
                    Ok(index)
                }
                15 => {
                    last = decode_index(source, &mut data, last)?;
                    Ok(last)
                }
                distance => vertex_fifo.read(distance),
            };
            let a = read_index(fea)?;
            let b = read_index(feb)?;
            let c = read_index(fec)?;

            write_triangle(destination, &mut written, size, a, b, c);

            vertex_fifo.push(a, 1);
            vertex_fifo.push(b, usize::from(feb == 0 || feb == 15));
            vertex_fifo.push(c, usize::from(fec == 0 || fec == 15));

            edge_fifo.push([b, a], 1);
            edge_fifo.push([c, b], 1);
            edge_fifo.push([a, c], 1);
        }
    }

    if data != safe_end {
        return Err(invalid("index stream has trailing data"));
    }
    Ok(())
}

/// Decodes an index stream that carries no triangle connectivity.
pub fn decode_index_sequence(
    destination: &mut [u8],
    count: usize,
    size: usize,
    source: &[u8],
) -> Result<()> {
    if size != 2 && size != 4 {
        return Err(invalid("index byteStride must be 2 or 4"));
    }
    if source.len() < 1 + count + 4 {
        return Err(invalid("index sequence is truncated"));
    }
    if source[0] & 0xf0 != SEQUENCE_HEADER {
        return Err(invalid("index sequence header is invalid"));
    }
    let version = source[0] & 0x0f;
    if version > MAX_INDEX_VERSION {
        return Err(GltfError::Unsupported(format!(
            "EXT_meshopt_compression index codec version {version}"
        )));
    }

    let mut data = 1usize;
    let safe_end = source.len() - 4;
    let mut last = [0u32; 2];

    for i in 0..count {
        if data >= safe_end {
            return Err(invalid("index sequence data is truncated"));
        }
        let value = decode_vbyte(source, &mut data)?;
        let baseline = (value & 1) as usize;
        let value = value >> 1;
        let delta = (value >> 1) ^ (0u32.wrapping_sub(value & 1));
        let index = last[baseline].wrapping_add(delta);
        last[baseline] = index;
        write_index(destination, i * size, size, index);
    }

    if data != safe_end {
        return Err(invalid("index sequence has trailing data"));
    }
    Ok(())
}

struct IndexFifo<T> {
    entries: [Option<T>; 16],
    offset: usize,
}

impl<T: Copy> IndexFifo<T> {
    fn new() -> Self {
        Self {
            entries: [None; 16],
            offset: 0,
        }
    }

    /// Distance 1 addresses the most recent entry before the write cursor.
    fn read(&self, distance: usize) -> Result<T> {
        self.entries[self.offset.wrapping_sub(distance) & 15]
            .ok_or_else(|| invalid("index stream references an unwritten FIFO entry"))
    }

    fn push(&mut self, value: T, advance: usize) {
        // Vertex reuse can write the current slot without advancing the cursor.
        self.entries[self.offset] = Some(value);
        self.offset = (self.offset + advance) & 15;
    }
}

fn decode_vbyte(source: &[u8], pos: &mut usize) -> Result<u32> {
    let lead = *source
        .get(*pos)
        .ok_or_else(|| invalid("variable-length index is truncated"))?;
    *pos += 1;
    if lead < 128 {
        return Ok(u32::from(lead));
    }
    let mut result = u32::from(lead & 127);
    for shift in [7, 14, 21, 28] {
        let group = *source
            .get(*pos)
            .ok_or_else(|| invalid("variable-length index is truncated"))?;
        *pos += 1;
        result |= u32::from(group & 127) << shift;
        if group < 128 {
            return Ok(result);
        }
    }
    Err(invalid("variable-length index is unterminated"))
}

fn decode_index(source: &[u8], pos: &mut usize, last: u32) -> Result<u32> {
    let value = decode_vbyte(source, pos)?;
    let delta = (value >> 1) ^ (0u32.wrapping_sub(value & 1));
    Ok(last.wrapping_add(delta))
}

fn write_triangle(
    destination: &mut [u8],
    written: &mut usize,
    size: usize,
    a: u32,
    b: u32,
    c: u32,
) {
    write_index(destination, *written, size, a);
    write_index(destination, *written + size, size, b);
    write_index(destination, *written + 2 * size, size, c);
    *written += 3 * size;
}

fn write_index(destination: &mut [u8], offset: usize, size: usize, index: u32) {
    if size == 2 {
        destination[offset..offset + 2].copy_from_slice(&(index as u16).to_le_bytes());
    } else {
        destination[offset..offset + 4].copy_from_slice(&index.to_le_bytes());
    }
}

/// Applies one of the reversible attribute filters in place.
pub fn apply_filter(
    data: &mut [u8],
    filter: MeshoptFilter,
    count: usize,
    stride: usize,
) -> Result<()> {
    match filter {
        MeshoptFilter::None => Ok(()),
        MeshoptFilter::Octahedral => {
            if stride != 4 && stride != 8 {
                return Err(invalid("OCTAHEDRAL filter needs a 4 or 8 byte stride"));
            }
            filter_octahedral(data, count, stride);
            Ok(())
        }
        MeshoptFilter::Quaternion => {
            if stride != 8 {
                return Err(invalid("QUATERNION filter needs an 8 byte stride"));
            }
            filter_quaternion(data, count);
            Ok(())
        }
        MeshoptFilter::Exponential => {
            if !stride.is_multiple_of(4) {
                return Err(invalid("EXPONENTIAL filter needs a 4 byte aligned stride"));
            }
            filter_exponential(data, count * (stride / 4));
            Ok(())
        }
        MeshoptFilter::Color => {
            if stride != 4 && stride != 8 {
                return Err(invalid("COLOR filter needs a 4 or 8 byte stride"));
            }
            filter_color(data, count, stride);
            Ok(())
        }
    }
}

/// Recovers RGBA from the luma/chroma form the color filter stores.
///
/// Colors are kept as Y/Co/Cg with the per-vertex scale folded into alpha: the
/// alpha channel's highest set bit gives the range the three chroma components
/// were quantized against, so alpha carries both the scale and, one bit lower,
/// the alpha value itself. Co and Cg are signed; Y and alpha are not.
fn filter_color(data: &mut [u8], count: usize, stride: usize) {
    let component = stride / 4;
    let max = if component == 1 {
        f32::from(u8::MAX)
    } else {
        f32::from(u16::MAX)
    };
    for i in 0..count {
        let base = i * stride;
        let unsigned =
            |data: &[u8], index: usize| read_unsigned(data, base + index * component, component);
        let signed =
            |data: &[u8], index: usize| read_signed(data, base + index * component, component);

        // Smear the highest set bit of alpha down: the result is the range the
        // chroma components were quantized against.
        let mut scale = unsigned(data, 3);
        scale |= scale >> 1;
        scale |= scale >> 2;
        scale |= scale >> 4;
        scale |= scale >> 8;

        let y = unsigned(data, 0);
        let co = signed(data, 1);
        let cg = signed(data, 2);
        let r = y + co - cg;
        let g = y + cg;
        let b = y - co - cg;

        // Alpha gains the bit the scale took from it, so it spans the same
        // range as the three colour components before scaling.
        let alpha = unsigned(data, 3);
        let a = ((alpha << 1) & scale) | (alpha & 1);

        let factor = max / scale as f32;
        let round = |value: i32| (value as f32 * factor + 0.5) as i32;
        for (index, value) in [r, g, b, a].into_iter().enumerate() {
            write_unsigned(data, base + index * component, component, round(value));
        }
    }
}

fn filter_octahedral(data: &mut [u8], count: usize, stride: usize) {
    let component = stride / 4;
    let max = if component == 1 {
        f32::from(i8::MAX)
    } else {
        f32::from(i16::MAX)
    };
    for i in 0..count {
        let base = i * stride;
        let read = |index: usize| read_signed(data, base + index * component, component);
        let x = read(0) as f32;
        let y = read(1) as f32;
        let z = read(2) as f32 - x.abs() - y.abs();

        // Points below the octahedron's equator fold back over its edges.
        let t = z.min(0.0);
        let x = x + if x >= 0.0 { t } else { -t };
        let y = y + if y >= 0.0 { t } else { -t };

        let scale = max / (x * x + y * y + z * z).sqrt();
        write_signed(data, base, component, round_signed(x * scale));
        write_signed(data, base + component, component, round_signed(y * scale));
        write_signed(
            data,
            base + 2 * component,
            component,
            round_signed(z * scale),
        );
    }
}

fn filter_quaternion(data: &mut [u8], count: usize) {
    let scale = 32767.0 / 2.0f32.sqrt();
    for i in 0..count {
        let base = i * 8;
        let input = [
            read_signed(data, base, 2) as f32,
            read_signed(data, base + 2, 2) as f32,
            read_signed(data, base + 4, 2) as f32,
        ];
        let packed = read_signed(data, base + 6, 2);

        // The low two bits name the dropped component; the rest is the scale.
        let s = (packed | 3) as f32;
        let ww = s * s * 2.0 - input[0] * input[0] - input[1] * input[1] - input[2] * input[2];
        let w = ww.max(0.0).sqrt();
        let ss = scale / s;

        let component = (packed & 3) as usize;
        for (axis, value) in input.iter().enumerate() {
            let slot = (component + axis + 1) & 3;
            write_signed(data, base + slot * 2, 2, round_signed(value * ss));
        }
        write_signed(data, base + component * 2, 2, round_signed(w * ss));
    }
}

fn filter_exponential(data: &mut [u8], count: usize) {
    for i in 0..count {
        let base = i * 4;
        let value =
            u32::from_le_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        let mantissa = ((value << 8) as i32) >> 8;
        let exponent = (value as i32) >> 24;
        // ldexp(mantissa, exponent) without touching libm.
        let scale = f32::from_bits(((exponent + 127) as u32) << 23);
        data[base..base + 4].copy_from_slice(&(scale * mantissa as f32).to_bits().to_le_bytes());
    }
}

fn read_signed(data: &[u8], offset: usize, size: usize) -> i32 {
    if size == 1 {
        i32::from(data[offset] as i8)
    } else {
        i32::from(i16::from_le_bytes([data[offset], data[offset + 1]]))
    }
}

fn write_signed(data: &mut [u8], offset: usize, size: usize, value: i32) {
    if size == 1 {
        data[offset] = value as u8;
    } else {
        data[offset..offset + 2].copy_from_slice(&(value as i16).to_le_bytes());
    }
}

fn read_unsigned(data: &[u8], offset: usize, size: usize) -> i32 {
    if size == 1 {
        i32::from(data[offset])
    } else {
        i32::from(u16::from_le_bytes([data[offset], data[offset + 1]]))
    }
}

fn write_unsigned(data: &mut [u8], offset: usize, size: usize, value: i32) {
    if size == 1 {
        data[offset] = value as u8;
    } else {
        data[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
    }
}

fn round_signed(value: f32) -> i32 {
    (value + if value >= 0.0 { 0.5 } else { -0.5 }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a version 0 vertex stream from raw per-byte delta planes.
    ///
    /// Every byte group is stored literally, which is the one encoding a test
    /// can spell out without pulling in an encoder.
    fn vertex_stream(planes: &[Vec<u8>], baseline: &[u8]) -> Vec<u8> {
        let count = planes[0].len();
        let aligned = (count + BYTE_GROUP_SIZE - 1) & !(BYTE_GROUP_SIZE - 1);
        let mut stream = vec![VERTEX_HEADER];
        for plane in planes {
            let groups = aligned / BYTE_GROUP_SIZE;
            let mut header = vec![0u8; groups.div_ceil(4)];
            for group in 0..groups {
                // Selector 3 is the literal 8-bit encoding in kBitsV0.
                header[group / 4] |= 3 << ((group % 4) * 2);
            }
            stream.extend_from_slice(&header);
            stream.extend_from_slice(plane);
            stream.resize(stream.len() + aligned - count, 0);
        }
        // The baseline vertex sits at the very end of the padded tail.
        stream.resize(stream.len() + TAIL_MIN_SIZE_V0 - baseline.len(), 0);
        stream.extend_from_slice(baseline);
        stream
    }

    fn zigzag(value: i8) -> u8 {
        ((value << 1) ^ (value >> 7)) as u8
    }

    #[test]
    fn vertex_deltas_accumulate_from_the_stream_tail() {
        let planes = vec![
            vec![zigzag(1), zigzag(2)],
            vec![zigzag(0), zigzag(-1)],
            vec![zigzag(0), zigzag(0)],
            vec![zigzag(-4), zigzag(0)],
        ];
        let stream = vertex_stream(&planes, &[10, 20, 30, 40]);

        let mut decoded = [0u8; 8];
        decode_vertex_buffer(&mut decoded, 2, 4, &stream).unwrap();

        // Each vertex is the previous one plus the unzigzagged delta, and the
        // first vertex starts from the baseline stored in the tail.
        assert_eq!(decoded, [11, 20, 30, 36, 13, 19, 30, 36]);
    }

    #[test]
    fn vertex_stream_rejects_a_truncated_tail() {
        let planes = vec![vec![0u8], vec![0], vec![0], vec![0]];
        let mut stream = vertex_stream(&planes, &[1, 2, 3, 4]);
        stream.truncate(stream.len() - 1);

        let mut decoded = [0u8; 4];
        assert!(decode_vertex_buffer(&mut decoded, 1, 4, &stream).is_err());
    }

    #[test]
    fn vertex_stream_rejects_reserved_channel_bits() {
        // One v1 vertex with four literal delta bytes and a padded tail.
        let mut stream = [0; 6 + TAIL_MIN_SIZE_V1];
        stream[..6].copy_from_slice(&[VERTEX_HEADER | 1, 0xff, 2, 0, 0, 0]);
        let channel_offset = stream.len() - 1;
        let mut decoded = [0; 4];
        for channel in [0x03, 0x04, 0x10, 0x11] {
            stream[channel_offset] = channel;
            let result = decode_vertex_buffer(&mut decoded, 1, 4, &stream);
            assert!(
                matches!(result, Err(GltfError::InvalidGltf(_))),
                "channel {channel:#04x}: {result:?}"
            );
        }

        // An XOR channel can use the high nibble to rotate the delta.
        stream[channel_offset] = 0x12;
        decode_vertex_buffer(&mut decoded, 1, 4, &stream).unwrap();
        assert_eq!(decoded, [1, 0, 0, 0]);
    }

    #[test]
    fn index_buffer_decodes_a_restarted_triangle() {
        // 0xfe selects the slow path with a full codeaux byte; a zero codeaux
        // resets the index counter and emits the next three fresh indices.
        let mut stream = vec![INDEX_HEADER | 1, 0xfe, 0x00];
        stream.resize(stream.len() + 16, 0);

        let mut decoded = [0u8; 6];
        decode_index_buffer(&mut decoded, 3, 2, &stream).unwrap();

        assert_eq!(decoded, [0, 0, 1, 0, 2, 0]);
    }

    #[test]
    fn index_buffer_rejects_unwritten_fifo_entries() {
        let cases: &[(&[u8], &[u8], u8)] = &[
            (&[0x10], &[], 0),        // Edge reuse before the first push.
            (&[0xf0, 0x30], &[], 0),  // Edge reuse beyond the three pushed edges.
            (&[0xf0, 0x03], &[], 0),  // Edge reuse with a missing third vertex.
            (&[0xf0], &[], 0x10),     // Table code reuses b.
            (&[0xf0], &[], 0x01),     // Table code reuses c.
            (&[0xfe], &[0x10], 0),    // Explicit code reuses b.
            (&[0xff], &[0x01, 0], 0), // Explicit code reuses c.
        ];
        for version in [0, 1] {
            for size in [2, 4] {
                for &(codes, data, table_entry) in cases {
                    let mut stream = vec![INDEX_HEADER | version];
                    stream.extend_from_slice(codes);
                    stream.extend_from_slice(data);
                    stream.push(table_entry);
                    stream.extend_from_slice(&[0; 15]);

                    let count = codes.len() * 3;
                    let mut decoded = vec![0; count * size];
                    let result = decode_index_buffer(&mut decoded, count, size, &stream);
                    assert!(
                        matches!(result, Err(GltfError::InvalidGltf(ref message))
                            if message.contains("unwritten FIFO entry")),
                        "version {version}, stride {size}, codes {codes:02x?}: {result:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn index_buffer_rejects_invalid_codeaux_table_entries() {
        for version in [0, 1] {
            let mut stream = [0; 18];
            stream[..2].copy_from_slice(&[INDEX_HEADER | version, 0xf0]);
            // The triangle uses entry zero. Invalid unused entries also matter.
            for (entry, value) in [(1, 0xf0), (1, 0x0f), (14, 1), (15, 1)] {
                stream[2 + entry] = value;
                let mut decoded = [0; 6];
                let result = decode_index_buffer(&mut decoded, 3, 2, &stream);
                assert!(
                    matches!(result, Err(GltfError::InvalidGltf(_))),
                    "version {version}, entry {entry}: {result:?}"
                );
                stream[2 + entry] = 0;
            }
        }
    }

    #[test]
    fn index_buffer_reuses_fifo_entries_after_wraparound() {
        // Six fresh triangles wrap both FIFOs. Then reuse an old edge and
        // allocate c, and use a table code to reuse an old vertex as b.
        let codes = [0xf0, 0xf0, 0xf0, 0xf0, 0xf0, 0xf0, 0xe0, 0xf1];
        let mut expected: Vec<u32> = (0..18).collect();
        expected.extend_from_slice(&[4, 3, 18, 19, 5, 20]);
        for version in [0, 1] {
            let mut stream = vec![INDEX_HEADER | version];
            stream.extend_from_slice(&codes);
            stream.extend_from_slice(&[0, 0xe0]);
            stream.extend_from_slice(&[0; 14]);

            for size in [2, 4] {
                let mut decoded = vec![0; expected.len() * size];
                decode_index_buffer(&mut decoded, expected.len(), size, &stream).unwrap();
                let expected_bytes: Vec<_> = expected
                    .iter()
                    .flat_map(|index| index.to_le_bytes().into_iter().take(size))
                    .collect();
                assert_eq!(decoded, expected_bytes);
            }
        }
    }

    #[test]
    fn index_buffer_decodes_and_reuses_explicit_indices() {
        // Three free indices (MAX, 1, 2), then a table code that reuses MAX.
        for version in [0, 1] {
            let mut stream = vec![INDEX_HEADER | version, 0xff, 0xf0, 0xff, 1, 4, 2, 0x30];
            stream.extend_from_slice(&[0; 15]);
            for size in [2, 4] {
                let mut decoded = vec![0; 6 * size];
                decode_index_buffer(&mut decoded, 6, size, &stream).unwrap();
                let expected: Vec<_> = [u32::MAX, 1, 2, 0, u32::MAX, 1]
                    .into_iter()
                    .flat_map(|index| index.to_le_bytes().into_iter().take(size))
                    .collect();
                assert_eq!(decoded, expected);
            }
        }
    }

    #[test]
    fn index_sequence_decodes_zigzag_deltas() {
        // Each byte is (zigzag(delta) << 1) | baseline selector.
        let stream = vec![SEQUENCE_HEADER, 0x00, 0x04, 0x04, 0, 0, 0, 0];

        let mut decoded = [0u8; 12];
        decode_index_sequence(&mut decoded, 3, 4, &stream).unwrap();

        assert_eq!(
            decoded,
            [0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
            "sequence indices are delta coded against two baselines"
        );
    }

    #[test]
    fn index_streams_require_the_fifth_varint_byte_to_terminate() {
        let mut triangle = [0; 3 + 5 + 16];
        triangle[..3].copy_from_slice(&[INDEX_HEADER | 1, 0xff, 0]);
        triangle[3..8].copy_from_slice(&[0x80, 0x80, 0x80, 0x80, 1]);
        let mut triangle_output = [0; 12];
        decode_index_buffer(&mut triangle_output, 3, 4, &triangle).unwrap();
        assert_eq!(&triangle_output[..4], &0x0800_0000u32.to_le_bytes());
        triangle[7] |= 0x80;
        assert!(matches!(
            decode_index_buffer(&mut triangle_output, 3, 4, &triangle),
            Err(GltfError::InvalidGltf(_))
        ));

        let mut sequence = [0; 1 + 5 + 4];
        sequence[0] = SEQUENCE_HEADER | 1;
        sequence[1..6].copy_from_slice(&[0x80, 0x80, 0x80, 0x80, 1]);
        let mut sequence_output = [0; 4];
        decode_index_sequence(&mut sequence_output, 1, 4, &sequence).unwrap();
        assert_eq!(sequence_output, 0x0400_0000u32.to_le_bytes());
        sequence[5] |= 0x80;
        assert!(matches!(
            decode_index_sequence(&mut sequence_output, 1, 4, &sequence),
            Err(GltfError::InvalidGltf(_))
        ));
    }

    /// Expected values come from an independent transcription of the reference
    /// `decodeFilterColor`, not from this port: alpha's highest set bit gives
    /// the range Y/Co/Cg were quantized against, so the same bytes decode to
    /// different colours depending on it, and only fixed numbers pin that.
    ///
    /// The third vertex of each case carries an alpha below the scale it
    /// implies, which is the only arrangement where the one-bit expansion of
    /// alpha changes the result. The reference scales in single precision, and
    /// at 16 bits that is visible: the third red component lands on 52531 in
    /// `f32` and on 52530 in double. The fourth carries an alpha whose highest
    /// set bit stands eight places clear of the next one, which is the only
    /// arrangement where the last step of the scale smear changes the range.
    #[test]
    fn color_filter_recovers_rgba_from_luma_chroma() {
        // Three vertices at 8 bits per component: scales 15, 63 and 15.
        let mut narrow = vec![
            10u8,
            (-3i8) as u8,
            2,
            15,
            40,
            5,
            (-7i8) as u8,
            63,
            8,
            (-2i8) as u8,
            3,
            9,
        ];
        apply_filter(&mut narrow, MeshoptFilter::Color, 3, 4).unwrap();
        assert_eq!(
            narrow,
            [85, 204, 187, 255, 210, 134, 170, 255, 51, 187, 119, 51]
        );

        // The same shape at 16 bits, where the scale smear needs its last step.
        let mut wide = Vec::new();
        for value in [
            800u16,
            (-100i16) as u16,
            60,
            1023,
            300,
            25,
            (-40i16) as u16,
            511,
            700,
            90,
            (-30i16) as u16,
            600,
            50000,
            (-3000i16) as u16,
            1000,
            32769,
        ] {
            wide.extend_from_slice(&value.to_le_bytes());
        }
        apply_filter(&mut wide, MeshoptFilter::Color, 4, 8).unwrap();
        let decoded: Vec<u16> = wide
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        assert_eq!(
            decoded,
            [
                40999, 55093, 53812, 65535, 46811, 33345, 40398, 65535, 52531, 42921, 40999, 11275,
                46000, 51000, 52000, 3
            ]
        );
    }

    #[test]
    fn octahedral_filter_restores_unit_length_vectors() {
        let mut data = vec![0u8, 0, 127, 0, 64, 0, 63, 7];
        apply_filter(&mut data, MeshoptFilter::Octahedral, 2, 4).unwrap();

        assert_eq!(&data[..4], &[0, 0, 127, 0]);
        let x = data[4] as i8 as f32;
        let y = data[5] as i8 as f32;
        let z = data[6] as i8 as f32;
        assert!(
            ((x * x + y * y + z * z).sqrt() - 127.0).abs() < 1.0,
            "decoded normal {x},{y},{z} is not unit length"
        );
        assert_eq!(data[7], 7, "the fourth component stays untouched");
    }

    #[test]
    fn quaternion_filter_restores_the_dropped_component() {
        // The low two bits of the last component name the dropped axis.
        let mut data = vec![0u8, 0, 0, 0, 0, 0, 3, 0];
        apply_filter(&mut data, MeshoptFilter::Quaternion, 1, 8).unwrap();

        let components: Vec<i16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        assert_eq!(components, [0, 0, 0, 32767]);
    }

    #[test]
    fn exponential_filter_rebuilds_floats() {
        let mut data = Vec::new();
        // Mantissa 1 with exponent 0, then mantissa -3 with exponent 1.
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&(((1i32 << 24) | 0x00fffffd) as u32).to_le_bytes());
        apply_filter(&mut data, MeshoptFilter::Exponential, 2, 4).unwrap();

        let decoded: Vec<f32> = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .collect();
        assert_eq!(decoded, [1.0, -6.0]);
    }

    #[test]
    fn buffer_view_size_must_match_the_declared_layout() {
        let mut destination = [0u8; 7];
        let error = decode_buffer_view(
            &mut destination,
            &[],
            MeshoptMode::Attributes,
            MeshoptFilter::None,
            2,
            4,
        )
        .unwrap_err();
        assert!(matches!(error, GltfError::InvalidGltf(_)));
    }
}
