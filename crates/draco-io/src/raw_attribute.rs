//! Attribute-buffer builders shared by the readers that parse typed component
//! arrays out of a text or fixed-layout binary format -- OBJ, PLY and STL.
//!
//! Each of those readers ends up with a `Vec<[f32; N]>` per attribute before
//! it ever touches a `Mesh`, and packing that into a `PointAttribute`'s buffer
//! is the same three lines regardless of which reader got there. glTF does not
//! share this: its accessors decode straight into the attribute's native byte
//! layout, so it never materializes typed component arrays in the first place.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};

/// Packs a `[f32; 3]`-per-point array into a new position/normal/color-shaped
/// attribute.
pub(crate) fn make_f32x3_attribute(
    attribute_type: GeometryAttributeType,
    values: &[[f32; 3]],
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(attribute_type, 3, DataType::Float32, false, values.len());
    write_components(attribute.buffer_mut().data_mut(), values, f32::to_le_bytes);
    attribute
}

/// Writes `values` into `data`, one entry after another, each component as the
/// four little-endian bytes `to_le_bytes` gives. `data` is the buffer of an
/// attribute initialised for `values.len()` entries of `N` such components.
///
/// Straight into the buffer: building each entry's bytes as a `Vec` first was
/// an allocation per point, most of the time a large PLY took to read.
pub(crate) fn write_components<T: Copy, const N: usize>(
    data: &mut [u8],
    values: &[[T; N]],
    to_le_bytes: impl Fn(T) -> [u8; 4],
) {
    let (words, _) = data.as_chunks_mut::<4>();
    for (word, &component) in words.iter_mut().zip(values.iter().flatten()) {
        *word = to_le_bytes(component);
    }
}

/// Packs a `[f32; 2]`-per-point array into a new texture-coordinate-shaped
/// attribute.
///
/// STL has no texture coordinates, so only OBJ and PLY call this.
#[cfg(any(feature = "obj-reader", feature = "ply-reader"))]
pub(crate) fn make_f32x2_attribute(
    attribute_type: GeometryAttributeType,
    values: &[[f32; 2]],
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(attribute_type, 2, DataType::Float32, false, values.len());
    write_components(attribute.buffer_mut().data_mut(), values, f32::to_le_bytes);
    attribute
}
