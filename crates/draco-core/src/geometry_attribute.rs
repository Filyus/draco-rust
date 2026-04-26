use crate::attribute_transform_data::AttributeTransformData;
use crate::data_buffer::DataBuffer;
use crate::draco_types::DataType;
use crate::geometry_indices::{AttributeValueIndex, PointIndex, INVALID_ATTRIBUTE_VALUE_INDEX};
use crate::status::DracoError;
use std::convert::TryFrom;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryAttributeType {
    Invalid = -1,
    Position = 0,
    Normal,
    Color,
    TexCoord,
    Generic,
}

impl TryFrom<u8> for GeometryAttributeType {
    type Error = DracoError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Position),
            1 => Ok(Self::Normal),
            2 => Ok(Self::Color),
            3 => Ok(Self::TexCoord),
            4 => Ok(Self::Generic),
            _ => Err(DracoError::DracoError(format!(
                "Invalid geometry attribute type: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeometryAttribute {
    attribute_type: GeometryAttributeType,
    data_type: DataType,
    num_components: u8,
    normalized: bool,
    byte_stride: i64,
    byte_offset: i64,
    unique_id: u32,
}

impl Default for GeometryAttribute {
    fn default() -> Self {
        Self {
            attribute_type: GeometryAttributeType::Invalid,
            data_type: DataType::Invalid,
            num_components: 0,
            normalized: false,
            byte_stride: 0,
            byte_offset: 0,
            unique_id: 0,
        }
    }
}

impl GeometryAttribute {
    // Attribute initialization requires 7 parameters to fully specify metadata:
    // type, components, data_type, normalized flag, num_values, byte_stride, byte_offset.
    // This matches the C++ PointAttribute::Init() signature and cannot be simplified
    // without breaking API compatibility or making attribute setup less explicit.
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        attribute_type: GeometryAttributeType,
        _buffer: Option<&DataBuffer>,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        byte_stride: i64,
        byte_offset: i64,
    ) {
        self.attribute_type = attribute_type;
        self.num_components = num_components;
        self.data_type = data_type;
        self.normalized = normalized;
        self.byte_stride = byte_stride;
        self.byte_offset = byte_offset;
    }

    pub fn attribute_type(&self) -> GeometryAttributeType {
        self.attribute_type
    }

    pub fn data_type(&self) -> DataType {
        self.data_type
    }

    pub fn num_components(&self) -> u8 {
        self.num_components
    }

    pub fn normalized(&self) -> bool {
        self.normalized
    }

    pub fn byte_stride(&self) -> i64 {
        self.byte_stride
    }

    pub fn byte_offset(&self) -> i64 {
        self.byte_offset
    }

    pub fn unique_id(&self) -> u32 {
        self.unique_id
    }

    pub fn set_unique_id(&mut self, id: u32) {
        self.unique_id = id;
    }

    pub fn set_attribute_type(&mut self, attribute_type: GeometryAttributeType) {
        self.attribute_type = attribute_type;
    }

    pub fn set_data_type(&mut self, data_type: DataType) {
        self.data_type = data_type;
    }

    pub fn set_num_components(&mut self, num_components: u8) {
        self.num_components = num_components;
    }
}

#[derive(Debug, Clone)]
pub struct PointAttribute {
    base: GeometryAttribute,
    buffer: DataBuffer,
    indices_map: Vec<AttributeValueIndex>,
    identity_mapping: bool,
    num_unique_entries: usize,
    attribute_transform_data: Option<Box<AttributeTransformData>>,
}

impl Default for PointAttribute {
    fn default() -> Self {
        Self {
            base: GeometryAttribute::default(),
            buffer: DataBuffer::new(),
            indices_map: Vec::new(),
            identity_mapping: true,
            num_unique_entries: 0,
            attribute_transform_data: None,
        }
    }
}

impl PointAttribute {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(
        &mut self,
        attribute_type: GeometryAttributeType,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        num_attribute_values: usize,
    ) {
        let byte_stride = (num_components as usize * data_type.byte_length()) as i64;
        self.base.init(
            attribute_type,
            None,
            num_components,
            data_type,
            normalized,
            byte_stride,
            0,
        );
        self.buffer
            .resize(num_attribute_values * byte_stride as usize);
        self.num_unique_entries = num_attribute_values;
        self.identity_mapping = true;
    }

    pub fn try_init(
        &mut self,
        attribute_type: GeometryAttributeType,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        num_attribute_values: usize,
    ) -> Result<(), DracoError> {
        let byte_stride = num_components as usize * data_type.byte_length();
        let buffer_size = num_attribute_values
            .checked_mul(byte_stride)
            .ok_or_else(|| {
                DracoError::DracoError("Point attribute buffer size overflow".to_string())
            })?;
        self.base.init(
            attribute_type,
            None,
            num_components,
            data_type,
            normalized,
            byte_stride as i64,
            0,
        );
        self.buffer.try_resize(buffer_size).map_err(|_| {
            DracoError::DracoError("Failed to allocate point attribute buffer".to_string())
        })?;
        self.num_unique_entries = num_attribute_values;
        self.identity_mapping = true;
        Ok(())
    }

    pub fn mapped_index(&self, point_index: PointIndex) -> AttributeValueIndex {
        if self.identity_mapping {
            AttributeValueIndex(point_index.0)
        } else if (point_index.0 as usize) < self.indices_map.len() {
            self.indices_map[point_index.0 as usize]
        } else {
            INVALID_ATTRIBUTE_VALUE_INDEX
        }
    }

    pub fn size(&self) -> usize {
        self.num_unique_entries
    }

    pub fn resize_unique_entries(&mut self, num_attribute_values: usize) -> Result<(), DracoError> {
        let byte_stride = self.byte_stride() as usize;
        let buffer_size = num_attribute_values
            .checked_mul(byte_stride)
            .ok_or_else(|| {
                DracoError::DracoError("Point attribute buffer size overflow".to_string())
            })?;
        self.buffer.try_resize(buffer_size).map_err(|_| {
            DracoError::DracoError("Failed to allocate point attribute buffer".to_string())
        })?;
        self.num_unique_entries = num_attribute_values;
        if self.identity_mapping {
            self.indices_map.clear();
        }
        Ok(())
    }

    pub fn buffer(&self) -> &DataBuffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut DataBuffer {
        &mut self.buffer
    }

    pub fn attribute_type(&self) -> GeometryAttributeType {
        self.base.attribute_type()
    }

    pub fn unique_id(&self) -> u32 {
        self.base.unique_id()
    }

    pub fn set_unique_id(&mut self, id: u32) {
        self.base.set_unique_id(id);
    }

    pub fn set_attribute_type(&mut self, attribute_type: GeometryAttributeType) {
        self.base.set_attribute_type(attribute_type);
    }

    pub fn set_data_type(&mut self, data_type: DataType) {
        self.base.set_data_type(data_type);
    }

    pub fn set_num_components(&mut self, num_components: u8) {
        self.base.set_num_components(num_components);
    }

    pub fn set_identity_mapping(&mut self) {
        self.identity_mapping = true;
        self.indices_map.clear();
    }

    pub fn set_explicit_mapping(&mut self, num_points: usize) {
        self.identity_mapping = false;
        self.indices_map
            .resize(num_points, INVALID_ATTRIBUTE_VALUE_INDEX);
    }

    pub fn set_point_map_entry(
        &mut self,
        point_index: PointIndex,
        entry_index: AttributeValueIndex,
    ) {
        self.try_set_point_map_entry(point_index, entry_index)
            .expect("point map entry must be in range");
    }

    pub fn try_set_point_map_entry(
        &mut self,
        point_index: PointIndex,
        entry_index: AttributeValueIndex,
    ) -> Result<(), DracoError> {
        if self.identity_mapping {
            return Ok(());
        }
        let Some(slot) = self.indices_map.get_mut(point_index.0 as usize) else {
            return Err(DracoError::DracoError(
                "Point map entry index out of range".to_string(),
            ));
        };
        *slot = entry_index;
        Ok(())
    }

    pub fn set_attribute_transform_data(&mut self, data: AttributeTransformData) {
        self.attribute_transform_data = Some(Box::new(data));
    }

    pub fn attribute_transform_data(&self) -> Option<&AttributeTransformData> {
        self.attribute_transform_data.as_deref()
    }

    pub fn data_type(&self) -> DataType {
        self.base.data_type()
    }

    pub fn normalized(&self) -> bool {
        self.base.normalized()
    }

    pub fn num_components(&self) -> u8 {
        self.base.num_components()
    }

    pub fn byte_stride(&self) -> i64 {
        self.base.byte_stride()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_set_point_map_entry_rejects_out_of_range_point() {
        let mut attribute = PointAttribute::new();
        attribute.set_explicit_mapping(1);

        assert!(attribute
            .try_set_point_map_entry(PointIndex(1), AttributeValueIndex(0))
            .is_err());
    }
}
