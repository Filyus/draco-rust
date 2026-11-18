#![no_main]

use draco_gltf::meshopt::{decode_buffer_view, MeshoptFilter, MeshoptMode};
use libfuzzer_sys::fuzz_target;

fn reference(source: &[u8], mode: MeshoptMode, count: usize, stride: usize) -> Option<Vec<u8>> {
    let byte_len = count * stride;
    let mut output = vec![0u64; byte_len.div_ceil(8)];
    // SAFETY: the aligned output has at least count * stride writable bytes,
    // and source is readable for the duration of the reference decoder call.
    let result = unsafe {
        match mode {
            MeshoptMode::Attributes => meshopt::ffi::meshopt_decodeVertexBuffer(
                output.as_mut_ptr().cast(),
                count,
                stride,
                source.as_ptr(),
                source.len(),
            ),
            MeshoptMode::Triangles => meshopt::ffi::meshopt_decodeIndexBuffer(
                output.as_mut_ptr().cast(),
                count,
                stride,
                source.as_ptr(),
                source.len(),
            ),
            MeshoptMode::Indices => meshopt::ffi::meshopt_decodeIndexSequence(
                output.as_mut_ptr().cast(),
                count,
                stride,
                source.as_ptr(),
                source.len(),
            ),
        }
    };
    (result == 0).then(|| {
        output
            .into_iter()
            .flat_map(u64::to_ne_bytes)
            .take(byte_len)
            .collect()
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&[mode, stride, count_low, count_high], source)) = data.split_first_chunk::<4>()
    else {
        return;
    };
    if source.len() > 16 * 1024 {
        return;
    }

    // Keep the layout realistic so changes to payload bytes reach the codec.
    // The largest decoded buffer is 511 elements of 256 bytes each.
    let count = usize::from(u16::from_le_bytes([count_low, count_high]) & 511);
    let index_stride = 2 << (stride & 1);
    let (mode, count, stride) = match mode % 3 {
        0 => (
            MeshoptMode::Attributes,
            count,
            4 * (usize::from(stride & 63) + 1),
        ),
        1 => (MeshoptMode::Triangles, count * 3, index_stride),
        _ => (MeshoptMode::Indices, count, index_stride),
    };
    let mut decoded = vec![0; count * stride];
    // These constructions must be rejected irrespective of what meshoptimizer
    // chooses to do with malformed streams.
    let mut invalid_edge = [0; 18];
    invalid_edge[0] = 0xe1;
    invalid_edge[1] = (count_low % 15) << 4;
    assert!(decode_buffer_view(
        &mut [0; 12],
        &invalid_edge,
        MeshoptMode::Triangles,
        MeshoptFilter::None,
        3,
        4
    )
    .is_err());
    let mut invalid_varint = [0; 10];
    invalid_varint[0] = 0xd1;
    for (dst, value) in invalid_varint[1..6]
        .iter_mut()
        .zip(source.iter().copied().chain(std::iter::repeat(0)))
    {
        *dst = value | 0x80;
    }
    assert!(decode_buffer_view(
        &mut [0; 4],
        &invalid_varint,
        MeshoptMode::Indices,
        MeshoptFilter::None,
        1,
        4
    )
    .is_err());
    let mut invalid_triangle_varint = [0; 24];
    invalid_triangle_varint[..3].copy_from_slice(&[0xe1, 0xff, 0]);
    invalid_triangle_varint[3..8].copy_from_slice(&invalid_varint[1..6]);
    assert!(decode_buffer_view(
        &mut [0; 12],
        &invalid_triangle_varint,
        MeshoptMode::Triangles,
        MeshoptFilter::None,
        3,
        4
    )
    .is_err());
    if decode_buffer_view(
        &mut decoded,
        source,
        mode,
        MeshoptFilter::None,
        count,
        stride,
    )
    .is_ok()
    {
        if let Some(expected) = reference(source, mode, count, stride) {
            assert_eq!(decoded, expected, "reference decoder disagrees");
        }
        // A successful decode must populate the entire view, regardless of
        // what was in the caller's buffer before decoding.
        let mut replay = vec![0xff; decoded.len()];
        decode_buffer_view(
            &mut replay,
            source,
            mode,
            MeshoptFilter::None,
            count,
            stride,
        )
        .unwrap();
        assert_eq!(decoded, replay);
    }

    // Generate a valid index sequence from fuzz input. The reference encoder
    // supplies the expected result even when mutations never form a valid stream.
    let indices: Vec<u32> = source
        .chunks_exact(2)
        .take(256)
        .map(|bytes| u32::from(u16::from_le_bytes([bytes[0], bytes[1]])))
        .collect();
    if !indices.is_empty() {
        let encoded = meshopt::encode_index_sequence(&indices, 65536).unwrap();
        let mut actual = vec![0; indices.len() * 4];
        decode_buffer_view(
            &mut actual,
            &encoded,
            MeshoptMode::Indices,
            MeshoptFilter::None,
            indices.len(),
            4,
        )
        .unwrap();
        let expected: Vec<_> = indices
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect();
        assert_eq!(actual, expected);

        let triangle_count = indices.len() / 3 * 3;
        if triangle_count > 0 {
            let encoded = meshopt::encode_index_buffer(&indices[..triangle_count], 65536).unwrap();
            let expected = reference(&encoded, MeshoptMode::Triangles, triangle_count, 4).unwrap();
            let mut actual = vec![0; expected.len()];
            decode_buffer_view(
                &mut actual,
                &encoded,
                MeshoptMode::Triangles,
                MeshoptFilter::None,
                triangle_count,
                4,
            )
            .unwrap();
            assert_eq!(actual, expected);

            // The last two entries are zero; all other nibbles are below 15.
            let mut invalid = encoded;
            let table = invalid.len() - 16;
            let entry = usize::from(count_low) % 16;
            invalid[table + entry] = if entry >= 14 {
                count_high | 1
            } else if count_high & 1 == 0 {
                0xf0 | (count_high & 15)
            } else {
                (count_high & 0xf0) | 15
            };
            assert!(decode_buffer_view(
                &mut actual,
                &invalid,
                MeshoptMode::Triangles,
                MeshoptFilter::None,
                triangle_count,
                4
            )
            .is_err());
        }
    }

    let vertices: Vec<[u8; 4]> = source
        .chunks_exact(4)
        .take(256)
        .map(|bytes| [bytes[0], bytes[1], bytes[2], bytes[3]])
        .collect();
    if !vertices.is_empty() {
        let encoded = meshopt::encode_vertex_buffer_level(
            &vertices,
            i32::from(count_high & 3),
            Some(i32::from(count_low & 1)),
        )
        .unwrap();
        let mut actual = vec![0; vertices.len() * 4];
        decode_buffer_view(
            &mut actual,
            &encoded,
            MeshoptMode::Attributes,
            MeshoptFilter::None,
            vertices.len(),
            4,
        )
        .unwrap();
        let expected: Vec<_> = vertices.iter().flatten().copied().collect();
        assert_eq!(actual, expected);

        if encoded[0] == 0xa1 {
            let mut invalid = encoded;
            let last = invalid.len() - 1;
            invalid[last] = [3, 4, 0x10, 0x11][usize::from(count_high) & 3];
            assert!(decode_buffer_view(
                &mut actual,
                &invalid,
                MeshoptMode::Attributes,
                MeshoptFilter::None,
                vertices.len(),
                4
            )
            .is_err());
        }
    }
});
