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
fuzz_target!(|data: &[u8]| {
    decode_as_mesh(data);
    decode_as_point_cloud(data);
});

fn decode_as_mesh(data: &[u8]) {
    let mut buffer = DecoderBuffer::new(data).with_limits(DecodeLimits::fuzzing());
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let _ = decoder.decode(&mut buffer, &mut mesh);
}

fn decode_as_point_cloud(data: &[u8]) {
    let mut buffer = DecoderBuffer::new(data).with_limits(DecodeLimits::fuzzing());
    let mut point_cloud = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let _ = decoder.decode(&mut buffer, &mut point_cloud);
}
