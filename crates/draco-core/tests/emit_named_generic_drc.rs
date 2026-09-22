//! Writes a point cloud whose generic attributes carry names, for upstream
//! C++ Draco to read back.
//!
//! Naming a generic attribute is the part of a splat-shaped point cloud that
//! only the other implementation can confirm: the key is a convention rather
//! than a field in the bitstream. Upstream uses `"name"` on an
//! `AttributeMetadata` — `obj_decoder.cc` writes it and `obj_encoder.cc` reads
//! it back with `GetAttributeMetadataByStringEntry("name", …)` — and this
//! emits a file to check that a stream from here answers that call.
//!
//! Verified 2026-09-18 against Draco 1.5.7 built from
//! `draco-build-trees/1.5.7`: all three generic attributes came back with
//! their names, their component counts and their unique ids. The reader was a
//! short C++ program against that build, asking each generic attribute for its
//! `"name"` entry and printing it beside the component count and the id.
//!
//! ```text
//! cargo test -p draco-core --features encoder,decoder \
//!     --test emit_named_generic_drc -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use draco_core::{
    DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute,
    PointCloud, PointCloudEncoder,
};

const NUM_POINTS: usize = 32;

fn attribute(kind: GeometryAttributeType, components: u8, seed: f32) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, NUM_POINTS);
    let buffer = attribute.buffer_mut();
    for point in 0..NUM_POINTS {
        for component in 0..components as usize {
            let value = seed + point as f32 * 0.5 + component as f32 * 0.125;
            let offset = (point * components as usize + component) * 4;
            buffer.write(offset, &value.to_le_bytes());
        }
    }
    attribute
}

#[test]
#[ignore = "emits a file for the C++ decoder; run with --ignored --nocapture"]
fn emit() {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(attribute(GeometryAttributeType::Position, 3, 0.0));

    for (index, (name, components)) in [("scale", 3u8), ("rotation", 4), ("opacity", 1)]
        .into_iter()
        .enumerate()
    {
        let id = cloud.add_attribute(attribute(
            GeometryAttributeType::Generic,
            components,
            10.0 + index as f32,
        ));
        let unique_id = cloud.attribute(id).unique_id();
        let mut metadata = Metadata::new();
        metadata.set_string("name", name).expect("string entry");
        cloud
            .metadata_or_insert()
            .set_attribute_metadata(unique_id, metadata);
    }

    let mut options = EncoderOptions::new();
    for id in 0..cloud.num_attributes() {
        options.set_attribute_int(id, "quantization_bits", 14);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");

    let path = std::env::temp_dir()
        .join("named_generic.drc")
        .to_string_lossy()
        .into_owned();
    // Written and synced through the one handle: a reopened read-only handle
    // cannot be synced on Windows, and the C++ decoder reads this next.
    use std::io::Write as _;
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(buffer.data()).expect("write");
    file.flush().expect("flush");
    file.sync_all().expect("fsync");
    println!("WROTE {path} ({} bytes)", buffer.data().len());
}
