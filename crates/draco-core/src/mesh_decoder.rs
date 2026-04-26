use crate::compression_config::EncodedGeometryType;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::mesh::Mesh;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::sequential_generic_attribute_decoder::SequentialGenericAttributeDecoder;
use crate::sequential_integer_attribute_decoder::SequentialIntegerAttributeDecoder;
use crate::status::{DracoError, Status};

use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::corner_table::CornerTable;
use crate::geometry_indices::AttributeValueIndex;
use crate::geometry_indices::{
    CornerIndex, FaceIndex, PointIndex, VertexIndex, INVALID_CORNER_INDEX, INVALID_VERTEX_INDEX,
};

use crate::mesh_edgebreaker_decoder::MeshEdgebreakerDecoder;
use crate::test_event_log;
use crate::version::{version_at_least, version_less_than, VERSION_FLAGS_INTRODUCED};

fn validate_num_attributes_in_decoder(
    num_attributes_in_decoder: usize,
    remaining_bytes: usize,
) -> Result<(), DracoError> {
    // Each attribute must have at least type, data type, component count,
    // normalized flag, unique id, and a decoder type byte. Reject impossible
    // counts before reserving vectors from untrusted input.
    const MIN_ATTRIBUTE_BYTES: usize = 6;
    if num_attributes_in_decoder == 0
        || num_attributes_in_decoder > remaining_bytes / MIN_ATTRIBUTE_BYTES
    {
        return Err(DracoError::DracoError(
            "Invalid number of attributes".to_string(),
        ));
    }
    Ok(())
}

fn validate_num_components(num_components: u8) -> Result<(), DracoError> {
    if num_components == 0 {
        return Err(DracoError::DracoError(
            "Invalid attribute component count".to_string(),
        ));
    }
    Ok(())
}

fn copy_point_mapping(
    source: &PointAttribute,
    target: &mut PointAttribute,
    num_points: usize,
) -> Result<(), DracoError> {
    target.set_explicit_mapping(num_points);
    for point in 0..num_points {
        let point_id = PointIndex(point as u32);
        target.try_set_point_map_entry(point_id, source.mapped_index(point_id))?;
    }
    Ok(())
}

fn build_vertex_to_data_map_from_corner_map(
    corner_table: &CornerTable,
    data_to_corner_map: &[u32],
) -> Result<Vec<i32>, DracoError> {
    let mut vertex_to_data_map = vec![-1i32; corner_table.num_vertices()];
    for (i, &corner_id) in data_to_corner_map.iter().enumerate() {
        let corner = CornerIndex(corner_id);
        if corner == INVALID_CORNER_INDEX {
            continue;
        }
        if corner.0 as usize >= corner_table.num_corners() {
            return Err(DracoError::DracoError(
                "Data-to-corner map references an invalid corner".to_string(),
            ));
        }
        let vertex = corner_table.vertex(corner);
        if vertex == INVALID_VERTEX_INDEX {
            continue;
        }
        let Some(slot) = vertex_to_data_map.get_mut(vertex.0 as usize) else {
            return Err(DracoError::DracoError(
                "Data-to-corner map references an invalid vertex".to_string(),
            ));
        };
        *slot = i as i32;
    }
    Ok(vertex_to_data_map)
}

fn upsert_portable_attribute(
    portable_attributes_by_id: &mut Vec<(i32, PointAttribute)>,
    att_id: i32,
    portable: PointAttribute,
) {
    if let Some((_, existing)) = portable_attributes_by_id
        .iter_mut()
        .find(|(id, _)| *id == att_id)
    {
        *existing = portable;
    } else {
        portable_attributes_by_id.push((att_id, portable));
    }
}

pub struct MeshDecoder {
    geometry_type: EncodedGeometryType,
    method: u8,
    flags: u16,
    version_major: u8,
    version_minor: u8,
    corner_table: Option<Box<CornerTable>>,
    edgebreaker_data_to_corner_map: Option<Vec<u32>>,
    edgebreaker_attribute_seam_corners: Vec<Vec<u32>>,
    edgebreaker_attribute_corner_tables: Vec<CornerTable>,
    edgebreaker_attribute_vertices_on_seam: Vec<Vec<bool>>,
    edgebreaker_processed_connectivity_corners: Vec<u32>,
    edgebreaker_vertex_to_corner_map: Vec<u32>,
    edgebreaker_is_vert_hole: Vec<bool>,
    traversal_method: u8,
}

impl Default for MeshDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshDecoder {
    pub fn new() -> Self {
        Self {
            geometry_type: EncodedGeometryType::TriangularMesh,
            method: 0,
            flags: 0,
            version_major: 0,
            version_minor: 0,
            corner_table: None,
            edgebreaker_data_to_corner_map: None,
            edgebreaker_attribute_seam_corners: Vec::new(),
            edgebreaker_attribute_corner_tables: Vec::new(),
            edgebreaker_attribute_vertices_on_seam: Vec::new(),
            edgebreaker_processed_connectivity_corners: Vec::new(),
            edgebreaker_vertex_to_corner_map: Vec::new(),
            edgebreaker_is_vert_hole: Vec::new(),
            traversal_method: 0,
        }
    }

    pub fn decode(&mut self, in_buffer: &mut DecoderBuffer, out_mesh: &mut Mesh) -> Status {
        // 1. Decode Header
        self.decode_header(in_buffer)?;

        // 2. Decode Metadata
        if (self.flags & 0x8000) != 0 {
            self.decode_metadata(in_buffer)?;
        }

        if self.geometry_type == EncodedGeometryType::PointCloud {
            #[cfg(feature = "point_cloud_decode")]
            {
                // Point cloud files (geometry_type == 0) have no connectivity.
                // Delegate to PointCloudDecoder which reads num_points + attributes
                // directly into the Mesh's underlying PointCloud.
                let mut pc_decoder = crate::point_cloud_decoder::PointCloudDecoder::new();
                return pc_decoder.decode_after_header(
                    self.version_major,
                    self.version_minor,
                    self.method,
                    in_buffer,
                    &mut *out_mesh,
                );
            }
            #[cfg(not(feature = "point_cloud_decode"))]
            {
                return Err(DracoError::DracoError(
                    "Point cloud decode support is disabled".to_string(),
                ));
            }
        }

        // 3. Decode Connectivity
        self.decode_connectivity(in_buffer, out_mesh)?;

        // 4. Decode Attributes
        self.decode_attributes(in_buffer, out_mesh)
    }

    /// Test helper: Returns a reference to the decoded corner table (if any).
    /// This is useful in unit tests that wish to compare encoder/decoder
    /// corner table structures without accessing internal decoder types.
    pub fn get_corner_table_ref(&self) -> Option<&crate::corner_table::CornerTable> {
        self.corner_table.as_deref()
    }

    fn decode_metadata(&self, in_buffer: &mut DecoderBuffer) -> Result<(), DracoError> {
        if version_less_than(
            self.version_major,
            self.version_minor,
            VERSION_FLAGS_INTRODUCED,
        ) {
            return Ok(());
        }

        // Draco metadata is encoded using varints and length-prefixed names
        // (see src/draco/metadata/metadata_decoder.cc).
        let num_attribute_metadata = in_buffer.decode_varint().map_err(|_| {
            DracoError::DracoError("Failed to read attribute metadata count".to_string())
        })? as u32;
        for _ in 0..num_attribute_metadata {
            let _att_unique_id = in_buffer.decode_varint().map_err(|_| {
                DracoError::DracoError("Failed to read attribute unique ID".to_string())
            })? as u32;
            self.skip_metadata(in_buffer)?;
        }
        self.skip_metadata(in_buffer)?; // Geometry metadata
        Ok(())
    }

    // &self is needed for method signature consistency even though only used in recursive calls.
    // This maintains a uniform API where all decoder methods take &self, and the recursive
    // nature means the parameter is semantically meaningful for the call chain.
    #[allow(clippy::only_used_in_recursion)]
    fn skip_metadata(&self, in_buffer: &mut DecoderBuffer) -> Result<(), DracoError> {
        let num_entries = in_buffer.decode_varint().map_err(|_| {
            DracoError::DracoError("Failed to read metadata entries count".to_string())
        })? as u32;
        for _ in 0..num_entries {
            // Name: u8 length + bytes.
            let name_len = in_buffer.decode_u8().map_err(|_| {
                DracoError::DracoError("Failed to read metadata entry name length".to_string())
            })? as usize;
            if in_buffer.remaining_size() < name_len {
                return Err(DracoError::DracoError(
                    "Failed to read metadata entry name".to_string(),
                ));
            }
            in_buffer.try_advance(name_len)?;

            let data_size = in_buffer.decode_varint().map_err(|_| {
                DracoError::DracoError("Failed to read metadata entry data size".to_string())
            })? as usize;
            if data_size == 0 {
                return Err(DracoError::DracoError(
                    "Invalid metadata entry data size".to_string(),
                ));
            }
            if in_buffer.remaining_size() < data_size {
                return Err(DracoError::DracoError(
                    "Failed to read metadata entry value".to_string(),
                ));
            }
            in_buffer.try_advance(data_size)?;
        }

        let num_sub_metadata = in_buffer
            .decode_varint()
            .map_err(|_| DracoError::DracoError("Failed to read sub-metadata count".to_string()))?
            as u32;
        if num_sub_metadata as usize > in_buffer.remaining_size() {
            return Err(DracoError::DracoError(
                "Invalid sub-metadata count".to_string(),
            ));
        }
        for _ in 0..num_sub_metadata {
            let name_len = in_buffer.decode_u8().map_err(|_| {
                DracoError::DracoError("Failed to read sub-metadata name length".to_string())
            })? as usize;
            if in_buffer.remaining_size() < name_len {
                return Err(DracoError::DracoError(
                    "Failed to read sub-metadata name".to_string(),
                ));
            }
            in_buffer.try_advance(name_len)?;
            self.skip_metadata(in_buffer)?;
        }
        Ok(())
    }

    fn decode_header(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let mut magic = [0u8; 5];
        buffer.decode_bytes(&mut magic)?;
        if &magic != b"DRACO" {
            return Err(DracoError::DracoError("Invalid magic".to_string()));
        }

        self.version_major = buffer.decode_u8()?;
        self.version_minor = buffer.decode_u8()?;
        buffer.set_version(self.version_major, self.version_minor);

        let g_type = buffer.decode_u8()?;
        self.geometry_type = match g_type {
            0 => EncodedGeometryType::PointCloud,
            1 => EncodedGeometryType::TriangularMesh,
            _ => return Err(DracoError::DracoError("Invalid geometry type".to_string())),
        };

        self.method = buffer.decode_u8()?;

        // Flags field is always present in the binary header (C++ reads unconditionally).
        // The VERSION_FLAGS_INTRODUCED constant refers to when flag bits gained meaning,
        // not when the bytes were added to the format.
        self.flags = buffer
            .decode_u16()
            .map_err(|_| DracoError::DracoError("Failed to decode flags".to_string()))?;

        Ok(())
    }

    fn decode_connectivity(&mut self, buffer: &mut DecoderBuffer, mesh: &mut Mesh) -> Status {
        if self.method == 1 {
            let mut eb_decoder = MeshEdgebreakerDecoder::new();
            eb_decoder.decode_connectivity(buffer, mesh)?;

            // Preserve edgebreaker-derived maps for attribute decoding.
            self.edgebreaker_data_to_corner_map = eb_decoder.take_data_to_corner_map();
            self.edgebreaker_attribute_seam_corners = eb_decoder.take_attribute_seam_corners();
            self.edgebreaker_processed_connectivity_corners =
                eb_decoder.get_processed_connectivity_corners().to_vec();
            self.edgebreaker_vertex_to_corner_map = eb_decoder.get_vertex_to_corner_map().to_vec();
            self.edgebreaker_is_vert_hole = eb_decoder.take_is_vert_hole();
            self.traversal_method = eb_decoder.get_traversal_decoder_type();

            // Use the edgebreaker decoder's corner table with proper opposite mappings
            // instead of building a new one from mesh faces
            if let Some(ct) = eb_decoder.take_corner_table() {
                self.corner_table = Some(Box::new(ct));
            } else {
                return Err(DracoError::DracoError(
                    "Edgebreaker decoder did not provide corner table".to_string(),
                ));
            }
            self.rebuild_edgebreaker_attribute_corner_tables()?;
            self.assign_edgebreaker_points_to_corners(mesh)?;
        } else {
            // Sequential connectivity encoding
            // C++ MeshSequentialDecoder uses raw u32 for v < 2.2, varint for v >= 2.2
            let seq_uses_varint = version_at_least(self.version_major, self.version_minor, (2, 2));
            let (num_faces, num_points) = if !seq_uses_varint {
                #[cfg(not(feature = "legacy_bitstream_decode"))]
                {
                    return Err(DracoError::BitstreamVersionUnsupported);
                }
                #[cfg(feature = "legacy_bitstream_decode")]
                {
                    let nf = buffer.decode_u32()? as usize;
                    let np = buffer.decode_u32()? as usize;
                    (nf, np)
                }
            } else {
                let nf = buffer.decode_varint()? as usize;
                let np = buffer.decode_varint()? as usize;
                (nf, np)
            };
            let num_indices = validate_mesh_index_count(num_faces)?;
            mesh.set_num_points(num_points);

            if num_faces > 0 && num_points > 0 {
                let connectivity_method = buffer.decode_u8()?;
                if connectivity_method == 0 {
                    // Compressed
                    let mut encoded_indices = make_zeroed_indices(num_indices)?;
                    let options = crate::symbol_encoding::SymbolEncodingOptions::default();
                    if !crate::symbol_encoding::decode_symbols(
                        num_indices,
                        1,
                        &options,
                        buffer,
                        &mut encoded_indices,
                    ) {
                        return Err(DracoError::DracoError(
                            "Failed to decode compressed sequential connectivity".to_string(),
                        ));
                    }
                    let mut indices = make_zeroed_indices(num_indices)?;
                    let mut last_index_value = 0i32;
                    for (dst, encoded_val) in indices.iter_mut().zip(encoded_indices) {
                        let mut index_diff = (encoded_val >> 1) as i32;
                        if (encoded_val & 1) != 0 {
                            if index_diff > last_index_value {
                                return Err(DracoError::DracoError(
                                    "Sequential connectivity index underflow".to_string(),
                                ));
                            }
                            index_diff = -index_diff;
                        } else if index_diff > i32::MAX - last_index_value {
                            return Err(DracoError::DracoError(
                                "Sequential connectivity index overflow".to_string(),
                            ));
                        }
                        let index_value = last_index_value + index_diff;
                        *dst = index_value as u32;
                        last_index_value = index_value;
                    }
                    mesh.try_set_num_faces(num_faces)?;
                    mesh.set_faces_from_flat_indices(&indices);
                } else if connectivity_method == 1 {
                    // Raw - bulk read indices from buffer
                    if num_points < 256 {
                        let bytes_needed = num_indices;
                        let bytes = buffer.decode_slice(bytes_needed).map_err(|_| {
                            DracoError::DracoError("Not enough data for u8 indices".to_string())
                        })?;
                        mesh.try_set_num_faces(num_faces)?;
                        mesh.set_faces_from_u8_indices(bytes);
                    } else if num_points < 65536 {
                        let bytes_needed = num_indices.checked_mul(2).ok_or_else(|| {
                            DracoError::DracoError("Mesh u16 index byte count overflow".to_string())
                        })?;
                        let bytes = buffer.decode_slice(bytes_needed).map_err(|_| {
                            DracoError::DracoError("Not enough data for u16 indices".to_string())
                        })?;
                        mesh.try_set_num_faces(num_faces)?;
                        mesh.set_faces_from_le_u16_indices(bytes);
                    } else if num_points < (1 << 21) && seq_uses_varint {
                        mesh.try_set_num_faces(num_faces)?;
                        for face_id in 0..num_faces {
                            mesh.set_face_from_indices(
                                face_id,
                                [
                                    buffer.decode_varint()? as u32,
                                    buffer.decode_varint()? as u32,
                                    buffer.decode_varint()? as u32,
                                ],
                            );
                        }
                    } else {
                        let bytes_needed = num_indices.checked_mul(4).ok_or_else(|| {
                            DracoError::DracoError("Mesh u32 index byte count overflow".to_string())
                        })?;
                        let bytes = buffer.decode_slice(bytes_needed).map_err(|_| {
                            DracoError::DracoError("Not enough data for u32 indices".to_string())
                        })?;
                        mesh.try_set_num_faces(num_faces)?;
                        mesh.set_faces_from_le_u32_indices(bytes);
                    }
                } else {
                    return Err(DracoError::DracoError(format!(
                        "Unsupported sequential connectivity method: {}",
                        connectivity_method
                    )));
                }
                // If sequential mode uses compressed connectivity, we may need
                // to remap indices for deduplication. For raw mode above,
                // face indices match the flat array.

                // Note: Sequential encoding does NOT use a CornerTable.
                // C++ MeshSequentialDecoder::DecodeConnectivity() just calls mesh->AddFace()
                // and uses LinearSequencer for attribute decoding (identity mapping).
                // Corner tables are only needed for Edgebreaker's mesh prediction schemes.
                // self.corner_table remains None for sequential decoding.
            }
        }

        Ok(())
    }

    fn make_attribute_corner_table(
        base_ct: &CornerTable,
        seam_corners: &[u32],
    ) -> Result<(CornerTable, Vec<bool>), DracoError> {
        let mut ct = base_ct.clone();
        let mut is_edge_on_seam = vec![false; base_ct.num_corners()];
        let mut is_vertex_on_seam = vec![false; base_ct.num_vertices()];

        for &c_u32 in seam_corners {
            let c = CornerIndex(c_u32);
            if c == INVALID_CORNER_INDEX {
                continue;
            }
            if c.0 as usize >= base_ct.num_corners() {
                return Err(DracoError::DracoError(
                    "Invalid Edgebreaker attribute seam corner".to_string(),
                ));
            }
            is_edge_on_seam[c.0 as usize] = true;
            ct.set_opposite(c, INVALID_CORNER_INDEX);

            let next_vertex = base_ct.vertex(base_ct.next(c));
            if next_vertex != crate::geometry_indices::INVALID_VERTEX_INDEX {
                is_vertex_on_seam[next_vertex.0 as usize] = true;
            }
            let previous_vertex = base_ct.vertex(base_ct.previous(c));
            if previous_vertex != crate::geometry_indices::INVALID_VERTEX_INDEX {
                is_vertex_on_seam[previous_vertex.0 as usize] = true;
            }

            let opp = base_ct.opposite(c);
            if opp != INVALID_CORNER_INDEX {
                if opp.0 as usize >= base_ct.num_corners() {
                    return Err(DracoError::DracoError(
                        "Invalid Edgebreaker attribute seam opposite corner".to_string(),
                    ));
                }
                is_edge_on_seam[opp.0 as usize] = true;
                ct.set_opposite(opp, INVALID_CORNER_INDEX);

                let next_vertex = base_ct.vertex(base_ct.next(opp));
                if next_vertex != crate::geometry_indices::INVALID_VERTEX_INDEX {
                    is_vertex_on_seam[next_vertex.0 as usize] = true;
                }
                let previous_vertex = base_ct.vertex(base_ct.previous(opp));
                if previous_vertex != crate::geometry_indices::INVALID_VERTEX_INDEX {
                    is_vertex_on_seam[previous_vertex.0 as usize] = true;
                }
            }
        }

        let seam_opposite = |corner: CornerIndex| -> CornerIndex {
            if corner == INVALID_CORNER_INDEX {
                return INVALID_CORNER_INDEX;
            }
            if is_edge_on_seam[corner.0 as usize] {
                INVALID_CORNER_INDEX
            } else {
                base_ct.opposite(corner)
            }
        };
        let seam_swing_left = |corner: CornerIndex| -> CornerIndex {
            base_ct.next(seam_opposite(base_ct.next(corner)))
        };

        ct.corner_to_vertex_map
            .fill(crate::geometry_indices::INVALID_VERTEX_INDEX);
        ct.vertex_corners.clear();

        let mut num_new_vertices = 0usize;
        for v in 0..base_ct.num_vertices() {
            let c = base_ct.left_most_corner(VertexIndex(v as u32));
            if c == INVALID_CORNER_INDEX {
                continue;
            }

            let mut first_vertex_id = VertexIndex(num_new_vertices as u32);
            num_new_vertices += 1;

            let mut first_c = c;
            if is_vertex_on_seam[v] {
                let mut act_c = seam_swing_left(first_c);
                while act_c != INVALID_CORNER_INDEX {
                    first_c = act_c;
                    act_c = seam_swing_left(act_c);
                }
            }

            ct.corner_to_vertex_map[first_c.0 as usize] = first_vertex_id;
            ct.vertex_corners.push(first_c);

            let mut act_c = base_ct.swing_right(first_c);
            while act_c != INVALID_CORNER_INDEX && act_c != first_c {
                if is_edge_on_seam[base_ct.next(act_c).0 as usize] {
                    first_vertex_id = VertexIndex(num_new_vertices as u32);
                    num_new_vertices += 1;
                    ct.vertex_corners.push(act_c);
                }
                ct.corner_to_vertex_map[act_c.0 as usize] = first_vertex_id;
                act_c = base_ct.swing_right(act_c);
            }
        }

        ct.num_original_vertices = ct.vertex_corners.len();
        ct.num_isolated_vertices = 0;
        ct.num_degenerated_faces = base_ct.num_degenerated_faces;

        Ok((ct, is_vertex_on_seam))
    }

    fn rebuild_edgebreaker_attribute_corner_tables(&mut self) -> Status {
        self.edgebreaker_attribute_corner_tables.clear();
        self.edgebreaker_attribute_vertices_on_seam.clear();
        let Some(base_ct) = self.corner_table.as_deref() else {
            return Ok(());
        };
        for seam_corners in &self.edgebreaker_attribute_seam_corners {
            let (corner_table, vertices_on_seam) =
                Self::make_attribute_corner_table(base_ct, seam_corners)?;
            self.edgebreaker_attribute_corner_tables.push(corner_table);
            self.edgebreaker_attribute_vertices_on_seam
                .push(vertices_on_seam);
        }
        Ok(())
    }

    fn assign_edgebreaker_points_to_corners(&self, mesh: &mut Mesh) -> Status {
        if self.edgebreaker_attribute_corner_tables.is_empty() {
            return Ok(());
        }
        let Some(base_ct) = self.corner_table.as_deref() else {
            return Ok(());
        };

        let num_corners = base_ct.num_corners();
        let mut point_to_corner_map: Vec<u32> = Vec::new();
        let mut corner_to_point_map = vec![u32::MAX; num_corners];

        for v in 0..base_ct.num_vertices() {
            let mut c = base_ct.left_most_corner(VertexIndex(v as u32));
            if c == INVALID_CORNER_INDEX {
                continue;
            }

            let mut first_corner = c;
            let is_vert_hole = self
                .edgebreaker_is_vert_hole
                .get(v)
                .copied()
                .unwrap_or_else(|| {
                    Self::is_vertex_on_boundary_impl(base_ct, VertexIndex(v as u32))
                });
            if !is_vert_hole {
                for (attr_index, attr_ct) in
                    self.edgebreaker_attribute_corner_tables.iter().enumerate()
                {
                    let base_vertex = base_ct.vertex(c);
                    let Some(vertices_on_seam) =
                        self.edgebreaker_attribute_vertices_on_seam.get(attr_index)
                    else {
                        continue;
                    };
                    if base_vertex == crate::geometry_indices::INVALID_VERTEX_INDEX
                        || !vertices_on_seam
                            .get(base_vertex.0 as usize)
                            .copied()
                            .unwrap_or(false)
                    {
                        continue;
                    }
                    let vertex_at_first = attr_ct.vertex(c);
                    let mut act_c = base_ct.swing_right(c);
                    let mut seam_found = false;
                    while act_c != INVALID_CORNER_INDEX && act_c != c {
                        if attr_ct.vertex(act_c) != vertex_at_first {
                            first_corner = act_c;
                            seam_found = true;
                            break;
                        }
                        act_c = base_ct.swing_right(act_c);
                    }
                    if seam_found {
                        break;
                    }
                }
            }

            c = first_corner;
            corner_to_point_map[c.0 as usize] = point_to_corner_map.len() as u32;
            point_to_corner_map.push(c.0);

            let mut prev_c = c;
            c = base_ct.swing_right(c);
            while c != INVALID_CORNER_INDEX && c != first_corner {
                let attribute_seam = self
                    .edgebreaker_attribute_corner_tables
                    .iter()
                    .any(|attr_ct| attr_ct.vertex(c) != attr_ct.vertex(prev_c));
                if attribute_seam {
                    corner_to_point_map[c.0 as usize] = point_to_corner_map.len() as u32;
                    point_to_corner_map.push(c.0);
                } else {
                    corner_to_point_map[c.0 as usize] = corner_to_point_map[prev_c.0 as usize];
                }
                prev_c = c;
                c = base_ct.swing_right(c);
            }
        }

        for face_id in 0..mesh.num_faces() {
            let base = face_id * 3;
            let p0 = corner_to_point_map[base];
            let p1 = corner_to_point_map[base + 1];
            let p2 = corner_to_point_map[base + 2];
            if p0 == u32::MAX || p1 == u32::MAX || p2 == u32::MAX {
                return Err(DracoError::DracoError(
                    "Failed to assign Edgebreaker corner point".to_string(),
                ));
            }
            mesh.set_face(
                FaceIndex(face_id as u32),
                [PointIndex(p0), PointIndex(p1), PointIndex(p2)],
            );
        }
        mesh.set_num_points(point_to_corner_map.len());

        Ok(())
    }

    fn decode_attributes(&mut self, buffer: &mut DecoderBuffer, mesh: &mut Mesh) -> Status {
        // Both MeshSequentialEncoding and MeshEdgebreakerEncoding use a u8 for the number of
        // attribute decoders.
        let num_attributes_decoders = buffer.decode_u8()? as usize;
        let num_points = mesh.num_points();

        // For Edgebreaker, traversal sequencing is controlled per attribute decoder.
        // We'll derive the correct (point_ids, data_to_corner_map) later for each decoder payload
        // based on its traversal_method.
        let point_ids = if self.method == 0 {
            make_point_ids(num_points)?
        } else {
            Vec::new()
        };
        let data_to_corner_map: Option<Vec<u32>> = None;

        let pc_decoder = PointCloudDecoder::new();
        let bitstream_version: u16 =
            ((self.version_major as u16) << 8) | (self.version_minor as u16);

        struct PendingQuant {
            att_id: i32,
            portable: PointAttribute,
            transform: AttributeQuantizationTransform,
        }

        struct PendingNormal {
            att_id: i32,
            portable: PointAttribute,
            quantization_bits: u8,
        }

        // (1) Attribute decoder identifiers.
        // For Edgebreaker this ties each decoder payload to attribute connectivity data.
        let mut att_data_id_by_decoder: Vec<u8> = vec![0; num_attributes_decoders];
        let mut encoder_type_by_decoder: Vec<u8> = vec![0; num_attributes_decoders];
        let mut traversal_method_by_decoder: Vec<u8> = vec![0; num_attributes_decoders];
        if self.method == 1 {
            for i in 0..num_attributes_decoders {
                att_data_id_by_decoder[i] = buffer.decode_u8()?;
                encoder_type_by_decoder[i] = buffer.decode_u8()?;
                // traversal_method was added in v1.2. For older streams, default to
                // DEPTH_FIRST (0).
                if bitstream_version >= 0x0102 {
                    traversal_method_by_decoder[i] = buffer.decode_u8()?;
                } else if !cfg!(feature = "legacy_bitstream_decode") {
                    return Err(DracoError::BitstreamVersionUnsupported);
                }
            }
        }

        // (2) Attribute decoder data.
        let mut att_ids_by_decoder: Vec<Vec<i32>> = Vec::with_capacity(num_attributes_decoders);
        let mut decoder_types_by_decoder: Vec<Vec<u8>> =
            Vec::with_capacity(num_attributes_decoders);

        for _ in 0..num_attributes_decoders {
            let num_attributes_in_decoder: usize = if bitstream_version < 0x0200 {
                if !cfg!(feature = "legacy_bitstream_decode") {
                    return Err(DracoError::BitstreamVersionUnsupported);
                }
                buffer.decode_u32()? as usize
            } else {
                buffer.decode_varint()? as usize
            };
            if num_attributes_in_decoder == 0 {
                return Err(DracoError::DracoError(
                    "Invalid number of attributes".to_string(),
                ));
            }
            validate_num_attributes_in_decoder(num_attributes_in_decoder, buffer.remaining_size())?;

            let mut att_ids: Vec<i32> = Vec::with_capacity(num_attributes_in_decoder);
            let mut decoder_types: Vec<u8> = Vec::with_capacity(num_attributes_in_decoder);

            for _ in 0..num_attributes_in_decoder {
                let att_type_val = buffer.decode_u8()?;
                let att_type = GeometryAttributeType::try_from(att_type_val)?;

                let data_type_val = buffer.decode_u8()?;
                let data_type = DataType::try_from(data_type_val)?;

                let num_components = buffer.decode_u8()?;
                validate_num_components(num_components)?;
                let normalized = buffer.decode_u8()? != 0;
                let unique_id: u32 = if bitstream_version < 0x0103 {
                    if !cfg!(feature = "legacy_bitstream_decode") {
                        return Err(DracoError::BitstreamVersionUnsupported);
                    }
                    buffer.decode_u16()? as u32
                } else {
                    buffer.decode_varint()? as u32
                };

                let mut att = PointAttribute::new();
                att.try_init(att_type, num_components, data_type, normalized, num_points)?;
                att.set_unique_id(unique_id);
                let att_id = mesh.add_attribute(att);
                att_ids.push(att_id);

                if self.method == 1 {
                    let att_mut = mesh.try_attribute_mut(att_id)?;
                    att_mut.set_explicit_mapping(num_points);
                    for i in 0..num_points {
                        att_mut.try_set_point_map_entry(
                            PointIndex(i as u32),
                            AttributeValueIndex(i as u32),
                        )?;
                    }
                }
            }

            for _ in 0..num_attributes_in_decoder {
                decoder_types.push(buffer.decode_u8()?);
            }

            att_ids_by_decoder.push(att_ids);
            decoder_types_by_decoder.push(decoder_types);
        }

        // (3) Attribute decoder payloads.
        let mut portable_attributes_by_id: Vec<(i32, PointAttribute)> = Vec::new();
        for dec_i in 0..num_attributes_decoders {
            let att_ids = &att_ids_by_decoder[dec_i];
            let decoder_types = &decoder_types_by_decoder[dec_i];

            // For edgebreaker, build an attribute-specific corner table (seams) if needed.
            // Corner indices remain stable because we only break opposite links.
            let mut attr_corner_table: Option<CornerTable> = None;
            if self.method == 1 {
                let att_data_id = att_data_id_by_decoder[dec_i] as usize;
                let uses_attribute_connectivity =
                    encoder_type_by_decoder.get(dec_i).copied().unwrap_or(0) != 0;
                if uses_attribute_connectivity
                    && att_data_id < self.edgebreaker_attribute_seam_corners.len()
                {
                    if let Some(ct) = self.edgebreaker_attribute_corner_tables.get(att_data_id) {
                        attr_corner_table = Some(ct.clone());
                    }
                }
            }

            // Determine the corner table used for prediction within this decoder.
            // For edgebreaker, seams may split vertex fans and change the effective
            // traversal sequence used by predictors.
            let mut point_ids_for_decoder: Option<Vec<PointIndex>> = None;
            let mut data_to_corner_map_for_decoder: Option<Vec<u32>> = None;
            let mut vertex_to_data_map_for_decoder: Option<Vec<i32>> = None;
            if self.method == 1 {
                // If we have an attribute-specific seam corner table, recompute vertex
                // corners after breaking opposites so we can derive the correct number
                // of entries for this decoder.
                if let Some(ref ct) = attr_corner_table {
                    let (ids, map, v_map) =
                        Self::generate_point_ids_and_corners_dfs_for_table(mesh, ct, &[])?;
                    point_ids_for_decoder = Some(ids);
                    data_to_corner_map_for_decoder = Some(map);
                    vertex_to_data_map_for_decoder = Some(v_map);
                }

                // Note: For edgebreaker, we intentionally do NOT take a traversal
                // mapping from `MeshEdgebreakerDecoder::assign_points_to_corners()`.
                // The C++ decoder derives its attribute traversal from
                // `MeshTraversalSequencer` (with no corner_order set), i.e. from
                // deterministic traversal over the reconstructed corner table.
                // Mixing a connectivity-derived map with a separately generated
                // vertex_to_data_map can desynchronize prediction decoding.
            }

            let corner_table_for_decoder: Option<&CornerTable> =
                if let Some(ref ct) = attr_corner_table {
                    Some(ct)
                } else {
                    self.corner_table.as_deref()
                };

            // Optional vertex_to_data_map derived from the chosen data_to_corner_map.
            // (Needed by mesh prediction schemes to map corner-table vertices -> data ids.)
            // For edgebreaker, derive per-decoder traversal sequencing when seams are not
            // applied (per-vertex attributes). This sequencing must match the bitstream
            // traversal_method to keep prediction-scheme side streams (e.g. crease flags)
            // synchronized.
            let mut sequenced_point_ids: Option<Vec<PointIndex>> = None;
            let mut sequenced_data_to_corner_map: Option<Vec<u32>> = None;
            let mut sequenced_vertex_to_data_map: Option<Vec<i32>> = None;

            // Generate point_ids using traversal method.
            // For Edgebreaker, the decoder should match the encoder's traversal method.
            // The per-decoder traversal method is stored in traversal_method_by_decoder.
            // - traversal_method == 1 (PREDICTION_DEGREE): uses MaxPredictionDegree traversal
            // - traversal_method == 0 (DEPTH_FIRST): uses DFS traversal
            // Note: self.traversal_method is the edgebreaker decoder type (0=Standard, 1=Predictive, 2=Valence),
            // which is different from the per-decoder traversal method.
            if sequenced_point_ids.is_none() {
                // Get the per-decoder traversal method (Speed 0 uses PREDICTION_DEGREE=1, others use DEPTH_FIRST=0)
                let per_decoder_traversal =
                    if self.method == 1 && dec_i < traversal_method_by_decoder.len() {
                        traversal_method_by_decoder[dec_i]
                    } else {
                        0
                    };
                // For sequential encoding (method 0), use identity permutation
                // because the encoder writes positions in point ID order [0, 1, 2, ...].
                // For edgebreaker (method 1), use DFS/prediction traversal to match encoder.
                if self.method == 0 {
                    // Sequential encoding: C++ uses LinearSequencer which generates
                    // identity mapping [0, 1, 2, ..., num_points-1] and calls
                    // SetIdentityMapping() for attributes. No corner table or
                    // data_to_corner_map is needed.
                    // Use the mesh-wide identity sequence allocated above instead
                    // of rebuilding an identical vector for each decoder.
                    // sequenced_data_to_corner_map remains None - not needed for sequential
                } else {
                    // Edgebreaker decoding: traversal method depends on the per-decoder
                    // traversal method written by the encoder.
                    // - per_decoder_traversal == 1 (PREDICTION_DEGREE): MaxPredictionDegree traversal (speed 0)
                    // - per_decoder_traversal == 0 (DEPTH_FIRST): DFS traversal (speed >= 1)

                    if per_decoder_traversal == 1 {
                        // Speed 0: use MaxPredictionDegree traversal
                        let (ids, map, v_map) = self
                            .generate_point_ids_and_corners_max_prediction_degree(
                                mesh,
                                &self.edgebreaker_processed_connectivity_corners,
                            )?;
                        sequenced_point_ids = Some(ids);
                        sequenced_data_to_corner_map = Some(map);
                        sequenced_vertex_to_data_map = Some(v_map); // Use directly from traversal
                    } else {
                        // Speed >= 1: use DFS with sequential faces. The traversal helper
                        // already uses CornerIndex(3 * face_id) when no explicit seeds are
                        // provided, so avoid allocating a temporary seed vector here.
                        let (ids, map, v_map) =
                            self.generate_point_ids_and_corners_dfs(mesh, &[])?;
                        sequenced_point_ids = Some(ids);
                        sequenced_data_to_corner_map = Some(map);
                        sequenced_vertex_to_data_map = Some(v_map); // Use directly from DFS traversal
                    }
                }
            }

            // Generate vertex_to_data_map from the traversal result (only if not already set).
            // This is needed by predictors (like Parallelogram) to find references by point index.
            // Only needed for Edgebreaker (method 1) since sequential encoding uses only
            // Difference prediction which doesn't need mesh connectivity.
            if self.method == 1 && sequenced_vertex_to_data_map.is_none() {
                if let Some(ref map) = sequenced_data_to_corner_map {
                    let ct = self.corner_table.as_ref().ok_or_else(|| {
                        DracoError::DracoError(
                            "Edgebreaker attribute traversal missing corner table".to_string(),
                        )
                    })?;
                    sequenced_vertex_to_data_map =
                        Some(build_vertex_to_data_map_from_corner_map(ct, map)?);
                }
            }

            // Choose which point sequence to use for decoding values in this decoder.
            // If seams were applied, we derived a per-decoder point id list (possibly
            // containing repeats). Otherwise, fall back to the mesh-wide sequence.
            let point_ids_for_values: &[PointIndex] = if let Some(ref ids) = point_ids_for_decoder {
                ids
            } else if let Some(ref ids) = sequenced_point_ids {
                ids
            } else {
                &point_ids
            };
            let data_to_corner_map_override_for_values: Option<&[u32]> =
                if let Some(ref map) = data_to_corner_map_for_decoder {
                    Some(map.as_slice())
                } else if let Some(ref map) = sequenced_data_to_corner_map {
                    Some(map.as_slice())
                } else {
                    data_to_corner_map.as_deref()
                };
            let vertex_to_data_map_override_for_values: Option<&[i32]> =
                if point_ids_for_decoder.is_some() {
                    vertex_to_data_map_for_decoder.as_deref()
                } else {
                    sequenced_vertex_to_data_map.as_deref()
                };

            let mut pending_quant: Vec<PendingQuant> = Vec::new();
            let mut pending_normals: Vec<PendingNormal> = Vec::new();

            for (local_i, &att_id) in att_ids.iter().enumerate() {
                let decoder_type = decoder_types[local_i];
                {
                    let att = mesh.try_attribute_mut(att_id)?;
                    if att.size() != point_ids_for_values.len() {
                        att.resize_unique_entries(point_ids_for_values.len())?;
                    }
                }
                match decoder_type {
                    0 => {
                        let mut att_decoder = SequentialGenericAttributeDecoder::new();
                        att_decoder.init(&pc_decoder, att_id);
                        att_decoder.decode_values(mesh, point_ids_for_values, buffer)?;
                    }
                    1 => {
                        let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                        att_decoder.init(&pc_decoder, att_id);
                        let portable_parent_attribute = if bitstream_version >= 0x0200 {
                            let pos_att_id =
                                mesh.named_attribute_id(GeometryAttributeType::Position);
                            portable_attributes_by_id
                                .iter()
                                .find(|(id, _)| *id == pos_att_id)
                                .map(|(_, att)| att)
                        } else {
                            None
                        };
                        if !att_decoder.decode_values(
                            mesh,
                            point_ids_for_values,
                            buffer,
                            corner_table_for_decoder,
                            data_to_corner_map_override_for_values,
                            vertex_to_data_map_override_for_values,
                            None,
                            portable_parent_attribute,
                            None,
                        ) {
                            return Err(DracoError::DracoError(
                                "Failed to decode integer attribute values".to_string(),
                            ));
                        }
                    }
                    2 => {
                        let mut portable = PointAttribute::default();
                        let (original_type, original_num_components) = {
                            let original = mesh.try_attribute(att_id)?;
                            (original.attribute_type(), original.num_components())
                        };
                        portable.try_init(
                            original_type,
                            original_num_components,
                            DataType::Uint32,
                            false,
                            point_ids_for_values.len(),
                        )?;
                        #[allow(unused_mut)]
                        let mut transform = AttributeQuantizationTransform::new();
                        // Legacy compatibility shim: C++ bitstreams with version < 2.0 store
                        // quantization params before the integer values, while v2.0+ stores
                        // them after the values. Rust-generated files never use the legacy
                        // layout, so this peek-ahead only exists to decode genuine old C++ files.
                        let quant_skip_bytes = if bitstream_version < 0x0200 {
                            #[cfg(not(feature = "legacy_bitstream_decode"))]
                            {
                                return Err(DracoError::BitstreamVersionUnsupported);
                            }
                            #[cfg(feature = "legacy_bitstream_decode")]
                            {
                                let saved_pos = buffer.position();
                                let method_byte = buffer.decode_u8().map_err(|_| {
                                    DracoError::DracoError(
                                        "Failed to read prediction method".to_string(),
                                    )
                                })?;
                                if method_byte != 0xFF {
                                    let _transform_byte = buffer.decode_u8().map_err(|_| {
                                        DracoError::DracoError(
                                            "Failed to read transform type".to_string(),
                                        )
                                    })?;
                                }
                                let original = mesh.try_attribute(att_id)?;
                                if !transform.decode_parameters(original, buffer) {
                                    return Err(DracoError::DracoError(
                                        "Failed to decode quantization parameters (v<2.0)"
                                            .to_string(),
                                    ));
                                }
                                let bytes_consumed = buffer.position() - saved_pos;
                                let pred_header_bytes = if method_byte != 0xFF { 2 } else { 1 };
                                let skip = bytes_consumed - pred_header_bytes;
                                buffer.set_position(saved_pos).map_err(|_| {
                                    DracoError::DracoError(
                                        "Failed to reset buffer position".to_string(),
                                    )
                                })?;
                                skip
                            }
                        } else {
                            0
                        };
                        let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                        att_decoder.init(&pc_decoder, att_id);
                        let mut skip_hook_fn = move |buf: &mut DecoderBuffer<'_>| -> bool {
                            if quant_skip_bytes > 0 {
                                if buf.try_advance(quant_skip_bytes).is_err() {
                                    return false;
                                }
                            }
                            true
                        };
                        let pre_hook_opt: Option<&mut dyn FnMut(&mut DecoderBuffer<'_>) -> bool> =
                            if quant_skip_bytes > 0 {
                                Some(&mut skip_hook_fn)
                            } else {
                                None
                            };
                        let portable_parent_attribute = if bitstream_version >= 0x0200 {
                            let pos_att_id =
                                mesh.named_attribute_id(GeometryAttributeType::Position);
                            portable_attributes_by_id
                                .iter()
                                .find(|(id, _)| *id == pos_att_id)
                                .map(|(_, att)| att)
                        } else {
                            None
                        };
                        if !att_decoder.decode_values(
                            mesh,
                            point_ids_for_values,
                            buffer,
                            corner_table_for_decoder,
                            data_to_corner_map_override_for_values,
                            vertex_to_data_map_override_for_values,
                            Some(&mut portable),
                            portable_parent_attribute,
                            pre_hook_opt,
                        ) {
                            return Err(DracoError::DracoError(
                                "Failed to decode quantized portable values".to_string(),
                            ));
                        }
                        pending_quant.push(PendingQuant {
                            att_id,
                            portable,
                            transform,
                        });
                    }
                    3 => {
                        let mut portable = PointAttribute::default();
                        portable.try_init(
                            GeometryAttributeType::Generic,
                            2,
                            DataType::Uint32,
                            false,
                            point_ids_for_values.len(),
                        )?;
                        // Legacy compatibility shim: C++ bitstreams with version < 2.0 store
                        // normal octahedron quantization bits after the prediction header but
                        // before integer values. Rust-generated files never use this layout.
                        #[allow(unused_mut)]
                        let mut quant_bits: u8 = 0;
                        let normal_skip_bytes = if bitstream_version < 0x0200 {
                            #[cfg(not(feature = "legacy_bitstream_decode"))]
                            {
                                return Err(DracoError::BitstreamVersionUnsupported);
                            }
                            #[cfg(feature = "legacy_bitstream_decode")]
                            {
                                let saved_pos = buffer.position();
                                // Skip prediction_method + transform_type
                                let method_byte = buffer.decode_u8().map_err(|_| {
                                    DracoError::DracoError(
                                        "Failed to read prediction method".to_string(),
                                    )
                                })?;
                                if method_byte != 0xFF {
                                    let _transform_byte = buffer.decode_u8().map_err(|_| {
                                        DracoError::DracoError(
                                            "Failed to read transform type".to_string(),
                                        )
                                    })?;
                                }
                                // Read quant_bits at the correct position
                                quant_bits = buffer.decode_u8().map_err(|_| {
                                    DracoError::DracoError(
                                        "Failed to read normal quant_bits".to_string(),
                                    )
                                })?;
                                if !AttributeOctahedronTransform::is_valid_quantization_bits(
                                    quant_bits as i32,
                                ) {
                                    return Err(DracoError::DracoError(
                                        "Invalid normal quantization bits".to_string(),
                                    ));
                                }
                                let bytes_consumed = buffer.position() - saved_pos;
                                let pred_header_bytes = if method_byte != 0xFF { 2 } else { 1 };
                                let skip = bytes_consumed - pred_header_bytes;
                                buffer.set_position(saved_pos).map_err(|_| {
                                    DracoError::DracoError(
                                        "Failed to reset buffer position".to_string(),
                                    )
                                })?;
                                skip
                            }
                        } else {
                            0
                        };
                        let mut att_decoder = SequentialIntegerAttributeDecoder::new();
                        att_decoder.init(&pc_decoder, att_id);
                        let mut normal_skip_fn = move |buf: &mut DecoderBuffer<'_>| -> bool {
                            if normal_skip_bytes > 0 {
                                if buf.try_advance(normal_skip_bytes).is_err() {
                                    return false;
                                }
                            }
                            true
                        };
                        let normal_hook: Option<&mut dyn FnMut(&mut DecoderBuffer<'_>) -> bool> =
                            if normal_skip_bytes > 0 {
                                Some(&mut normal_skip_fn)
                            } else {
                                None
                            };
                        let portable_parent_attribute = if bitstream_version >= 0x0200 {
                            let pos_att_id =
                                mesh.named_attribute_id(GeometryAttributeType::Position);
                            portable_attributes_by_id
                                .iter()
                                .find(|(id, _)| *id == pos_att_id)
                                .map(|(_, att)| att)
                        } else {
                            None
                        };
                        if !att_decoder.decode_values(
                            mesh,
                            point_ids_for_values,
                            buffer,
                            corner_table_for_decoder,
                            data_to_corner_map_override_for_values,
                            vertex_to_data_map_override_for_values,
                            Some(&mut portable),
                            portable_parent_attribute,
                            normal_hook,
                        ) {
                            return Err(DracoError::DracoError(
                                "Failed to decode normal portable values".to_string(),
                            ));
                        }
                        pending_normals.push(PendingNormal {
                            att_id,
                            portable,
                            quantization_bits: quant_bits,
                        });
                    }
                    _ => {
                        return Err(DracoError::DracoError(format!(
                            "Unsupported sequential decoder type: {}",
                            decoder_type
                        )));
                    }
                }
            }

            // Decode transform data for all attributes.
            // For C++ files with bitstream version < 2.0, quantization params were already
            // decoded before integer values (legacy peek-ahead above). For v >= 2.0
            // (including all Rust-generated files), they are decoded here after all values.
            for (local_i, &att_id) in att_ids.iter().enumerate() {
                match decoder_types[local_i] {
                    2 => {
                        if bitstream_version >= 0x0200 {
                            let idx = pending_quant
                                .iter()
                                .position(|p| p.att_id == att_id)
                                .ok_or_else(|| {
                                    DracoError::DracoError(
                                        "Missing pending quant entry".to_string(),
                                    )
                                })?;
                            let original = mesh.try_attribute(att_id)?;
                            if !pending_quant[idx]
                                .transform
                                .decode_parameters(original, buffer)
                            {
                                return Err(DracoError::DracoError(
                                    "Failed to decode quantization parameters".to_string(),
                                ));
                            }
                        }
                    }
                    3 => {
                        if bitstream_version >= 0x0200 {
                            let idx = pending_normals
                                .iter()
                                .position(|p| p.att_id == att_id)
                                .ok_or_else(|| {
                                    DracoError::DracoError(
                                        "Missing pending normal entry".to_string(),
                                    )
                                })?;
                            let bits = buffer.decode_u8()?;
                            if !AttributeOctahedronTransform::is_valid_quantization_bits(
                                bits as i32,
                            ) {
                                return Err(DracoError::DracoError(
                                    "Invalid normal quantization bits".to_string(),
                                ));
                            }
                            pending_normals[idx].quantization_bits = bits;
                        }
                    }
                    _ => {}
                }
            }

            // Apply inverse transforms.
            for q in &pending_quant {
                let dst = mesh.try_attribute_mut(q.att_id)?;
                if dst.size() != q.portable.size() {
                    dst.resize_unique_entries(q.portable.size())?;
                }
                if !q.transform.inverse_transform_attribute(&q.portable, dst) {
                    return Err(DracoError::DracoError(
                        "Failed to dequantize attribute".to_string(),
                    ));
                }
            }
            for n in &pending_normals {
                let mut oct = AttributeOctahedronTransform::new(-1);
                if !oct.set_parameters(n.quantization_bits as i32) {
                    return Err(DracoError::DracoError(
                        "Invalid normal quantization bits".to_string(),
                    ));
                }
                let dst = mesh.try_attribute_mut(n.att_id)?;
                if dst.size() != n.portable.size() {
                    dst.resize_unique_entries(n.portable.size())?;
                }
                if !oct.inverse_transform_attribute(&n.portable, dst) {
                    return Err(DracoError::DracoError(
                        "Failed to decode normals".to_string(),
                    ));
                }
            }

            // Apply UpdatePointToAttributeIndexMapping for Edgebreaker (method 1)
            // This creates the final mapping from mesh points to attribute values,
            // matching C++ MeshTraversalSequencer::UpdatePointToAttributeIndexMapping.
            //
            // The key insight: values are stored in data_id order (determined by DFS).
            // vertex_to_data_map[v] tells us which data_id holds vertex v's value.
            // In the decoder, mesh point == corner table vertex (since faces are built from CT).
            // So point p should get value from data_id = vertex_to_data_map[p].
            if self.method == 1 {
                let mapping_v_map = vertex_to_data_map_for_decoder
                    .as_deref()
                    .or(sequenced_vertex_to_data_map.as_deref());
                if let Some(v_map) = mapping_v_map {
                    let num_points = mesh.num_points();
                    let mut point_to_value: Vec<Option<AttributeValueIndex>> =
                        vec![None; num_points];
                    if let Some(ct) = corner_table_for_decoder {
                        for face_id in 0..mesh.num_faces() {
                            let face = mesh.face(FaceIndex(face_id as u32));
                            for corner_offset in 0..3 {
                                let corner = CornerIndex((face_id * 3 + corner_offset) as u32);
                                let vertex = ct.vertex(corner);
                                let point = face[corner_offset].0 as usize;
                                if point < point_to_value.len()
                                    && vertex != INVALID_VERTEX_INDEX
                                    && (vertex.0 as usize) < v_map.len()
                                    && v_map[vertex.0 as usize] >= 0
                                {
                                    point_to_value[point] =
                                        Some(AttributeValueIndex(v_map[vertex.0 as usize] as u32));
                                }
                            }
                        }
                    } else {
                        for p in 0..num_points {
                            if p < v_map.len() && v_map[p] >= 0 {
                                point_to_value[p] = Some(AttributeValueIndex(v_map[p] as u32));
                            }
                        }
                    }

                    for &att_id in att_ids {
                        let att = mesh.try_attribute_mut(att_id)?;
                        att.set_explicit_mapping(num_points);
                        for (point, value) in point_to_value.iter().enumerate() {
                            if let Some(value) = value {
                                att.try_set_point_map_entry(PointIndex(point as u32), *value)?;
                            }
                        }
                    }
                }
            }

            for q in pending_quant {
                let mut portable = q.portable;
                copy_point_mapping(
                    mesh.try_attribute(q.att_id)?,
                    &mut portable,
                    mesh.num_points(),
                )?;
                upsert_portable_attribute(&mut portable_attributes_by_id, q.att_id, portable);
            }
            for n in pending_normals {
                let mut portable = n.portable;
                copy_point_mapping(
                    mesh.try_attribute(n.att_id)?,
                    &mut portable,
                    mesh.num_points(),
                )?;
                upsert_portable_attribute(&mut portable_attributes_by_id, n.att_id, portable);
            }
        }

        Ok(())
    }

    /// Discovery-order traversal: use the order points were created during reconstruction.
    #[allow(dead_code)]
    fn generate_point_ids_and_corners_discovery(&self, mesh: &Mesh) -> (Vec<PointIndex>, Vec<u32>) {
        let num_points = mesh.num_points();
        let mut point_ids = Vec::with_capacity(num_points);
        let mut data_to_corner_map = Vec::with_capacity(num_points);

        for i in 0..num_points {
            let pid = PointIndex(i as u32);
            point_ids.push(pid);
            let corner = self
                .edgebreaker_vertex_to_corner_map
                .get(i)
                .cloned()
                .unwrap_or(u32::MAX);
            data_to_corner_map.push(if corner == u32::MAX { 0 } else { corner });
        }

        (point_ids, data_to_corner_map)
    }

    #[allow(dead_code)]
    fn generate_point_ids_and_corners_dfs(
        &self,
        mesh: &Mesh,
        processed_connectivity_corners: &[u32],
    ) -> Result<(Vec<PointIndex>, Vec<u32>, Vec<i32>), DracoError> {
        let corner_table = self.corner_table.as_ref().ok_or_else(|| {
            DracoError::DracoError(
                "Edgebreaker DFS attribute traversal missing corner table".to_string(),
            )
        })?;
        Self::generate_point_ids_and_corners_dfs_for_table(
            mesh,
            corner_table,
            processed_connectivity_corners,
        )
    }

    fn generate_point_ids_and_corners_dfs_for_table(
        mesh: &Mesh,
        corner_table: &CornerTable,
        processed_connectivity_corners: &[u32],
    ) -> Result<(Vec<PointIndex>, Vec<u32>, Vec<i32>), DracoError> {
        let num_vertices = corner_table.num_vertices();
        let num_faces = corner_table.num_faces();

        let mut point_ids = Vec::with_capacity(num_vertices);
        let mut data_to_corner_map = Vec::with_capacity(num_vertices);
        let mut vertex_to_data_map = vec![-1i32; num_vertices];
        let mut visited_vertices = vec![false; num_vertices];
        let mut visited_faces = vec![false; num_faces];
        let event_log_enabled = test_event_log::enabled();

        // Helper to get mesh PointIndex from corner (matches C++ Mesh::CornerToPointId)
        let corner_to_point_id = |c: CornerIndex| -> PointIndex {
            if c == INVALID_CORNER_INDEX {
                return PointIndex(u32::MAX);
            }
            let face_id = FaceIndex(c.0 / 3);
            let corner_offset = (c.0 % 3) as usize;
            mesh.face(face_id)[corner_offset]
        };

        // Visit a corner table vertex and record it as a point ID.
        // This matches C++ MeshAttributeIndicesEncodingObserver::OnNewVertexVisited
        // which gets point_id from mesh_->face(corner / 3)[corner % 3]

        // DFS traversal matching C++ DepthFirstTraverser::TraverseFromCorner exactly
        let mut traverse_from_corner =
            |start_corner: CornerIndex,
             point_ids: &mut Vec<PointIndex>,
             vertex_to_data_map: &mut Vec<i32>,
             visited_vertices: &mut Vec<bool>,
             visited_faces: &mut Vec<bool>| {
                let start_face = corner_table.face(start_corner);
                if start_face == crate::geometry_indices::INVALID_FACE_INDEX {
                    return;
                }
                if visited_faces[start_face.0 as usize] {
                    return; // Already traversed
                }

                let mut corner_stack: Vec<CornerIndex> = Vec::new();
                corner_stack.push(start_corner);

                // For the first face, check the remaining corners as they may not be processed yet.
                // C++ visits Next, then Previous vertices BEFORE the main loop.
                let next_vert = corner_table.vertex(corner_table.next(start_corner));
                let prev_vert = corner_table.vertex(corner_table.previous(start_corner));

                if next_vert == crate::geometry_indices::INVALID_VERTEX_INDEX
                    || prev_vert == crate::geometry_indices::INVALID_VERTEX_INDEX
                {
                    return;
                }

                // Visit Next vertex
                if !visited_vertices[next_vert.0 as usize] {
                    visited_vertices[next_vert.0 as usize] = true;
                    let next_corner = corner_table.next(start_corner);
                    let point_id = corner_to_point_id(next_corner);
                    let data_id = point_ids.len() as i32;
                    vertex_to_data_map[next_vert.0 as usize] = data_id;
                    if event_log_enabled {
                        test_event_log::record_event(format!(
                            "MAP:{}->v{}",
                            next_corner.0, next_vert.0
                        ));
                        test_event_log::record_event(format!(
                            "MAP_POINT:{}->p{}",
                            next_corner.0, point_id.0
                        ));
                    }
                    point_ids.push(point_id);
                    data_to_corner_map.push(next_corner.0);
                }
                // Visit Previous vertex
                if !visited_vertices[prev_vert.0 as usize] {
                    visited_vertices[prev_vert.0 as usize] = true;
                    let prev_corner = corner_table.previous(start_corner);
                    let point_id = corner_to_point_id(prev_corner);
                    let data_id = point_ids.len() as i32;
                    vertex_to_data_map[prev_vert.0 as usize] = data_id;
                    if event_log_enabled {
                        test_event_log::record_event(format!(
                            "MAP:{}->v{}",
                            prev_corner.0, prev_vert.0
                        ));
                        test_event_log::record_event(format!(
                            "MAP_POINT:{}->p{}",
                            prev_corner.0, point_id.0
                        ));
                    }
                    point_ids.push(point_id);
                    data_to_corner_map.push(prev_corner.0);
                }

                // Start the actual traversal (matching C++ while loop)
                while let Some(mut corner_id) = corner_stack.pop() {
                    let mut face_id = corner_table.face(corner_id);

                    // Make sure the face hasn't been visited yet
                    if corner_id == INVALID_CORNER_INDEX || visited_faces[face_id.0 as usize] {
                        continue; // This face has been already traversed
                    }

                    loop {
                        visited_faces[face_id.0 as usize] = true;

                        let vert_id = corner_table.vertex(corner_id);
                        if vert_id == crate::geometry_indices::INVALID_VERTEX_INDEX {
                            break;
                        }

                        if !visited_vertices[vert_id.0 as usize] {
                            let on_boundary =
                                Self::is_vertex_on_boundary_impl(corner_table, vert_id);
                            visited_vertices[vert_id.0 as usize] = true;
                            let point_id = corner_to_point_id(corner_id);
                            let data_id = point_ids.len() as i32;
                            vertex_to_data_map[vert_id.0 as usize] = data_id;
                            if event_log_enabled {
                                test_event_log::record_event(format!(
                                    "MAP:{}->v{}",
                                    corner_id.0, vert_id.0
                                ));
                                test_event_log::record_event(format!(
                                    "MAP_POINT:{}->p{}",
                                    corner_id.0, point_id.0
                                ));
                            }
                            point_ids.push(point_id);
                            data_to_corner_map.push(corner_id.0);

                            if !on_boundary {
                                // Continue to right corner (GetRightCorner = Opposite(Next))
                                corner_id = corner_table.opposite(corner_table.next(corner_id));
                                if corner_id == INVALID_CORNER_INDEX {
                                    break;
                                }
                                face_id = corner_table.face(corner_id);
                                continue;
                            }
                        }

                        // The current vertex has been already visited or it was on a boundary.
                        // We need to determine whether we can visit any of its neighboring faces.
                        let right_corner_id = corner_table.opposite(corner_table.next(corner_id)); // GetRightCorner
                        let left_corner_id =
                            corner_table.opposite(corner_table.previous(corner_id)); // GetLeftCorner

                        let right_face_id = if right_corner_id == INVALID_CORNER_INDEX {
                            crate::geometry_indices::INVALID_FACE_INDEX
                        } else {
                            corner_table.face(right_corner_id)
                        };
                        let left_face_id = if left_corner_id == INVALID_CORNER_INDEX {
                            crate::geometry_indices::INVALID_FACE_INDEX
                        } else {
                            corner_table.face(left_corner_id)
                        };

                        let right_visited = right_face_id
                            == crate::geometry_indices::INVALID_FACE_INDEX
                            || visited_faces[right_face_id.0 as usize];
                        let left_visited = left_face_id
                            == crate::geometry_indices::INVALID_FACE_INDEX
                            || visited_faces[left_face_id.0 as usize];

                        if right_visited {
                            if left_visited {
                                // Both neighboring faces are visited. End reached.
                                break;
                            } else {
                                // Go to the left face
                                corner_id = left_corner_id;
                                face_id = left_face_id;
                            }
                        } else if left_visited {
                            // Left face visited, go to the right one
                            corner_id = right_corner_id;
                            face_id = right_face_id;
                        } else {
                            // Both neighboring faces are unvisited, we need to visit both.
                            // Split the traversal.
                            // First make the top of the current corner stack point to the left face
                            // (this one will be processed second).
                            // Add a new corner to the top of the stack (right face needs to be
                            // traversed first).
                            corner_stack.push(left_corner_id);
                            corner_stack.push(right_corner_id);
                            break;
                        }
                    }
                }
            };

        // Run the traverser in the same way as C++ MeshTraversalSequencer:
        // - If a corner_order is provided, process only those corners.
        // - Otherwise, process sequential CornerIndex(3 * face_id).
        if !processed_connectivity_corners.is_empty() {
            for &c in processed_connectivity_corners {
                traverse_from_corner(
                    CornerIndex(c),
                    &mut point_ids,
                    &mut vertex_to_data_map,
                    &mut visited_vertices,
                    &mut visited_faces,
                );
            }
        } else {
            for f in 0..num_faces {
                if !visited_faces[f] {
                    traverse_from_corner(
                        CornerIndex((f * 3) as u32),
                        &mut point_ids,
                        &mut vertex_to_data_map,
                        &mut visited_vertices,
                        &mut visited_faces,
                    );
                }
            }
        }

        Ok((point_ids, data_to_corner_map, vertex_to_data_map))
    }

    #[allow(dead_code)]
    fn generate_point_ids_and_corners_max_prediction_degree(
        &self,
        mesh: &Mesh,
        _processed_connectivity_corners: &[u32],
    ) -> Result<(Vec<PointIndex>, Vec<u32>, Vec<i32>), DracoError> {
        // Matches C++ MaxPredictionDegreeTraverser (MESH_TRAVERSAL_PREDICTION_DEGREE).
        let corner_table = self.corner_table.as_ref().ok_or_else(|| {
            DracoError::DracoError(
                "Edgebreaker prediction-degree traversal missing corner table".to_string(),
            )
        })?;
        let num_vertices = corner_table.num_vertices();
        let num_faces = corner_table.num_faces();

        let mut point_ids = Vec::with_capacity(num_vertices);
        let mut data_to_corner_map = Vec::with_capacity(num_vertices);
        // Build vertex_to_data_map during traversal: vertex_to_data_map[vertex_id] = data_id
        // where data_id is the index into point_ids where this vertex was first visited.
        let mut vertex_to_data_map: Vec<i32> = vec![-1; num_vertices];

        let mut visited_vertices = vec![false; num_vertices];
        let mut visited_faces = vec![false; num_faces];
        let mut prediction_degree: Vec<i32> = vec![0; num_vertices];
        let event_log_enabled = test_event_log::enabled();

        // Buckets (stacks) for priorities 0..2.
        let mut stacks: [Vec<CornerIndex>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut best_priority: usize = 0;

        // Helper to get mesh PointIndex from corner (matches C++ Mesh::CornerToPointId)
        let corner_to_point_id = |c: CornerIndex| -> PointIndex {
            if c == INVALID_CORNER_INDEX {
                return PointIndex(u32::MAX);
            }
            let face_id = FaceIndex(c.0 / 3);
            let corner_offset = (c.0 % 3) as usize;
            mesh.face(face_id)[corner_offset]
        };

        let visit_vertex = |v: VertexIndex,
                            c: CornerIndex,
                            point_ids: &mut Vec<PointIndex>,
                            data_to_corner_map: &mut Vec<u32>,
                            visited_vertices: &mut [bool],
                            vertex_to_data_map: &mut [i32]| {
            if v == INVALID_VERTEX_INDEX {
                return;
            }
            let vi = v.0 as usize;
            if vi >= visited_vertices.len() {
                return;
            }
            if !visited_vertices[vi] {
                visited_vertices[vi] = true;
                // Record vertex->data_id mapping BEFORE pushing to point_ids
                // data_id is current length of point_ids (0-indexed sequence number)
                vertex_to_data_map[vi] = point_ids.len() as i32;
                // Use corner_to_point_id to get mesh PointIndex from corner
                let point_id = corner_to_point_id(c);
                if event_log_enabled {
                    test_event_log::record_event(format!("MAP:{}->v{}", c.0, v.0));
                    test_event_log::record_event(format!("MAP_POINT:{}->p{}", c.0, point_id.0));
                }
                point_ids.push(point_id);
                data_to_corner_map.push(c.0);
            }
        };

        let compute_priority = |corner_id: CornerIndex,
                                visited_vertices: &[bool],
                                prediction_degree: &mut [i32]|
         -> usize {
            if corner_id == INVALID_CORNER_INDEX {
                return 2;
            }
            let v_tip = corner_table.vertex(corner_id);
            if v_tip == INVALID_VERTEX_INDEX {
                return 2;
            }
            let vi = v_tip.0 as usize;
            if vi < visited_vertices.len() && visited_vertices[vi] {
                return 0;
            }
            if vi < prediction_degree.len() {
                prediction_degree[vi] += 1;
                if prediction_degree[vi] > 1 {
                    1
                } else {
                    2
                }
            } else {
                2
            }
        };

        let add_corner_to_stack = |ci: CornerIndex,
                                   priority: usize,
                                   stacks: &mut [Vec<CornerIndex>; 3],
                                   best_priority: &mut usize| {
            let p = priority.min(2);
            stacks[p].push(ci);
            if p < *best_priority {
                *best_priority = p;
            }
        };

        let pop_next_corner =
            |stacks: &mut [Vec<CornerIndex>; 3], best_priority: &mut usize| -> CornerIndex {
                for p in *best_priority..3 {
                    if let Some(ci) = stacks[p].pop() {
                        *best_priority = p;
                        return ci;
                    }
                }
                INVALID_CORNER_INDEX
            };

        let clear_stacks = |stacks: &mut [Vec<CornerIndex>; 3]| {
            stacks[0].clear();
            stacks[1].clear();
            stacks[2].clear();
        };

        let traverse_from_corner =
            |start_corner: CornerIndex,
             point_ids: &mut Vec<PointIndex>,
             data_to_corner_map: &mut Vec<u32>,
             visited_vertices: &mut Vec<bool>,
             visited_faces: &mut Vec<bool>,
             prediction_degree: &mut Vec<i32>,
             stacks: &mut [Vec<CornerIndex>; 3],
             best_priority: &mut usize,
             vertex_to_data_map: &mut Vec<i32>| {
                let start_face = corner_table.face(start_corner);
                if start_face == crate::geometry_indices::INVALID_FACE_INDEX {
                    return;
                }
                if visited_faces[start_face.0 as usize] {
                    return;
                }

                clear_stacks(stacks);
                stacks[0].push(start_corner);
                *best_priority = 0;

                // Pre-visit next, prev and tip vertices.
                let next_c = corner_table.next(start_corner);
                let prev_c = corner_table.previous(start_corner);
                visit_vertex(
                    corner_table.vertex(next_c),
                    next_c,
                    point_ids,
                    data_to_corner_map,
                    visited_vertices,
                    vertex_to_data_map,
                );
                visit_vertex(
                    corner_table.vertex(prev_c),
                    prev_c,
                    point_ids,
                    data_to_corner_map,
                    visited_vertices,
                    vertex_to_data_map,
                );
                visit_vertex(
                    corner_table.vertex(start_corner),
                    start_corner,
                    point_ids,
                    data_to_corner_map,
                    visited_vertices,
                    vertex_to_data_map,
                );

                loop {
                    let mut corner_id = pop_next_corner(stacks, best_priority);
                    if corner_id == INVALID_CORNER_INDEX {
                        break;
                    }
                    let face_id0 = corner_table.face(corner_id);
                    if face_id0 == crate::geometry_indices::INVALID_FACE_INDEX {
                        continue;
                    }
                    if visited_faces[face_id0.0 as usize] {
                        continue;
                    }

                    loop {
                        let face_id = corner_table.face(corner_id);
                        if face_id == crate::geometry_indices::INVALID_FACE_INDEX {
                            break;
                        }
                        visited_faces[face_id.0 as usize] = true;

                        let vert_id = corner_table.vertex(corner_id);
                        if vert_id != INVALID_VERTEX_INDEX {
                            let vi = vert_id.0 as usize;
                            if vi < visited_vertices.len() && !visited_vertices[vi] {
                                visit_vertex(
                                    vert_id,
                                    corner_id,
                                    point_ids,
                                    data_to_corner_map,
                                    visited_vertices,
                                    vertex_to_data_map,
                                );
                            }
                        }

                        let right_corner_id = corner_table.right_corner(corner_id);
                        let left_corner_id = corner_table.left_corner(corner_id);
                        let right_face_id = if right_corner_id == INVALID_CORNER_INDEX {
                            crate::geometry_indices::INVALID_FACE_INDEX
                        } else {
                            corner_table.face(right_corner_id)
                        };
                        let left_face_id = if left_corner_id == INVALID_CORNER_INDEX {
                            crate::geometry_indices::INVALID_FACE_INDEX
                        } else {
                            corner_table.face(left_corner_id)
                        };

                        let is_right_face_visited = right_face_id
                            == crate::geometry_indices::INVALID_FACE_INDEX
                            || visited_faces[right_face_id.0 as usize];
                        let is_left_face_visited = left_face_id
                            == crate::geometry_indices::INVALID_FACE_INDEX
                            || visited_faces[left_face_id.0 as usize];

                        if !is_left_face_visited {
                            let priority = compute_priority(
                                left_corner_id,
                                visited_vertices,
                                prediction_degree,
                            );
                            if is_right_face_visited && priority <= *best_priority {
                                corner_id = left_corner_id;
                                continue;
                            }
                            add_corner_to_stack(left_corner_id, priority, stacks, best_priority);
                        }

                        if !is_right_face_visited {
                            let priority = compute_priority(
                                right_corner_id,
                                visited_vertices,
                                prediction_degree,
                            );
                            if priority <= *best_priority {
                                corner_id = right_corner_id;
                                continue;
                            }
                            add_corner_to_stack(right_corner_id, priority, stacks, best_priority);
                        }

                        break;
                    }
                }
            };

        // C++ DECODER traverses faces SEQUENTIALLY (face 0, face 1, face 2, ...)
        // NOT using processed_connectivity_corners (that's only for the ENCODER)!
        // See C++ MeshTraversalSequencer::GenerateSequenceInternal() - when corner_order_ is null,
        // it does: for (int i = 0; i < num_faces; ++i) ProcessCorner(CornerIndex(3 * i));
        for f in 0..num_faces {
            if visited_faces[f] {
                continue;
            }
            let first_corner = corner_table.first_corner(FaceIndex(f as u32));
            traverse_from_corner(
                first_corner,
                &mut point_ids,
                &mut data_to_corner_map,
                &mut visited_vertices,
                &mut visited_faces,
                &mut prediction_degree,
                &mut stacks,
                &mut best_priority,
                &mut vertex_to_data_map,
            );
        }

        Ok((point_ids, data_to_corner_map, vertex_to_data_map))
    }

    #[allow(dead_code)]
    fn is_vertex_on_boundary(&self, corner_table: &CornerTable, vert_id: VertexIndex) -> bool {
        let start_c = corner_table.left_most_corner(vert_id);
        if start_c == INVALID_CORNER_INDEX {
            return true;
        }
        let mut c = start_c;
        loop {
            // Edge (c, next(c)) is incident to v.
            if corner_table.opposite(c) == INVALID_CORNER_INDEX {
                return true;
            }
            // Edge (prev(c), c) is also incident to v.
            if corner_table.opposite(corner_table.previous(c)) == INVALID_CORNER_INDEX {
                return true;
            }
            c = corner_table.swing_right(c);
            if c == INVALID_CORNER_INDEX {
                return true;
            }
            if c == start_c {
                break;
            }
        }
        false
    }

    /// Helper function to check if a vertex is on the boundary
    /// Matches C++ CornerTable::IsOnBoundary
    fn is_vertex_on_boundary_impl(
        corner_table: &crate::corner_table::CornerTable,
        v: VertexIndex,
    ) -> bool {
        let corner = corner_table.left_most_corner(v);
        if corner == INVALID_CORNER_INDEX {
            return true; // Isolated vertex - treat as boundary
        }
        // C++ checks: if (SwingLeft(corner) == kInvalidCornerIndex) return true;
        if corner_table.swing_left(corner) == INVALID_CORNER_INDEX {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_corner_table_rejects_out_of_range_seam_corner() {
        let mut corner_table = CornerTable::new(1);
        corner_table.set_face_vertices(FaceIndex(0), PointIndex(0), PointIndex(1), PointIndex(2));

        let invalid_corner = corner_table.num_corners() as u32;
        let status = MeshDecoder::make_attribute_corner_table(&corner_table, &[invalid_corner]);

        assert!(status.is_err());
    }

    #[test]
    fn vertex_to_data_map_rejects_out_of_range_corner() {
        let mut corner_table = CornerTable::new(1);
        corner_table.set_face_vertices(FaceIndex(0), PointIndex(0), PointIndex(1), PointIndex(2));

        let invalid_corner = corner_table.num_corners() as u32;
        let status = build_vertex_to_data_map_from_corner_map(&corner_table, &[invalid_corner]);

        assert!(status.is_err());
    }
}

fn validate_mesh_index_count(num_faces: usize) -> Result<usize, DracoError> {
    num_faces
        .checked_mul(3)
        .ok_or_else(|| DracoError::DracoError("Mesh face index count overflow".to_string()))
}

fn make_zeroed_indices(num_indices: usize) -> Result<Vec<u32>, DracoError> {
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(num_indices)
        .map_err(|_| DracoError::DracoError("Failed to allocate mesh indices".to_string()))?;
    indices.resize(num_indices, 0);
    Ok(indices)
}

fn make_point_ids(num_points: usize) -> Result<Vec<PointIndex>, DracoError> {
    let mut point_ids = Vec::new();
    point_ids
        .try_reserve_exact(num_points)
        .map_err(|_| DracoError::DracoError("Failed to allocate point ids".to_string()))?;
    for i in 0..num_points {
        point_ids.push(PointIndex(i as u32));
    }
    Ok(point_ids)
}
