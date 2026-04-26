use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::status::DracoError;

#[derive(Debug, Default, Clone)]
pub struct PointCloud {
    attributes: Vec<PointAttribute>,
    num_points: usize,
}

impl PointCloud {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_num_points(&mut self, num_points: usize) {
        self.num_points = num_points;
    }

    pub fn add_attribute(&mut self, mut attribute: PointAttribute) -> i32 {
        if self.num_points == 0 && attribute.size() > 0 {
            self.num_points = attribute.size();
        }
        // Assign unique id if not set?
        // In C++, it seems to just add it.
        // But we need to handle unique ids.
        // For now, just push.
        let id = self.attributes.len() as i32;
        attribute.set_unique_id(id as u32);
        self.attributes.push(attribute);
        id
    }

    pub fn num_attributes(&self) -> i32 {
        self.attributes.len() as i32
    }

    pub fn attribute(&self, att_id: i32) -> &PointAttribute {
        &self.attributes[att_id as usize]
    }

    pub fn try_attribute(&self, att_id: i32) -> Result<&PointAttribute, DracoError> {
        let Some(attribute) = (att_id >= 0)
            .then_some(att_id as usize)
            .and_then(|index| self.attributes.get(index))
        else {
            return Err(DracoError::DracoError(
                "Point cloud attribute id out of range".to_string(),
            ));
        };
        Ok(attribute)
    }

    pub fn attribute_mut(&mut self, att_id: i32) -> &mut PointAttribute {
        &mut self.attributes[att_id as usize]
    }

    pub fn try_attribute_mut(&mut self, att_id: i32) -> Result<&mut PointAttribute, DracoError> {
        let Some(attribute) = (att_id >= 0)
            .then_some(att_id as usize)
            .and_then(|index| self.attributes.get_mut(index))
        else {
            return Err(DracoError::DracoError(
                "Point cloud attribute id out of range".to_string(),
            ));
        };
        Ok(attribute)
    }

    pub fn named_attribute_id(&self, att_type: GeometryAttributeType) -> i32 {
        for (i, att) in self.attributes.iter().enumerate() {
            if att.attribute_type() == att_type {
                return i as i32;
            }
        }
        -1
    }

    pub fn named_attribute(&self, att_type: GeometryAttributeType) -> Option<&PointAttribute> {
        let id = self.named_attribute_id(att_type);
        if id >= 0 {
            Some(&self.attributes[id as usize])
        } else {
            None
        }
    }

    pub fn num_points(&self) -> usize {
        self.num_points
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
