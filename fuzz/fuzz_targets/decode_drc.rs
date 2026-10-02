#![no_main]

use draco_core::decode_limits::DecodeLimits;
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use libfuzzer_sys::fuzz_target;

// The fuzz crate enables the legacy decode features for this target so the same
// coverage-guided campaign exercises shipped legacy `.drc` support as well as
// the current bitstream paths.
//
// `DecodeLimits::fuzzing()` is deliberately far tighter than the shipped
// defaults, for the reason `fbx_read_scene` gives: the decoder does not cap
// reconstructed geometry, so a header naming a hundred million points is a
// legitimate multi-gigabyte decode and `-rss_limit_mb` fires on it, drowning
// real findings. Under the tight ceilings an allocation failure that still
// occurs is a genuine bug. The shipped defaults stay covered by
// `decode_limits`' own tests, which decode real streams under them.
//
// A point cloud is decoded twice, on one thread and on two, and the two must
// agree: both refuse the stream, or both read the same bytes for every
// attribute. A fuzzing build lowers the gates that hand a cloud to threads
// (see draco-core's `parallel` module), so an input of a few hundred bytes
// reaches the side-by-side decode, its shared budget and its give-up, which at
// the shipped gates only a stream of a megabyte does.
fuzz_target!(|data: &[u8]| {
    decode_as_mesh(data);
    let in_order = decode_as_point_cloud(data, 1);
    let side_by_side = decode_as_point_cloud(data, 2);
    assert!(
        in_order == side_by_side,
        "one thread and two read the stream differently: {} against {}",
        describe(&in_order),
        describe(&side_by_side)
    );
});

fn decode_as_mesh(data: &[u8]) {
    let mut buffer = DecoderBuffer::new(data).with_limits(DecodeLimits::fuzzing());
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder.set_threads(1);
    let _ = decoder.decode(&mut buffer, &mut mesh);
}

/// Every attribute's bytes, or `None` where the stream was refused.
fn decode_as_point_cloud(data: &[u8], threads: i32) -> Option<Vec<Vec<u8>>> {
    let mut buffer = DecoderBuffer::new(data).with_limits(DecodeLimits::fuzzing());
    let mut point_cloud = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    decoder.set_threads(threads);
    decoder.decode(&mut buffer, &mut point_cloud).ok()?;
    Some(
        (0..point_cloud.num_attributes())
            .map(|id| point_cloud.attribute(id).buffer().data().to_vec())
            .collect(),
    )
}

fn describe(decoded: &Option<Vec<Vec<u8>>>) -> String {
    match decoded {
        None => "a refusal".to_string(),
        Some(attributes) => format!(
            "{} attributes of {:?} bytes",
            attributes.len(),
            attributes.iter().map(Vec::len).collect::<Vec<_>>()
        ),
    }
}
