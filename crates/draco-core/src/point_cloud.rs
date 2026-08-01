use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::metadata::{AttributeMetadata, GeometryMetadata, Metadata};
use crate::status::DracoError;

/// Point cloud geometry with typed attributes and optional metadata.
#[derive(Debug, Default, Clone)]
pub struct PointCloud {
    attributes: Vec<PointAttribute>,
    num_points: usize,
    metadata: Option<GeometryMetadata>,
}

impl PointCloud {
    /// Creates an empty point cloud.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the number of logical points.
    pub fn set_num_points(&mut self, num_points: usize) {
        self.num_points = num_points;
    }

    /// Adds an attribute and assigns it a unique id matching its attribute id.
    pub fn add_attribute(&mut self, mut attribute: PointAttribute) -> i32 {
        if self.num_points == 0 && attribute.size() > 0 {
            self.num_points = attribute.size();
        }
        let id = self.attributes.len() as i32;
        attribute.set_unique_id(id as u32);
        self.attributes.push(attribute);
        id
    }

    /// Adds an attribute while preserving its existing unique id.
    pub fn add_attribute_preserve_unique_id(&mut self, attribute: PointAttribute) -> i32 {
        if self.num_points == 0 && attribute.size() > 0 {
            self.num_points = attribute.size();
        }
        let id = self.attributes.len() as i32;
        self.attributes.push(attribute);
        id
    }

    /// Places an attribute at `att_id`, growing the attribute list if needed.
    ///
    /// Mirrors C++ `PointCloud::SetAttribute`: the attribute's unique id is set
    /// to `att_id`. Any vacancies created when growing the list are filled with
    /// empty attributes.
    pub fn set_attribute(&mut self, att_id: i32, mut attribute: PointAttribute) {
        debug_assert!(att_id >= 0);
        let index = att_id as usize;
        if index >= self.attributes.len() {
            self.attributes.resize_with(index + 1, PointAttribute::new);
        }
        attribute.set_unique_id(att_id as u32);
        self.attributes[index] = attribute;
    }

    /// Returns the number of attributes.
    pub fn num_attributes(&self) -> i32 {
        self.attributes.len() as i32
    }

    /// Returns the attribute id for the given Draco unique id, or -1.
    ///
    /// Mirrors C++ `PointCloud::GetAttributeIdByUniqueId`.
    pub fn attribute_id_by_unique_id(&self, unique_id: u32) -> i32 {
        for (i, att) in self.attributes.iter().enumerate() {
            if att.unique_id() == unique_id {
                return i as i32;
            }
        }
        -1
    }

    /// Returns the attribute with the given Draco unique id.
    ///
    /// Mirrors C++ `PointCloud::GetAttributeByUniqueId`.
    pub fn attribute_by_unique_id(&self, unique_id: u32) -> Option<&PointAttribute> {
        let id = self.attribute_id_by_unique_id(unique_id);
        (id >= 0).then(|| &self.attributes[id as usize])
    }

    /// Returns an attribute by attribute id.
    pub fn attribute(&self, att_id: i32) -> &PointAttribute {
        &self.attributes[att_id as usize]
    }

    /// Fallibly returns an attribute by attribute id.
    pub fn try_attribute(&self, att_id: i32) -> Result<&PointAttribute, DracoError> {
        let Some(attribute) = (att_id >= 0)
            .then_some(att_id as usize)
            .and_then(|index| self.attributes.get(index))
        else {
            return Err(DracoError::General(
                "Point cloud attribute id out of range".to_string(),
            ));
        };
        Ok(attribute)
    }

    /// Returns a mutable attribute by attribute id.
    pub fn attribute_mut(&mut self, att_id: i32) -> &mut PointAttribute {
        &mut self.attributes[att_id as usize]
    }

    /// Fallibly returns a mutable attribute by attribute id.
    pub fn try_attribute_mut(&mut self, att_id: i32) -> Result<&mut PointAttribute, DracoError> {
        let Some(attribute) = (att_id >= 0)
            .then_some(att_id as usize)
            .and_then(|index| self.attributes.get_mut(index))
        else {
            return Err(DracoError::General(
                "Point cloud attribute id out of range".to_string(),
            ));
        };
        Ok(attribute)
    }

    /// Returns the first attribute id with the requested semantic type, or -1.
    pub fn named_attribute_id(&self, att_type: GeometryAttributeType) -> i32 {
        for (i, att) in self.attributes.iter().enumerate() {
            if att.attribute_type() == att_type {
                return i as i32;
            }
        }
        -1
    }

    /// Returns the first attribute with the requested semantic type.
    pub fn named_attribute(&self, att_type: GeometryAttributeType) -> Option<&PointAttribute> {
        let id = self.named_attribute_id(att_type);
        if id >= 0 {
            Some(&self.attributes[id as usize])
        } else {
            None
        }
    }

    /// Returns the number of logical points.
    pub fn num_points(&self) -> usize {
        self.num_points
    }

    /// Returns geometry metadata, if present.
    pub fn metadata(&self) -> Option<&GeometryMetadata> {
        self.metadata.as_ref()
    }

    /// Returns mutable geometry metadata, if present.
    pub fn metadata_mut(&mut self) -> Option<&mut GeometryMetadata> {
        self.metadata.as_mut()
    }

    /// Returns geometry metadata, inserting an empty block when absent.
    pub fn metadata_or_insert(&mut self) -> &mut GeometryMetadata {
        self.metadata.get_or_insert_with(GeometryMetadata::new)
    }

    /// Replaces geometry metadata.
    pub fn set_metadata(&mut self, metadata: Option<GeometryMetadata>) {
        self.metadata = metadata;
    }

    /// Finds per-attribute metadata by Draco attribute unique id.
    pub fn attribute_metadata_by_unique_id(
        &self,
        attribute_unique_id: u32,
    ) -> Option<&AttributeMetadata> {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.attribute_metadata_by_unique_id(attribute_unique_id))
    }

    /// Finds per-attribute metadata by a string metadata entry.
    pub fn attribute_metadata_by_string_entry(
        &self,
        entry_name: &str,
        entry_value: &str,
    ) -> Option<&AttributeMetadata> {
        self.metadata.as_ref().and_then(|metadata| {
            metadata.attribute_metadata_by_string_entry(entry_name, entry_value)
        })
    }

    /// Sets metadata for an attribute id.
    pub fn set_attribute_metadata(
        &mut self,
        att_id: i32,
        metadata: Metadata,
    ) -> Result<(), DracoError> {
        let unique_id = self.try_attribute(att_id)?.unique_id();
        self.metadata_or_insert()
            .set_attribute_metadata(unique_id, metadata);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_attribute_rejects_out_of_range_ids() {
        let mut point_cloud = PointCloud::new();

        assert!(point_cloud.try_attribute(-1).is_err());
        assert!(point_cloud.try_attribute(0).is_err());
        assert!(point_cloud.try_attribute_mut(-1).is_err());
        assert!(point_cloud.try_attribute_mut(0).is_err());
    }
}
