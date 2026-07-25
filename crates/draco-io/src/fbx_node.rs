//! The FBX document tree: what both containers decode to and encode from.
//!
//! An FBX document is a tree of named records, each carrying a list of typed
//! properties and a list of children. Nothing above this file needs to know
//! whether those records arrived as binary node records or as ASCII text, and
//! nothing below it needs to know what `Objects` or `Connections` mean.
//!
//! This lives apart from either container because the two halves of the crate
//! are independently selectable: [`crate::fbx_container`] is behind
//! `fbx-reader`, [`crate::fbx_writer`] behind `fbx-writer`, and a type they
//! both name cannot sit inside either.

/// An FBX node with properties and children.
#[derive(Debug, Clone)]
pub struct FbxNode {
    /// Node name, such as `Objects`, `Geometry`, `Model`, or `Connections`.
    pub name: String,
    /// Properties stored directly on this node.
    pub properties: Vec<FbxProperty>,
    /// Child nodes nested under this node.
    pub children: Vec<FbxNode>,
}

/// FBX property value.
#[derive(Debug, Clone)]
pub enum FbxProperty {
    /// Boolean property.
    Bool(bool),
    /// Single-byte `Z` property, kept unsigned.
    ///
    /// The reverse-engineered specification calls `Z` a signed `i8`, while
    /// `ufbx` -- the de-facto compatibility oracle, and what Blender ships --
    /// reads all of `B`, `C` and `Z` as unsigned bytes. This follows `ufbx`.
    U8(u8),
    /// 16-bit signed integer property.
    I16(i16),
    /// 32-bit signed integer property.
    I32(i32),
    /// 64-bit signed integer property.
    I64(i64),
    /// 32-bit floating-point property.
    F32(f32),
    /// 64-bit floating-point property.
    F64(f64),
    /// UTF-8-ish string property decoded lossily from FBX bytes.
    String(String),
    /// Raw binary property.
    Raw(Vec<u8>),
    /// Boolean array property.
    BoolArray(Vec<bool>),
    /// 32-bit signed integer array property.
    I32Array(Vec<i32>),
    /// 64-bit signed integer array property.
    I64Array(Vec<i64>),
    /// 32-bit floating-point array property.
    F32Array(Vec<f32>),
    /// 64-bit floating-point array property.
    F64Array(Vec<f64>),
}
