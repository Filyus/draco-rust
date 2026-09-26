//! `KHR_draco_mesh_compression` decoding for callers that bring their own glTF
//! document model.
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

use std::collections::BTreeMap;

use draco_core::{DecodeLimits, DecoderBuffer, Mesh, MeshDecoder};

use crate::extensions::{parse_draco_extension, DracoContract};
use crate::json::Value;
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
        let contract = parse_draco_extension(Some(value))?
            .ok_or_else(|| Error::Extension("missing Draco extension".into()))?;
        Ok(Self::from_contract(contract))
    }

    pub(crate) fn from_contract(contract: DracoContract) -> Self {
        Self {
            buffer_view: contract.buffer_view,
            attributes: contract.attributes,
        }
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
        let mesh = decode_payload(payload, &contract.limits)?;
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
    limits: DecodeLimits,
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
            limits: DecodeLimits::default(),
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
        self.limits = limits;
        self
    }

    /// Selects the validation profile.
    #[must_use]
    pub const fn with_profile(mut self, profile: ValidationProfile) -> Self {
        self.profile = profile;
        self
    }
}

pub(crate) fn decode_payload(payload: &[u8], limits: &DecodeLimits) -> Result<Mesh> {
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(
            &mut DecoderBuffer::new(payload).with_limits(*limits),
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
