//! Decoder for [`KeyframeAnimation`].
//!
//! Mirrors C++ Draco's `KeyframeAnimationDecoder`, a thin wrapper around the
//! sequential point-cloud decoder.

use crate::decoder_buffer::DecoderBuffer;
use crate::keyframe_animation::KeyframeAnimation;
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::status::Status;

/// Decodes a Draco bitstream into [`KeyframeAnimation`] data.
#[derive(Debug, Default)]
pub struct KeyframeAnimationDecoder;

impl KeyframeAnimationDecoder {
    /// Creates a new keyframe animation decoder.
    pub fn new() -> Self {
        Self
    }

    /// Decodes `in_buffer` into `animation`. Mirrors C++
    /// `KeyframeAnimationDecoder::Decode`.
    pub fn decode(
        &mut self,
        in_buffer: &mut DecoderBuffer,
        animation: &mut KeyframeAnimation,
    ) -> Status {
        let mut point_cloud = PointCloud::new();
        let mut decoder = PointCloudDecoder::new();
        decoder.decode(in_buffer, &mut point_cloud)?;
        *animation = KeyframeAnimation::from_point_cloud(point_cloud);
        Ok(())
    }
}
