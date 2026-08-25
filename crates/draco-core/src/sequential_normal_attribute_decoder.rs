//! Normal sequential attribute decoder.
//!
//! [`SequentialNormalAttributeDecoder`] decodes octahedron-encoded normals: the
//! integer decoder reads the quantized 2D octahedral pair into a portable
//! attribute, and the caller's inverse octahedral transform reconstructs unit
//! normals from it. Decode-side counterpart of
//! `SequentialNormalAttributeEncoder`, and port of Draco's
//! `sequential_normal_attribute_decoder.h`.
//!
//! Upstream needs nothing here beyond the attribute id: its prediction scheme
//! factory downcasts the decoder to `MeshDecoder` and takes the corner table
//! off it, so one class serves both geometry types. This port passes the mesh
//! data down explicitly instead, which is why the mesh-only arguments below are
//! options -- a point cloud has none of them and passes `None`.

use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme::EntryToPointIdMap;
use crate::sequential_integer_attribute_decoder::SequentialIntegerAttributeDecoder;
use crate::status::{DracoError, Status};

/// How the portable attribute may be sized before a value has been read.
pub enum PortableExtent {
    /// A count the connectivity already produced. The mesh decoder's point-id
    /// list comes out of the decoded corner table, so it is backed by data and
    /// can be reserved in one go.
    Decoded(usize),
    /// A count the header states and nothing has yet backed. A point cloud has
    /// no connectivity to derive one from, so the buffer grows as values arrive
    /// rather than trusting the claim.
    Declared(usize),
}

pub struct SequentialNormalAttributeDecoder {
    base: SequentialIntegerAttributeDecoder,
    quantization_bits: u8,
}

impl Default for SequentialNormalAttributeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialNormalAttributeDecoder {
    pub fn new() -> Self {
        Self {
            base: SequentialIntegerAttributeDecoder::new(),
            quantization_bits: 0,
        }
    }

    /// The octahedron bit count read from the stream, once `decode_values` has
    /// run. Zero for 2.0 and later, where the count follows the values and the
    /// caller takes it from the transform data instead.
    pub fn quantization_bits(&self) -> u8 {
        self.quantization_bits
    }

    /// Refuses an attribute the octahedral encoding cannot describe, as
    /// upstream's `Init` does: three components and float32, checked before a
    /// byte is read rather than when the inverse transform later fails on the
    /// same thing.
    pub fn init(
        &mut self,
        decoder: &PointCloudDecoder,
        point_cloud: &PointCloud,
        attribute_id: i32,
    ) -> Status {
        self.base.init(decoder, attribute_id);

        let attribute = point_cloud.try_attribute(attribute_id)?;
        if attribute.num_components() != 3 {
            return Err(DracoError::general(format!(
                "Normal decoding needs 3 components, attribute {attribute_id} has {}",
                attribute.num_components()
            )));
        }
        if attribute.data_type() != DataType::Float32 {
            return Err(DracoError::general(format!(
                "Normal decoding needs float32 values, attribute {attribute_id} is {:?}",
                attribute.data_type()
            )));
        }
        Ok(())
    }

    /// Reads the octahedron's bit count out of a pre-2.0 stream and reports how
    /// many bytes the integer decode has to step over to reach the values.
    ///
    /// Below 2.0 the count rides between the prediction header and the integer
    /// values; 2.0 and later carry it after them, in the transform data. The
    /// buffer is left where it was found, because the integer decoder reads
    /// that same prediction header itself.
    #[cfg(feature = "legacy_bitstream_decode")]
    fn read_inline_quantization_bits(
        &mut self,
        buffer: &mut DecoderBuffer,
    ) -> Result<usize, DracoError> {
        let saved_pos = buffer.position();
        let method_byte = buffer
            .decode_u8()
            .map_err(|_| DracoError::general("Failed to read prediction method".to_string()))?;
        let carries_transform = crate::point_cloud_decoder::carries_transform_byte(method_byte);
        if carries_transform {
            buffer
                .decode_u8()
                .map_err(|_| DracoError::general("Failed to read transform type".to_string()))?;
        }
        let quantization_bits = buffer
            .decode_u8()
            .map_err(|_| DracoError::general("Failed to read normal quant_bits".to_string()))?;
        if !AttributeOctahedronTransform::is_valid_quantization_bits(quantization_bits as i32) {
            return Err(DracoError::general(
                "Invalid normal quantization bits".to_string(),
            ));
        }
        self.quantization_bits = quantization_bits;

        let bytes_consumed = buffer.position() - saved_pos;
        let header_bytes = if carries_transform { 2 } else { 1 };
        buffer
            .set_position(saved_pos)
            .map_err(|_| DracoError::general("Failed to reset buffer position".to_string()))?;
        Ok(bytes_consumed - header_bytes)
    }

    /// Decodes the octahedral pair into a portable attribute and hands it back
    /// for the caller's inverse transform.
    ///
    /// The mesh-only arguments are the corner table and its two maps, plus the
    /// portable position that a 2.0+ geometric-normal prediction predicts from;
    /// a point cloud passes `None` for all of them.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_values(
        &mut self,
        point_cloud: &mut PointCloud,
        point_ids: EntryToPointIdMap<'_>,
        buffer: &mut DecoderBuffer,
        bitstream_version: u16,
        extent: PortableExtent,
        corner_table: Option<&CornerTable>,
        data_to_corner_map: Option<&[u32]>,
        vertex_to_data_map: Option<&[i32]>,
        portable_parent_attribute: Option<&PointAttribute>,
    ) -> Result<PointAttribute, DracoError> {
        let mut portable = PointAttribute::default();
        match extent {
            PortableExtent::Decoded(size) => portable.try_init(
                GeometryAttributeType::Generic,
                2,
                DataType::Uint32,
                false,
                size,
            )?,
            PortableExtent::Declared(size) => portable.init_deferred(
                GeometryAttributeType::Generic,
                2,
                DataType::Uint32,
                false,
                size,
            )?,
        }

        let skip_bytes = if bitstream_version < 0x0200 {
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            {
                return Err(DracoError::bitstream_version_unsupported());
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            {
                self.read_inline_quantization_bits(buffer)?
            }
        } else {
            0
        };

        let mut skip = move |buf: &mut DecoderBuffer<'_>| -> bool {
            skip_bytes == 0 || buf.try_advance(skip_bytes).is_ok()
        };
        let hook: Option<&mut dyn FnMut(&mut DecoderBuffer<'_>) -> bool> = if skip_bytes > 0 {
            Some(&mut skip)
        } else {
            None
        };

        self.base.decode_values(
            point_cloud,
            point_ids,
            buffer,
            corner_table,
            data_to_corner_map,
            vertex_to_data_map,
            Some(&mut portable),
            portable_parent_attribute,
            hook,
        )?;
        Ok(portable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_with_normal(num_components: u8, data_type: DataType) -> PointCloud {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(4);
        let mut normal = PointAttribute::new();
        normal.init(
            GeometryAttributeType::Normal,
            num_components,
            data_type,
            true,
            4,
        );
        point_cloud.add_attribute(normal);
        point_cloud
    }

    /// Upstream's `Init` refuses both of these before reading a byte. The port
    /// used to reach the same verdict only when the inverse octahedral
    /// transform later failed on the component count, and never at all for the
    /// data type.
    #[test]
    fn init_refuses_an_attribute_the_octahedral_encoding_cannot_describe() {
        let decoder = PointCloudDecoder::new();

        let two_components = cloud_with_normal(2, DataType::Float32);
        let error = SequentialNormalAttributeDecoder::new()
            .init(&decoder, &two_components, 0)
            .expect_err("a two-component normal is not octahedral");
        assert!(
            error.to_string().contains("3 components"),
            "unexpected message: {error}"
        );

        let integers = cloud_with_normal(3, DataType::Uint32);
        let error = SequentialNormalAttributeDecoder::new()
            .init(&decoder, &integers, 0)
            .expect_err("octahedral normals reconstruct into float32");
        assert!(
            error.to_string().contains("float32"),
            "unexpected message: {error}"
        );

        let normals = cloud_with_normal(3, DataType::Float32);
        SequentialNormalAttributeDecoder::new()
            .init(&decoder, &normals, 0)
            .expect("a three-component float32 normal is what the encoder writes");
    }
}
