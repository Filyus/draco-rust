/// Index of a unique attribute value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttributeValueIndex(pub u32);

/// Index of a point in point-cloud or mesh geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PointIndex(pub u32);

/// Index of a corner-table vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VertexIndex(pub u32);

/// Index of a corner in triangle corner-table topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CornerIndex(pub u32);

impl std::ops::Add<u32> for CornerIndex {
    type Output = CornerIndex;
    fn add(self, rhs: u32) -> CornerIndex {
        CornerIndex(self.0 + rhs)
    }
}

impl std::ops::Sub<u32> for CornerIndex {
    type Output = CornerIndex;
    fn sub(self, rhs: u32) -> CornerIndex {
        CornerIndex(self.0 - rhs)
    }
}

impl From<CornerIndex> for u32 {
    fn from(ci: CornerIndex) -> u32 {
        ci.0
    }
}

/// Index of a triangle face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceIndex(pub u32);

/// Sentinel attribute value index used for missing mappings.
pub const INVALID_ATTRIBUTE_VALUE_INDEX: AttributeValueIndex = AttributeValueIndex(u32::MAX);
/// Sentinel point index.
pub const INVALID_POINT_INDEX: PointIndex = PointIndex(u32::MAX);
/// Sentinel corner-table vertex index.
pub const INVALID_VERTEX_INDEX: VertexIndex = VertexIndex(u32::MAX);
/// Sentinel corner index.
pub const INVALID_CORNER_INDEX: CornerIndex = CornerIndex(u32::MAX);
/// Sentinel face index.
pub const INVALID_FACE_INDEX: FaceIndex = FaceIndex(u32::MAX);

impl From<u32> for AttributeValueIndex {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<AttributeValueIndex> for u32 {
    fn from(v: AttributeValueIndex) -> Self {
        v.0
    }
}

impl From<u32> for PointIndex {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<PointIndex> for u32 {
    fn from(v: PointIndex) -> Self {
        v.0
    }
}

impl From<usize> for PointIndex {
    fn from(v: usize) -> Self {
        Self(v as u32)
    }
}

impl From<PointIndex> for usize {
    fn from(v: PointIndex) -> Self {
        v.0 as usize
    }
}
