//! `KHR_draco_mesh_compression` for hosts with their own glTF document model.
//!
//! A host that parsed the file and loaded its buffers itself passes in the
//! extension object ([`DracoPrimitiveExtension::from_json`]), the bytes of its
//! buffer view, and what the primitive's accessors declare
//! ([`DracoPrimitiveContract`]). [`DracoPrimitiveExtension::decode`] then does
//! what [`Import::read_primitive`](crate::Import::read_primitive) does and
//! returns the same [`PackedGeometry`].
//!
//! ```
//! use draco_gltf::{DracoPrimitiveContract, DracoPrimitiveExtension, JsonValue};
//!
//! let json = JsonValue::parse(br#"{"bufferView":3,"attributes":{"POSITION":0}}"#)?;
//! let extension = DracoPrimitiveExtension::from_json(&json)?;
//! assert_eq!(extension.buffer_view(), 3);
//! assert_eq!(extension.unique_id("POSITION"), Some(0));
//!
//! let contract = DracoPrimitiveContract::new().with_attribute("POSITION", 24, false);
//! // `payload` is the byte range of buffer view 3:
//! // let geometry = extension.decode(payload, &contract)?;
//! # let _ = contract;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! `DracoPrimitiveEncoding::encode` (feature `draco-encode`) goes the other
//! way: bitstream, extension object and accessor declarations for a
//! [`PackedGeometry`].

use std::collections::BTreeMap;

use draco_core::{DecodeLimits, DecoderBuffer, Mesh, MeshDecoder};

use crate::extensions::{parse_draco_extension, DracoContract};
use crate::json::{Tape, Value};
use crate::{Error, GeometryError, PackedGeometry, PrimitiveMode, Result, ValidationProfile};

/// A parsed `KHR_draco_mesh_compression` primitive extension object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DracoPrimitiveExtension {
    buffer_view: usize,
    attributes: Vec<(String, u32)>,
}

impl DracoPrimitiveExtension {
    /// Parses the value of a primitive's `KHR_draco_mesh_compression` key.
    /// Checks the object's shape only.
    pub fn from_json(value: &Value) -> Result<Self> {
        let contract = parse_draco_extension(Some(Tape::from_value(value).root()))?
            .ok_or_else(|| Error::Extension("missing Draco extension".into()))?;
        Ok(Self::from_contract(contract))
    }

    pub(crate) fn from_contract(contract: DracoContract) -> Self {
        Self {
            buffer_view: contract.buffer_view,
            attributes: contract.attributes,
        }
    }

    /// Serializes the extension object.
    pub fn to_json(&self) -> Value {
        Value::object([
            ("bufferView", Value::from(self.buffer_view)),
            (
                "attributes",
                Value::Object(
                    self.attributes
                        .iter()
                        .map(|(semantic, unique_id)| {
                            (semantic.clone(), Value::from(u64::from(*unique_id)))
                        })
                        .collect(),
                ),
            ),
        ])
    }

    /// Returns the index of the buffer view holding the Draco bitstream.
    pub const fn buffer_view(&self) -> usize {
        self.buffer_view
    }

    /// Iterates `(semantic, Draco unique id)` in the extension's order.
    pub fn attributes(&self) -> impl ExactSizeIterator<Item = (&str, u32)> + '_ {
        self.attributes
            .iter()
            .map(|(semantic, unique_id)| (semantic.as_str(), *unique_id))
    }

    /// Returns the Draco unique id of `semantic`, or `None` for an attribute
    /// the extension does not compress (an ordinary accessor).
    pub fn unique_id(&self, semantic: &str) -> Option<u32> {
        self.attributes
            .iter()
            .find(|(name, _)| name == semantic)
            .map(|(_, unique_id)| *unique_id)
    }

    /// Decodes `payload`, the bytes of [`Self::buffer_view`].
    ///
    /// Returns the compressed attributes only, as a `u32`-indexed triangle
    /// list, with the accessors' `normalized` flags applied.
    pub fn decode(
        &self,
        payload: &[u8],
        contract: &DracoPrimitiveContract,
    ) -> Result<PackedGeometry> {
        let mesh = decode_payload(payload, &contract.decode)?;
        self.pack(&mesh, contract)
    }

    /// Checks a decoded mesh against `contract` and packs it.
    pub(crate) fn pack(
        &self,
        mesh: &Mesh,
        contract: &DracoPrimitiveContract,
    ) -> Result<PackedGeometry> {
        validate_decoded_counts(mesh, contract)?;
        let normalized = contract
            .attributes
            .iter()
            .map(|(semantic, declared)| (semantic.clone(), declared.normalized))
            .collect();
        let geometry = PackedGeometry::from_draco_mesh(mesh, &self.attributes, &normalized)?;
        geometry.validate(contract.profile)?;
        Ok(geometry)
    }
}

/// What a host document declares about one Draco-compressed primitive.
///
/// List every accessor the primitive names, compressed or not: a count the
/// stream cannot supply is refused.
#[derive(Clone, Debug)]
pub struct DracoPrimitiveContract {
    attributes: BTreeMap<String, DeclaredAccessor>,
    indices: Option<u64>,
    mode: PrimitiveMode,
    decode: crate::DracoDecodeOptions,
    profile: ValidationProfile,
}

#[derive(Clone, Copy, Debug)]
struct DeclaredAccessor {
    count: u64,
    normalized: bool,
}

impl Default for DracoPrimitiveContract {
    fn default() -> Self {
        Self::new()
    }
}

impl DracoPrimitiveContract {
    /// `TRIANGLES`, default decode limits, [`ValidationProfile::Gltf21Draft`].
    pub fn new() -> Self {
        Self {
            attributes: BTreeMap::new(),
            indices: None,
            mode: PrimitiveMode::Triangles,
            decode: crate::DracoDecodeOptions::default(),
            profile: ValidationProfile::Gltf21Draft,
        }
    }

    /// Declares the `count` and `normalized` of `semantic`'s accessor.
    #[must_use]
    pub fn with_attribute(
        mut self,
        semantic: impl Into<String>,
        count: u64,
        normalized: bool,
    ) -> Self {
        self.attributes
            .insert(semantic.into(), DeclaredAccessor { count, normalized });
        self
    }

    /// Declares the `count` of the primitive's `indices` accessor.
    #[must_use]
    pub const fn with_indices(mut self, count: u64) -> Self {
        self.indices = Some(count);
        self
    }

    /// Declares the primitive's `mode`. It only decides whether the index
    /// count is checked, which it is for `TRIANGLES`.
    #[must_use]
    pub const fn with_mode(mut self, mode: PrimitiveMode) -> Self {
        self.mode = mode;
        self
    }

    /// Caps what one decode may allocate and reconstruct.
    #[must_use]
    pub const fn with_limits(mut self, limits: DecodeLimits) -> Self {
        self.decode.limits = limits;
        self
    }

    /// Lets a point-cloud stream decode on `threads`: `1`, the default, keeps
    /// it on the calling thread, `0` takes as many as the machine has up to
    /// sixteen. A mesh stream decodes on the calling thread whatever this is.
    #[must_use]
    pub const fn with_threads(mut self, threads: i32) -> Self {
        self.decode.threads = threads;
        self
    }

    /// Selects the validation profile.
    #[must_use]
    pub const fn with_profile(mut self, profile: ValidationProfile) -> Self {
        self.profile = profile;
        self
    }
}

pub(crate) fn decode_payload(payload: &[u8], options: &crate::DracoDecodeOptions) -> Result<Mesh> {
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder.set_threads(options.threads);
    decoder
        .decode(
            &mut DecoderBuffer::new(payload).with_limits(options.limits),
            &mut mesh,
        )
        .map_err(Error::Decode)?;
    Ok(mesh)
}

pub(crate) fn validate_decoded_counts(
    mesh: &Mesh,
    contract: &DracoPrimitiveContract,
) -> Result<()> {
    let decoded_points = u64::try_from(mesh.num_points())
        .map_err(|_| Error::ResourceLimit("decoded Draco point count exceeds u64".into()))?;
    for (semantic, declared) in &contract.attributes {
        // Only an accessor that promises more vertices than the stream can
        // supply is fatal: the missing ones have nowhere to come from.
        //
        // The other direction is what real encoders emit. Draco stores
        // connectivity per position vertex and re-splits it at attribute
        // seams while decoding, so a mesh whose normals or texture
        // coordinates break along an edge decodes to more points than the
        // accessor written before compression declares. glTF-Pipeline,
        // Blender and the Draco encoder itself all produce such files —
        // Three.js's ferrari.glb among them — and every browser viewer
        // reads them, because the decoded geometry is self-consistent:
        // indices, positions and attributes all come out of the same
        // stream. Refusing them would reject working files over metadata
        // the extension has already superseded.
        if declared.count > decoded_points {
            return Err(GeometryError::DracoAccessorCount {
                semantic: semantic.clone(),
                decoded: decoded_points,
                declared: declared.count,
            }
            .into());
        }
    }

    if contract.mode == PrimitiveMode::Triangles {
        if let Some(declared) = contract.indices {
            let decoded = mesh
                .num_faces()
                .checked_mul(3)
                .and_then(|count| u64::try_from(count).ok())
                .ok_or_else(|| {
                    Error::ResourceLimit("decoded Draco index count exceeds u64".into())
                })?;
            if declared != decoded {
                return Err(GeometryError::DracoAccessorCount {
                    semantic: "indices".into(),
                    decoded,
                    declared,
                }
                .into());
            }
        }
    }
    Ok(())
}

#[cfg(feature = "draco-encode")]
pub use encode::{DracoAccessor, DracoPrimitiveEncoding};

#[cfg(feature = "draco-encode")]
mod encode {
    use draco_core::draco_types::DataType;

    use super::DracoPrimitiveExtension;
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    use crate::compression::{decode_encoded, encode_primitive, keeps_vertex_order};
    use crate::geometry::{gltf_type_for_num_components, AccessorSource, DecodedAccessor};
    use crate::gltf_error::GltfError;
    use crate::{ComponentType, CompressionOptions, Error, PackedGeometry, PrimitiveMode, Result};

    /// A primitive encoded for a host document.
    ///
    /// Store [`Self::bytes`] in a buffer view, put [`Self::extension`] under
    /// the primitive's `extensions`, declare the accessors as
    /// [`Self::accessors`] and [`Self::index_count`] say (without
    /// `bufferView`), and list the extension in `extensionsUsed` -- and in
    /// `extensionsRequired` unless a fallback is kept.
    ///
    /// ```
    /// use draco_gltf::{
    ///     ComponentType, CompressionOptions, DracoPrimitiveEncoding, PackedAttribute,
    ///     PackedGeometry, PackedIndices, PrimitiveMode,
    /// };
    ///
    /// let corners: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    /// let bytes = corners.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    /// let position = PackedAttribute::new("POSITION", 3, 3, ComponentType::F32, false, bytes)?;
    /// let indices = PackedIndices::new(3, ComponentType::U8, vec![0, 1, 2])?;
    /// let geometry = PackedGeometry::new(PrimitiveMode::Triangles, vec![position], Some(indices))?;
    ///
    /// let encoded = DracoPrimitiveEncoding::encode(&geometry, &CompressionOptions::default())?;
    /// assert_eq!(encoded.index_count(), 3);
    /// assert_eq!(encoded.accessors()[0].semantic(), "POSITION");
    /// let extension = encoded.extension(0).to_json();
    /// assert_eq!(extension["bufferView"].as_u64(), Some(0));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[derive(Clone, Debug)]
    pub struct DracoPrimitiveEncoding {
        bytes: Vec<u8>,
        attributes: Vec<(String, u32)>,
        accessors: Vec<DracoAccessor>,
        index_count: usize,
        normalized: BTreeMap<String, bool>,
        position_bits: Option<u8>,
        decoded: OnceLock<PackedGeometry>,
    }

    /// What one attribute accessor of an encoded primitive declares.
    #[derive(Clone, Debug, PartialEq)]
    pub struct DracoAccessor {
        semantic: String,
        count: usize,
        components: u8,
        component_type: ComponentType,
        normalized: bool,
        bounds: Option<(Vec<f64>, Vec<f64>)>,
    }

    impl DracoAccessor {
        /// Returns the glTF attribute semantic.
        pub fn semantic(&self) -> &str {
            &self.semantic
        }

        /// Returns the accessor `count`: the vertices the stream decodes to,
        /// which may differ from the input's.
        pub const fn count(&self) -> usize {
            self.count
        }

        /// Returns the number of components per element.
        pub const fn components(&self) -> u8 {
            self.components
        }

        /// Returns the accessor `type`, such as `"VEC3"`.
        pub fn accessor_type(&self) -> &'static str {
            gltf_type_for_num_components(self.components)
                .expect("encoded attributes have one to four components")
        }

        /// Returns the accessor `componentType`.
        pub const fn component_type(&self) -> ComponentType {
            self.component_type
        }

        /// Returns the accessor `normalized` flag, taken from the input.
        pub const fn normalized(&self) -> bool {
            self.normalized
        }

        /// Returns `POSITION`'s `min` and `max` as decoded, after quantization;
        /// `None` for other attributes.
        pub fn bounds(&self) -> Option<(&[f64], &[f64])> {
            self.bounds
                .as_ref()
                .map(|(min, max)| (min.as_slice(), max.as_slice()))
        }
    }

    impl DracoPrimitiveEncoding {
        /// Encodes a `TRIANGLES`, `TRIANGLE_STRIP` or `TRIANGLE_FAN` primitive
        /// as [`Import::compress_primitive`](crate::Import::compress_primitive)
        /// does. Strips and fans become triangle lists, so write `mode` as
        /// `TRIANGLES`. `options.mode` is ignored; `max_output_bytes` caps the
        /// bitstream.
        pub fn encode(geometry: &PackedGeometry, options: &CompressionOptions) -> Result<Self> {
            let mode = geometry.mode();
            if !matches!(
                mode,
                PrimitiveMode::Triangles
                    | PrimitiveMode::TriangleStrip
                    | PrimitiveMode::TriangleFan
            ) {
                return Err(Error::Extension(format!(
                    "KHR_draco_mesh_compression encoding supports only TRIANGLES, \
                     TRIANGLE_STRIP and TRIANGLE_FAN, not {mode:?}"
                )));
            }
            let source = PackedSource(geometry);
            let attributes: Vec<(String, usize)> = geometry
                .attributes()
                .iter()
                .enumerate()
                .map(|(index, attribute)| (attribute.semantic().to_owned(), index))
                .collect();
            let indices = geometry.indices().map(|_| source.index_accessor());
            let (mesh, mapping) =
                crate::decode_geometry(&source, mode.to_gltf(), &attributes, indices)?;
            let normalized: BTreeMap<String, bool> = geometry
                .attributes()
                .iter()
                .map(|attribute| (attribute.semantic().to_owned(), attribute.normalized()))
                .collect();
            let encoded = encode_primitive(mesh, mapping, options)?;
            if let Some(limit) = options.max_output_bytes {
                if encoded.bytes.len() > limit {
                    return Err(Error::ResourceLimit(format!(
                        "Draco bitstream size {} exceeds limit {limit}",
                        encoded.bytes.len()
                    )));
                }
            }

            let accessors = encoded
                .mapping
                .iter()
                .zip(&encoded.layout.attributes)
                .map(|((semantic, _), layout)| DracoAccessor {
                    semantic: semantic.clone(),
                    count: encoded.layout.points,
                    components: layout.components,
                    component_type: layout.component_type,
                    normalized: normalized.get(semantic).copied().unwrap_or(false),
                    bounds: layout.position_bounds.clone(),
                })
                .collect();
            Ok(Self {
                index_count: encoded.layout.index_count,
                bytes: encoded.bytes,
                attributes: encoded.mapping,
                accessors,
                normalized,
                position_bits: options.quantization.position,
                decoded: OnceLock::new(),
            })
        }

        /// Borrows the Draco bitstream.
        pub fn bytes(&self) -> &[u8] {
            &self.bytes
        }

        /// Takes the Draco bitstream.
        pub fn into_bytes(self) -> Vec<u8> {
            self.bytes
        }

        /// Returns the extension object for buffer view `buffer_view`.
        pub fn extension(&self, buffer_view: usize) -> DracoPrimitiveExtension {
            DracoPrimitiveExtension {
                buffer_view,
                attributes: self.attributes.clone(),
            }
        }

        /// Returns the attribute accessor declarations, in extension order.
        pub fn accessors(&self) -> &[DracoAccessor] {
            &self.accessors
        }

        /// Returns the index accessor's `count`; its type is `UNSIGNED_INT`.
        pub const fn index_count(&self) -> usize {
            self.index_count
        }

        /// Decodes [`Self::bytes`], on first call, into what readers will see.
        ///
        /// Write an uncompressed fallback from this rather than from the
        /// input: the spec asks for fallback data "decompressed from the Draco
        /// buffer".
        pub fn decoded(&self) -> Result<&PackedGeometry> {
            if let Some(decoded) = self.decoded.get() {
                return Ok(decoded);
            }
            let decoded = decode_encoded(
                &self.bytes,
                &self.attributes,
                self.reported(),
                &self.normalized,
            )?;
            Ok(self.decoded.get_or_init(|| decoded))
        }

        /// Returns whether every vertex of `input`, the encoded geometry,
        /// decodes at its own index (within quantization).
        ///
        /// Morph targets stay valid only if it does. EdgeBreaker renumbers
        /// vertices; use `encoding_method: 1` for primitives with targets.
        pub fn keeps_vertex_order(&self, input: &PackedGeometry) -> Result<bool> {
            Ok(keeps_vertex_order(
                input,
                self.decoded()?,
                self.position_bits,
            ))
        }

        fn reported(&self) -> (usize, usize) {
            (
                self.accessors.first().map_or(0, DracoAccessor::count),
                self.index_count,
            )
        }
    }

    /// Feeds packed geometry to `decode_geometry`, the builder the document
    /// path uses. Accessor `n` is attribute `n`; the next one is the indices.
    struct PackedSource<'a>(&'a PackedGeometry);

    impl PackedSource<'_> {
        fn index_accessor(&self) -> usize {
            self.0.attributes().len()
        }
    }

    impl AccessorSource for PackedSource<'_> {
        fn read_attribute(
            &self,
            accessor: usize,
            expected_types: &[&str],
            allowed_component_types: &[u32],
        ) -> std::result::Result<DecodedAccessor, GltfError> {
            let attribute = self.0.attributes().get(accessor).ok_or_else(|| {
                GltfError::InvalidGltf(format!("attribute {accessor} out of range"))
            })?;
            let accessor_type = gltf_type_for_num_components(attribute.components())?;
            let component_type = attribute.component_type();
            if !expected_types.contains(&accessor_type)
                || !allowed_component_types.contains(&component_type.to_gltf())
            {
                return Err(GltfError::Unsupported(format!(
                    "{} layout {accessor_type} of {component_type:?} is not permitted",
                    attribute.semantic()
                )));
            }
            DecodedAccessor::new(
                attribute.count(),
                attribute.components(),
                data_type(component_type)?,
                attribute.normalized(),
                attribute.bytes().to_vec(),
            )
        }

        fn read_indices(&self, accessor: usize) -> std::result::Result<Vec<u32>, GltfError> {
            let indices = self
                .0
                .indices()
                .filter(|_| accessor == self.index_accessor())
                .ok_or_else(|| {
                    GltfError::InvalidGltf(format!("indices {accessor} out of range"))
                })?;
            let width = indices.component_type().byte_width();
            indices
                .bytes()
                .chunks_exact(width)
                .map(|value| {
                    Ok(match indices.component_type() {
                        ComponentType::U8 => u32::from(value[0]),
                        ComponentType::U16 => u32::from(u16::from_le_bytes([value[0], value[1]])),
                        ComponentType::U32 => {
                            u32::from_le_bytes([value[0], value[1], value[2], value[3]])
                        }
                        other => {
                            return Err(GltfError::Unsupported(format!(
                                "index component type {other:?}"
                            )))
                        }
                    })
                })
                .collect()
        }
    }

    /// Draco storage for a glTF 2.0 component type.
    fn data_type(component_type: ComponentType) -> std::result::Result<DataType, GltfError> {
        Ok(match component_type {
            ComponentType::I8 => DataType::Int8,
            ComponentType::U8 => DataType::Uint8,
            ComponentType::I16 => DataType::Int16,
            ComponentType::U16 => DataType::Uint16,
            ComponentType::U32 => DataType::Uint32,
            ComponentType::F32 => DataType::Float32,
            other => {
                return Err(GltfError::Unsupported(format!(
                    "component type {other:?} cannot be Draco-encoded as glTF 2.0"
                )))
            }
        })
    }
}
