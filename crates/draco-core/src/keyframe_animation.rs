//! Keyframe animation container built on top of [`PointCloud`].
//!
//! Mirrors C++ Draco's `draco/animation/keyframe_animation.h`. A
//! [`KeyframeAnimation`] is a point cloud whose first attribute (unique id `0`)
//! always stores per-frame `f32` timestamps and whose remaining attributes each
//! store one keyframe track. Because Draco implements keyframe animation as a
//! point-cloud-like sequential stream, encode/decode reuse the existing
//! sequential point-cloud path (see [`crate::keyframe_animation_encoder`] and
//! [`crate::keyframe_animation_decoder`]).

use crate::draco_types::DataType;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::point_cloud::PointCloud;

/// Attribute unique id reserved for the timestamp track.
const TIMESTAMP_ID: u32 = 0;

/// Holds keyframe animation data as a [`PointCloud`].
///
/// The first attribute is always the timestamp track. Each additional attribute
/// is one keyframe track with the same number of frames.
#[derive(Debug, Default, Clone)]
pub struct KeyframeAnimation {
    point_cloud: PointCloud,
}

impl KeyframeAnimation {
    /// Creates an empty keyframe animation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps an existing point cloud (e.g. a freshly decoded one).
    pub fn from_point_cloud(point_cloud: PointCloud) -> Self {
        Self { point_cloud }
    }

    /// Returns the number of animation frames (equal to the point count).
    pub fn num_frames(&self) -> i32 {
        self.point_cloud.num_points() as i32
    }

    /// Sets the number of animation frames (equal to the point count).
    pub fn set_num_frames(&mut self, num_frames: i32) {
        self.point_cloud.set_num_points(num_frames.max(0) as usize);
    }

    /// Returns the number of keyframe tracks (attributes minus the timestamp).
    pub fn num_animations(&self) -> i32 {
        self.point_cloud.num_attributes() - 1
    }

    /// Sets the per-frame timestamps. Must be called only once, before any
    /// keyframe data is added unless a track was added first.
    ///
    /// Returns `false` if a timestamp track already holds data or if the frame
    /// count is inconsistent with previously added keyframes. Mirrors C++
    /// `KeyframeAnimation::SetTimestamps`.
    pub fn set_timestamps(&mut self, timestamps: &[f32]) -> bool {
        let num_frames = timestamps.len() as i32;
        if self.point_cloud.num_attributes() > 0 {
            // Timestamp attribute may be set only once.
            match self.timestamps() {
                Some(ts) if ts.size() > 0 => return false,
                _ => {}
            }
            // Frame count must match keyframes added earlier.
            if num_frames != self.num_frames() {
                return false;
            }
        } else {
            // This is the first attribute.
            self.set_num_frames(num_frames);
        }

        let mut timestamp_att = PointAttribute::new();
        timestamp_att.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Float32,
            false,
            num_frames as usize,
        );
        let bytes: &[u8] = bytemuck::cast_slice(timestamps);
        timestamp_att.buffer_mut().write(0, bytes);
        self.point_cloud
            .set_attribute(TIMESTAMP_ID as i32, timestamp_att);
        true
    }

    /// Adds one keyframe track and returns its animation id, or `-1` on error.
    ///
    /// `num_components` is the number of scalar components per frame, and `data`
    /// holds `num_components * num_frames` values laid out frame-major. Mirrors
    /// C++ `KeyframeAnimation::AddKeyframes`.
    pub fn add_keyframes<T: bytemuck::NoUninit>(
        &mut self,
        data_type: DataType,
        num_components: u32,
        data: &[T],
    ) -> i32 {
        if num_components == 0 {
            return -1;
        }
        // Reserve attribute id 0 for timestamps if nothing has been added yet.
        if self.point_cloud.num_attributes() == 0 {
            let mut temp_att = PointAttribute::new();
            temp_att.init(
                GeometryAttributeType::Generic,
                num_components as u8,
                data_type,
                false,
                0,
            );
            self.point_cloud.add_attribute(temp_att);
            self.set_num_frames(data.len() as i32 / num_components as i32);
        }

        if data.len() != num_components as usize * self.num_frames() as usize {
            return -1;
        }

        let mut keyframe_att = PointAttribute::new();
        keyframe_att.init(
            GeometryAttributeType::Generic,
            num_components as u8,
            data_type,
            false,
            self.num_frames() as usize,
        );
        let bytes: &[u8] = bytemuck::cast_slice(data);
        keyframe_att.buffer_mut().write(0, bytes);
        self.point_cloud.add_attribute(keyframe_att)
    }

    /// Returns the timestamp track, if present.
    pub fn timestamps(&self) -> Option<&PointAttribute> {
        self.point_cloud.attribute_by_unique_id(TIMESTAMP_ID)
    }

    /// Returns the keyframe track identified by `animation_id`, if present.
    pub fn keyframes(&self, animation_id: i32) -> Option<&PointAttribute> {
        if animation_id < 0 {
            return None;
        }
        self.point_cloud.attribute_by_unique_id(animation_id as u32)
    }

    /// Returns the underlying point cloud.
    pub fn point_cloud(&self) -> &PointCloud {
        &self.point_cloud
    }

    /// Consumes the animation, returning the underlying point cloud.
    pub fn into_point_cloud(self) -> PointCloud {
        self.point_cloud
    }
}
