//! FBX binary container decoder: bytes in, a tree of [`FbxNode`](crate::fbx_container::FbxNode) out.
//!
//! This layer knows the container and nothing about what the document means.
//! It validates the header, walks node records, decodes property records and
//! their optionally deflated arrays, enforces the [`crate::FbxDecodeLimits`]
//! budget against untrusted input, and checks the footer.
//!
//! Everything above it -- objects, connections, materials, skins, animation --
//! lives in [`crate::fbx_reader`] and consumes only the node tree. That is not
//! an aspiration: [`crate::fbx_ascii`] is a second decoder for the text
//! container, and it feeds the same scene layer without a single change to it.

use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

/// The document tree this decoder produces, re-exported so
/// `draco_io::fbx_container::FbxNode` keeps resolving now that the type is
/// shared with the writer.
pub use crate::fbx_node::{FbxNode, FbxProperty};
use crate::fbx_options::{FbxByteOrder, FbxReadOptions};
use crate::fbx_scene::{push_warning, FbxWarning, FbxWarningCode};
use crate::traits::ReadFromBytes;
use draco_core::mesh::Mesh;

/// FBX file magic: "Kaydara FBX Binary  \0"
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// Byte 21 of the fixed magic, immediately after the NUL terminator.
const FBX_MAGIC_TAIL: u8 = 0x1A;

/// `FixedMagic[22]`, `EndianMarker[1]`, `Version[4]`.
///
/// The layout is fixed, so this offset does not depend on byte order.
const FBX_HEADER_LEN: u64 = 27;

/// Marks the start of the binary footer, right after the root terminator.
const FBX_FOOTER_ID: [u8; 16] = [
    0xFA, 0xBC, 0xAB, 0x09, 0xD0, 0xC8, 0xD4, 0x66, 0xB1, 0x76, 0xFB, 0x83, 0x1C, 0xF7, 0x26, 0x7E,
];

/// Closes the file, after the repeated version and 120 bytes of padding.
const FBX_FOOTER_MAGIC: [u8; 16] = [
    0xF8, 0x5A, 0x8C, 0x6A, 0xDE, 0xF5, 0xD9, 0x7E, 0xEC, 0xE9, 0x0C, 0xE3, 0x75, 0x8F, 0x29, 0x0B,
];

/// Oldest supported version.
///
/// FBX 3000-era files reuse the same magic but lay out `Objects` differently,
/// with pre-7000 multi-value arrays. This reader has never understood them: it
/// used to return an empty scene, which reads as "the file had no meshes"
/// rather than "this file is not supported". Rejecting them says so outright.
const FBX_MIN_VERSION: u32 = 6000;

/// Newest supported version. Beyond this the node record layout is unknown, and
/// guessing a record width from a garbage version corrupts the whole parse.
const FBX_MAX_VERSION: u32 = 8000;

/// FBX reader for binary FBX files.
pub struct FbxReader<R: Read + Seek = BufReader<File>> {
    reader: R,
    version: u32,
    byte_order: FbxByteOrder,
    options: FbxReadOptions,
    /// Total input length, captured once so record offsets can be bounds
    /// checked without trusting the file's own claims.
    file_len: u64,
    budget: DecodeBudget,
    /// Non-fatal container-layout notices raised while reading nodes.
    ///
    /// Merged into [`crate::FbxScene::warnings`] by `read_scene`. Deviations
    /// that are tolerated in lenient mode are reported here rather than
    /// silently accepted.
    warnings: Vec<FbxWarning>,
    /// Nodes parsed from an ASCII document, which has no binary layout to walk.
    ///
    /// Present only for the ASCII container; `read_nodes` serves these instead
    /// of decoding records, so every entry point above it is unaware of which
    /// container it was given.
    ascii_nodes: Option<Vec<FbxNode>>,
}

/// Running totals for the limits that apply to a whole document.
///
/// Reset at the start of every [`FbxReader::read_nodes`] call: that method is
/// re-entrant (both `read_scene` and `read_meshes` call it), and carrying
/// totals across calls would fail the second read of a file the first read
/// accepted.
#[derive(Debug, Clone, Copy, Default)]
struct DecodeBudget {
    nodes: u64,
    array_raw_bytes: u64,
}

/// FBX reader backed by in-memory bytes.
pub type FbxMemoryReader = FbxReader<Cursor<Vec<u8>>>;

impl FbxReader<BufReader<File>> {
    /// Open an FBX file from a path, using default read options.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::open_with_options(path, FbxReadOptions::default())
    }

    /// Open an FBX file from a path with explicit read options.
    pub fn open_with_options<P: AsRef<Path>>(path: P, options: FbxReadOptions) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::new_with_options(reader, options)
    }
}

impl FbxReader<Cursor<Vec<u8>>> {
    /// Create an FBX reader from in-memory bytes, using default read options.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> io::Result<Self> {
        Self::new(Cursor::new(bytes.into()))
    }

    /// Create an FBX reader from in-memory bytes with explicit read options.
    pub fn from_bytes_with_options(
        bytes: impl Into<Vec<u8>>,
        options: FbxReadOptions,
    ) -> io::Result<Self> {
        Self::new_with_options(Cursor::new(bytes.into()), options)
    }

    /// Read all meshes directly from in-memory bytes.
    pub fn read_from_bytes(bytes: &[u8]) -> io::Result<Vec<Mesh>> {
        let mut reader = Self::from_bytes(bytes.to_vec())?;
        reader.read_meshes()
    }
}

impl ReadFromBytes for FbxReader<Cursor<Vec<u8>>> {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        Self::from_bytes(bytes.to_vec())
    }
}

impl<R: Read + Seek> FbxReader<R> {
    /// Create a new FBX reader from a reader, using default read options.
    pub fn new(reader: R) -> io::Result<Self> {
        Self::new_with_options(reader, FbxReadOptions::default())
    }

    /// Create a new FBX reader from a reader with explicit read options.
    ///
    /// The header itself is parsed under `options`, so the options cannot be
    /// changed after construction.
    pub fn new_with_options(mut reader: R, options: FbxReadOptions) -> io::Result<Self> {
        let file_len = reader.seek(SeekFrom::End(0))?;
        reader.rewind()?;

        if file_len > options.limits.max_file_bytes {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "FBX: input is {file_len} bytes, over the {} byte limit",
                    options.limits.max_file_bytes
                ),
            ));
        }
        if file_len < FBX_HEADER_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("FBX: input is {file_len} bytes, shorter than the 27-byte header"),
            ));
        }

        // `FixedMagic[22] EndianMarker[1] Version[4]`, read in one go.
        let mut header = [0u8; FBX_HEADER_LEN as usize];
        reader.read_exact(&mut header)?;

        // The ASCII container has no header to validate, so it is parsed whole
        // here and served from `read_nodes` like any other document. Doing it
        // at construction rather than at each entry point is what lets
        // `read_scene`, `read_meshes` and the `Reader` traits all accept ASCII
        // without knowing it exists.
        if crate::fbx_ascii::is_ascii_fbx(&header) {
            reader.rewind()?;
            let mut text = Vec::with_capacity(file_len as usize);
            reader.read_to_end(&mut text)?;
            let nodes = crate::fbx_ascii::parse_ascii_nodes(&text, &options)?;
            return Ok(Self {
                reader,
                // The ASCII container records its version inside the node
                // tree rather than in a header, and `check_version` has
                // already rejected anything pre-7000.
                version: 7000,
                byte_order: FbxByteOrder::Little,
                options,
                file_len,
                budget: DecodeBudget::default(),
                warnings: Vec::new(),
                ascii_nodes: Some(nodes),
            });
        }

        if &header[..21] != FBX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a valid binary FBX file",
            ));
        }
        if header[21] != FBX_MAGIC_TAIL {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FBX: fixed magic ends with {:#04x}, expected {FBX_MAGIC_TAIL:#04x}",
                    header[21]
                ),
            ));
        }

        // ufbx treats any non-zero marker as big-endian; strict mode accepts
        // only the two documented values.
        let marker = header[22];
        if options.strict && marker > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("FBX: endian marker {marker:#04x} is neither 0 nor 1"),
            ));
        }
        let byte_order = if marker == 0 {
            FbxByteOrder::Little
        } else {
            FbxByteOrder::Big
        };

        let version = byte_order.u32([header[23], header[24], header[25], header[26]]);
        if !(FBX_MIN_VERSION..=FBX_MAX_VERSION).contains(&version) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FBX: version {version} is outside the supported \
                     {FBX_MIN_VERSION}..={FBX_MAX_VERSION} range \
                     (pre-6000 files use a layout this reader does not implement)"
                ),
            ));
        }

        Ok(Self {
            reader,
            version,
            ascii_nodes: None,
            byte_order,
            options,
            file_len,
            budget: DecodeBudget::default(),
            warnings: Vec::new(),
        })
    }

    /// Records a tolerated container-layout deviation, or fails in strict mode.
    fn note_deviation(
        &mut self,
        code: FbxWarningCode,
        message: String,
        subject: Option<&str>,
    ) -> io::Result<()> {
        if self.options.strict {
            return Err(io::Error::new(io::ErrorKind::InvalidData, message));
        }
        push_warning(&mut self.warnings, code, message, subject);
        Ok(())
    }

    /// Container-layout notices collected by the most recent read.
    pub fn warnings(&self) -> &[FbxWarning] {
        &self.warnings
    }

    /// Folds notices raised by the scene layer back into this reader.
    ///
    /// `read_meshes` collects geometry notices separately because it borrows
    /// the reader immutably while decoding, then hands them back here so a
    /// caller sees the same list either entry point produces.
    pub(crate) fn extend_warnings(&mut self, warnings: impl IntoIterator<Item = FbxWarning>) {
        self.warnings.extend(warnings);
    }

    /// Fails when `requested` bytes cannot possibly be in the file.
    ///
    /// Checked before every length-prefixed allocation so a hostile header
    /// cannot make us reserve gigabytes for data that is not there.
    fn check_available(&mut self, requested: u64, what: &str) -> io::Result<()> {
        let position = self.reader.stream_position()?;
        let remaining = self.file_len.saturating_sub(position);
        if requested > remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FBX: {what} at offset {position} claims {requested} bytes, \
                     but only {remaining} remain in the file"
                ),
            ));
        }
        Ok(())
    }

    fn limit_exceeded(what: &str, value: u64, limit: u64) -> io::Error {
        io::Error::new(
            io::ErrorKind::OutOfMemory,
            format!("FBX: {what} is {value}, over the {limit} limit"),
        )
    }

    /// Get the FBX file version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Byte order selected by this file's header endian marker.
    pub fn byte_order(&self) -> FbxByteOrder {
        self.byte_order
    }

    /// Read options this reader was constructed with.
    pub fn options(&self) -> &FbxReadOptions {
        &self.options
    }

    /// Check if this is FBX 7.5+ (uses 64-bit offsets).
    fn is_64bit(&self) -> bool {
        self.version >= 7500
    }

    /// Read a node record.
    fn read_node(&mut self, depth: u32) -> io::Result<Option<FbxNode>> {
        if depth > self.options.limits.max_depth {
            return Err(Self::limit_exceeded(
                "node nesting depth",
                depth.into(),
                self.options.limits.max_depth.into(),
            ));
        }
        let order = self.byte_order;
        let (end_offset, num_properties, property_list_len, name_len) = if self.is_64bit() {
            let mut buf = [0u8; 25];
            self.reader.read_exact(&mut buf)?;
            let end_offset = order.u64([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]);
            let num_properties = order.u64([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ]);
            let property_list_len = order.u64([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ]);
            let name_len = buf[24];
            (
                end_offset,
                num_properties as u32,
                property_list_len,
                name_len,
            )
        } else {
            let mut buf = [0u8; 13];
            self.reader.read_exact(&mut buf)?;
            let end_offset = order.u32([buf[0], buf[1], buf[2], buf[3]]) as u64;
            let num_properties = order.u32([buf[4], buf[5], buf[6], buf[7]]);
            let property_list_len = order.u32([buf[8], buf[9], buf[10], buf[11]]) as u64;
            let name_len = buf[12];
            (end_offset, num_properties, property_list_len, name_len)
        };

        // The terminator is `end_offset == 0 && name_len == 0`, matching ufbx.
        // A canonical writer zeroes the whole record, so anything left set is
        // worth reporting even though we still accept it.
        if end_offset == 0 && name_len == 0 {
            if num_properties != 0 || property_list_len != 0 {
                self.note_deviation(
                    FbxWarningCode::MalformedNullRecord,
                    format!(
                        "FBX: null record has non-zero property fields \
                         (count {num_properties}, list length {property_list_len})"
                    ),
                    None,
                )?;
            }
            return Ok(None);
        }

        // A *named* record may still declare `end_offset == 0`; Maya emits
        // these (see `maya_zero_end_*` in the ufbx corpus). ufbx treats the
        // node as having no children and resumes after its property list, so
        // there is no end offset to bounds check or seek to.
        let declared_end = (end_offset != 0).then_some(end_offset);
        if declared_end.is_none() {
            self.note_deviation(
                FbxWarningCode::MissingNodeEndOffset,
                "FBX: a named node declares no end offset; reading it without children".to_string(),
                None,
            )?;
        }

        // A record must end after it starts and inside the file. Without the
        // first check a backwards `end_offset` makes the seek below rewind,
        // and the caller re-reads the same record forever.
        let record_end = self.reader.stream_position()?;
        if let Some(end) = declared_end {
            if end < record_end {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "FBX: node record at {record_end} claims it ends at {end}, \
                         before its own header"
                    ),
                ));
            }
            if end > self.file_len {
                if self.options.strict {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "FBX: node record claims it ends at {end}, past the {} byte file",
                            self.file_len
                        ),
                    ));
                }
                // Some exporters emit a bogus trailing offset. Treat it as the
                // end of this sibling list rather than failing a good file.
                return Ok(None);
            }
        }

        self.budget.nodes += 1;
        if self.budget.nodes > self.options.limits.max_nodes {
            return Err(Self::limit_exceeded(
                "node count",
                self.budget.nodes,
                self.options.limits.max_nodes,
            ));
        }

        // `name_len` is a u8, so it needs no limit of its own.
        let mut name_bytes = vec![0u8; name_len as usize];
        self.reader.read_exact(&mut name_bytes)?;
        let name = String::from_utf8_lossy(&name_bytes).to_string();

        // Bound the count before reserving for it: `num_properties` comes
        // straight from the file and feeds `Vec::with_capacity`.
        if u64::from(num_properties) > self.options.limits.max_properties_per_node {
            return Err(Self::limit_exceeded(
                "property count on one node",
                num_properties.into(),
                self.options.limits.max_properties_per_node,
            ));
        }
        // `property_list_len` is the authoritative size of the property block.
        // Honouring it lets a node whose properties decoded wrongly re-sync at
        // the child records instead of consuming them as property data.
        let properties_start = self.reader.stream_position()?;
        let properties_end = properties_start
            .checked_add(property_list_len)
            .filter(|end| *end <= declared_end.unwrap_or(self.file_len) && *end <= self.file_len)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "FBX: node '{name}' declares a {property_list_len} byte property list \
                         at offset {properties_start}, which runs past its own end"
                    ),
                )
            })?;

        let mut properties = Vec::with_capacity(num_properties as usize);
        for _ in 0..num_properties {
            properties.push(self.read_property()?);
        }

        let properties_read_to = self.reader.stream_position()?;
        if properties_read_to != properties_end {
            self.note_deviation(
                FbxWarningCode::PropertyListLengthMismatch,
                format!(
                    "FBX: node '{name}' property list ended at {properties_read_to}, \
                     but its header declared {properties_end}"
                ),
                Some(&name),
            )?;
            self.reader.seek(SeekFrom::Start(properties_end))?;
        }

        // Read children. A node without a declared end has none: `ufbx` stops
        // its child loop immediately for those, and following the property
        // list with a terminator search would consume the next sibling.
        let mut children = Vec::new();
        if let Some(end) = declared_end {
            let current_pos = self.reader.stream_position()?;
            if current_pos < end {
                while let Some(child) = self.read_node(depth + 1)? {
                    children.push(child);
                }
            }
            // Resynchronize on the declared end; children may have stopped
            // short of it, and trailing slack is legal.
            self.reader.seek(SeekFrom::Start(end))?;
        }

        Ok(Some(FbxNode {
            name,
            properties,
            children,
        }))
    }

    /// Read a property.
    fn read_property(&mut self) -> io::Result<FbxProperty> {
        let order = self.byte_order;
        let mut type_code = [0u8; 1];
        self.reader.read_exact(&mut type_code)?;

        match type_code[0] {
            // Single-byte scalars are never byte-swapped.
            b'B' | b'C' => {
                let mut v = [0u8; 1];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::Bool(v[0] != 0))
            }
            b'Z' => {
                let mut v = [0u8; 1];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::U8(v[0]))
            }
            b'Y' => {
                let mut v = [0u8; 2];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I16(order.i16(v)))
            }
            b'I' => {
                let mut v = [0u8; 4];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I32(order.i32(v)))
            }
            b'L' => {
                let mut v = [0u8; 8];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I64(order.i64(v)))
            }
            b'F' => {
                let mut v = [0u8; 4];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::F32(order.f32(v)))
            }
            b'D' => {
                let mut v = [0u8; 8];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::F64(order.f64(v)))
            }
            // The length is byte-swapped; the payload bytes never are.
            b'S' | b'R' => {
                let mut len_bytes = [0u8; 4];
                self.reader.read_exact(&mut len_bytes)?;
                let len = u64::from(order.u32(len_bytes));
                let is_string = type_code[0] == b'S';
                let (limit, what) = if is_string {
                    (self.options.limits.max_string_bytes, "string property")
                } else {
                    // Embedded textures land here through `Video.Content`.
                    (self.options.limits.max_blob_bytes, "raw property")
                };
                if len > limit {
                    return Err(Self::limit_exceeded(what, len, limit));
                }
                self.check_available(len, what)?;
                let mut data = vec![0u8; len as usize];
                self.reader.read_exact(&mut data)?;
                if is_string {
                    Ok(FbxProperty::String(
                        String::from_utf8_lossy(&data).to_string(),
                    ))
                } else {
                    Ok(FbxProperty::Raw(data))
                }
            }
            // `b` is a bool array and `c` a byte array; both are one byte per
            // element, so neither is ever byte-swapped.
            b'b' | b'c' => Ok(FbxProperty::BoolArray(self.read_array_bool()?)),
            b'i' => Ok(FbxProperty::I32Array(self.read_array_i32()?)),
            b'l' => Ok(FbxProperty::I64Array(self.read_array_i64()?)),
            b'f' => Ok(FbxProperty::F32Array(self.read_array_f32()?)),
            b'd' => Ok(FbxProperty::F64Array(self.read_array_f64()?)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown property type: {}", type_code[0] as char),
            )),
        }
    }

    /// Read array header and return (length, encoding, compressed_length).
    fn read_array_header(&mut self) -> io::Result<(u32, u32, u32)> {
        let order = self.byte_order;
        let mut buf = [0u8; 12];
        self.reader.read_exact(&mut buf)?;
        let array_len = order.u32([buf[0], buf[1], buf[2], buf[3]]);
        let encoding = order.u32([buf[4], buf[5], buf[6], buf[7]]);
        let compressed_len = order.u32([buf[8], buf[9], buf[10], buf[11]]);
        Ok((array_len, encoding, compressed_len))
    }

    /// Read array data, decompressing and byte-swapping as needed.
    ///
    /// `element_size` drives the big-endian conversion, which happens once in
    /// bulk here so the little-endian path stays a plain `from_le_bytes`
    /// decode in the callers.
    fn read_array_data(
        &mut self,
        len: u32,
        encoding: u32,
        compressed_len: u32,
        element_size: usize,
    ) -> io::Result<Vec<u8>> {
        let limits = self.options.limits;
        let element_count = u64::from(len);
        if element_count > limits.max_array_elements {
            return Err(Self::limit_exceeded(
                "array element count",
                element_count,
                limits.max_array_elements,
            ));
        }
        // `usize` is 32-bit on wasm32, so this product genuinely overflows
        // there rather than merely in theory.
        let uncompressed_size =
            element_count
                .checked_mul(element_size as u64)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("FBX: array of {element_count} x {element_size} bytes overflows"),
                    )
                })?;
        if uncompressed_size > limits.max_array_raw_bytes {
            return Err(Self::limit_exceeded(
                "array size",
                uncompressed_size,
                limits.max_array_raw_bytes,
            ));
        }
        self.budget.array_raw_bytes = self
            .budget
            .array_raw_bytes
            .saturating_add(uncompressed_size);
        if self.budget.array_raw_bytes > limits.max_total_array_raw_bytes {
            return Err(Self::limit_exceeded(
                "total decoded array bytes",
                self.budget.array_raw_bytes,
                limits.max_total_array_raw_bytes,
            ));
        }
        let uncompressed_size = usize::try_from(uncompressed_size).map_err(|_| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!("FBX: array of {uncompressed_size} bytes does not fit in memory"),
            )
        })?;

        let order = self.byte_order;
        let mut data = self.read_array_payload(encoding, compressed_len, uncompressed_size)?;
        order.swap_elements_in_place(&mut data, element_size);
        Ok(data)
    }

    fn read_array_payload(
        &mut self,
        encoding: u32,
        compressed_len: u32,
        uncompressed_size: usize,
    ) -> io::Result<Vec<u8>> {
        if encoding == 0 {
            // For a raw array the stored length must equal the element extent
            // exactly; ufbx enforces the same equality.
            if compressed_len != 0 && compressed_len as usize != uncompressed_size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "FBX: uncompressed array stores {compressed_len} bytes \
                         but its elements span {uncompressed_size}"
                    ),
                ));
            }
            self.check_available(uncompressed_size as u64, "uncompressed array")?;
            let mut data = vec![0u8; uncompressed_size];
            self.reader.read_exact(&mut data)?;
            Ok(data)
        } else if encoding == 1 {
            // Deflate/zlib compressed
            self.check_available(compressed_len.into(), "compressed array")?;
            let mut compressed = vec![0u8; compressed_len as usize];
            self.reader.read_exact(&mut compressed)?;

            #[cfg(feature = "compression")]
            {
                use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;
                // Bounding the output makes a zip bomb an error instead of an
                // out-of-memory abort; the exact-size check then rejects a
                // stream that does not describe this array.
                let data = decompress_to_vec_zlib_with_limit(&compressed, uncompressed_size)
                    .map_err(|error| {
                        // Only the status: `DecompressError`'s `Debug` carries
                        // the whole partial output, which would put megabytes
                        // of payload into the message.
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("FBX: array decompression failed ({:?})", error.status),
                        )
                    })?;
                if data.len() != uncompressed_size {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "FBX: compressed array decoded to {} bytes, expected {uncompressed_size}",
                            data.len()
                        ),
                    ));
                }
                Ok(data)
            }

            #[cfg(not(feature = "compression"))]
            {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "FBX array compression not supported (enable 'compression' feature)",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown array encoding: {}", encoding),
            ))
        }
    }

    /// Byte-array payload (`b`/`c`); single bytes are never byte-swapped.
    fn read_array_bool(&mut self) -> io::Result<Vec<bool>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(len, encoding, compressed_len, 1)?;
        Ok(data.into_iter().map(|b| b != 0).collect())
    }

    fn read_array_i32(&mut self) -> io::Result<Vec<i32>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(len, encoding, compressed_len, 4)?;
        Ok(data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn read_array_i64(&mut self) -> io::Result<Vec<i64>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(len, encoding, compressed_len, 8)?;
        Ok(data
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    fn read_array_f32(&mut self) -> io::Result<Vec<f32>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(len, encoding, compressed_len, 4)?;
        Ok(data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn read_array_f64(&mut self) -> io::Result<Vec<f64>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(len, encoding, compressed_len, 8)?;
        Ok(data
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    /// Read all top-level nodes.
    ///
    /// Safe to call more than once: per-document budgets restart here, so a
    /// second read of the same file behaves exactly like the first.
    pub fn read_nodes(&mut self) -> io::Result<Vec<FbxNode>> {
        // An ASCII document was parsed whole at construction; hand back a copy
        // so this stays re-entrant like the binary path.
        if let Some(nodes) = &self.ascii_nodes {
            return Ok(nodes.clone());
        }
        // Seek to start of nodes (after the fixed-size header).
        self.reader.seek(SeekFrom::Start(FBX_HEADER_LEN))?;
        self.budget = DecodeBudget::default();

        let mut nodes = Vec::new();
        while let Some(node) = self.read_node(0)? {
            nodes.push(node);
        }
        if self.options.strict {
            self.validate_footer()?;
        }
        Ok(nodes)
    }

    /// Checks the binary footer that follows the root terminator.
    ///
    /// Strict mode only. `ufbx` never looks at the footer, and shipping
    /// exporters get its padding wrong often enough that rejecting on it by
    /// default would fail files every other reader accepts.
    fn validate_footer(&mut self) -> io::Result<()> {
        let start = self.reader.stream_position()?;
        let mut footer = Vec::new();
        self.reader.read_to_end(&mut footer)?;

        if footer.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("FBX: no footer after the root terminator at offset {start}"),
            ));
        }
        if !footer.starts_with(&FBX_FOOTER_ID) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("FBX: footer at offset {start} does not begin with the footer id"),
            ));
        }
        if !footer.ends_with(&FBX_FOOTER_MAGIC) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FBX: file does not end with the footer magic".to_string(),
            ));
        }

        // Layout after the id: 4 zero bytes, alignment padding, the version
        // repeated, 120 zero bytes, then the closing magic. A footer can begin
        // with the id and end with the magic and still be far too short to
        // hold the middle, so every step back from the end must be checked.
        let Some((version_start, version_end)) = footer
            .len()
            .checked_sub(FBX_FOOTER_MAGIC.len())
            .and_then(|end| end.checked_sub(120))
            .and_then(|end| Some((end.checked_sub(4)?, end)))
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FBX: footer is only {} bytes, too short to hold a version",
                    footer.len()
                ),
            ));
        };
        let repeated = self.byte_order.u32([
            footer[version_start],
            footer[version_start + 1],
            footer[version_start + 2],
            footer[version_start + 3],
        ]);
        if repeated != self.version {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "FBX: footer repeats version {repeated}, but the header declared {}",
                    self.version
                ),
            ));
        }
        if footer[version_end..footer.len() - FBX_FOOTER_MAGIC.len()]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "FBX: footer padding after the version is not zeroed".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fbx_options::FbxDecodeLimits;
    use std::io::Cursor;

    #[test]
    fn test_fbx_magic() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.extend_from_slice(&[0x1A, 0x00]); // Unknown bytes
        data.extend_from_slice(&7300u32.to_le_bytes()); // Version 7.3
                                                        // Add null record to end nodes
        data.extend_from_slice(&[0u8; 13]);

        let cursor = Cursor::new(data);
        let reader = FbxReader::new(cursor).unwrap();
        assert_eq!(reader.version(), 7300);
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"Not an FBX file at all";
        let cursor = Cursor::new(data.to_vec());
        assert!(FbxReader::new(cursor).is_err());
    }

    #[test]
    fn memory_reader_reads_an_empty_scene() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.extend_from_slice(&[0x1A, 0x00]);
        data.extend_from_slice(&7300u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 13]);

        let mut reader = FbxReader::new(Cursor::new(data)).unwrap();
        let scene = reader.read_scene().unwrap();
        assert!(scene.root_nodes.is_empty());
    }

    /// Builds a header plus `body`, using the given endian marker.
    fn header_with(marker: u8, version: u32, body: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.push(FBX_MAGIC_TAIL);
        data.push(marker);
        if marker == 0 {
            data.extend_from_slice(&version.to_le_bytes());
        } else {
            data.extend_from_slice(&version.to_be_bytes());
        }
        data.extend_from_slice(body);
        data
    }

    #[test]
    fn endian_marker_selects_big_endian_and_its_version() {
        let data = header_with(1, 7500, &[0u8; 25]);
        let reader = FbxReader::new(Cursor::new(data)).unwrap();
        assert_eq!(reader.byte_order(), FbxByteOrder::Big);
        assert_eq!(reader.version(), 7500);
    }

    #[test]
    fn strict_mode_rejects_an_undocumented_endian_marker() {
        let data = header_with(2, 7500, &[0u8; 25]);
        // Lenient follows ufbx: any non-zero marker means big-endian.
        assert!(FbxReader::new(Cursor::new(data.clone())).is_ok());
        assert!(
            FbxReader::new_with_options(Cursor::new(data), FbxReadOptions::strict()).is_err(),
            "strict mode should reject a marker that is neither 0 nor 1"
        );
    }

    #[test]
    fn truncated_magic_tail_is_rejected() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.push(0x00); // should be 0x1A
        data.push(0x00);
        data.extend_from_slice(&7500u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 25]);
        assert!(FbxReader::new(Cursor::new(data)).is_err());
    }

    #[test]
    fn pre_6000_versions_are_rejected_rather_than_read_as_empty() {
        // These used to yield a scene with no nodes, which reads as "the file
        // had no meshes" instead of "this layout is unsupported".
        let data = header_with(0, 3000, &[0u8; 13]);
        let error = FbxReader::new(Cursor::new(data))
            .err()
            .expect("expected rejection");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("3000"), "{error}");
    }

    #[test]
    fn a_backwards_end_offset_is_rejected_instead_of_looping() {
        // `end_offset` points back into the header. Before the bounds check
        // the reader seeked backwards and re-read this record forever.
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // end_offset = 1
        body.extend_from_slice(&0u32.to_le_bytes()); // num_properties
        body.extend_from_slice(&0u32.to_le_bytes()); // property_list_len
        body.push(0); // name_len
        let data = header_with(0, 7400, &body);

        let mut reader = FbxReader::new(Cursor::new(data)).unwrap();
        let error = reader.read_nodes().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn an_oversized_array_is_refused_without_allocating() {
        // A 12-byte array header claiming ~4G elements: the old reader
        // multiplied it out and asked the allocator for 34 GB.
        let mut body = Vec::new();
        let node_start = 27u32;
        let node_len = 13 + 1 + 1 + 12;
        body.extend_from_slice(&(node_start + node_len).to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes()); // one property
        body.extend_from_slice(&13u32.to_le_bytes()); // property_list_len
        body.push(1); // name_len
        body.push(b'X');
        body.push(b'd'); // f64 array
        body.extend_from_slice(&u32::MAX.to_le_bytes()); // element count
        body.extend_from_slice(&0u32.to_le_bytes()); // encoding: raw
        body.extend_from_slice(&0u32.to_le_bytes()); // compressed length
        let data = header_with(0, 7400, &body);

        let mut reader = FbxReader::new(Cursor::new(data)).unwrap();
        let error = reader.read_nodes().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    }

    #[test]
    fn reading_the_same_document_twice_succeeds() {
        // `read_nodes` is re-entrant; a per-document budget that accumulated
        // across calls would fail the second read of an accepted file.
        let data = header_with(0, 7400, &[0u8; 13]);
        let mut reader = FbxReader::new(Cursor::new(data)).unwrap();
        assert!(reader.read_nodes().is_ok());
        assert!(reader.read_nodes().is_ok());
    }

    #[test]
    fn a_short_footer_is_refused_instead_of_panicking() {
        // Found by `cargo fuzz run fbx_read_scene` within minutes of the
        // target existing. A footer can begin with the id and end with the
        // magic while being far too short to hold the version and padding
        // between them; stepping back from the end then wrapped around and
        // indexed out of bounds.
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.push(FBX_MAGIC_TAIL);
        data.push(0);
        data.extend_from_slice(&7400u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 13]); // root terminator
        data.extend_from_slice(&FBX_FOOTER_ID);
        data.extend_from_slice(&FBX_FOOTER_MAGIC);

        let mut reader =
            FbxReader::new_with_options(Cursor::new(data), FbxReadOptions::strict()).unwrap();
        let error = reader.read_nodes().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("too short"), "{error}");
    }

    #[test]
    fn repeated_deviations_collapse_into_one_counted_warning() {
        // A malformed pattern repeated across a large file must not produce
        // one warning per node.
        let mut warnings = Vec::new();
        for _ in 0..5 {
            push_warning(
                &mut warnings,
                FbxWarningCode::PropertyListLengthMismatch,
                "mismatch".to_string(),
                Some("Geometry"),
            );
        }
        push_warning(
            &mut warnings,
            FbxWarningCode::PropertyListLengthMismatch,
            "mismatch".to_string(),
            Some("Model"),
        );

        assert_eq!(warnings.len(), 2, "distinct subjects stay distinct");
        assert_eq!(warnings[0].count, 5);
        assert_eq!(warnings[0].to_string(), "mismatch (x5)");
        assert_eq!(warnings[1].count, 1);
        assert_eq!(warnings[1].to_string(), "mismatch");
    }

    #[test]
    fn a_tolerated_deviation_is_reported_and_strict_mode_rejects_it() {
        // A terminator carrying non-zero property fields: accepted with a
        // notice, refused outright when strict.
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.push(FBX_MAGIC_TAIL);
        data.push(0);
        data.extend_from_slice(&7400u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // end_offset
        data.extend_from_slice(&3u32.to_le_bytes()); // num_properties, should be 0
        data.extend_from_slice(&0u32.to_le_bytes()); // property_list_len
        data.push(0); // name_len

        let mut reader = FbxReader::new(Cursor::new(data.clone())).unwrap();
        let scene = reader.read_scene().unwrap();
        assert_eq!(scene.warnings.len(), 1);
        assert_eq!(scene.warnings[0].code, FbxWarningCode::MalformedNullRecord);
        assert!(!scene.warnings[0].code.is_data_loss());
        assert_eq!(scene.warnings[0].code.as_str(), "malformed-null-record");

        let strict = FbxReader::new_with_options(Cursor::new(data), FbxReadOptions::strict())
            .unwrap()
            .read_scene();
        assert!(strict.is_err(), "strict mode should reject the deviation");
    }

    #[test]
    fn a_file_over_the_size_limit_is_refused() {
        let data = header_with(0, 7400, &[0u8; 13]);
        let options = FbxReadOptions::default()
            .with_limits(FbxDecodeLimits::default().with_max_file_bytes(8));
        let error = FbxReader::new_with_options(Cursor::new(data), options)
            .err()
            .expect("expected rejection");
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    }
}
