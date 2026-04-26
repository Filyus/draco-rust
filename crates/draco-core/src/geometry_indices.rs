#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttributeValueIndex(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PointIndex(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VertexIndex(pub u32);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FaceIndex(pub u32);

pub const INVALID_ATTRIBUTE_VALUE_INDEX: AttributeValueIndex = AttributeValueIndex(u32::MAX);
pub const INVALID_POINT_INDEX: PointIndex = PointIndex(u32::MAX);
pub const INVALID_VERTEX_INDEX: VertexIndex = VertexIndex(u32::MAX);
pub const INVALID_CORNER_INDEX: CornerIndex = CornerIndex(u32::MAX);
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
