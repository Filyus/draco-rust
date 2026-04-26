use crate::corner_table::CornerTable;

#[derive(Clone)]
pub struct MeshPredictionSchemeData<'a> {
    corner_table: Option<&'a CornerTable>,
    vertex_to_data_map: Option<&'a [i32]>,
    data_to_corner_map: Option<&'a [u32]>,
}

impl<'a> Default for MeshPredictionSchemeData<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> MeshPredictionSchemeData<'a> {
    pub fn new() -> Self {
        Self {
            corner_table: None,
            vertex_to_data_map: None,
            data_to_corner_map: None,
        }
    }

    pub fn set(
        &mut self,
        corner_table: &'a CornerTable,
        data_to_corner_map: &'a [u32],
        vertex_to_data_map: &'a [i32],
    ) {
        self.corner_table = Some(corner_table);
        self.data_to_corner_map = Some(data_to_corner_map);
        self.vertex_to_data_map = Some(vertex_to_data_map);
    }

    pub fn corner_table(&self) -> Option<&'a CornerTable> {
        self.corner_table
    }

    pub fn vertex_to_data_map(&self) -> Option<&'a [i32]> {
        self.vertex_to_data_map
    }

    pub fn data_to_corner_map(&self) -> Option<&'a [u32]> {
        self.data_to_corner_map
    }
}
