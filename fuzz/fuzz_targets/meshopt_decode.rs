#![no_main]

use draco_gltf::meshopt::{decode_buffer_view, MeshoptFilter, MeshoptMode};
use libfuzzer_sys::fuzz_target;

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
});
