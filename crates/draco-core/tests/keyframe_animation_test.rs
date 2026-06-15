use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::keyframe_animation::KeyframeAnimation;
use draco_core::keyframe_animation_decoder::KeyframeAnimationDecoder;
use draco_core::keyframe_animation_encoder::KeyframeAnimationEncoder;

fn read_floats(att: &draco_core::geometry_attribute::PointAttribute) -> Vec<f32> {
    let buffer = att.buffer();
    let count = att.size() * att.num_components() as usize;
    (0..count)
        .map(|i| {
            let mut bytes = [0u8; 4];
            buffer.read(i * 4, &mut bytes);
            f32::from_le_bytes(bytes)
        })
        .collect()
}

#[test]
fn timestamps_then_keyframes_roundtrip() {
    let timestamps = [0.0f32, 0.5, 1.0, 1.5];
    // Two-component (e.g. 2D position) track, frame-major.
    let track = [
        0.0f32, 0.0, // frame 0
        1.0, 2.0, // frame 1
        3.0, 4.0, // frame 2
        5.0, 6.0, // frame 3
    ];

    let mut animation = KeyframeAnimation::new();
    assert!(animation.set_timestamps(&timestamps));
    let anim_id = animation.add_keyframes(DataType::Float32, 2, &track);
    assert_eq!(anim_id, 1, "first keyframe track gets attribute id 1");
    assert_eq!(animation.num_frames(), 4);
    assert_eq!(animation.num_animations(), 1);

    let mut enc_buffer = EncoderBuffer::new();
    let status = KeyframeAnimationEncoder::new().encode_keyframe_animation(
        &animation,
        &EncoderOptions::new(),
        &mut enc_buffer,
    );
    assert!(status.is_ok(), "encode failed: {:?}", status.err());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded = KeyframeAnimation::new();
    let status = KeyframeAnimationDecoder::new().decode(&mut dec_buffer, &mut decoded);
    assert!(status.is_ok(), "decode failed: {:?}", status.err());

    assert_eq!(decoded.num_frames(), 4);
    assert_eq!(decoded.num_animations(), 1);

    let decoded_ts = decoded.timestamps().expect("timestamps present");
    assert_eq!(decoded_ts.num_components(), 1);
    assert_eq!(read_floats(decoded_ts), timestamps);

    let decoded_track = decoded.keyframes(1).expect("track present");
    assert_eq!(decoded_track.num_components(), 2);
    assert_eq!(read_floats(decoded_track), track);
}

#[test]
fn keyframes_before_timestamps_reserves_attribute_zero() {
    let track = [10.0f32, 20.0, 30.0];
    let timestamps = [0.0f32, 1.0, 2.0];

    let mut animation = KeyframeAnimation::new();
    // Adding keyframes first reserves attribute id 0 for the timestamps, so the
    // first real track lands at id 1, matching C++ behavior.
    let anim_id = animation.add_keyframes(DataType::Float32, 1, &track);
    assert_eq!(anim_id, 1);
    assert_eq!(animation.num_frames(), 3);
    assert!(animation.set_timestamps(&timestamps));

    let mut enc_buffer = EncoderBuffer::new();
    KeyframeAnimationEncoder::new()
        .encode_keyframe_animation(&animation, &EncoderOptions::new(), &mut enc_buffer)
        .expect("encode");

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded = KeyframeAnimation::new();
    KeyframeAnimationDecoder::new()
        .decode(&mut dec_buffer, &mut decoded)
        .expect("decode");

    assert_eq!(read_floats(decoded.timestamps().unwrap()), timestamps);
    assert_eq!(read_floats(decoded.keyframes(1).unwrap()), track);
}

#[test]
fn set_timestamps_rejects_double_set_and_mismatched_frames() {
    let mut animation = KeyframeAnimation::new();
    assert!(animation.set_timestamps(&[0.0, 1.0, 2.0]));
    // Second call with populated timestamps must fail.
    assert!(!animation.set_timestamps(&[0.0, 1.0, 2.0]));

    // Mismatched frame count is rejected when keyframes exist first.
    let mut other = KeyframeAnimation::new();
    other.add_keyframes(DataType::Float32, 1, &[1.0f32, 2.0, 3.0]);
    assert!(!other.set_timestamps(&[0.0, 1.0]));
}

#[test]
fn add_keyframes_rejects_zero_components_and_bad_length() {
    let mut animation = KeyframeAnimation::new();
    assert_eq!(animation.add_keyframes(DataType::Float32, 0, &[1.0f32]), -1);

    animation.set_timestamps(&[0.0, 1.0, 2.0]);
    // 2 components but 5 values is not a whole number of frames.
    assert_eq!(
        animation.add_keyframes(DataType::Float32, 2, &[1.0f32, 2.0, 3.0, 4.0, 5.0]),
        -1
    );
}
