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

    let buffer = attribute.buffer_mut();
    for (i, value) in values.iter().enumerate() {
        let bytes: Vec<u8> = value
            .iter()
            .flat_map(|component| component.to_le_bytes())
            .collect();
        buffer.write(i * 12, &bytes);
    }

    attribute
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

    let buffer = attribute.buffer_mut();
    for (i, value) in values.iter().enumerate() {
        let bytes: Vec<u8> = value
            .iter()
            .flat_map(|component| component.to_le_bytes())
            .collect();
        buffer.write(i * 8, &bytes);
    }

    attribute
}
