//! Binary FBX container encoder: a tree of [`FbxNode`] in, bytes out.
//!
//! The mirror of [`crate::fbx_container`], and the only place in the writer
//! that knows about bytes at all. Everything above it decides *what* records
//! the document contains; this decides how a record is spelled.
//!
//! Two things here cannot be expressed as a node and never will be. A node
//! record stores the offset of its own end, which is only known once its
//! children are written, so encoding seeks backwards to patch three header
//! fields -- that is the whole of the `Seek` requirement. And the file ends
//! with a footer of fixed bytes whose padding is measured from the stream
//! position, which is not part of the tree in any sense.

use std::io::{self, Seek, SeekFrom, Write};

use crate::fbx_ascii_syntax::FBX_VERSION;
use crate::fbx_node::{FbxNode, FbxProperty};

/// Statistics about FBX array compression performed by the binary writer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FbxWriteStats {
    /// Number of arrays that were actually stored with zlib encoding.
    pub compressed_arrays: usize,
    /// Size of those arrays before zlib compression.
    pub compressed_raw_bytes: usize,
    /// Size of those arrays after zlib compression.
    pub compressed_stored_bytes: usize,
}

impl std::ops::AddAssign for FbxWriteStats {
    fn add_assign(&mut self, other: Self) {
        self.compressed_arrays += other.compressed_arrays;
        self.compressed_raw_bytes += other.compressed_raw_bytes;
        self.compressed_stored_bytes += other.compressed_stored_bytes;
    }
}

/// FBX file magic: "Kaydara FBX Binary  \0"
pub(crate) const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// Size of a null record for 64-bit FBX
const NULL_RECORD_SIZE_64: usize = 25;

/// Size of a null record for 32-bit FBX
const NULL_RECORD_SIZE_32: usize = 13;

/// Marks the start of the binary footer, right after the root terminator.
const FBX_FOOTER_ID: [u8; 16] = [
    0xFA, 0xBC, 0xAB, 0x09, 0xD0, 0xC8, 0xD4, 0x66, 0xB1, 0x76, 0xFB, 0x83, 0x1C, 0xF7, 0x26, 0x7E,
];

/// Closes the file, after the repeated version and 120 bytes of padding.
const FBX_FOOTER_MAGIC: [u8; 16] = [
    0xF8, 0x5A, 0x8C, 0x6A, 0xDE, 0xF5, 0xD9, 0x7E, 0xEC, 0xE9, 0x0C, 0xE3, 0x75, 0x8F, 0x29, 0x0B,
];

/// Array encoding choices for one document.
///
/// Belongs to the encoder rather than to the document: whether an array is
/// deflated is a property of how the tree is spelled, not of what it holds,
/// and [`FbxProperty`] accordingly has nowhere to record it.
pub(crate) struct WriterOptions {
    pub(crate) compress: bool,
    pub(crate) compression_threshold: usize,
    pub(crate) compression_level: u8,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            compress: false,
            compression_threshold: 128,
            compression_level: 2,
        }
    }
}

/// Writes one node and everything under it.
///
/// The bridge between the tree and the byte writer: every property variant is
/// spelled here, including the four the rest of the writer never constructs
/// (`Bool`, `U8`, scalar `F32`, `BoolArray`). Leaving those unhandled would
/// make the tree a type the encoder cannot fully accept, which is the thing
/// this module exists to rule out.
pub(crate) fn encode_node<W: Write + Seek>(
    writer: &mut W,
    node: &FbxNode,
    is_64: bool,
    options: &WriterOptions,
) -> io::Result<FbxWriteStats> {
    let mut record = NodeWriter::start(writer, &node.name, is_64)?;
    let mut stats = FbxWriteStats::default();
    for property in &node.properties {
        match property {
            FbxProperty::Bool(value) => record.write_property_bool(*value)?,
            FbxProperty::U8(value) => record.write_property_u8(*value)?,
            FbxProperty::I16(value) => record.write_property_i16(*value)?,
            FbxProperty::I32(value) => record.write_property_i32(*value)?,
            FbxProperty::I64(value) => record.write_property_i64(*value)?,
            FbxProperty::F32(value) => record.write_property_f32(*value)?,
            FbxProperty::F64(value) => record.write_property_f64(*value)?,
            FbxProperty::String(value) => record.write_property_string(value)?,
            FbxProperty::Raw(value) => record.write_property_raw(value)?,
            FbxProperty::BoolArray(values) => {
                stats += record.write_property_bool_array(values, options)?
            }
            FbxProperty::I32Array(values) => {
                stats += record.write_property_i32_array(values, options)?
            }
            FbxProperty::I64Array(values) => {
                stats += record.write_property_i64_array(values, options)?
            }
            FbxProperty::F32Array(values) => {
                stats += record.write_property_f32_array(values, options)?
            }
            FbxProperty::F64Array(values) => {
                stats += record.write_property_f64_array(values, options)?
            }
        }
    }
    if node.children.is_empty() {
        record.finish()?;
    } else {
        let mut child_stats = FbxWriteStats::default();
        record.finish_with_children(|w| {
            for child in &node.children {
                child_stats += encode_node(w, child, is_64, options)?;
            }
            Ok(())
        })?;
        stats += child_stats;
    }
    Ok(stats)
}

/// Helper struct for writing FBX nodes.
struct NodeWriter<'a, W: Write + Seek> {
    writer: &'a mut W,
    start_pos: u64,
    properties_start: u64,
    num_properties: u64,
    is_64: bool,
}

impl<'a, W: Write + Seek> NodeWriter<'a, W> {
    fn start(writer: &'a mut W, name: &str, is_64: bool) -> io::Result<Self> {
        let start_pos = writer.stream_position()?;

        // Write placeholder for end offset, num properties, property list len
        let header_size = if is_64 { 24 } else { 12 }; // 3 * 8 or 3 * 4
        writer.write_all(&vec![0u8; header_size])?;

        // Write name length and name
        writer.write_all(&[name.len() as u8])?;
        writer.write_all(name.as_bytes())?;

        let properties_start = writer.stream_position()?;

        Ok(Self {
            writer,
            start_pos,
            properties_start,
            num_properties: 0,
            is_64,
        })
    }

    /// `C`, the one-byte boolean. Written as `b'T'`/`b'Y'` for true and
    /// `b'F'`/`b'N'` for false by different exporters; this uses 1 and 0,
    /// which every reader including ours accepts.
    fn write_property_bool(&mut self, value: bool) -> io::Result<()> {
        self.writer.write_all(b"C")?;
        self.writer.write_all(&[u8::from(value)])?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_u8(&mut self, value: u8) -> io::Result<()> {
        self.writer.write_all(b"Z")?;
        self.writer.write_all(&[value])?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_i16(&mut self, value: i16) -> io::Result<()> {
        self.writer.write_all(b"Y")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_i32(&mut self, value: i32) -> io::Result<()> {
        self.writer.write_all(b"I")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_i64(&mut self, value: i64) -> io::Result<()> {
        self.writer.write_all(b"L")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_f32(&mut self, value: f32) -> io::Result<()> {
        self.writer.write_all(b"F")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_f64(&mut self, value: f64) -> io::Result<()> {
        self.writer.write_all(b"D")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_string(&mut self, value: &str) -> io::Result<()> {
        self.writer.write_all(b"S")?;
        self.writer.write_all(&(value.len() as u32).to_le_bytes())?;
        self.writer.write_all(value.as_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_bool_array(
        &mut self,
        values: &[bool],
        options: &WriterOptions,
    ) -> io::Result<FbxWriteStats> {
        self.write_array_property(b'b', values, options, |v, out| out.push(u8::from(*v)))
    }

    fn write_property_f64_array(
        &mut self,
        values: &[f64],
        options: &WriterOptions,
    ) -> io::Result<FbxWriteStats> {
        self.write_array_property(b'd', values, options, |v, out| {
            out.extend_from_slice(&v.to_le_bytes())
        })
    }

    fn write_property_i32_array(
        &mut self,
        values: &[i32],
        options: &WriterOptions,
    ) -> io::Result<FbxWriteStats> {
        self.write_array_property(b'i', values, options, |v, out| {
            out.extend_from_slice(&v.to_le_bytes())
        })
    }

    fn write_property_i64_array(
        &mut self,
        values: &[i64],
        options: &WriterOptions,
    ) -> io::Result<FbxWriteStats> {
        self.write_array_property(b'l', values, options, |v, out| {
            out.extend_from_slice(&v.to_le_bytes())
        })
    }

    fn write_property_f32_array(
        &mut self,
        values: &[f32],
        options: &WriterOptions,
    ) -> io::Result<FbxWriteStats> {
        self.write_array_property(b'f', values, options, |v, out| {
            out.extend_from_slice(&v.to_le_bytes())
        })
    }

    fn write_property_raw(&mut self, data: &[u8]) -> io::Result<()> {
        self.writer.write_all(b"R")?;
        self.writer.write_all(&(data.len() as u32).to_le_bytes())?;
        self.writer.write_all(data)?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_array_property<T, F>(
        &mut self,
        type_code: u8,
        values: &[T],
        options: &WriterOptions,
        write_element: F,
    ) -> io::Result<FbxWriteStats>
    where
        F: Fn(&T, &mut Vec<u8>),
    {
        self.writer.write_all(&[type_code])?;
        self.writer
            .write_all(&(values.len() as u32).to_le_bytes())?;

        // Serialized directly into a pre-sized buffer rather than building a
        // fresh heap Vec per element and flattening them: for a 263k-vertex
        // mesh's position/normal/uv/color/index arrays combined, that used to
        // be several million short-lived allocations, most of what this
        // function cost independent of compression. `size_of::<T>()` is the
        // exact serialized width for every type this is called with --
        // bool, f32, f64, i32, i64 -- so the buffer never has to regrow.
        let mut raw_data = Vec::with_capacity(values.len() * std::mem::size_of::<T>());
        for value in values {
            write_element(value, &mut raw_data);
        }
        let raw_size = raw_data.len();

        // Decide whether to compress
        let should_compress = options.compress && raw_size >= options.compression_threshold;

        #[cfg(feature = "compression")]
        if should_compress {
            use miniz_oxide::deflate::compress_to_vec_zlib;
            // `options.compression_level` defaults to 2 (see
            // FbxWriter::with_compression_level) rather than the conventional
            // 6. Mesh attribute arrays are the near-noise deflate is bad at: a
            // 263k-vertex benchmark mesh (positions/normals/uvs/colors, on
            // miniz_oxide 0.9.1) measured 1 at 184ms/4.66MB, 2 at
            // 215ms/4.00MB, then 3-9 climbing to 2.57s while size mostly
            // stays flat or worsens (9 lands back at 4.01MB). 2 is the
            // default because it is the last point where a real size drop
            // still costs a small, roughly-linear amount of time; past it the
            // curve steepens for a shrinking, sometimes negative, return.
            // Decoded content is identical at every level, as it has to be:
            // the level only shapes how hard the encoder looks for matches,
            // never what the decompressor reads back.
            let compressed = compress_to_vec_zlib(&raw_data, options.compression_level);

            // Only use compression if it actually saves space
            if compressed.len() < raw_size {
                self.writer.write_all(&1u32.to_le_bytes())?; // encoding = 1 (zlib)
                self.writer
                    .write_all(&(compressed.len() as u32).to_le_bytes())?;
                self.writer.write_all(&compressed)?;
                self.num_properties += 1;
                return Ok(FbxWriteStats {
                    compressed_arrays: 1,
                    compressed_raw_bytes: raw_size,
                    compressed_stored_bytes: compressed.len(),
                });
            }
        }

        // Write uncompressed (or if compression didn't help)
        #[cfg(not(feature = "compression"))]
        let _ = should_compress; // Suppress unused warning

        self.writer.write_all(&0u32.to_le_bytes())?; // encoding = 0 (uncompressed)
        self.writer.write_all(&(raw_size as u32).to_le_bytes())?;
        self.writer.write_all(&raw_data)?;
        self.num_properties += 1;
        Ok(FbxWriteStats::default())
    }

    fn finish(self) -> io::Result<()> {
        // Write null record to end children section
        write_null_record(self.writer, self.is_64)?;
        self.finalize_header()
    }

    fn finish_with_children<F>(self, write_children: F) -> io::Result<()>
    where
        F: FnOnce(&mut W) -> io::Result<()>,
    {
        let properties_end = self.writer.stream_position()?;
        let property_list_len = properties_end - self.properties_start;

        // Write children
        write_children(self.writer)?;

        // Write null record to end children
        write_null_record(self.writer, self.is_64)?;

        let end_pos = self.writer.stream_position()?;

        // Write the header
        self.writer.seek(SeekFrom::Start(self.start_pos))?;
        if self.is_64 {
            self.writer.write_all(&end_pos.to_le_bytes())?;
            self.writer.write_all(&self.num_properties.to_le_bytes())?;
            self.writer.write_all(&property_list_len.to_le_bytes())?;
        } else {
            self.writer.write_all(&(end_pos as u32).to_le_bytes())?;
            self.writer
                .write_all(&(self.num_properties as u32).to_le_bytes())?;
            self.writer
                .write_all(&(property_list_len as u32).to_le_bytes())?;
        }

        // Seek back to end
        self.writer.seek(SeekFrom::Start(end_pos))?;
        Ok(())
    }

    fn finalize_header(self) -> io::Result<()> {
        let end_pos = self.writer.stream_position()?;
        let null_size = if self.is_64 {
            NULL_RECORD_SIZE_64
        } else {
            NULL_RECORD_SIZE_32
        };
        let property_list_len = if self.num_properties > 0 {
            end_pos - self.properties_start - null_size as u64
        } else {
            0u64
        };

        // Write the header
        self.writer.seek(SeekFrom::Start(self.start_pos))?;
        if self.is_64 {
            self.writer.write_all(&end_pos.to_le_bytes())?;
            self.writer.write_all(&self.num_properties.to_le_bytes())?;
            self.writer.write_all(&property_list_len.to_le_bytes())?;
        } else {
            self.writer.write_all(&(end_pos as u32).to_le_bytes())?;
            self.writer
                .write_all(&(self.num_properties as u32).to_le_bytes())?;
            self.writer
                .write_all(&(property_list_len as u32).to_le_bytes())?;
        }

        // Seek back to end
        self.writer.seek(SeekFrom::Start(end_pos))?;
        Ok(())
    }
}

pub(crate) fn write_null_record<W: Write>(writer: &mut W, is_64: bool) -> io::Result<()> {
    let size = if is_64 {
        NULL_RECORD_SIZE_64
    } else {
        NULL_RECORD_SIZE_32
    };
    writer.write_all(&vec![0u8; size])
}

/// Writes the conventional binary footer.
///
/// The previous implementation emitted 20 zero bytes followed by the first
/// four bytes of the footer id, and stopped there: no repeated version, no
/// closing magic. Nothing complained because neither ufbx nor Blender reads
/// the footer at all, but the output was not a conventional FBX file and the
/// reader's strict mode rejects it.
pub(crate) fn write_footer<W: Write + Seek>(writer: &mut W) -> io::Result<()> {
    writer.write_all(&FBX_FOOTER_ID)?;
    writer.write_all(&[0u8; 4])?;

    // Alignment padding is measured from here and is never empty: a position
    // that is already 16-byte aligned still gets a full 16 bytes.
    let position = writer.stream_position()?;
    let mut padding = (16 - (position % 16)) % 16;
    if padding == 0 {
        padding = 16;
    }
    writer.write_all(&vec![0u8; padding as usize])?;

    writer.write_all(&FBX_VERSION.to_le_bytes())?;
    writer.write_all(&[0u8; 120])?;
    writer.write_all(&FBX_FOOTER_MAGIC)?;
    Ok(())
}

// The decoder is the only thing that can check this encoder, and it lives
// behind the other feature.
#[cfg(all(test, feature = "fbx-reader"))]
mod tests {
    use super::*;
    use crate::fbx_container::FbxMemoryReader;
    use std::io::Cursor;

    /// Encodes `nodes` into a document the reader will accept, and reads it
    /// back into a node tree.
    fn round_trip(nodes: &[FbxNode], options: &WriterOptions) -> Vec<FbxNode> {
        let mut cursor = Cursor::new(Vec::new());
        cursor.write_all(FBX_MAGIC).unwrap();
        cursor.write_all(&[0x1A, 0x00]).unwrap();
        cursor.write_all(&FBX_VERSION.to_le_bytes()).unwrap();
        let is_64 = FBX_VERSION >= 7500;
        for node in nodes {
            encode_node(&mut cursor, node, is_64, options).unwrap();
        }
        write_null_record(&mut cursor, is_64).unwrap();
        write_footer(&mut cursor).unwrap();

        let mut reader = FbxMemoryReader::from_bytes(cursor.into_inner()).unwrap();
        reader.read_nodes().unwrap()
    }

    /// Every property variant must survive encoding.
    ///
    /// Four of the fourteen -- `Bool`, `U8`, scalar `F32` and `BoolArray` --
    /// are never produced by the document builder, so this is the only thing
    /// that checks how they are spelled. Getting one wrong would be invisible
    /// until some future document needed it.
    #[test]
    fn every_property_variant_survives_a_round_trip() {
        let node = FbxNode {
            name: "Everything".to_string(),
            properties: vec![
                FbxProperty::Bool(true),
                FbxProperty::U8(200),
                FbxProperty::I16(-3),
                FbxProperty::I32(-70_000),
                FbxProperty::I64(-5_000_000_000),
                FbxProperty::F32(0.5),
                FbxProperty::F64(-0.25),
                FbxProperty::String("name\u{0}\u{1}Class".to_string()),
                FbxProperty::Raw(vec![1, 2, 3]),
                FbxProperty::BoolArray(vec![true, false, true]),
                FbxProperty::I32Array(vec![1, -2, 3]),
                FbxProperty::I64Array(vec![4, -5]),
                FbxProperty::F32Array(vec![1.5, -2.5]),
                FbxProperty::F64Array(vec![3.5, -4.5]),
            ],
            children: Vec::new(),
        };

        let read = round_trip(std::slice::from_ref(&node), &WriterOptions::default());
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "Everything");
        assert_eq!(
            format!("{:?}", read[0].properties),
            format!("{:?}", node.properties)
        );
    }

    /// Compressed arrays must decode to the same values.
    ///
    /// The encoder keeps the deflated form only when it came out shorter, so
    /// the payload has to be long enough to actually compress -- a short array
    /// silently takes the uncompressed branch and tests nothing.
    #[test]
    fn a_compressed_array_decodes_to_the_same_values() {
        let values: Vec<f64> = (0..512).map(f64::from).collect();
        let node = FbxNode {
            name: "Vertices".to_string(),
            properties: vec![FbxProperty::F64Array(values.clone())],
            children: Vec::new(),
        };
        let options = WriterOptions {
            compress: true,
            compression_threshold: 128,
            compression_level: 2,
        };

        let read = round_trip(std::slice::from_ref(&node), &options);
        match &read[0].properties[0] {
            FbxProperty::F64Array(decoded) => assert_eq!(decoded, &values),
            other => panic!("expected an f64 array, got {other:?}"),
        }
    }

    /// Children must nest, and the end offset each record stores has to point
    /// past the last of them -- that offset is written by seeking backwards
    /// once the children are done, which is the one thing here that a plain
    /// forward writer could not do.
    #[test]
    fn nested_children_survive_the_backpatched_end_offset() {
        let node = FbxNode {
            name: "Objects".to_string(),
            properties: Vec::new(),
            children: vec![FbxNode {
                name: "Geometry".to_string(),
                properties: vec![FbxProperty::I64(42)],
                children: vec![FbxNode {
                    name: "Vertices".to_string(),
                    properties: vec![FbxProperty::F64Array(vec![1.0, 2.0, 3.0])],
                    children: Vec::new(),
                }],
            }],
        };

        let read = round_trip(std::slice::from_ref(&node), &WriterOptions::default());
        assert_eq!(read[0].children[0].name, "Geometry");
        assert_eq!(read[0].children[0].children[0].name, "Vertices");
        assert_eq!(format!("{:?}", read), format!("{:?}", vec![node]));
    }
}
