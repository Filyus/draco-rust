//! Byte-level size accounting for a single successful encode.

use crate::geometry_attribute::GeometryAttributeType;

/// Byte sizes written for one source attribute during encoding.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EncodedAttributeSizeStats {
    /// Source attribute id in the input geometry.
    pub source_attribute_id: i32,
    /// Semantic type of the source attribute.
    pub attribute_type: GeometryAttributeType,
    /// Draco unique id assigned to the attribute.
    pub unique_id: u32,
    /// Bytes written while encoding this attribute's portable values.
    ///
    /// This is zero when the values are part of a joint payload such as the
    /// point-cloud KD-tree stream.
    pub portable_attribute_bytes: usize,
    /// Bytes written for parameters needed to undo this attribute's transform.
    pub transform_data_bytes: usize,
}

impl EncodedAttributeSizeStats {
    /// Total portable-value and transform-parameter bytes for this attribute.
    pub fn total_attribute_bytes(&self) -> usize {
        self.portable_attribute_bytes + self.transform_data_bytes
    }
}

/// Byte-level breakdown of one successful encode.
///
/// `total_bytes` counts the bytes appended to the output buffer by this encode,
/// even when the buffer already held other data. The sections add up to that
/// total. Point-cloud KD-tree encoding writes all portable values into one
/// joint stream; those bytes are reported in
/// `joint_portable_attributes_bytes`, while per-attribute transform parameters
/// remain in `attributes`.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct EncodedSizeStats {
    /// Total number of bytes appended by this encode.
    pub total_bytes: usize,
    /// Draco header, optional metadata, geometry headers, attribute metadata,
    /// and attribute-encoder descriptors.
    pub header_and_metadata_bytes: usize,
    /// Bytes written for mesh connectivity. Always zero for point clouds.
    pub connectivity_bytes: usize,
    /// Bytes written for portable attribute values that are jointly encoded.
    /// This is used by point-cloud KD-tree encoding and is zero otherwise.
    pub joint_portable_attributes_bytes: usize,
    /// Whether portable value bytes are attributed exactly to each source
    /// attribute. False when a joint coder emits one shared payload.
    pub per_attribute_portable_bytes_exact: bool,
    /// Per-attribute portable value and transform-parameter bytes.
    pub attributes: Vec<EncodedAttributeSizeStats>,
}

impl EncodedSizeStats {
    pub(crate) fn with_attributes(
        attributes: impl IntoIterator<Item = (i32, GeometryAttributeType, u32)>,
    ) -> Self {
        Self {
            per_attribute_portable_bytes_exact: true,
            attributes: attributes
                .into_iter()
                .map(
                    |(source_attribute_id, attribute_type, unique_id)| EncodedAttributeSizeStats {
                        source_attribute_id,
                        attribute_type,
                        unique_id,
                        portable_attribute_bytes: 0,
                        transform_data_bytes: 0,
                    },
                )
                .collect(),
            ..Self::default()
        }
    }

    pub(crate) fn attribute_mut(
        &mut self,
        source_attribute_id: i32,
    ) -> &mut EncodedAttributeSizeStats {
        let index =
            usize::try_from(source_attribute_id).expect("encoded attribute ids are non-negative");
        let attribute = self
            .attributes
            .get_mut(index)
            .expect("size stats are initialized for every source attribute");
        debug_assert_eq!(attribute.source_attribute_id, source_attribute_id);
        attribute
    }

    pub(crate) fn accounted_bytes(&self) -> usize {
        self.header_and_metadata_bytes
            + self.connectivity_bytes
            + self.joint_portable_attributes_bytes
            + self
                .attributes
                .iter()
                .map(EncodedAttributeSizeStats::total_attribute_bytes)
                .sum::<usize>()
    }
}
