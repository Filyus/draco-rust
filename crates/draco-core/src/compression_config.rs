//! Bitstream configuration enums and constants.
//!
//! Defines [`EncodedGeometryType`] (point cloud vs. triangular mesh), the
//! encoding-method enums (sequential vs. EdgeBreaker, KD-tree vs. sequential
//! point cloud), and related constants written into the Draco header. Port of
//! Draco's encoder/decoder config definitions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedGeometryType {
    InvalidGeometryType = -1,
    PointCloud = 0,
    TriangularMesh = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointCloudEncodingMethod {
    PointCloudSequentialEncoding = 0,
    PointCloudKdTreeEncoding = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshEncodingMethod {
    MeshSequentialEncoding = 0,
    MeshEdgebreakerEncoding = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeEncoderType {
    BasicAttributeEncoder = 0,
    MeshTraversalAttributeEncoder = 1,
    KdTreeAttributeEncoder = 2,
}
