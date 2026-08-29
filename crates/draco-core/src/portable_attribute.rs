//! The parent attribute a prediction scheme reads.
//!
//! Upstream gives every integer-decoded attribute a portable `int32` copy and
//! binds *that* as a prediction scheme's parent -- `SetPredictionSchemeParentAttributes`
//! reaches it through `GetPortableAttribute` and fails the scheme outright when
//! it is missing -- so a predictor never reads the attribute it was asked to
//! code, only its portable representation. This repository honoured that by
//! convention, and the convention broke twice: the `DataType::Uint32` arm of
//! two predictors read the value unsigned, putting every position above
//! `i32::MAX` a whole `2^32` from the number the correction was computed
//! against, and the encoder-side geometric-normal predictor read positions at
//! the attribute's own type, so a float position reached it at both ends of the
//! `i64` range. Both bugs were invisible to review because nothing in the
//! types said a predictor may not do this.
//!
//! The invariant is structural here: a predictor never holds a
//! [`PointAttribute`]. It holds a [`PredictionParent`], which exposes no
//! buffer, no byte stride and no data type -- only the point-to-entry lookup
//! and the two canonical widening reads. Obtaining any other value out of the
//! parent is unrepresentable. Constructing one is the privilege of the sites
//! that own the portable copy: [`PredictionParent::portable`] validates the
//! declared type against the set the portable pass writes, and
//! [`PredictionParent::legacy`] is the pre-2.0 backwards-compatibility binding
//! where upstream hands the attribute itself.

use crate::draco_types::DataType;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{AttributeValueIndex, PointIndex};
use crate::status::DracoError;

/// The attribute a prediction scheme reads its parent through.
///
/// See the [module documentation](self) for why this is not a `&PointAttribute`.
#[derive(Clone, Copy, Debug)]
pub struct PredictionParent<'a> {
    att: &'a PointAttribute,
}

impl<'a> PredictionParent<'a> {
    /// Binds the portable `int32` copy as a prediction parent.
    ///
    /// This is the binding every encoder makes and every 2.0+ decoder makes
    /// when a portable copy exists. It validates the declared type against the
    /// set the portable pass writes -- integral types of at most 32 bits -- and
    /// refuses anything else: a float or 64-bit attribute cannot be what the
    /// portable pass produced, so a caller reaching one has lost the portable
    /// copy somewhere and the scheme must fail the way upstream's does when
    /// `GetPortableAttribute` returns null. The values read back are the
    /// portable `int32` values no matter which of the accepted types the
    /// attribute declares: a `uint32` attribute stores its values above
    /// `i32::MAX` as the negatives the portable copy carries.
    pub fn portable(att: &'a PointAttribute) -> Result<Self, DracoError> {
        if !matches!(
            att.data_type(),
            DataType::Int8
                | DataType::Uint8
                | DataType::Int16
                | DataType::Uint16
                | DataType::Int32
                | DataType::Uint32
        ) {
            return Err(DracoError::invalid_parameter(format!(
                "A prediction parent must be the portable int32 copy, got a {:?} attribute",
                att.data_type()
            )));
        }
        Ok(Self { att })
    }

    /// Binds the attribute itself, for bitstreams below 2.0.
    ///
    /// Upstream's `SequentialAttributeDecoder::InitPredictionScheme` has a
    /// `DRACO_BACKWARDS_COMPATIBILITY_SUPPORTED` branch that passes
    /// `point_cloud()->attribute(att_id)` -- the attribute, dequantized in
    /// place by then, at whatever type it declares. The canonical reads below
    /// keep that contract; `portable` is the one that validates, so a caller
    /// that means the portable copy has no route to this binding.
    // Only the sequential decoders and encoders reach this binding, and both
    // are feature-gated; with neither feature the constructor would sit
    // unused.
    #[cfg(any(feature = "decoder", feature = "encoder"))]
    pub(crate) fn legacy(att: &'a PointAttribute) -> Self {
        Self { att }
    }

    pub fn attribute_type(&self) -> GeometryAttributeType {
        self.att.attribute_type()
    }

    pub fn num_components(&self) -> u8 {
        self.att.num_components()
    }

    /// The attribute value entry a point reads from.
    pub fn mapped_index(&self, point_id: PointIndex) -> AttributeValueIndex {
        self.att.mapped_index(point_id)
    }

    /// The portable value at `entry`/`component`, widened to `i64`.
    ///
    /// The one widening implementation in the crate. Integral types widen
    /// numerically except `uint32`, which is read as the `int32` the portable
    /// copy carries; the remaining arms only exist for the pre-2.0 binding,
    /// where upstream reads the attribute at its declared type and converts --
    /// a float position truncates, a non-finite one reads as nothing.
    pub fn read_component_as_i64(&self, entry: usize, component: usize) -> Option<i64> {
        let buffer = self.att.buffer();
        let byte_stride = usize::try_from(self.att.byte_stride()).ok()?;
        let byte_offset = entry
            .checked_mul(byte_stride)?
            .checked_add(component.checked_mul(self.att.data_type().byte_length())?)?;

        match self.att.data_type() {
            DataType::Int8 => Some(i8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as i64),
            DataType::Uint8 => {
                Some(u8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as i64)
            }
            DataType::Int16 => {
                Some(i16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as i64)
            }
            DataType::Uint16 => {
                Some(u16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as i64)
            }
            DataType::Int32 => {
                Some(i32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as i64)
            }
            // Read in the portable representation the encoder predicted from,
            // which is `int32` whatever the attribute declares. A `uint32`
            // attribute read unsigned puts every value above `i32::MAX` a
            // whole `2^32` from the number the correction was computed
            // against.
            DataType::Uint32 => {
                Some(i32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as i64)
            }
            DataType::Int64 => Some(i64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?)),
            DataType::Uint64 => {
                i64::try_from(u64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?)).ok()
            }
            DataType::Float32 => {
                float_to_i64(f32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as f64)
            }
            DataType::Float64 => {
                float_to_i64(f64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?))
            }
            DataType::Bool => Some(u8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as i64),
            _ => None,
        }
    }

    /// Three components at once, for the position parents.
    pub fn read_vector3_as_i64(&self, entry: usize, out: &mut [i64; 3]) -> bool {
        for (c, slot) in out.iter_mut().enumerate() {
            let Some(value) = self.read_component_as_i64(entry, c) else {
                return false;
            };
            *slot = value;
        }
        true
    }

    /// The parent value read at its declared type and converted to `f32`.
    ///
    /// Only the deprecated texture-coordinate predictor reads this way:
    /// upstream's pre-2.0 tex-coord prediction works on real position values,
    /// not portable ones, and its decoder hands the attribute itself over.
    #[cfg(any(
        feature = "legacy_bitstream_decode",
        feature = "legacy_bitstream_encode"
    ))]
    pub(crate) fn read_component_as_f32(&self, entry: usize, component: usize) -> Option<f32> {
        let buffer = self.att.buffer();
        let byte_stride = usize::try_from(self.att.byte_stride()).ok()?;
        let byte_offset = entry
            .checked_mul(byte_stride)?
            .checked_add(component.checked_mul(self.att.data_type().byte_length())?)?;

        match self.att.data_type() {
            DataType::Int8 => Some(i8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as f32),
            DataType::Uint8 => {
                Some(u8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as f32)
            }
            DataType::Int16 => {
                Some(i16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as f32)
            }
            DataType::Uint16 => {
                Some(u16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as f32)
            }
            DataType::Int32 => {
                Some(i32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as f32)
            }
            DataType::Uint32 => {
                Some(u32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as f32)
            }
            DataType::Float32 => Some(f32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?)),
            DataType::Float64 => {
                Some(f64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?) as f32)
            }
            _ => None,
        }
    }
}

fn read_bytes<const N: usize>(
    buffer: &crate::data_buffer::DataBuffer,
    byte_offset: usize,
) -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    if !buffer.try_read(byte_offset, &mut bytes) {
        return None;
    }
    Some(bytes)
}

fn float_to_i64(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    if value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return None;
    }
    Some(value as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute_of_type(data_type: DataType) -> PointAttribute {
        let mut att = PointAttribute::new();
        att.init(GeometryAttributeType::Position, 3, data_type, false, 1);
        att
    }

    #[test]
    #[cfg(any(feature = "decoder", feature = "encoder"))]
    fn test_read_component_as_i64_rejects_nan() {
        let mut att = attribute_of_type(DataType::Float32);
        att.buffer_mut().write(0, &f32::NAN.to_le_bytes());
        att.buffer_mut().write(4, &0.0f32.to_le_bytes());
        att.buffer_mut().write(8, &0.0f32.to_le_bytes());

        let parent = PredictionParent::legacy(&att);
        assert_eq!(parent.read_component_as_i64(0, 0), None);
    }

    #[test]
    fn test_read_component_as_i64_accepts_integer_positions() {
        let mut att = attribute_of_type(DataType::Int32);
        att.buffer_mut().write(0, &123i32.to_le_bytes());
        att.buffer_mut().write(4, &(-7i32).to_le_bytes());
        att.buffer_mut().write(8, &99i32.to_le_bytes());

        let parent = PredictionParent::portable(&att).expect("int32 is a portable type");
        let mut out = [0i64; 3];
        assert!(parent.read_vector3_as_i64(0, &mut out));
        assert_eq!(out, [123, -7, 99]);
    }

    #[test]
    fn test_read_component_as_i64_reads_a_uint32_position_as_the_portable_int32() {
        let mut att = attribute_of_type(DataType::Uint32);
        att.buffer_mut().write(0, &0u32.to_le_bytes());
        att.buffer_mut().write(4, &7u32.to_le_bytes());
        att.buffer_mut().write(8, &0xFFFF_FF00u32.to_le_bytes());

        let parent = PredictionParent::portable(&att).expect("uint32 is a portable type");
        let mut out = [0i64; 3];
        assert!(parent.read_vector3_as_i64(0, &mut out));
        // The encoder predicted from the portable `int32`, where these bits
        // read -256, not 4_294_967_040.
        assert_eq!(out, [0, 7, -256]);
    }

    #[test]
    fn test_read_component_as_i64_rejects_truncated_buffer() {
        let mut att = attribute_of_type(DataType::Int32);
        att.buffer_mut().write(0, &123i32.to_le_bytes());
        att.buffer_mut().write(4, &(-7i32).to_le_bytes());
        att.buffer_mut().resize(8);

        let parent = PredictionParent::portable(&att).expect("int32 is a portable type");
        assert_eq!(parent.read_component_as_i64(0, 2), None);
        let mut out = [0i64; 3];
        assert!(!parent.read_vector3_as_i64(0, &mut out));
    }

    /// The validation is what keeps a float or 64-bit attribute from ever
    /// becoming a portable parent -- the route both bugs travelled.
    #[test]
    fn portable_refuses_every_type_the_portable_pass_cannot_have_written() {
        for data_type in [
            DataType::Float32,
            DataType::Float64,
            DataType::Int64,
            DataType::Uint64,
        ] {
            let att = attribute_of_type(data_type);
            assert!(
                PredictionParent::portable(&att).is_err(),
                "{data_type:?} must not bind as a portable parent"
            );
        }
        for data_type in [
            DataType::Int8,
            DataType::Uint8,
            DataType::Int16,
            DataType::Uint16,
            DataType::Int32,
            DataType::Uint32,
        ] {
            let att = attribute_of_type(data_type);
            assert!(
                PredictionParent::portable(&att).is_ok(),
                "{data_type:?} is a type the portable pass writes"
            );
        }
    }
}
