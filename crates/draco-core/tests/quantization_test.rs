use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_encoder::PointCloudEncoder;

#[test]
fn test_quantization_encoding() {
    let mut pc = PointCloud::new();
    let mut pos_att = PointAttribute::new();

    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    let buffer = pos_att.buffer_mut();
    for (i, &position) in positions.iter().enumerate() {
        let bytes = position.to_le_bytes();
        buffer.write(i * 4, &bytes);
    }

    pc.add_attribute(pos_att);

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut buffer);

    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());
    assert!(buffer.size() > 0);
}
