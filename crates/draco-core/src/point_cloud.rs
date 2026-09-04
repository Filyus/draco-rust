use std::collections::HashMap;

use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{AttributeValueIndex, PointIndex};
use crate::metadata::{AttributeMetadata, GeometryMetadata, Metadata};
use crate::status::{DracoError, Status};

/// Point cloud geometry with typed attributes and optional metadata.
#[derive(Debug, Default, Clone)]
pub struct PointCloud {
    attributes: Vec<PointAttribute>,
    num_points: usize,
    metadata: Option<GeometryMetadata>,
    /// The value storage and explicit maps of attributes a `clear` dropped,
    /// handed to the next attributes added so a decode into a cloud that has
    /// already decoded grows into the last decode's allocations rather than
    /// making new ones. Empty on a cloud that has never been cleared.
    spare_storage: Vec<(Vec<u8>, Vec<AttributeValueIndex>)>,
}

impl PointCloud {
    /// Creates an empty point cloud.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops every attribute, point and the metadata, keeping the allocated
    /// capacity of the attribute list and the attributes' own storage.
    ///
    /// What a decode does to the cloud it is given, so that decoding into one
    /// that already holds geometry replaces it rather than adding to it. The
    /// values and the explicit maps of the dropped attributes are kept empty
    /// and handed to the next attributes added, so the caller decoding many
    /// files into one cloud reuses the allocations of the last one; the cloud
    /// therefore holds the memory of the largest geometry it has decoded until
    /// [`release_spare_storage`](Self::release_spare_storage) or drop.
    pub fn clear(&mut self) {
        for mut attribute in self.attributes.drain(..) {
            let storage = attribute.take_storage();
            if storage.0.capacity() > 0 || storage.1.capacity() > 0 {
                self.spare_storage.push(storage);
            }
        }
        self.num_points = 0;
        self.metadata = None;
    }

    /// Frees the storage [`clear`](Self::clear) retained from earlier
    /// attributes.
    pub fn release_spare_storage(&mut self) {
        self.spare_storage = Vec::new();
    }

    /// Hands a spare storage to an attribute that has none of its own.
    fn adopt_spare_storage(&mut self, attribute: &mut PointAttribute) {
        if attribute.buffer().has_storage() {
            return;
        }
        if let Some(storage) = self.spare_storage.pop() {
            attribute.adopt_storage(storage);
        }
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
        self.adopt_spare_storage(&mut attribute);
        self.attributes.push(attribute);
        id
    }

    /// Adds an attribute while preserving its existing unique id.
    pub fn add_attribute_preserve_unique_id(&mut self, mut attribute: PointAttribute) -> i32 {
        if self.num_points == 0 && attribute.size() > 0 {
            self.num_points = attribute.size();
        }
        let id = self.attributes.len() as i32;
        self.adopt_spare_storage(&mut attribute);
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
            return Err(DracoError::general(
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
            return Err(DracoError::general(
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
    /// Merges bit-identical values in every attribute.
    ///
    /// Port of upstream's `PointCloud::DeduplicateAttributeValues`, which its
    /// OBJ and PLY readers and its `TriangleSoupMeshBuilder` all run before
    /// the encoder sees the geometry. Fails on an attribute whose type
    /// upstream's own switch does not cover, which is what upstream does too.
    pub fn deduplicate_attribute_values(&mut self) -> Status {
        if self.num_points() == 0 {
            return Ok(());
        }
        for att_id in 0..self.num_attributes() {
            self.attribute_mut(att_id).deduplicate_values()?;
        }
        Ok(())
    }

    /// Merges points whose attribute values all coincide, keeping the order in
    /// which they first appear.
    ///
    /// Port of upstream's `PointCloud::DeduplicatePointIds`. Two points are the
    /// same point when every attribute maps them to the same value, which is
    /// why [`deduplicate_attribute_values`](Self::deduplicate_attribute_values)
    /// runs first: without it two vertices carrying equal bytes still hold
    /// distinct value indices and nothing merges.
    pub fn deduplicate_point_ids(&mut self) {
        self.deduplicate_point_ids_returning_map();
    }

    /// [`deduplicate_point_ids`](Self::deduplicate_point_ids), handing back the
    /// old-point-to-new-point map when anything merged.
    ///
    /// A mesh needs the map: its faces name points, and upstream's `Mesh`
    /// override remaps them right after the point cloud's own part runs.
    pub(crate) fn deduplicate_point_ids_returning_map(&mut self) -> Option<Vec<u32>> {
        let num_points = self.num_points();
        if num_points == 0 || self.num_attributes() == 0 {
            return None;
        }

        let key_of = |pc: &Self, point: usize| -> Vec<u32> {
            (0..pc.num_attributes())
                .map(|att_id| {
                    pc.attribute(att_id)
                        .mapped_index(PointIndex(point as u32))
                        .0
                })
                .collect()
        };

        let mut first_seen: HashMap<Vec<u32>, u32> = HashMap::with_capacity(num_points);
        let mut index_map: Vec<u32> = Vec::with_capacity(num_points);
        let mut unique_points: Vec<u32> = Vec::new();
        let mut num_unique = 0u32;
        for point in 0..num_points {
            match first_seen.entry(key_of(self, point)) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    index_map.push(*entry.get());
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(num_unique);
                    index_map.push(num_unique);
                    unique_points.push(point as u32);
                    num_unique += 1;
                }
            }
        }
        if num_unique as usize == num_points {
            return None;
        }

        // Each attribute's new map is built whole and installed whole. Not
        // through `set_point_map_entry`, which validates each entry against
        // the value count and panics on one it does not like: an attribute
        // with no values maps every point to the invalid index, that index is
        // what a survivor carries, and reinstalling it is not an error -- it
        // is the same map, shorter. The encoder refuses such an attribute
        // later, where the refusal can be reported.
        for att_id in 0..self.num_attributes() {
            let values: Vec<AttributeValueIndex> = unique_points
                .iter()
                .map(|old| self.attribute(att_id).mapped_index(PointIndex(*old)))
                .collect();
            self.attribute_mut(att_id)
                .set_explicit_mapping_from(&values);
        }
        self.set_num_points(num_unique as usize);
        Some(index_map)
    }

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
    use crate::draco_types::DataType;
    use crate::geometry_indices::INVALID_ATTRIBUTE_VALUE_INDEX;

    fn attribute_with_values(num_values: usize, fill: u8) -> PointAttribute {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            num_values,
        );
        attribute.buffer_mut().data_mut().fill(fill);
        attribute.set_explicit_mapping(num_values);
        attribute
    }

    /// A cloud that has been cleared hands the dropped attributes' storage to
    /// the next attributes added, and what they read from it is what a fresh
    /// attribute reads: zeros where nothing was written, the invalid index
    /// where no mapping was set.
    #[test]
    fn clear_keeps_attribute_storage_for_the_next_attributes_and_hands_it_over_empty() {
        let mut point_cloud = PointCloud::new();
        point_cloud.add_attribute(attribute_with_values(100, 0xAB));
        point_cloud.clear();
        assert_eq!(point_cloud.num_attributes(), 0);
        assert_eq!(point_cloud.spare_storage.len(), 1);

        let mut next = PointAttribute::new();
        next.init_deferred(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            10,
        )
        .unwrap();
        let id = point_cloud.add_attribute(next);
        assert!(point_cloud.spare_storage.is_empty());
        let next = point_cloud.attribute_mut(id);
        assert!(next.buffer().has_storage(), "the storage was handed over");
        assert_eq!(next.buffer().data_size(), 0);
        next.resize_unique_entries(10).unwrap();
        assert!(next.buffer().data().iter().all(|&b| b == 0));
        next.set_explicit_mapping(10);
        assert!((0..10).all(|p| next.mapped_index(PointIndex(p)) == INVALID_ATTRIBUTE_VALUE_INDEX));
    }

    /// An attribute that already owns storage keeps it; the spare stays for
    /// one that does not.
    #[test]
    fn an_attribute_with_its_own_storage_does_not_take_a_spare() {
        let mut point_cloud = PointCloud::new();
        point_cloud.add_attribute(attribute_with_values(100, 0xAB));
        point_cloud.clear();
        point_cloud.add_attribute(attribute_with_values(5, 0xCD));
        assert_eq!(point_cloud.spare_storage.len(), 1);
        assert!(point_cloud
            .attribute(0)
            .buffer()
            .data()
            .iter()
            .all(|&b| b == 0xCD));
    }

    #[test]
    fn release_spare_storage_drops_what_clear_kept() {
        let mut point_cloud = PointCloud::new();
        point_cloud.add_attribute(attribute_with_values(100, 0xAB));
        point_cloud.clear();
        point_cloud.release_spare_storage();
        assert!(point_cloud.spare_storage.is_empty());
    }

    #[test]
    fn try_attribute_rejects_out_of_range_ids() {
        let mut point_cloud = PointCloud::new();

        assert!(point_cloud.try_attribute(-1).is_err());
        assert!(point_cloud.try_attribute(0).is_err());
        assert!(point_cloud.try_attribute_mut(-1).is_err());
        assert!(point_cloud.try_attribute_mut(0).is_err());
    }

    /// One position repeated, under identity mapping: the values merge and the
    /// mapping becomes explicit, because two points now name one value.
    #[test]
    fn identical_values_merge_and_the_mapping_turns_explicit() {
        use crate::draco_types::DataType;

        let positions: [f32; 12] = [
            0.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, //
            1.0, 0.0, 0.0, // the duplicate
            0.0, 1.0, 0.0,
        ];
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            4,
        );
        let bytes: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        attribute.buffer_mut().data_mut().copy_from_slice(&bytes);
        attribute.set_identity_mapping();

        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(4);
        point_cloud.add_attribute(attribute);
        point_cloud
            .deduplicate_attribute_values()
            .expect("supported");

        let attribute = point_cloud.attribute(0);
        assert_eq!(attribute.size(), 3, "the duplicate value survived");
        assert!(!attribute.is_mapping_identity());
        assert_eq!(
            attribute.mapped_index(PointIndex(1)),
            AttributeValueIndex(1)
        );
        assert_eq!(
            attribute.mapped_index(PointIndex(2)),
            AttributeValueIndex(1),
            "the duplicate does not point at the value that replaced it"
        );
        assert_eq!(
            attribute.mapped_index(PointIndex(3)),
            AttributeValueIndex(2)
        );
    }

    /// Points are the same point when every attribute maps them to the same
    /// value, and the survivors keep the order they arrived in.
    #[test]
    fn points_naming_the_same_values_merge_in_arrival_order() {
        use crate::draco_types::DataType;

        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            2,
        );
        let bytes: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        attribute.buffer_mut().data_mut().copy_from_slice(&bytes);
        // Four points over two values: 0, 1, 1, 0.
        attribute.set_explicit_mapping_from(&[
            AttributeValueIndex(0),
            AttributeValueIndex(1),
            AttributeValueIndex(1),
            AttributeValueIndex(0),
        ]);

        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(4);
        point_cloud.add_attribute(attribute);
        point_cloud.deduplicate_point_ids();

        assert_eq!(point_cloud.num_points(), 2);
        let attribute = point_cloud.attribute(0);
        assert_eq!(
            attribute.mapped_index(PointIndex(0)),
            AttributeValueIndex(0)
        );
        assert_eq!(
            attribute.mapped_index(PointIndex(1)),
            AttributeValueIndex(1)
        );
    }

    /// Upstream's switch covers nothing 64 bits wide, and returns an error
    /// rather than leaving the values alone. So does this.
    #[test]
    fn a_width_upstream_does_not_deduplicate_is_refused() {
        use crate::draco_types::DataType;

        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float64,
            false,
            2,
        );
        attribute.set_identity_mapping();

        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(2);
        point_cloud.add_attribute(attribute);
        assert!(point_cloud.deduplicate_attribute_values().is_err());
    }
}
