//! Encoder for [`KeyframeAnimation`].
//!
//! Mirrors C++ Draco's `KeyframeAnimationEncoder`, a thin wrapper around the
//! sequential point-cloud encoder so animation encoding stays separate from
//! general geometry compression.

use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::keyframe_animation::KeyframeAnimation;
use crate::point_cloud_encoder::PointCloudEncoder;
use crate::status::Status;

/// Encodes [`KeyframeAnimation`] data into a Draco bitstream.
#[derive(Debug, Default)]
pub struct KeyframeAnimationEncoder;

impl KeyframeAnimationEncoder {
    /// Creates a new keyframe animation encoder.
    pub fn new() -> Self {
        Self
    }

    /// Encodes `animation` into `out_buffer` using the sequential point-cloud
    /// path. Mirrors C++ `KeyframeAnimationEncoder::EncodeKeyframeAnimation`.
    pub fn encode_keyframe_animation(
        &mut self,
        animation: &KeyframeAnimation,
        options: &EncoderOptions,
        out_buffer: &mut EncoderBuffer,
    ) -> Status {
        // Force the sequential encoding method (C++ wraps
        // PointCloudSequentialEncoder directly).
        let mut options = options.clone();
        options.set_encoding_method(0);

        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(animation.point_cloud().clone());
        encoder.encode(&options, out_buffer)
    }
}
