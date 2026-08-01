//! EdgeBreaker connectivity decoder.
//!
//! [`MeshEdgebreakerDecoder`] reconstructs mesh topology from the EdgeBreaker
//! symbol stream (C, L, R, S, E), rebuilding the corner table and the
//! data-to-corner maps the attribute decoders need. Per-symbol traversal is
//! delegated to a pluggable traversal decoder. Port of Draco's
//! `mesh_edgebreaker_decoder.h`.

use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::edgebreaker_connectivity_decoder::{
    EdgebreakerConnectivityDecoder, EdgebreakerTraversalDecoder,
};
use crate::geometry_indices::{CornerIndex, FaceIndex, PointIndex, VertexIndex};
use crate::mesh::Mesh;
use crate::mesh_edgebreaker_shared::{EdgeFaceName, EdgebreakerSymbol, TopologySplitEventData};
use crate::rans_bit_decoder::RAnsBitDecoder;
use crate::status::{error_status, DracoError, Status};

pub struct MeshEdgebreakerDecoder {
    data_to_corner_map: Option<Vec<u32>>,
    attribute_seam_corners: Vec<Vec<u32>>,
    // Traversal order for attribute decoding (matches C++ processed_connectivity_corners_)
    processed_connectivity_corners: Vec<u32>,
    // Corner table built during connectivity decoding, with proper opposite mappings
    corner_table: Option<crate::corner_table::CornerTable>,
    traversal_decoder_type: u8,
    vertex_to_corner_map: Vec<u32>,
    is_vert_hole: Vec<bool>,
}

impl Default for MeshEdgebreakerDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshEdgebreakerDecoder {
    pub fn new() -> Self {
        Self {
            data_to_corner_map: None,
            attribute_seam_corners: Vec::new(),
            processed_connectivity_corners: Vec::new(),
            corner_table: None,
            traversal_decoder_type: 0,
            vertex_to_corner_map: Vec::new(),
            is_vert_hole: Vec::new(),
        }
    }

    pub fn get_corner_table(&self) -> Option<&crate::corner_table::CornerTable> {
        self.corner_table.as_ref()
    }

    pub fn take_corner_table(&mut self) -> Option<crate::corner_table::CornerTable> {
        self.corner_table.take()
    }

    pub fn take_data_to_corner_map(&mut self) -> Option<Vec<u32>> {
        self.data_to_corner_map.take()
    }

    pub fn take_attribute_seam_corners(&mut self) -> Vec<Vec<u32>> {
        std::mem::take(&mut self.attribute_seam_corners)
    }

    pub fn get_attribute_seam_corners(&self, attribute_index: usize) -> Option<&Vec<u32>> {
        self.attribute_seam_corners.get(attribute_index)
    }

    pub fn get_processed_connectivity_corners(&self) -> &[u32] {
        &self.processed_connectivity_corners
    }

    pub fn get_vertex_to_corner_map(&self) -> &[u32] {
        &self.vertex_to_corner_map
    }

    pub fn take_is_vert_hole(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.is_vert_hole)
    }

    pub fn get_traversal_decoder_type(&self) -> u8 {
        self.traversal_decoder_type
    }

    pub fn decode_connectivity(
        &mut self,
        in_buffer: &mut DecoderBuffer,
        out_mesh: &mut Mesh,
    ) -> Status {
        self.data_to_corner_map = None;

        let version_major = in_buffer.version_major();
        let version_minor = in_buffer.version_minor();
        let bitstream_version = crate::version::bitstream_version(version_major, version_minor);
        if bitstream_version < 0x0202 && !cfg!(feature = "legacy_bitstream_decode") {
            return Err(DracoError::bitstream_version_unsupported());
        }

        // Traversal decoder type is always present (C++ reads unconditionally in InitializeDecoder).
        self.traversal_decoder_type = in_buffer.decode_u8().map_err(|_| {
            DracoError::general("Failed to read traversal decoder type".to_string())
        })?;
        // Type 0 = Standard, Type 1 = Predictive (deprecated), Type 2 = Valence.
        if self.traversal_decoder_type > 2 {
            return Err(DracoError::general(format!(
                "Unsupported Edgebreaker traversal decoder type: {}",
                self.traversal_decoder_type
            )));
        }
        if self.traversal_decoder_type == 1 && !cfg!(feature = "legacy_bitstream_decode") {
            return Err(DracoError::unsupported_feature(
                "Edgebreaker predictive traversal decode is not supported".to_string(),
            ));
        }
        if self.traversal_decoder_type == 2 && !cfg!(feature = "edgebreaker_valence_decode") {
            return Err(DracoError::general(
                "Edgebreaker valence traversal decode support is disabled".to_string(),
            ));
        }

        let mut _num_new_vertices = 0;
        if bitstream_version < 0x0202 {
            if bitstream_version < 0x0200 {
                _num_new_vertices = in_buffer.decode_u32().map_err(|_| {
                    DracoError::general("Failed to read num_new_vertices".to_string())
                })?;
            } else {
                _num_new_vertices = in_buffer.decode_varint().map_err(|_| {
                    DracoError::general("Failed to read num_new_vertices".to_string())
                })? as u32;
            }
        }

        let num_encoded_vertices = if bitstream_version < 0x0200 {
            in_buffer.decode_u32().map_err(|_| {
                DracoError::general("Failed to read num_encoded_vertices".to_string())
            })?
        } else {
            in_buffer.decode_varint().map_err(|_| {
                DracoError::general("Failed to read num_encoded_vertices".to_string())
            })? as u32
        };

        let num_faces = if bitstream_version < 0x0200 {
            in_buffer
                .decode_u32()
                .map_err(|_| DracoError::general("Failed to read num_faces".to_string()))?
        } else {
            in_buffer
                .decode_varint()
                .map_err(|_| DracoError::general("Failed to read num_faces".to_string()))?
                as u32
        };

        let num_attribute_data = in_buffer
            .decode_u8()
            .map_err(|_| DracoError::general("Failed to read attribute data count".to_string()))?;

        // Reject impossible geometry counts before allocating face and
        // connectivity storage. These mirror the C++ MeshEdgebreakerDecoderImpl
        // checks and are pure geometric invariants, so they remain valid for
        // heavily compressed (valence/rANS) streams where the input byte size is
        // not a usable bound. Cheap, run once per decode, off the hot path.
        if num_faces > u32::MAX / 3 {
            return Err(DracoError::general(
                "Edgebreaker num_faces exceeds maximum".to_string(),
            ));
        }
        if num_encoded_vertices as u64 > num_faces as u64 * 3 {
            return Err(DracoError::general(
                "Edgebreaker num_encoded_vertices exceeds 3 * num_faces".to_string(),
            ));
        }
        // A manifold mesh with |num_faces| faces needs at least 3*num_faces/2
        // edges, while |num_encoded_vertices| vertices can form at most
        // V*(V-1)/2 edges. If the latter is smaller the counts are impossible.
        let min_num_face_edges = 3u64 * num_faces as u64 / 2;
        let num_encoded_vertices_64 = num_encoded_vertices as u64;
        let max_num_vertex_edges =
            num_encoded_vertices_64 * num_encoded_vertices_64.saturating_sub(1) / 2;
        if max_num_vertex_edges < min_num_face_edges {
            return Err(DracoError::general(
                "Edgebreaker vertex/face counts cannot form a manifold mesh".to_string(),
            ));
        }

        let num_symbols = if bitstream_version < 0x0200 {
            in_buffer
                .decode_u32()
                .map_err(|_| DracoError::general("Failed to read symbol count".to_string()))?
                as usize
        } else {
            in_buffer
                .decode_varint()
                .map_err(|_| DracoError::general("Failed to read symbol count".to_string()))?
                as usize
        };

        let num_split_symbols =
            if bitstream_version < 0x0200 {
                in_buffer.decode_u32().map_err(|_| {
                    DracoError::general("Failed to read split symbol count".to_string())
                })? as usize
            } else {
                in_buffer.decode_varint().map_err(|_| {
                    DracoError::general("Failed to read split symbol count".to_string())
                })? as usize
            };

        // Symbol-count invariants (mirrors C++): the initial face of each
        // connected component may be unencoded, so |num_faces| is between
        // |num_symbols| and num_symbols * 4/3, and split symbols are a subset of
        // all symbols. These bound the connectivity allocations below without
        // relying on input size.
        if (num_faces as usize) < num_symbols {
            return Err(DracoError::general(
                "Edgebreaker num_faces is smaller than num_symbols".to_string(),
            ));
        }
        let max_encoded_faces = num_symbols + num_symbols / 3;
        if num_faces as usize > max_encoded_faces {
            return Err(DracoError::general(
                "Edgebreaker num_faces exceeds maximum implied by num_symbols".to_string(),
            ));
        }
        if num_split_symbols > num_symbols {
            return Err(DracoError::general(
                "Edgebreaker num_split_symbols exceeds num_symbols".to_string(),
            ));
        }

        // Deliberately not sized from |num_faces| here. The faces are written
        // further down from the corner table the traversal actually built, so
        // reserving for the declared count buys nothing and lets a few hundred
        // bytes of malformed input ask for gigabytes -- ahead of the checks
        // below that would have rejected the stream anyway.
        out_mesh.set_num_points(num_encoded_vertices as usize);

        // Read hole/topology split events.
        // Draco stores these events inline for v2.2+, but for older streams (<2.2)
        // they are stored after the traversal buffer, and the traversal buffer size
        // is explicitly encoded.
        let (topology_split_data, topology_split_decoded_bytes) = if bitstream_version < 0x0202 {
            let encoded_connectivity_size = if bitstream_version < 0x0200 {
                in_buffer.decode_u32().map_err(|_| {
                    DracoError::general("Failed to read encoded_connectivity_size".to_string())
                })? as usize
            } else {
                in_buffer.decode_varint().map_err(|_| {
                    DracoError::general("Failed to read encoded_connectivity_size".to_string())
                })? as usize
            };

            if encoded_connectivity_size == 0
                || encoded_connectivity_size > in_buffer.remaining_size()
            {
                return Err(DracoError::general(
                    "Invalid encoded_connectivity_size".to_string(),
                ));
            }

            // Decode events from a temporary buffer starting at the end of the
            // traversal buffer, while keeping |in_buffer| positioned at the start
            // of the traversal buffer.
            let remaining = in_buffer.remaining_data();
            let events_slice = &remaining[encoded_connectivity_size..];
            let mut event_buffer = DecoderBuffer::new(events_slice);
            event_buffer.set_version(version_major, version_minor);

            let (events, decoded_bytes) = Self::decode_hole_and_topology_split_events(
                &mut event_buffer,
                bitstream_version,
                num_faces as usize,
            )?;
            (events, decoded_bytes)
        } else {
            let events = Self::decode_topology_split_events_inline(
                in_buffer,
                bitstream_version,
                num_faces as usize,
            )?;
            (events, 0)
        };

        // Validate split data count.
        if topology_split_data.len() > num_split_symbols {
            return Err(error_status(format!(
                "Split event count exceeds split-symbol count (split_symbols={num_split_symbols}, events={})",
                topology_split_data.len()
            )));
        }

        // Read symbol stream
        // The encoder generates symbols Top-Down (Root->Leaf).
        // The decoder must process them Bottom-Up (Leaf->Root).
        // So we must reverse the stream.
        // NOTE: The valence (type 2) and predictive (type 1) traversals read the
        // main symbol stream on demand as region 1 below, so skip the eager
        // standard-path decode here.
        let symbols = if self.traversal_decoder_type == 1 || self.traversal_decoder_type == 2 {
            Vec::new()
        } else {
            Self::decode_symbol_stream(in_buffer, num_symbols)?
        };

        // Reconstruct topology.
        // Draco allows up to (num_encoded_vertices + num_split_symbols) vertices during
        // connectivity decoding because split symbols can introduce temporary vertices
        // that are eliminated during deduplication.
        let max_num_vertices = (num_encoded_vertices as usize).saturating_add(num_split_symbols);

        self.reconstruct_mesh(
            &symbols,
            &topology_split_data,
            out_mesh,
            num_faces as usize,
            max_num_vertices,
            num_attribute_data,
            num_symbols,
            in_buffer,
        )?;

        // For pre-v2.2 streams, the hole/topology split event payload was decoded
        // from a temporary buffer, and the main buffer is now positioned at the
        // start of that payload. Advance it so attribute decoding starts at the
        // correct location.
        if topology_split_decoded_bytes > 0 {
            if topology_split_decoded_bytes > in_buffer.remaining_size() {
                return Err(DracoError::general(
                    "Invalid topology split decoded byte count".to_string(),
                ));
            }
            in_buffer.try_advance(topology_split_decoded_bytes)?;
        }

        Ok(())
    }

    fn decode_hole_and_topology_split_events(
        in_buffer: &mut DecoderBuffer,
        bitstream_version: u16,
        num_faces: usize,
    ) -> Result<(Vec<TopologySplitEventData>, usize), DracoError> {
        // Matches MeshEdgebreakerDecoderImpl::DecodeHoleAndTopologySplitEvents.
        let num_topology_splits = if bitstream_version < 0x0200 {
            in_buffer.decode_u32().map_err(|_| {
                DracoError::general("Failed to read num_topology_splits".to_string())
            })?
        } else {
            in_buffer.decode_varint().map_err(|_| {
                DracoError::general("Failed to read num_topology_splits".to_string())
            })? as u32
        };

        if num_topology_splits as usize > num_faces {
            return Err(DracoError::general(
                "Topology split count exceeds face count".to_string(),
            ));
        }

        // Every topology split event consumes payload bytes - a varint pair, or
        // a fixed u32/u32/u8 below 1.2 - so one byte per event is a floor, not
        // an average. That makes this guard sound where the one-bit-per-point
        // guards were not, and it is what stops a tiny malformed input driving a
        // large CPU-only scan through events that are not there. It stays.
        if num_topology_splits as usize > in_buffer.remaining_size() {
            return Err(DracoError::general(format!(
                "Topology split count {num_topology_splits} exceeds the {} bytes left to hold it",
                in_buffer.remaining_size()
            )));
        }

        // Cap the pre-reservation by the remaining byte budget as a second line
        // of defense against malformed counts.
        let mut events: Vec<TopologySplitEventData> =
            Vec::with_capacity((num_topology_splits as usize).min(in_buffer.remaining_size()));
        if num_topology_splits > 0 {
            if bitstream_version < 0x0102 {
                // Legacy (<1.2): absolute IDs + explicit edge byte.
                for _ in 0..num_topology_splits {
                    let split_symbol_id = in_buffer.decode_u32().map_err(|_| {
                        DracoError::general("Failed to read split_symbol_id".to_string())
                    })?;
                    let source_symbol_id = in_buffer.decode_u32().map_err(|_| {
                        DracoError::general("Failed to read source_symbol_id".to_string())
                    })?;
                    let edge_data = in_buffer.decode_u8().map_err(|_| {
                        DracoError::general("Failed to read source_edge byte".to_string())
                    })?;
                    events.push(TopologySplitEventData {
                        split_symbol_id,
                        source_symbol_id,
                        source_edge: if (edge_data & 1) == 0 {
                            crate::mesh_edgebreaker_shared::EdgeFaceName::LeftFaceEdge
                        } else {
                            crate::mesh_edgebreaker_shared::EdgeFaceName::RightFaceEdge
                        },
                    });
                }
            } else {
                // Delta + varint IDs.
                let mut last_source_symbol_id: i32 = 0;
                for _ in 0..num_topology_splits {
                    let delta = in_buffer.decode_varint().map_err(|_| {
                        DracoError::general("Failed to read source symbol delta".to_string())
                    })? as i32;
                    // Wrapping matches C++ int arithmetic; malformed deltas must
                    // not panic under overflow checks.
                    let source_symbol_id = last_source_symbol_id.wrapping_add(delta);

                    let split_delta = in_buffer.decode_varint().map_err(|_| {
                        DracoError::general("Failed to read split symbol delta".to_string())
                    })? as i32;
                    if split_delta > source_symbol_id {
                        return Err(DracoError::general(
                            "Invalid split symbol delta".to_string(),
                        ));
                    }
                    let split_symbol_id = source_symbol_id.wrapping_sub(split_delta);

                    events.push(TopologySplitEventData {
                        split_symbol_id: split_symbol_id as u32,
                        source_symbol_id: source_symbol_id as u32,
                        source_edge: crate::mesh_edgebreaker_shared::EdgeFaceName::LeftFaceEdge,
                    });

                    last_source_symbol_id = source_symbol_id;
                }

                // Split edges are bit-coded; for <2.2 streams the decoder reads 2 bits.
                if !events.is_empty() {
                    in_buffer.start_bit_decoding(false).map_err(|_| {
                        DracoError::general(
                            "Failed to start bit decoding for split-event source_edge bits"
                                .to_string(),
                        )
                    })?;
                    for event in &mut events {
                        let bits = if bitstream_version < 0x0202 { 2 } else { 1 };
                        let edge_data =
                            in_buffer
                                .decode_least_significant_bits32(bits)
                                .map_err(|_| {
                                    DracoError::general(
                                        "Failed to read split-event source_edge bits".to_string(),
                                    )
                                })?;
                        event.source_edge = if (edge_data & 1) == 0 {
                            crate::mesh_edgebreaker_shared::EdgeFaceName::LeftFaceEdge
                        } else {
                            crate::mesh_edgebreaker_shared::EdgeFaceName::RightFaceEdge
                        };
                    }
                    in_buffer.end_bit_decoding();
                }
            }
        }

        Self::skip_hole_events(in_buffer, bitstream_version)?;
        Ok((events, in_buffer.position()))
    }

    fn skip_hole_events(
        in_buffer: &mut DecoderBuffer,
        bitstream_version: u16,
    ) -> Result<(), DracoError> {
        // Hole events are present only for older streams (<2.1). The C++ decoder
        // parses them but never uses them (dead/legacy data), so we just need to
        // advance the buffer past the hole event data.
        let mut num_hole_events: u32 = 0;
        if bitstream_version < 0x0200 {
            num_hole_events = in_buffer
                .decode_u32()
                .map_err(|_| DracoError::general("Failed to read num_hole_events".to_string()))?;
        } else if bitstream_version < 0x0201 {
            num_hole_events = in_buffer
                .decode_varint()
                .map_err(|_| DracoError::general("Failed to read num_hole_events".to_string()))?
                as u32;
        }

        if num_hole_events > 0 {
            if bitstream_version < 0x0102 {
                for _ in 0..num_hole_events {
                    // Legacy: raw i32 symbol id.
                    let _sym_id: i32 = in_buffer.decode::<i32>().map_err(|_| {
                        DracoError::general("Failed to read hole event".to_string())
                    })?;
                }
            } else {
                // Delta + varint.
                let mut last_symbol_id: i32 = 0;
                for _ in 0..num_hole_events {
                    let delta = in_buffer.decode_varint().map_err(|_| {
                        DracoError::general("Failed to read hole event delta".to_string())
                    })? as i32;
                    let _sym_id = last_symbol_id + delta;
                    last_symbol_id = _sym_id;
                }
            }
        }

        Ok(())
    }

    fn decode_topology_split_events_inline(
        in_buffer: &mut DecoderBuffer,
        bitstream_version: u16,
        num_faces: usize,
    ) -> Result<Vec<TopologySplitEventData>, DracoError> {
        // Inline event format is only used in v2.2+ streams.
        if bitstream_version < 0x0202 {
            return Ok(Vec::new());
        }

        let num_events = in_buffer
            .decode_varint()
            .map_err(|_| DracoError::general("Failed to read split event count".to_string()))?
            as usize;
        if num_events > num_faces {
            return Err(DracoError::general(
                "Topology split count exceeds face count".to_string(),
            ));
        }
        // Cap the pre-reservation by the remaining byte budget: each event
        // consumes at least one byte, so a larger count cannot be satisfied.
        let mut events = Vec::with_capacity(num_events.min(in_buffer.remaining_size()));

        if num_events > 0 {
            let mut last_source_symbol_id: i32 = 0;
            for _ in 0..num_events {
                let delta = in_buffer.decode_varint().map_err(|_| {
                    DracoError::general("Failed to read source symbol delta".to_string())
                })? as i32;
                // Wrapping matches C++ int arithmetic; malformed deltas must not
                // panic under overflow checks.
                let source_symbol_id = last_source_symbol_id.wrapping_add(delta);

                let split_delta = in_buffer.decode_varint().map_err(|_| {
                    DracoError::general("Failed to read split symbol delta".to_string())
                })? as i32;
                let split_symbol_id = source_symbol_id.wrapping_sub(split_delta);

                events.push(TopologySplitEventData {
                    split_symbol_id: split_symbol_id as u32,
                    source_symbol_id: source_symbol_id as u32,
                    source_edge: crate::mesh_edgebreaker_shared::EdgeFaceName::LeftFaceEdge,
                });

                last_source_symbol_id = source_symbol_id;
            }
        }

        if num_events > 0 {
            in_buffer.start_bit_decoding(false).map_err(|_| {
                DracoError::general(
                    "Failed to start bit decoding for split-event source_edge bits".to_string(),
                )
            })?;
            for event in &mut events {
                let edge_bit = in_buffer.decode_least_significant_bits32(1).map_err(|_| {
                    DracoError::general("Failed to read split-event source_edge bit".to_string())
                })?;
                event.source_edge = if edge_bit == 0 {
                    crate::mesh_edgebreaker_shared::EdgeFaceName::LeftFaceEdge
                } else {
                    crate::mesh_edgebreaker_shared::EdgeFaceName::RightFaceEdge
                };
            }
            in_buffer.end_bit_decoding();
        }

        Ok(events)
    }

    // NOTE: Legacy (<2.2) split/hole event decoding is handled by
    // decode_hole_and_topology_split_events().

    fn topology_bit_pattern_to_symbol_id(topology: u32) -> Result<u32, DracoError> {
        // Draco topology bit patterns:
        // C=0, S=1, L=3, R=5, E=7.
        // Map them to our internal symbol IDs: C=0,S=1,L=2,R=3,E=4.
        match topology {
            0 => Ok(EdgebreakerSymbol::Center as u32),
            1 => Ok(EdgebreakerSymbol::Split as u32),
            3 => Ok(EdgebreakerSymbol::Left as u32),
            5 => Ok(EdgebreakerSymbol::Right as u32),
            7 => Ok(EdgebreakerSymbol::End as u32),
            _ => Err(DracoError::general(format!(
                "Invalid Edgebreaker topology bit pattern: {topology}"
            ))),
        }
    }

    // Mesh reconstruction requires 8 parameters: symbols, topology split data, output mesh,
    // size constraints, attribute count, num_symbols for valence path, and decoder buffer. Each
    // parameter controls a different aspect of the complex topology reconstruction process.
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_mesh<'a>(
        &mut self,
        symbols: &[u32],
        topology_split_data: &[TopologySplitEventData],
        mesh: &mut Mesh,
        total_num_faces: usize,
        max_num_vertices: usize,
        num_attribute_data: u8,
        num_symbols: usize,
        in_buffer: &mut DecoderBuffer<'a>,
    ) -> Result<usize, DracoError> {
        // Standard traversal pre-decodes |symbols|; valence/predictive read them on
        // demand, so fall back to the header symbol count.
        let actual_num_symbols =
            if self.traversal_decoder_type == 1 || self.traversal_decoder_type == 2 {
                num_symbols
            } else {
                symbols.len()
            };

        if actual_num_symbols == 0 {
            let corner_table = crate::corner_table::CornerTable::new(0);
            self.corner_table = Some(corner_table);
            self.data_to_corner_map = Some(Vec::new());
            return Ok(0);
        }

        let bitstream_version = in_buffer.bitstream_version();

        // For v < 2.2, start face configuration bits are stored as a raw bit buffer
        // (u64 size prefix + raw bits), NOT as rANS-encoded data.
        // For v >= 2.2, they use RAnsBitDecoder.
        let mut start_face_decoder = RAnsBitDecoder::new();
        let mut has_start_face_bits = false;
        let mut start_face_bits_legacy: Option<Vec<bool>> = None;
        // Pre-1.0 (bitstream < 2.2) valence streams carry a separate main traversal
        // symbol stream (region 1) that the standard path consumes via
        // `decode_symbol_stream`. The valence path skips that (`symbols = Vec::new()`),
        // so it must read region 1 here — otherwise the start-face region and the
        // per-context symbols below land on the wrong bytes. Its bits are replayed
        // for the "no active context" case in the valence decoder.
        #[cfg(any(
            feature = "edgebreaker_valence_decode",
            feature = "legacy_bitstream_decode"
        ))]
        let mut legacy_direct_symbol_bits: Option<Vec<bool>> = None;

        if bitstream_version < 0x0202 {
            // The valence (type 2) and predictive (type 1) traversals both consume
            // the main traversal symbol stream as region 1 (the standard path reads
            // it via decode_symbol_stream instead).
            #[cfg(any(
                feature = "edgebreaker_valence_decode",
                feature = "legacy_bitstream_decode"
            ))]
            if self.traversal_decoder_type == 1 || self.traversal_decoder_type == 2 {
                in_buffer.start_bit_decoding(true).map_err(|_| {
                    DracoError::general(
                        "Failed to start valence main-symbol bit decoding".to_string(),
                    )
                })?;
                // Each direct symbol is 1 or 3 bits, at most |num_symbols| of them.
                let cap = actual_num_symbols
                    .saturating_mul(3)
                    .min(in_buffer.remaining_size().saturating_mul(8));
                let mut bits = Vec::new();
                for _ in 0..cap {
                    match in_buffer.decode_least_significant_bits32(1) {
                        Ok(v) => bits.push(v != 0),
                        Err(_) => break,
                    }
                }
                in_buffer.end_bit_decoding();
                legacy_direct_symbol_bits = Some(bits);
            }

            // Read raw bit buffer for start faces (region 2 for valence streams;
            // the standard path already consumed its symbol stream, so this is its
            // first region).
            in_buffer.start_bit_decoding(true).map_err(|_| {
                DracoError::general("Failed to start start-face bit decoding".to_string())
            })?;
            // Pre-read a generous number of bits (one per component, max = num_symbols)
            // We read up to actual_num_symbols bits; unused ones are harmless
            let num_bits_to_read = actual_num_symbols.min(in_buffer.remaining_size() * 8);
            let mut bits = Vec::with_capacity(num_bits_to_read);
            for _ in 0..num_bits_to_read {
                match in_buffer.decode_least_significant_bits32(1) {
                    Ok(v) => bits.push(v != 0),
                    Err(_) => break,
                }
            }
            in_buffer.end_bit_decoding();
            start_face_bits_legacy = Some(bits);
        } else {
            has_start_face_bits = start_face_decoder.start_decoding(in_buffer);
        }

        // The declared face count bounds the traversal; the mesh itself is
        // sized below from the corner table the traversal actually produced.
        let mut connectivity_decoder = EdgebreakerConnectivityDecoder::try_new(
            total_num_faces as i32,
            max_num_vertices as i32,
        )?;

        // Choose traversal decoder based on the traversal_decoder_type read earlier.
        #[allow(unused_assignments)]
        let mut start_face_decoder_opt: Option<RAnsBitDecoder> = None;
        #[allow(unused_assignments)]
        let mut has_start_face_bits_flag = false;
        #[allow(unused_assignments)]
        let mut processed_connectivity_corners: Vec<u32> = Vec::new();

        // Valence and predictive modes both save the seam decoders to use after
        // connectivity (positioned between start faces and the per-context /
        // prediction streams).
        #[cfg(any(
            feature = "edgebreaker_valence_decode",
            feature = "legacy_bitstream_decode"
        ))]
        let mut legacy_seam_decoders: Vec<RAnsBitDecoder> = Vec::new();
        let remove_invalid_vertices = num_attribute_data == 0 || bitstream_version < 0x0202;

        let num_vertices = if self.traversal_decoder_type == 1 {
            // Predictive mode (legacy, pre-0.10.0). Buffer order mirrors valence:
            // start faces (already read), attribute seams, then a split-symbol
            // count and the binary prediction stream (vs valence's contexts).
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            {
                return Err(DracoError::general(
                    "Edgebreaker predictive traversal decode support is disabled".to_string(),
                ));
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            {
                for _ in 0..num_attribute_data {
                    let mut seam_decoder = RAnsBitDecoder::new();
                    if !seam_decoder.start_decoding(in_buffer) {
                        return Err(DracoError::general(
                            "Failed to start attribute seam decoding for predictive".to_string(),
                        ));
                    }
                    legacy_seam_decoders.push(seam_decoder);
                }

                // Split-symbol count (raw int32 pre-2.0); already folded into
                // max_num_vertices, so read only to advance the buffer.
                if in_buffer.decode_u32().is_err() {
                    return Err(DracoError::general(
                        "Failed to read predictive split-symbol count".to_string(),
                    ));
                }
                // Binary prediction stream (whether each prediction was correct).
                let mut prediction_decoder = RAnsBitDecoder::new();
                if !prediction_decoder.start_decoding(in_buffer) {
                    return Err(DracoError::general(
                        "Failed to start predictive prediction stream".to_string(),
                    ));
                }

                let mut predictive_decoder = crate::mesh_edgebreaker_traversal_predictive_decoder::MeshEdgebreakerTraversalPredictiveDecoder::new(
                    start_face_decoder,
                    has_start_face_bits,
                    topology_split_data.to_vec(),
                    start_face_bits_legacy.take(),
                    legacy_direct_symbol_bits.take(),
                    prediction_decoder,
                    max_num_vertices,
                );

                let nv = connectivity_decoder.decode_connectivity(
                    actual_num_symbols as i32,
                    &mut predictive_decoder,
                    remove_invalid_vertices,
                )? as usize;

                has_start_face_bits_flag = predictive_decoder.has_start_face_bits;
                start_face_decoder_opt = Some(predictive_decoder.start_face_decoder);
                processed_connectivity_corners = predictive_decoder.processed_connectivity_corners;
                nv
            }
        } else if self.traversal_decoder_type == 2 {
            // Valence mode
            #[cfg(not(feature = "edgebreaker_valence_decode"))]
            {
                return Err(DracoError::general(
                    "Edgebreaker valence traversal decode support is disabled".to_string(),
                ));
            }
            #[cfg(feature = "edgebreaker_valence_decode")]
            {
                // For valence traversal, the buffer order is:
                // 1. Start face bits (already decoded above)
                // 2. Attribute seam decoders (need to skip past their size prefix to read context symbols)
                // 3. Context symbols
                //
                // Start attribute seam decoders to position buffer past them
                for _ in 0..num_attribute_data {
                    let mut seam_decoder = RAnsBitDecoder::new();
                    if !seam_decoder.start_decoding(in_buffer) {
                        return Err(DracoError::general(
                            "Failed to start attribute seam decoding for valence".to_string(),
                        ));
                    }
                    legacy_seam_decoders.push(seam_decoder);
                }

                let mut valence_decoder = crate::mesh_edgebreaker_traversal_valence_decoder::MeshEdgebreakerTraversalValenceDecoder::new(
                start_face_decoder,
                has_start_face_bits,
                topology_split_data.to_vec(),
                start_face_bits_legacy.take(),
                legacy_direct_symbol_bits.take(),
            );
                // Initialize contexts by reading counts/symbol arrays from the buffer
                if !valence_decoder.init_from_buffer(
                    in_buffer,
                    max_num_vertices,
                    bitstream_version,
                    actual_num_symbols,
                ) {
                    return Err(DracoError::general(
                        "Failed to init valence traversal decoder".to_string(),
                    ));
                }

                let nv = connectivity_decoder.decode_connectivity(
                    actual_num_symbols as i32,
                    &mut valence_decoder,
                    remove_invalid_vertices,
                )? as usize;

                // Don't end seam decoders yet - we need to decode from them after corner table is built

                // Extract state we need after the decoder is consumed
                has_start_face_bits_flag = valence_decoder.has_start_face_bits;
                start_face_decoder_opt = Some(valence_decoder.start_face_decoder);
                processed_connectivity_corners = valence_decoder.processed_connectivity_corners;
                nv
            }
        } else {
            let mut traversal_decoder = InternalTraversalDecoder::new(
                symbols,
                topology_split_data,
                start_face_decoder,
                has_start_face_bits,
                start_face_bits_legacy.take(),
                max_num_vertices,
            );

            let nv = connectivity_decoder.decode_connectivity(
                actual_num_symbols as i32,
                &mut traversal_decoder,
                remove_invalid_vertices,
            )? as usize;

            has_start_face_bits_flag = traversal_decoder.has_start_face_bits;
            start_face_decoder_opt = Some(traversal_decoder.start_face_decoder);
            processed_connectivity_corners = traversal_decoder.processed_connectivity_corners;
            nv
        };

        if has_start_face_bits_flag {
            if let Some(mut sfd) = start_face_decoder_opt {
                sfd.end_decoding();
            }
        }

        // Reverse the connectivity corner order to match the encoder-side
        // reversal applied before attribute sequencing.
        let mut processed = processed_connectivity_corners;
        processed.reverse();
        self.processed_connectivity_corners = processed;

        // Store the corner table and truncate to the actual vertex count
        let mut ct = connectivity_decoder.corner_table;
        ct.vertex_corners.truncate(num_vertices);
        // Size the mesh from what was decoded rather than from what the header
        // claimed, now that the two are known to agree.
        mesh.try_set_num_faces(ct.num_faces())?;
        self.corner_table = Some(ct);
        connectivity_decoder.is_vert_hole.truncate(num_vertices);
        self.is_vert_hole = connectivity_decoder.is_vert_hole;

        // Initialize vertex_to_corner_map
        self.vertex_to_corner_map = vec![u32::MAX; num_vertices];
        if let Some(ct) = &self.corner_table {
            for v in 0..num_vertices {
                let corner = ct.left_most_corner(VertexIndex(v as u32));
                if corner != crate::geometry_indices::INVALID_CORNER_INDEX {
                    self.vertex_to_corner_map[v] = corner.0;
                }
            }
        }

        self.assign_points_to_corners(mesh)?;

        // Decode attribute seams.
        // For valence mode, we already started the seam decoders before reading context symbols
        // to properly position the buffer. Now we need to decode from them.
        self.attribute_seam_corners.clear();

        let uses_legacy_attribute_connectivity = bitstream_version < 0x0201;

        if self.traversal_decoder_type == 1 || self.traversal_decoder_type == 2 {
            // Valence/predictive mode - use the seam decoders we already started
            #[cfg(not(any(
                feature = "edgebreaker_valence_decode",
                feature = "legacy_bitstream_decode"
            )))]
            {
                return Err(DracoError::general(
                    "Edgebreaker valence traversal decode support is disabled".to_string(),
                ));
            }
            #[cfg(any(
                feature = "edgebreaker_valence_decode",
                feature = "legacy_bitstream_decode"
            ))]
            for mut seam_decoder in legacy_seam_decoders.into_iter() {
                let mut seam_corners = Vec::new();
                if let Some(ct) = &self.corner_table {
                    for f in 0..mesh.num_faces() {
                        for k in 0..3 {
                            let c = (f * 3 + k) as u32;
                            let opp = ct.opposite(CornerIndex(c));
                            if opp != crate::geometry_indices::INVALID_CORNER_INDEX {
                                let opp_face = (opp.0 / 3) as usize;
                                if uses_legacy_attribute_connectivity {
                                    if seam_decoder.decode_next_bit() {
                                        seam_corners.push(c);
                                    }
                                } else if f < opp_face && seam_decoder.decode_next_bit() {
                                    seam_corners.push(c);
                                }
                            } else {
                                seam_corners.push(c);
                            }
                        }
                    }
                }
                seam_decoder.end_decoding();
                self.attribute_seam_corners.push(seam_corners);
            }
        } else {
            // Non-valence mode - start seam decoders from buffer now
            for _ in 0..num_attribute_data {
                let mut seam_corners = Vec::new();
                let mut seam_decoder = RAnsBitDecoder::new();
                if !seam_decoder.start_decoding(in_buffer) {
                    return Err(DracoError::general(
                        "Failed to start seam decoding".to_string(),
                    ));
                }

                if let Some(ct) = &self.corner_table {
                    for f in 0..mesh.num_faces() {
                        for k in 0..3 {
                            let c = (f * 3 + k) as u32;
                            let opp = ct.opposite(CornerIndex(c));
                            if opp != crate::geometry_indices::INVALID_CORNER_INDEX {
                                let opp_face = (opp.0 / 3) as usize;
                                if uses_legacy_attribute_connectivity {
                                    if seam_decoder.decode_next_bit() {
                                        seam_corners.push(c);
                                    }
                                } else if f < opp_face && seam_decoder.decode_next_bit() {
                                    seam_corners.push(c);
                                }
                            } else {
                                seam_corners.push(c);
                            }
                        }
                    }
                }
                seam_decoder.end_decoding();
                self.attribute_seam_corners.push(seam_corners);
            }
        }

        Ok(mesh.num_faces())
    }

    pub fn decode_symbol_stream(
        in_buffer: &mut DecoderBuffer,
        num_symbols: usize,
    ) -> Result<Vec<u32>, DracoError> {
        if num_symbols == 0 {
            return Ok(Vec::new());
        }

        // Traversal symbols are stored as a size-prefixed bit sequence.
        in_buffer.start_bit_decoding(true).map_err(|_| {
            DracoError::general("Failed to start traversal symbol bit decoding".to_string())
        })?;

        // Symbols are stored as a raw bit sequence (>=1 bit each), so the count
        // cannot exceed the remaining bit budget. Cap the pre-reservation so a
        // malformed count cannot trigger a multi-gigabyte allocation.
        let mut symbols =
            Vec::with_capacity(num_symbols.min(in_buffer.remaining_size().saturating_mul(8)));
        for _ in 0..num_symbols {
            let first_bit = in_buffer
                .decode_least_significant_bits32(1)
                .map_err(|_| DracoError::general("Failed to read traversal symbol".to_string()))?;
            let topology = if first_bit == 0 {
                0u32
            } else {
                let suffix = in_buffer.decode_least_significant_bits32(2).map_err(|_| {
                    DracoError::general("Failed to read traversal symbol suffix".to_string())
                })?;
                1u32 | (suffix << 1)
            };
            symbols.push(Self::topology_bit_pattern_to_symbol_id(topology)?);
        }

        // Skip to the end of the traversal symbol bit sequence so subsequent data
        // (start faces, seams) is aligned.
        in_buffer.end_bit_decoding();

        Ok(symbols)
    }

    fn assign_points_to_corners(&mut self, mesh: &mut Mesh) -> Result<(), DracoError> {
        // Matches C++ MeshEdgebreakerDecoderImpl::AssignPointsToCorners
        let corner_table = self.corner_table.as_ref().ok_or(DracoError::general(
            "Corner table not initialized".to_string(),
        ))?;

        // Reject an inconsistent corner table before the DFS below indexes the
        // per-vertex / per-face arrays by table-derived ids.
        if !corner_table.is_index_consistent() {
            return Err(DracoError::general(
                "Inconsistent corner table for attribute traversal".to_string(),
            ));
        }

        let num_vertices = corner_table.num_vertices();
        let num_faces = corner_table.num_faces();

        // If there are no attribute seams, the vertex indices from corner table
        // correspond directly to point IDs. However, they must be visited in
        // discovery order to match the attribute data stream.
        // Discovery order follows the symbol traversal: {Next, Prev, Corner} for each face.

        let mut point_ids = vec![PointIndex(u32::MAX); num_vertices];
        let mut data_to_corner_map = Vec::with_capacity(num_vertices);
        let mut visited_vertices = vec![false; num_vertices];
        let mut visited_faces = vec![false; num_faces];
        let mut next_point_id = 0;

        // DFS logic matching C++ DepthFirstTraverser::TraverseFromCorner exactly.
        let traverse_from_corner = |start_corner: CornerIndex,
                                    point_ids: &mut [PointIndex],
                                    data_to_corner_map: &mut Vec<u32>,
                                    visited_vertices: &mut [bool],
                                    visited_faces: &mut [bool],
                                    next_point_id: &mut u32| {
            let start_face = corner_table.face(start_corner);
            if start_face == crate::geometry_indices::INVALID_FACE_INDEX
                || visited_faces[start_face.0 as usize]
            {
                return;
            }

            let mut corner_stack = vec![start_corner];

            // Pre-visit next and prev vertices (matching C++ exactly - NOT the tip vertex)
            let next_c = corner_table.next(start_corner);
            let prev_c = corner_table.previous(start_corner);
            let next_vert = corner_table.vertex(next_c);
            let prev_vert = corner_table.vertex(prev_c);

            if next_vert == crate::geometry_indices::INVALID_VERTEX_INDEX
                || prev_vert == crate::geometry_indices::INVALID_VERTEX_INDEX
            {
                return;
            }

            // Visit next vertex
            if !visited_vertices[next_vert.0 as usize] {
                visited_vertices[next_vert.0 as usize] = true;
                point_ids[next_vert.0 as usize] = PointIndex(*next_point_id);
                *next_point_id += 1;
                data_to_corner_map.push(next_c.0);
            }
            // Visit prev vertex
            if !visited_vertices[prev_vert.0 as usize] {
                visited_vertices[prev_vert.0 as usize] = true;
                point_ids[prev_vert.0 as usize] = PointIndex(*next_point_id);
                *next_point_id += 1;
                data_to_corner_map.push(prev_c.0);
            }

            // Main traversal loop (matching C++ exactly)
            while let Some(corner_id) = corner_stack.pop() {
                let mut corner_id = corner_id;
                let mut face_id = corner_table.face(corner_id);

                // Check if face already visited (C++ does this at loop start)
                if corner_id == crate::geometry_indices::INVALID_CORNER_INDEX
                    || visited_faces[face_id.0 as usize]
                {
                    continue;
                }

                loop {
                    visited_faces[face_id.0 as usize] = true;

                    let vert_id = corner_table.vertex(corner_id);
                    if vert_id == crate::geometry_indices::INVALID_VERTEX_INDEX {
                        break;
                    }

                    if !visited_vertices[vert_id.0 as usize] {
                        // C++ checks IsOnBoundary: SwingLeft(LeftMostCorner(v)) == kInvalidCornerIndex
                        let lmc = corner_table.left_most_corner(vert_id);
                        let on_boundary = lmc == crate::geometry_indices::INVALID_CORNER_INDEX
                            || corner_table.swing_left(lmc)
                                == crate::geometry_indices::INVALID_CORNER_INDEX;
                        visited_vertices[vert_id.0 as usize] = true;
                        point_ids[vert_id.0 as usize] = PointIndex(*next_point_id);
                        *next_point_id += 1;
                        data_to_corner_map.push(corner_id.0);

                        if !on_boundary {
                            // Move to right corner and continue (C++ GetRightCorner = Opposite(Next))
                            corner_id = corner_table.right_corner(corner_id);
                            if corner_id == crate::geometry_indices::INVALID_CORNER_INDEX {
                                break;
                            }
                            face_id = corner_table.face(corner_id);
                            continue;
                        }
                    }

                    // Vertex already visited or on boundary - check neighbors
                    let right_corner_id = corner_table.right_corner(corner_id);
                    let left_corner_id = corner_table.left_corner(corner_id);

                    let right_face_id =
                        if right_corner_id == crate::geometry_indices::INVALID_CORNER_INDEX {
                            crate::geometry_indices::INVALID_FACE_INDEX
                        } else {
                            corner_table.face(right_corner_id)
                        };
                    let left_face_id =
                        if left_corner_id == crate::geometry_indices::INVALID_CORNER_INDEX {
                            crate::geometry_indices::INVALID_FACE_INDEX
                        } else {
                            corner_table.face(left_corner_id)
                        };

                    let right_visited = right_face_id
                        == crate::geometry_indices::INVALID_FACE_INDEX
                        || visited_faces[right_face_id.0 as usize];
                    let left_visited = left_face_id == crate::geometry_indices::INVALID_FACE_INDEX
                        || visited_faces[left_face_id.0 as usize];

                    if right_visited {
                        if left_visited {
                            // Both visited - break from inner loop
                            break;
                        } else {
                            // Only left unvisited - go to left
                            corner_id = left_corner_id;
                            face_id = left_face_id;
                        }
                    } else if left_visited {
                        // Only right unvisited - go to right
                        corner_id = right_corner_id;
                        face_id = right_face_id;
                    } else {
                        // Both unvisited - split traversal (C++ behavior)
                        // Replace top of stack with left (processed second)
                        // Push right (processed first - LIFO)
                        // Note: we already popped, so modify logic:
                        // Push left first, then right, then break
                        corner_stack.push(left_corner_id);
                        corner_stack.push(right_corner_id);
                        break;
                    }
                }
            }
        };

        // The C++ decoder ALWAYS uses sequential face order for attribute traversal.
        // The processed_connectivity_corners_ collected during symbol decoding
        // are only used for connectivity reconstruction, NOT for attribute traversal.
        // This matches C++ MeshTraversalSequencer::GenerateSequenceInternal which
        // uses sequential faces when corner_order_ is not set (decoder mode).
        //
        // The encoder and decoder have DIFFERENT corner tables - the encoder uses the
        // original mesh's corner table, while the decoder reconstructs one from symbols.
        // For roundtrip to work, the attribute data must be encoded/decoded in an order
        // that can be reconstructed independently by both encoder and decoder.
        //
        // The key insight is that both encoder and decoder do DFS traversal, but the
        // traversal visits corners and maps them to points via the MESH's face data.
        // Since the decoder's mesh faces are set from its reconstructed corner table,
        // the point assignments will match when using sequential face order.
        //
        // Use sequential face order, matching C++ decoder behavior.
        for f in 0..num_faces {
            if !visited_faces[f] {
                traverse_from_corner(
                    CornerIndex((f * 3) as u32),
                    &mut point_ids,
                    &mut data_to_corner_map,
                    &mut visited_vertices,
                    &mut visited_faces,
                    &mut next_point_id,
                );
            }
        }

        // Handle isolated vertices.
        for v in 0..num_vertices {
            if !visited_vertices[v] {
                point_ids[v] = PointIndex(next_point_id);
                next_point_id += 1;
                let c = corner_table.left_most_corner(VertexIndex(v as u32));
                data_to_corner_map.push(if c != crate::geometry_indices::INVALID_CORNER_INDEX {
                    c.0
                } else {
                    0
                });
            }
        }

        // Map corner table vertices to mesh face point indices.
        // In C++: face[c] = corner_table_->Vertex(start_corner + c).value()
        // Mesh point index == corner table vertex index (not data_id!).
        for f in 0..num_faces {
            let fid = FaceIndex(f as u32);
            let c0 = CornerIndex(f as u32 * 3);
            let v0 = corner_table.vertex(c0);
            let v1 = corner_table.vertex(corner_table.next(c0));
            let v2 = corner_table.vertex(corner_table.previous(c0));

            // Use vertex indices directly as point indices (matching C++)
            mesh.set_face(fid, [PointIndex(v0.0), PointIndex(v1.0), PointIndex(v2.0)]);
        }
        mesh.set_num_points(num_vertices);
        self.data_to_corner_map = Some(data_to_corner_map);

        Ok(())
    }
}

struct InternalTraversalDecoder<'a> {
    symbols: &'a [u32],
    symbol_index: usize,
    topology_split_data: &'a [TopologySplitEventData],
    /// Index pointing to the next event to check (counts down from len to 0).
    /// Unlike C++ which pops from the back, we track position from the end.
    split_event_remaining: usize,
    start_face_decoder: RAnsBitDecoder<'a>,
    has_start_face_bits: bool,
    /// For v < 2.2: pre-read start face configuration bits (raw bit buffer).
    /// When present, these are used instead of the RAnsBitDecoder.
    start_face_bits_legacy: Option<Vec<bool>>,
    start_face_bits_legacy_index: usize,
    processed_connectivity_corners: Vec<u32>,
}

impl<'a> InternalTraversalDecoder<'a> {
    fn new(
        symbols: &'a [u32],
        topology_split_data: &'a [TopologySplitEventData],
        start_face_decoder: RAnsBitDecoder<'a>,
        has_start_face_bits: bool,
        start_face_bits_legacy: Option<Vec<bool>>,
        _max_num_vertices: usize,
    ) -> Self {
        Self {
            symbols,
            symbol_index: 0,
            topology_split_data,
            split_event_remaining: topology_split_data.len(),
            start_face_decoder,
            has_start_face_bits,
            start_face_bits_legacy,
            start_face_bits_legacy_index: 0,
            processed_connectivity_corners: Vec::new(),
        }
    }
}

impl<'a> EdgebreakerTraversalDecoder for InternalTraversalDecoder<'a> {
    fn decode_symbol(&mut self) -> Result<u32, DracoError> {
        let val = *self
            .symbols
            .get(self.symbol_index)
            .ok_or_else(|| DracoError::general("Traversal symbol stream exhausted".to_string()))?;
        self.symbol_index += 1;
        Ok(val)
    }

    fn decode_start_face_configuration(&mut self) -> bool {
        // For v < 2.2: use pre-read raw bit buffer
        if let Some(ref bits) = self.start_face_bits_legacy {
            let idx = self.start_face_bits_legacy_index;
            self.start_face_bits_legacy_index += 1;
            return bits.get(idx).copied().unwrap_or(true);
        }
        // For v >= 2.2: use RAnsBitDecoder
        if self.has_start_face_bits {
            self.start_face_decoder.decode_next_bit()
        } else {
            true
        }
    }

    fn merge_vertices(&mut self, _p: VertexIndex, _n: VertexIndex) {
        // Points are logically merged in CT.
    }

    fn is_topology_split(&mut self, encoder_symbol_id: i32) -> Option<(EdgeFaceName, i32)> {
        // C++ checks from the back of the list (highest source_symbol_id first) and pops.
        // Events are sorted in ascending order by source_symbol_id.
        // We use split_event_remaining to track how many events are left (counting from end).
        if self.split_event_remaining > 0 {
            let event = &self.topology_split_data[self.split_event_remaining - 1];
            if event.source_symbol_id == encoder_symbol_id as u32 {
                // Found a match - consume this event (like C++ pop_back)
                self.split_event_remaining -= 1;
                return Some((event.source_edge, event.split_symbol_id as i32));
            } else if event.source_symbol_id > encoder_symbol_id as u32 {
                // This event's source_symbol_id is higher than what we're looking for.
                // Since encoder_symbol_id decreases, and we haven't matched, something's wrong.
                // Return invalid to signal an error (matching C++ behavior).
                return Some((EdgeFaceName::LeftFaceEdge, -1));
            }
            // event.source_symbol_id < encoder_symbol_id, we haven't reached this event yet
        }
        None
    }

    fn on_vertex_created(&mut self, _vertex: VertexIndex, _symbol_id: i32, _corner_index: i32) {
        // Connectivity reconstruction vertex creation - not attribute traversal order.
        // Don't log to test_event_log as this is a different phase than encoder's DFS traversal.
    }

    fn on_vertices_swapped(&mut self, _v1: VertexIndex, _v2: VertexIndex) {}

    fn on_start_face_decoded(&mut self, corner: CornerIndex) {
        // This corresponds to decoder init-corners / start-face handling, not the
        // per-face traversal order used for attribute sequencing.
        let _ = corner;
    }

    fn on_split_symbol_decoded(&mut self, corner: CornerIndex) {
        // Split symbol event bookkeeping is separate from the per-face traversal order.
        let _ = corner;
    }

    fn new_active_corner_reached(&mut self, corner: CornerIndex, _corner_table: &CornerTable) {
        // Matches C++ MeshEdgebreakerDecoderImpl::processed_connectivity_corners_:
        // store corners in the order they were visited during connectivity decoding.
        self.processed_connectivity_corners.push(corner.0);
    }
}

#[cfg(all(test, not(feature = "legacy_bitstream_decode")))]
mod tests {
    use super::*;

    // Predictive (type-1) traversal is supported when legacy_bitstream_decode is
    // enabled; the explicit rejection only applies without that feature.
    #[test]
    fn predictive_traversal_type_is_rejected_explicitly() {
        let mut buffer = DecoderBuffer::new(&[1]);
        let mut decoder = MeshEdgebreakerDecoder::new();
        let mut mesh = Mesh::new();

        let err = decoder
            .decode_connectivity(&mut buffer, &mut mesh)
            .expect_err("predictive traversal should be rejected before payload decode");

        assert_eq!(
            err,
            DracoError::unsupported_feature(
                "Edgebreaker predictive traversal decode is not supported".to_string()
            )
        );
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;

    fn append_varint(bytes: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    #[test]
    fn legacy_topology_split_count_cannot_exceed_face_count() {
        let bytes = 2u32.to_le_bytes();
        let mut buffer = DecoderBuffer::new(&bytes);
        buffer.set_version(1, 1);

        let err =
            MeshEdgebreakerDecoder::decode_hole_and_topology_split_events(&mut buffer, 0x0101, 1)
                .expect_err("split events must be bounded by face count");

        assert_eq!(
            err,
            DracoError::general("Topology split count exceeds face count".to_string())
        );
    }

    #[test]
    fn inline_topology_split_count_cannot_exceed_face_count() {
        let mut bytes = Vec::new();
        append_varint(&mut bytes, 2);
        let mut buffer = DecoderBuffer::new(&bytes);
        buffer.set_version(2, 2);

        let err =
            MeshEdgebreakerDecoder::decode_topology_split_events_inline(&mut buffer, 0x0202, 1)
                .expect_err("split events must be bounded by face count");

        assert_eq!(
            err,
            DracoError::general("Topology split count exceeds face count".to_string())
        );
    }
}
