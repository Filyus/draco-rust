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
