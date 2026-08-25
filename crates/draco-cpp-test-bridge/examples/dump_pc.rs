//! Decodes a `.drc` as a point cloud and writes every attribute value in
//! decode order, so two builds can be compared by content rather than by
//! point count. The point-cloud counterpart of `dump_decoded`.
//!
//! ```text
//! cargo run --release --example dump_pc -- file.drc out.bin
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect(".drc path");
    let out = args.next().expect("output path");
    let bytes = std::fs::read(&path).expect("read .drc");

    let mut buffer = DecoderBuffer::new(&bytes);
    let mut pc = PointCloud::new();
    if PointCloudDecoder::new().decode(&mut buffer, &mut pc).is_err() {
        std::process::exit(2);
    }

    let mut dump: Vec<u8> = Vec::new();
    dump.extend_from_slice(&(pc.num_points() as u64).to_le_bytes());
    dump.extend_from_slice(&(pc.num_attributes() as u64).to_le_bytes());
    for index in 0..pc.num_attributes() {
        let attribute = pc.attribute(index);
        dump.extend_from_slice(&(attribute.num_components() as u64).to_le_bytes());
        dump.extend_from_slice(&(attribute.byte_stride() as u64).to_le_bytes());
        let data = attribute.buffer().data();
        dump.extend_from_slice(&(data.len() as u64).to_le_bytes());
        dump.extend_from_slice(data);
    }
    std::fs::write(&out, &dump).expect("write dump");
}
