#![no_main]

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    decode_as_mesh(data);
    decode_as_point_cloud(data);
});

fn decode_as_mesh(data: &[u8]) {
    let mut buffer = DecoderBuffer::new(data);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let _ = decoder.decode(&mut buffer, &mut mesh);
}

fn decode_as_point_cloud(data: &[u8]) {
    let mut buffer = DecoderBuffer::new(data);
    let mut point_cloud = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let _ = decoder.decode(&mut buffer, &mut point_cloud);
}
