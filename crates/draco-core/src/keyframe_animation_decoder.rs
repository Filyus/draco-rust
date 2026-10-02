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
    ///
    /// The point cloud underneath is decoded with [`PointCloudDecoder`]'s
    /// default, on the calling thread. A caller that wants more threads decodes
    /// it the same way this does, with a decoder of its own:
    ///
    /// ```
    /// # fn decode(bytes: &[u8]) -> draco_core::Status {
    /// use draco_core::{DecoderBuffer, KeyframeAnimation, PointCloud, PointCloudDecoder};
    ///
    /// let mut point_cloud = PointCloud::new();
    /// let mut decoder = PointCloudDecoder::new();
    /// decoder.set_threads(0);
    /// decoder.decode(&mut DecoderBuffer::new(bytes), &mut point_cloud)?;
    /// let animation = KeyframeAnimation::from_point_cloud(point_cloud);
    /// # let _ = animation;
    /// # Ok(())
    /// # }
    /// ```
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
