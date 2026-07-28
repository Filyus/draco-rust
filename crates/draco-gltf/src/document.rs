//! Lossless glTF document model and typed object views.
//!
//! The JSON DOM is authoritative: typed views deliberately never own a second
//! schema copy, so draft fields and unknown extensions survive edits.

use std::marker::PhantomData;

use crate::json::Value;

use crate::{Error, Result};

macro_rules! index {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[doc = concat!("Typed index into the glTF `", stringify!($name), "[]` array.")]
        pub struct $name(pub usize);
        impl $name {
            /// Returns the zero-based array position represented by this index.
            pub const fn index(self) -> usize {
                self.0
            }
        }
    };
}

index!(AccessorIndex);
index!(AnimationIndex);
index!(BufferIndex);
index!(BufferViewIndex);
index!(CameraIndex);
index!(ExternalAssetIndex);
index!(FileIndex);
index!(ImageIndex);
index!(MaterialIndex);
index!(MeshIndex);
index!(NodeIndex);
index!(SamplerIndex);
index!(SceneIndex);
index!(ShapeIndex);
index!(SkinIndex);
index!(TextureIndex);

/// Specification profile used for strict validation and transformations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationProfile {
    /// Stable glTF 2.0 core.
    Gltf20,
    /// Pinned glTF 2.1 draft described in `GLTF_2_1_SNAPSHOT.md`.
    Gltf21Draft,
}

/// Core accessor component type definitions, including the glTF 2.1 draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentType {
    /// Signed 8-bit integer.
    I8 = 5120,
    /// Unsigned 8-bit integer.
    U8 = 5121,
    /// Signed 16-bit integer.
    I16 = 5122,
    /// Unsigned 16-bit integer.
    U16 = 5123,
    /// Unsigned 32-bit integer.
    U32 = 5125,
    /// 32-bit floating-point value.
    F32 = 5126,
    /// Signed 32-bit integer.
    I32 = 5124,
    /// 16-bit floating-point value from the draft profile.
    F16 = 5131,
    /// 64-bit floating-point value from the draft profile.
    F64 = 5130,
    /// Signed 64-bit integer from the draft profile.
    I64 = 5134,
    /// Unsigned 64-bit integer from the draft profile.
    U64 = 5135,
}

impl ComponentType {
    /// Converts a glTF numeric component type code to its typed representation.
    pub fn from_gltf(value: u64) -> Option<Self> {
        Some(match value {
            5120 => Self::I8,
            5121 => Self::U8,
            5122 => Self::I16,
            5123 => Self::U16,
            5125 => Self::U32,
            5126 => Self::F32,
            5124 => Self::I32,
            5131 => Self::F16,
            5130 => Self::F64,
            5134 => Self::I64,
            5135 => Self::U64,
            _ => return None,
        })
    }

    /// Returns the on-disk width of one scalar component in bytes.
    pub const fn byte_width(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 | Self::F16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 | Self::I64 | Self::U64 => 8,
        }
    }

    /// Returns the numeric glTF component type code.
    pub const fn to_gltf(self) -> u32 {
        self as u32
    }
}

/// Typed location of a primitive nested in one mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimitiveIndex {
    /// Mesh containing the primitive.
    pub mesh: MeshIndex,
    /// Zero-based primitive position inside the mesh.
    pub primitive: usize,
}

impl PrimitiveIndex {
    /// Creates a typed nested primitive location.
    pub const fn new(mesh: MeshIndex, primitive: usize) -> Self {
        Self { mesh, primitive }
    }
}

/// Semantically lossless glTF JSON document.
#[derive(Clone, Debug)]
pub struct Document {
    root: Value,
    original_json: Option<Vec<u8>>,
}

impl Document {
    /// Parses a JSON document, retaining the exact source bytes until mutation.
    ///
    /// ```
    /// # use draco_gltf::Document;
    /// let bytes = br#"{"asset":{"version":"2.0"},"meshes":[]}"#;
    /// let document = Document::from_json_bytes(bytes)?;
    /// assert_eq!(document.meshes().len(), 0);
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        let root = Value::parse(bytes).map_err(Error::Json)?;
        if !root.is_object() {
            return Err(Error::Validation(vec!["glTF root is not an object".into()]));
        }
        Ok(Self {
            root,
            original_json: Some(bytes.to_vec()),
        })
    }

    /// Creates a document from a JSON value.
    pub fn from_value(root: Value) -> Result<Self> {
        if !root.is_object() {
            return Err(Error::Validation(vec!["glTF root is not an object".into()]));
        }
        Ok(Self {
            root,
            original_json: None,
        })
    }

    /// Returns the complete lossless JSON value.
    pub fn as_value(&self) -> &Value {
        &self.root
    }

    /// Returns mutable JSON and marks the source representation dirty.
    pub fn as_value_mut(&mut self) -> &mut Value {
        self.original_json = None;
        &mut self.root
    }

    /// Serializes JSON, preserving original bytes when the document is untouched.
    ///
    /// ```
    /// # use draco_gltf::Document;
    /// let source = br#"{ "asset": { "version": "2.0" } }"#;
    /// let document = Document::from_json_bytes(source)?;
    /// assert_eq!(document.to_json_bytes()?, source);
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        match &self.original_json {
            Some(bytes) => Ok(bytes.clone()),
            None => Ok(self.root.to_vec()),
        }
    }

    /// Serializes the current DOM without insignificant JSON whitespace.
    ///
    /// Unlike [`Document::to_json_bytes`], this always serializes the parsed
    /// value, even when the document has not been changed. Object order and
    /// number lexemes are retained; keys and numbers are not normalized.
    ///
    /// ```
    /// # use draco_gltf::Document;
    /// let document = Document::from_json_bytes(
    ///     br#"{ "asset": { "version": "2.0" } }"#,
    /// )?;
    /// assert_eq!(
    ///     document.to_minified_json_bytes(),
    ///     br#"{"asset":{"version":"2.0"}}"#,
    /// );
    /// # Ok::<(), draco_gltf::Error>(())
    /// ```
    pub fn to_minified_json_bytes(&self) -> Vec<u8> {
        self.root.to_vec()
    }

    /// Performs basic structural checks, plus strict graph checks when enabled.
    ///
    /// Enable the `strict-validation` feature to additionally check core
    /// references, node trees, and finite ordered `POSITION` bounds. Compact
    /// readers retain local bounds checks while materializing individual data.
    pub fn validate(&self, profile: ValidationProfile) -> Result<()> {
        let asset = self
            .root
            .get("asset")
            .ok_or_else(|| Error::Validation(vec!["asset is missing or not an object".into()]))?;
        let version = asset
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Validation(vec!["asset.version is missing or not a string".into()])
            })?;
        match profile {
            ValidationProfile::Gltf20 if !version.starts_with("2.0") => {
                return Err(Error::Validation(vec![format!(
                    "asset.version {version:?} is not glTF 2.0"
                )]))
            }
            ValidationProfile::Gltf21Draft if !version.starts_with("2.") => {
                return Err(Error::Validation(vec![format!(
                    "asset.version {version:?} is not glTF 2.x"
                )]))
            }
            _ => {}
        }
        for name in [
            "accessors",
            "animations",
            "buffers",
            "bufferViews",
            "cameras",
            "externalAssets",
            "files",
            "images",
            "materials",
            "meshes",
            "nodes",
            "samplers",
            "scenes",
            "shapes",
            "skins",
            "textures",
        ] {
            if let Some(value) = self.root.get(name) {
                let array = value
                    .as_array()
                    .ok_or_else(|| Error::Validation(vec![format!("{name} is not an array")]))?;
                if array.iter().any(|object| !object.is_object()) {
                    return Err(Error::Validation(vec![format!(
                        "{name} contains a non-object entry"
                    )]));
                }
            }
        }
        if profile == ValidationProfile::Gltf20
            && (self.root.get("externalAssets").is_some()
                || self.root.get("files").is_some()
                || self.root.get("shapes").is_some())
        {
            return Err(Error::Validation(vec![
                "glTF 2.1 fields require the draft profile".into(),
            ]));
        }
        #[cfg(feature = "strict-validation")]
        validate_references(&self.root, profile)?;
        Ok(())
    }

    /// Iterates over accessor objects in document order.
    pub fn accessors(&self) -> Objects<'_, AccessorIndex> {
        self.objects("accessors")
    }
    /// Returns one accessor view by typed index.
    pub fn accessor(&self, index: AccessorIndex) -> Option<Accessor<'_>> {
        self.accessors().get(index).map(Accessor)
    }
    /// Iterates over animation objects in document order.
    pub fn animations(&self) -> Objects<'_, AnimationIndex> {
        self.objects("animations")
    }
    /// Returns one animation view by typed index.
    pub fn animation(&self, index: AnimationIndex) -> Option<Animation<'_>> {
        self.animations().get(index).map(Animation)
    }
    /// Iterates over buffer objects in document order.
    pub fn buffers(&self) -> Objects<'_, BufferIndex> {
        self.objects("buffers")
    }
    /// Returns one buffer view by typed index.
    pub fn buffer(&self, index: BufferIndex) -> Option<Buffer<'_>> {
        self.buffers().get(index).map(Buffer)
    }
    /// Iterates over buffer-view objects in document order.
    pub fn buffer_views(&self) -> Objects<'_, BufferViewIndex> {
        self.objects("bufferViews")
    }
    /// Returns one buffer-view by typed index.
    pub fn buffer_view(&self, index: BufferViewIndex) -> Option<BufferView<'_>> {
        self.buffer_views().get(index).map(BufferView)
    }
    /// Iterates over camera objects in document order.
    pub fn cameras(&self) -> Objects<'_, CameraIndex> {
        self.objects("cameras")
    }
    /// Iterates over draft external-asset declarations.
    pub fn external_assets(&self) -> Objects<'_, ExternalAssetIndex> {
        self.objects("externalAssets")
    }
    /// Returns one external-asset declaration by typed index.
    pub fn external_asset(&self, index: ExternalAssetIndex) -> Option<ExternalAsset<'_>> {
        self.external_assets().get(index).map(ExternalAsset)
    }
    /// Returns one camera view by typed index.
    pub fn camera(&self, index: CameraIndex) -> Option<Camera<'_>> {
        self.cameras().get(index).map(Camera)
    }
    /// Iterates over draft packaged-file declarations.
    pub fn files(&self) -> Objects<'_, FileIndex> {
        self.objects("files")
    }
    /// Returns one packaged-file view by typed index.
    pub fn file(&self, index: FileIndex) -> Option<File<'_>> {
        self.files().get(index).map(File)
    }
    /// Iterates over image objects in document order.
    pub fn images(&self) -> Objects<'_, ImageIndex> {
        self.objects("images")
    }
    /// Returns one image view by typed index.
    pub fn image(&self, index: ImageIndex) -> Option<Image<'_>> {
        self.images().get(index).map(Image)
    }
    /// Iterates over material objects in document order.
    pub fn materials(&self) -> Objects<'_, MaterialIndex> {
        self.objects("materials")
    }
    /// Returns one material view by typed index.
    pub fn material(&self, index: MaterialIndex) -> Option<Material<'_>> {
        self.materials().get(index).map(Material)
    }
    /// Iterates over mesh objects in document order.
    pub fn meshes(&self) -> Objects<'_, MeshIndex> {
        self.objects("meshes")
    }
    /// Returns one mesh view by typed index.
    pub fn mesh(&self, index: MeshIndex) -> Option<Mesh<'_>> {
        self.meshes().get(index).map(Mesh)
    }
    /// Iterates over node objects in document order.
    pub fn nodes(&self) -> Objects<'_, NodeIndex> {
        self.objects("nodes")
    }
    /// Returns one node view by typed index.
    pub fn node(&self, index: NodeIndex) -> Option<Node<'_>> {
        self.nodes().get(index).map(Node)
    }
    /// Iterates over sampler objects in document order.
    pub fn samplers(&self) -> Objects<'_, SamplerIndex> {
        self.objects("samplers")
    }
    /// Returns one sampler view by typed index.
    pub fn sampler(&self, index: SamplerIndex) -> Option<Sampler<'_>> {
        self.samplers().get(index).map(Sampler)
    }
    /// Iterates over scene objects in document order.
    pub fn scenes(&self) -> Objects<'_, SceneIndex> {
        self.objects("scenes")
    }
    /// Returns one scene view by typed index.
    pub fn scene(&self, index: SceneIndex) -> Option<Scene<'_>> {
        self.scenes().get(index).map(Scene)
    }
    /// Returns the document's preferred scene index, if declared.
    pub fn default_scene(&self) -> Option<SceneIndex> {
        index_value(&self.root, "scene").map(SceneIndex)
    }
    /// Returns the optional draft thumbnail image declared by `asset.thumbnail`.
    pub fn thumbnail(&self) -> Option<ImageIndex> {
        self.root
            .get("asset")
            .and_then(|asset| index_value(asset, "thumbnail"))
            .map(ImageIndex)
    }
    /// Iterates over draft shape declarations.
    pub fn shapes(&self) -> Objects<'_, ShapeIndex> {
        self.objects("shapes")
    }
    /// Returns one shape view by typed index.
    pub fn shape(&self, index: ShapeIndex) -> Option<Shape<'_>> {
        self.shapes().get(index).map(Shape)
    }
    /// Iterates over skin objects in document order.
    pub fn skins(&self) -> Objects<'_, SkinIndex> {
        self.objects("skins")
    }
    /// Returns one skin view by typed index.
    pub fn skin(&self, index: SkinIndex) -> Option<Skin<'_>> {
        self.skins().get(index).map(Skin)
    }
    /// Iterates over texture objects in document order.
    pub fn textures(&self) -> Objects<'_, TextureIndex> {
        self.objects("textures")
    }
    /// Returns one texture view by typed index.
    pub fn texture(&self, index: TextureIndex) -> Option<Texture<'_>> {
        self.textures().get(index).map(Texture)
    }

    /// Returns a primitive addressed by stable mesh and primitive indices.
    pub fn primitive(&self, mesh: MeshIndex, primitive: usize) -> Option<PrimitiveRef<'_>> {
        self.meshes()
            .get(mesh)?
            .value()
            .get("primitives")?
            .as_array()?
            .get(primitive)?;
        Some(PrimitiveRef {
            document: self,
            mesh,
            primitive,
        })
    }

    fn objects<I>(&self, key: &'static str) -> Objects<'_, I> {
        Objects {
            values: self.root.get(key).and_then(Value::as_array).unwrap_or(&[]),
            marker: PhantomData,
        }
    }
}

#[cfg(feature = "strict-validation")]
fn validate_references(root: &Value, profile: ValidationProfile) -> Result<()> {
    let len = |name: &str| -> usize {
        root.get(name)
            .and_then(Value::as_array)
            .map_or(0, <[Value]>::len)
    };
    let check = |value: &Value, field: &str, target: &str| -> Result<()> {
        if let Some(raw) = value.get(field) {
            let index = raw
                .as_u64()
                .ok_or_else(|| Error::Validation(vec![format!("{field} is not an index")]))?;
            let index = usize::try_from(index).map_err(|_| {
                Error::Validation(vec![format!("{field} does not fit the platform index")])
            })?;
            if index >= len(target) {
                return Err(Error::Validation(vec![format!(
                    "{field} references missing {target}[{index}]"
                )]));
            }
        }
        Ok(())
    };
    let required_index = |value: &Value, field: &str, target: &str| -> Result<()> {
        if value.get(field).is_none() {
            return Err(Error::Validation(vec![format!("{field} is missing")]));
        }
        check(value, field, target)
    };
    for view in root
        .get("bufferViews")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        check(view, "buffer", "buffers")?;
    }
    for accessor in root
        .get("accessors")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        check(accessor, "bufferView", "bufferViews")?;
        let component = accessor
            .get("componentType")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Validation(vec!["accessor componentType is missing".into()]))?;
        let component = ComponentType::from_gltf(component).ok_or_else(|| {
            Error::Validation(vec![format!(
                "unsupported accessor componentType {component}"
            )])
        })?;
        if profile == ValidationProfile::Gltf20
            && !matches!(
                component,
                ComponentType::I8
                    | ComponentType::U8
                    | ComponentType::I16
                    | ComponentType::U16
                    | ComponentType::U32
                    | ComponentType::F32
            )
        {
            return Err(Error::Validation(vec![format!(
                "accessor componentType {component:?} requires the glTF 2.1 draft profile"
            )]));
        }
        let kind = accessor
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Validation(vec!["accessor type is missing".into()]))?;
        if !matches!(
            kind,
            "SCALAR" | "VEC2" | "VEC3" | "VEC4" | "MAT2" | "MAT3" | "MAT4"
        ) {
            return Err(Error::Validation(vec![format!(
                "unsupported accessor type {kind:?}"
            )]));
        }
        if accessor.get("count").and_then(Value::as_u64).is_none() {
            return Err(Error::Validation(vec![
                "accessor count is missing or invalid".into(),
            ]));
        }
    }
    for image in root.get("images").and_then(Value::as_array).unwrap_or(&[]) {
        check(image, "bufferView", "bufferViews")?;
    }
    if let Some(asset) = root.get("asset") {
        check(asset, "thumbnail", "images")?;
    }
    for file in root.get("files").and_then(Value::as_array).unwrap_or(&[]) {
        if file.get("mimeType").and_then(Value::as_str).is_none() {
            return Err(Error::Validation(vec![
                "file mimeType is missing or not a string".into(),
            ]));
        }
        let has_uri = match file.get("uri") {
            Some(value) if value.as_str().is_some() => true,
            Some(_) => {
                return Err(Error::Validation(vec!["file uri is not a string".into()]));
            }
            None => false,
        };
        let has_buffer_view = match file.get("bufferView") {
            Some(value) if value.as_u64().is_some() => {
                check(file, "bufferView", "bufferViews")?;
                true
            }
            Some(_) => {
                return Err(Error::Validation(vec![
                    "file bufferView is not an index".into()
                ]));
            }
            None => false,
        };
        if has_uri == has_buffer_view {
            return Err(Error::Validation(vec![
                "file must contain exactly one of uri or bufferView".into(),
            ]));
        }
    }
    for asset in root
        .get("externalAssets")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        if asset.get("file").and_then(Value::as_u64).is_none() {
            return Err(Error::Validation(vec![
                "external asset file is missing or not an index".into(),
            ]));
        }
        check(asset, "file", "files")?;
    }
    for texture in root
        .get("textures")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        check(texture, "sampler", "samplers")?;
        check(texture, "source", "images")?;
    }
    for mesh in root.get("meshes").and_then(Value::as_array).unwrap_or(&[]) {
        for primitive in mesh
            .get("primitives")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            check(primitive, "indices", "accessors")?;
            check(primitive, "material", "materials")?;
            if let Some(attributes) = primitive.get("attributes").and_then(Value::as_object) {
                for (semantic, index) in attributes {
                    let index = index.as_u64().ok_or_else(|| {
                        Error::Validation(vec![format!(
                            "attribute {semantic} is not an accessor index"
                        )])
                    })?;
                    if usize::try_from(index)
                        .ok()
                        .is_none_or(|index| index >= len("accessors"))
                    {
                        return Err(Error::Validation(vec![format!(
                            "attribute {semantic} references missing accessors[{index}]"
                        )]));
                    }
                    if semantic == "POSITION" {
                        validate_position_accessor(
                            root,
                            usize::try_from(index).expect("accessor index was range checked"),
                        )?;
                    }
                }
            }
        }
    }
    for node in root.get("nodes").and_then(Value::as_array).unwrap_or(&[]) {
        for field in ["camera", "mesh", "skin"] {
            let target = match field {
                "camera" => "cameras",
                "mesh" => "meshes",
                _ => "skins",
            };
            check(node, field, target)?;
        }
        check(node, "externalAsset", "externalAssets")?;
        if let Some(volume) = node.get("boundingVolume") {
            if !volume.is_object() {
                return Err(Error::Validation(vec![
                    "node boundingVolume is not an object".into(),
                ]));
            }
            if volume.get("shape").and_then(Value::as_u64).is_none() {
                return Err(Error::Validation(vec![
                    "node boundingVolume shape is missing or not an index".into(),
                ]));
            }
            check(volume, "shape", "shapes")?;
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                if child.as_u64().is_none_or(|index| {
                    usize::try_from(index)
                        .ok()
                        .is_none_or(|index| index >= len("nodes"))
                }) {
                    return Err(Error::Validation(vec![
                        "node child references a missing node".into(),
                    ]));
                }
            }
        }
    }
    for scene in root.get("scenes").and_then(Value::as_array).unwrap_or(&[]) {
        if let Some(nodes) = scene.get("nodes").and_then(Value::as_array) {
            for node in nodes {
                if node.as_u64().is_none_or(|index| {
                    usize::try_from(index)
                        .ok()
                        .is_none_or(|index| index >= len("nodes"))
                }) {
                    return Err(Error::Validation(vec![
                        "scene references a missing node".into()
                    ]));
                }
            }
        }
    }
    validate_node_hierarchy(root)?;
    check(root, "scene", "scenes")?;
    for skin in root.get("skins").and_then(Value::as_array).unwrap_or(&[]) {
        check(skin, "inverseBindMatrices", "accessors")?;
        check(skin, "skeleton", "nodes")?;
        if let Some(joints) = skin.get("joints").and_then(Value::as_array) {
            for joint in joints {
                if joint.as_u64().is_none_or(|index| {
                    usize::try_from(index)
                        .ok()
                        .is_none_or(|index| index >= len("nodes"))
                }) {
                    return Err(Error::Validation(vec![
                        "skin references a missing joint node".into(),
                    ]));
                }
            }
        }
    }
    for mesh in root.get("meshes").and_then(Value::as_array).unwrap_or(&[]) {
        for primitive in mesh
            .get("primitives")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            for target in primitive
                .get("targets")
                .and_then(Value::as_array)
                .unwrap_or(&[])
            {
                if let Some(attributes) = target.as_object() {
                    for (semantic, index) in attributes {
                        if index.as_u64().is_none_or(|index| {
                            usize::try_from(index)
                                .ok()
                                .is_none_or(|index| index >= len("accessors"))
                        }) {
                            return Err(Error::Validation(vec![format!(
                                "morph target attribute {semantic} references a missing accessor"
                            )]));
                        }
                    }
                }
            }
        }
    }
    for animation in root
        .get("animations")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        let samplers = animation
            .get("samplers")
            .and_then(Value::as_array)
            .unwrap_or(&[]);
        for sampler in samplers {
            required_index(sampler, "input", "accessors")?;
            required_index(sampler, "output", "accessors")?;
            if let Some(interpolation) = sampler.get("interpolation").and_then(Value::as_str) {
                if !matches!(interpolation, "LINEAR" | "STEP" | "CUBICSPLINE") {
                    return Err(Error::Validation(vec![format!(
                        "animation sampler interpolation {interpolation:?} is invalid"
                    )]));
                }
            }
        }
        for channel in animation
            .get("channels")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            let sampler = channel.get("sampler").and_then(Value::as_u64);
            if sampler.is_none_or(|index| {
                usize::try_from(index)
                    .ok()
                    .is_none_or(|index| index >= samplers.len())
            }) {
                return Err(Error::Validation(vec![
                    "animation channel references a missing sampler".into(),
                ]));
            }
            if let Some(target) = channel.get("target") {
                check(target, "node", "nodes")?;
                let path = target.get("path").and_then(Value::as_str).ok_or_else(|| {
                    Error::Validation(vec!["animation channel target path is missing".into()])
                })?;
                // `pointer` is the path KHR_animation_pointer defines, and it
                // targets a JSON pointer rather than a node, which is why the
                // node index is absent on such a channel. The extension is
                // ratified, so a file using it is valid whether or not a
                // reader animates it: rejecting the document would lose the
                // geometry over an animation the reader was free to skip.
                let pointed = path == "pointer"
                    && target
                        .get("extensions")
                        .and_then(|extensions| extensions.get("KHR_animation_pointer"))
                        .is_some();
                if !pointed && !matches!(path, "translation" | "rotation" | "scale" | "weights") {
                    return Err(Error::Validation(vec![format!(
                        "animation channel target path {path:?} is invalid"
                    )]));
                }
            } else {
                return Err(Error::Validation(vec![
                    "animation channel target is missing".into(),
                ]));
            }
        }
    }
    validate_draco_extension(root, &check)?;
    if profile == ValidationProfile::Gltf21Draft {
        validate_shapes(root)?;
        validate_uids(root)?;
    }
    Ok(())
}

#[cfg(feature = "strict-validation")]
fn validate_position_accessor(root: &Value, index: usize) -> Result<()> {
    let accessor = root
        .get("accessors")
        .and_then(Value::as_array)
        .and_then(|accessors| accessors.get(index))
        .ok_or_else(|| Error::Validation(vec![format!("POSITION accessor {index} is missing")]))?;
    if accessor.get("type").and_then(Value::as_str) != Some("VEC3") {
        return Err(Error::Validation(vec![format!(
            "POSITION accessor {index} must have type VEC3"
        )]));
    }

    let mut bounds = Vec::with_capacity(2);
    for field in ["min", "max"] {
        let values = accessor
            .get(field)
            .and_then(Value::as_array)
            .filter(|values| values.len() == 3)
            .ok_or_else(|| {
                Error::Validation(vec![format!(
                    "POSITION accessor {index} must define three-component {field} bounds"
                )])
            })?;
        let values = values
            .iter()
            .map(|value| value.as_f64().filter(|value| value.is_finite()))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                Error::Validation(vec![format!(
                    "POSITION accessor {index} {field} bounds must be finite numbers"
                )])
            })?;
        bounds.push(values);
    }
    if bounds[0].iter().zip(&bounds[1]).any(|(min, max)| min > max) {
        return Err(Error::Validation(vec![format!(
            "POSITION accessor {index} min bounds exceed max bounds"
        )]));
    }
    Ok(())
}

#[cfg(feature = "strict-validation")]
fn validate_node_hierarchy(root: &Value) -> Result<()> {
    let nodes = root.get("nodes").and_then(Value::as_array).unwrap_or(&[]);
    let mut parents = vec![None; nodes.len()];
    let mut edges = vec![Vec::new(); nodes.len()];

    for (parent, node) in nodes.iter().enumerate() {
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            let child = child
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|child| *child < nodes.len())
                .ok_or_else(|| {
                    Error::Validation(vec!["node child references a missing node".into()])
                })?;
            if let Some(existing) = parents[child] {
                let message = if existing == parent {
                    format!("nodes[{parent}] lists child nodes[{child}] more than once")
                } else {
                    format!(
                        "nodes[{child}] has multiple parents: nodes[{existing}] and nodes[{parent}]"
                    )
                };
                return Err(Error::Validation(vec![message]));
            }
            parents[child] = Some(parent);
            edges[parent].push(child);
        }
    }

    // Iterative depth-first traversal avoids consuming the call stack on large
    // scenes while still detecting back edges in disconnected node trees.
    let mut state = vec![0u8; nodes.len()];
    for start in 0..nodes.len() {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        let mut stack = vec![(start, 0usize)];
        while let Some((node, next_child)) = stack.last_mut() {
            if *next_child == edges[*node].len() {
                state[*node] = 2;
                stack.pop();
                continue;
            }
            let child = edges[*node][*next_child];
            *next_child += 1;
            match state[child] {
                0 => {
                    state[child] = 1;
                    stack.push((child, 0));
                }
                1 => {
                    return Err(Error::Validation(vec![format!(
                        "node hierarchy contains a cycle through nodes[{child}]"
                    )]))
                }
                _ => {}
            }
        }
    }

    for (scene_index, scene) in root
        .get("scenes")
        .and_then(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        for root_node in scene.get("nodes").and_then(Value::as_array).unwrap_or(&[]) {
            let root_node = root_node
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|node| *node < nodes.len())
                .ok_or_else(|| Error::Validation(vec!["scene references a missing node".into()]))?;
            if let Some(parent) = parents[root_node] {
                return Err(Error::Validation(vec![format!(
                    "scenes[{scene_index}] uses nodes[{root_node}] as a root, but it is a child of nodes[{parent}]"
                )]));
            }
        }
    }

    Ok(())
}

#[cfg(feature = "strict-validation")]
fn validate_draco_extension(
    root: &Value,
    check: &impl Fn(&Value, &str, &str) -> Result<()>,
) -> Result<()> {
    const NAME: &str = crate::KHR_DRACO_MESH_COMPRESSION;
    let listed = |field: &str| {
        root.get(field)
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(NAME)))
    };
    let required = listed("extensionsRequired");
    for mesh in root.get("meshes").and_then(Value::as_array).unwrap_or(&[]) {
        for primitive in mesh
            .get("primitives")
            .and_then(Value::as_array)
            .unwrap_or(&[])
        {
            let Some(extension) = primitive
                .get("extensions")
                .and_then(|value| value.get(NAME))
            else {
                continue;
            };
            if !listed("extensionsUsed") {
                return Err(Error::Validation(vec![
                    "KHR_draco_mesh_compression is missing from extensionsUsed".into(),
                ]));
            }
            let mode = primitive.get("mode").and_then(Value::as_u64).unwrap_or(4);
            if !matches!(mode, 4 | 5) {
                return Err(Error::Validation(vec![
                    "KHR_draco_mesh_compression requires TRIANGLES or TRIANGLE_STRIP".into(),
                ]));
            }
            check(extension, "bufferView", "bufferViews")?;
            let attributes = extension
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    Error::Validation(vec!["Draco extension attributes is not an object".into()])
                })?;
            let primitive_attributes = primitive
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    Error::Validation(vec!["Draco primitive attributes is not an object".into()])
                })?;
            let mut unique_ids = std::collections::BTreeSet::new();
            for (semantic, unique_id) in attributes {
                let unique_id = unique_id
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        Error::Validation(vec![format!(
                            "Draco attribute {semantic:?} unique id is not a u32"
                        )])
                    })?;
                if !unique_ids.insert(unique_id) {
                    return Err(Error::Validation(vec![format!(
                        "Draco unique id {unique_id} is mapped more than once"
                    )]));
                }
                if primitive_attributes
                    .iter()
                    .all(|(name, _)| name != semantic)
                {
                    return Err(Error::Validation(vec![format!(
                        "Draco attribute {semantic:?} is absent from primitive attributes"
                    )]));
                }
            }
            if required {
                for (semantic, _) in attributes {
                    let accessor = primitive_attributes
                        .iter()
                        .find(|(name, _)| name == semantic)
                        .and_then(|(_, value)| value.as_u64())
                        .and_then(|value| usize::try_from(value).ok())
                        .and_then(|index| {
                            root.get("accessors").and_then(Value::as_array)?.get(index)
                        })
                        .ok_or_else(|| {
                            Error::Validation(vec!["Draco accessor is invalid".into()])
                        })?;
                    if accessor.get("bufferView").is_some() || accessor.get("sparse").is_some() {
                        return Err(Error::Validation(vec![
                            "Draco-only accessor must not retain raw buffer data".into(),
                        ]));
                    }
                }
                if let Some(index) = primitive.get("indices") {
                    let accessor = index
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .and_then(|index| {
                            root.get("accessors").and_then(Value::as_array)?.get(index)
                        })
                        .ok_or_else(|| {
                            Error::Validation(vec!["Draco index accessor is invalid".into()])
                        })?;
                    if accessor.get("bufferView").is_some() || accessor.get("sparse").is_some() {
                        return Err(Error::Validation(vec![
                            "Draco-only index accessor must not retain raw buffer data".into(),
                        ]));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "strict-validation")]
fn validate_shapes(root: &Value) -> Result<()> {
    const CORE_TYPES: [&str; 5] = ["box", "capsule", "cylinder", "plane", "sphere"];
    for shape in root.get("shapes").and_then(Value::as_array).unwrap_or(&[]) {
        let kind = shape.get("type").and_then(Value::as_str).ok_or_else(|| {
            Error::Validation(vec!["shape type is missing or not a string".into()])
        })?;
        if CORE_TYPES.contains(&kind) && !shape.get(kind).is_some_and(Value::is_object) {
            return Err(Error::Validation(vec![format!(
                "shape {kind:?} is missing its {kind:?} definition object"
            )]));
        }
    }
    Ok(())
}

#[cfg(feature = "strict-validation")]
fn validate_uids(root: &Value) -> Result<()> {
    use std::collections::BTreeMap;

    let mut names = BTreeMap::new();
    let mut uids = BTreeMap::new();
    for kind in [
        "accessors",
        "animations",
        "buffers",
        "bufferViews",
        "cameras",
        "externalAssets",
        "files",
        "images",
        "materials",
        "meshes",
        "nodes",
        "samplers",
        "scenes",
        "shapes",
        "skins",
        "textures",
    ] {
        for (index, value) in root
            .get(kind)
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let location = format!("{kind}[{index}]");
            if let Some(name) = value.get("name").and_then(Value::as_str) {
                names.insert(name, location.clone());
            }
            if let Some(uid) = value.get("uid") {
                let uid = uid.as_str().ok_or_else(|| {
                    Error::Validation(vec![format!("{location}.uid is not a string")])
                })?;
                if let Some(previous) = uids.insert(uid, location.clone()) {
                    return Err(Error::Validation(vec![format!(
                        "{location}.uid duplicates {previous}.uid"
                    )]));
                }
            }
        }
    }
    for (uid, location) in &uids {
        if let Some(named) = names.get(uid) {
            if named != location {
                return Err(Error::Validation(vec![format!(
                    "{location}.uid conflicts with {named}.name"
                )]));
            }
        }
    }
    Ok(())
}

/// Typed view of a root-level glTF object.
#[derive(Clone, Copy)]
pub struct ObjectRef<'a, I> {
    index: I,
    value: &'a Value,
}
impl<'a, I: Copy> ObjectRef<'a, I> {
    /// Returns the typed index of this object.
    pub fn index(self) -> I {
        self.index
    }
    /// Returns the underlying lossless JSON object.
    pub fn value(self) -> &'a Value {
        self.value
    }
    /// Returns the optional glTF object name.
    pub fn name(self) -> Option<&'a str> {
        self.value.get("name").and_then(Value::as_str)
    }
    /// Returns the optional draft UID.
    pub fn uid(self) -> Option<&'a str> {
        self.value.get("uid").and_then(Value::as_str)
    }
    /// Returns the object's unknown or extension fields.
    pub fn extensions(self) -> Option<&'a [(String, Value)]> {
        self.value.get("extensions").and_then(Value::as_object)
    }
    /// Returns the object's application-defined extras value.
    pub fn extras(self) -> Option<&'a Value> {
        self.value.get("extras")
    }
}

macro_rules! typed_object {
    ($name:ident, $index:ident) => {
        #[derive(Clone, Copy)]
        #[doc = concat!("Typed view of a glTF `", stringify!($name), "` object.")]
        pub struct $name<'a>(ObjectRef<'a, $index>);
        impl<'a> $name<'a> {
            /// Returns the typed index of this object.
            pub fn index(self) -> $index {
                self.0.index()
            }
            /// Returns the underlying lossless JSON object.
            pub fn value(self) -> &'a Value {
                self.0.value()
            }
            /// Returns the optional glTF object name.
            pub fn name(self) -> Option<&'a str> {
                self.0.name()
            }
            /// Returns the optional draft UID.
            pub fn uid(self) -> Option<&'a str> {
                self.0.uid()
            }
            /// Returns the object's application-defined extras value.
            pub fn extras(self) -> Option<&'a Value> {
                self.0.extras()
            }
            /// Returns the object's unknown or extension fields.
            pub fn extensions(self) -> Option<&'a [(String, Value)]> {
                self.0.extensions()
            }
        }
    };
}

typed_object!(Accessor, AccessorIndex);
typed_object!(Animation, AnimationIndex);
typed_object!(Buffer, BufferIndex);
typed_object!(BufferView, BufferViewIndex);
typed_object!(Camera, CameraIndex);
typed_object!(ExternalAsset, ExternalAssetIndex);
typed_object!(File, FileIndex);
typed_object!(Image, ImageIndex);
typed_object!(Material, MaterialIndex);
typed_object!(Mesh, MeshIndex);
typed_object!(Node, NodeIndex);
typed_object!(Sampler, SamplerIndex);
typed_object!(Scene, SceneIndex);
typed_object!(Shape, ShapeIndex);
typed_object!(Skin, SkinIndex);
typed_object!(Texture, TextureIndex);

impl<'a> Buffer<'a> {
    /// Returns the declared buffer length.
    pub fn byte_length(self) -> Option<u64> {
        self.value().get("byteLength").and_then(Value::as_u64)
    }
    /// Returns the optional external buffer URI.
    pub fn uri(self) -> Option<&'a str> {
        self.value().get("uri").and_then(Value::as_str)
    }
}

impl<'a> BufferView<'a> {
    /// Returns the referenced buffer index.
    pub fn buffer(self) -> Option<BufferIndex> {
        index_value(self.value(), "buffer").map(BufferIndex)
    }
    /// Returns the byte offset, defaulting to zero when omitted.
    pub fn byte_offset(self) -> u64 {
        self.value()
            .get("byteOffset")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }
    /// Returns the declared view length.
    pub fn byte_length(self) -> Option<u64> {
        self.value().get("byteLength").and_then(Value::as_u64)
    }
    /// Returns the optional interleaved byte stride.
    pub fn byte_stride(self) -> Option<u64> {
        self.value().get("byteStride").and_then(Value::as_u64)
    }
}

impl<'a> Accessor<'a> {
    /// Returns the optional source buffer-view index.
    pub fn buffer_view(self) -> Option<BufferViewIndex> {
        index_value(self.value(), "bufferView").map(BufferViewIndex)
    }
    /// Returns the number of accessor elements.
    pub fn count(self) -> Option<u64> {
        self.value().get("count").and_then(Value::as_u64)
    }
    /// Returns the typed component format.
    pub fn component_type(self) -> Option<ComponentType> {
        self.value()
            .get("componentType")
            .and_then(Value::as_u64)
            .and_then(ComponentType::from_gltf)
    }
    /// Returns the accessor shape such as `SCALAR` or `VEC3`.
    pub fn accessor_type(self) -> Option<&'a str> {
        self.value().get("type").and_then(Value::as_str)
    }
    /// Returns whether integer values are normalized on read.
    pub fn normalized(self) -> bool {
        matches!(self.value().get("normalized"), Some(Value::Bool(true)))
    }
}

impl<'a> Image<'a> {
    /// Returns the optional image URI.
    pub fn uri(self) -> Option<&'a str> {
        self.value().get("uri").and_then(Value::as_str)
    }
    /// Returns the optional image buffer-view index.
    pub fn buffer_view(self) -> Option<BufferViewIndex> {
        index_value(self.value(), "bufferView").map(BufferViewIndex)
    }
}

impl<'a> Texture<'a> {
    /// Returns the optional source image index.
    pub fn source(self) -> Option<ImageIndex> {
        index_value(self.value(), "source").map(ImageIndex)
    }
    /// Returns the optional sampler index.
    pub fn sampler(self) -> Option<SamplerIndex> {
        index_value(self.value(), "sampler").map(SamplerIndex)
    }
}

impl<'a> Node<'a> {
    /// Returns the optional mesh index.
    pub fn mesh(self) -> Option<MeshIndex> {
        index_value(self.value(), "mesh").map(MeshIndex)
    }
    /// Returns the optional camera index.
    pub fn camera(self) -> Option<CameraIndex> {
        index_value(self.value(), "camera").map(CameraIndex)
    }
    /// Returns the optional skin index.
    pub fn skin(self) -> Option<SkinIndex> {
        index_value(self.value(), "skin").map(SkinIndex)
    }
    /// Returns the optional external-asset index.
    pub fn external_asset(self) -> Option<ExternalAssetIndex> {
        index_value(self.value(), "externalAsset").map(ExternalAssetIndex)
    }
    /// Returns the optional node bounding-volume view.
    pub fn bounding_volume(self) -> Option<BoundingVolume<'a>> {
        self.value()
            .get("boundingVolume")
            .filter(|value| value.is_object())
            .map(BoundingVolume)
    }
    /// Iterates over child node indexes.
    pub fn children(self) -> impl Iterator<Item = NodeIndex> + 'a {
        self.value()
            .get("children")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(Value::as_u64)
            .filter_map(|index| usize::try_from(index).ok())
            .map(NodeIndex)
    }
}

impl<'a> Scene<'a> {
    /// Iterates over the scene's root node indexes.
    pub fn nodes(self) -> impl Iterator<Item = NodeIndex> + 'a {
        self.value()
            .get("nodes")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(Value::as_u64)
            .filter_map(|index| usize::try_from(index).ok())
            .map(NodeIndex)
    }
}

impl<'a> File<'a> {
    /// Returns the declared file MIME type.
    pub fn mime_type(self) -> Option<&'a str> {
        self.value().get("mimeType").and_then(Value::as_str)
    }
    /// Returns the optional external file URI.
    pub fn uri(self) -> Option<&'a str> {
        self.value().get("uri").and_then(Value::as_str)
    }
    /// Returns the optional embedded buffer-view index.
    pub fn buffer_view(self) -> Option<BufferViewIndex> {
        index_value(self.value(), "bufferView").map(BufferViewIndex)
    }
}

impl<'a> ExternalAsset<'a> {
    /// Returns the packaged file index backing this asset.
    pub fn file(self) -> Option<FileIndex> {
        index_value(self.value(), "file").map(FileIndex)
    }
}

/// Typed view of a node's draft bounding-volume object.
#[derive(Clone, Copy)]
pub struct BoundingVolume<'a>(&'a Value);
impl<'a> BoundingVolume<'a> {
    /// Returns the underlying bounding-volume JSON object.
    pub fn value(self) -> &'a Value {
        self.0
    }
    /// Returns the referenced draft shape index.
    pub fn shape(self) -> Option<ShapeIndex> {
        index_value(self.0, "shape").map(ShapeIndex)
    }
}

impl<'a> Shape<'a> {
    /// Returns the shape discriminator.
    pub fn shape_type(self) -> Option<&'a str> {
        self.value().get("type").and_then(Value::as_str)
    }
    /// Returns the shape definition under its discriminator key.
    pub fn definition(self) -> Option<&'a Value> {
        self.shape_type().and_then(|kind| self.value().get(kind))
    }
}

impl<'a> Mesh<'a> {
    /// Returns the number of primitives in the mesh.
    pub fn primitive_count(self) -> usize {
        self.value()
            .get("primitives")
            .and_then(Value::as_array)
            .map_or(0, <[Value]>::len)
    }
}

fn index_value(value: &Value, name: &str) -> Option<usize> {
    value
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
}

/// Typed iterator over a root-level glTF array.
pub struct Objects<'a, I> {
    values: &'a [Value],
    marker: PhantomData<I>,
}
impl<'a, I: From<usize> + Into<usize> + Copy> Objects<'a, I> {
    /// Returns the number of objects in the array.
    pub fn len(&self) -> usize {
        self.values.len()
    }
    /// Returns whether the array contains no objects.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    /// Returns an untyped object view for a typed array index.
    pub fn get(&self, index: I) -> Option<ObjectRef<'a, I>> {
        let index = index.into();
        self.values.get(index).map(|value| ObjectRef {
            index: I::from(index),
            value,
        })
    }
}
impl<'a, I: From<usize> + Into<usize> + Copy> IntoIterator for Objects<'a, I> {
    type Item = ObjectRef<'a, I>;
    type IntoIter = std::iter::Map<
        std::iter::Enumerate<std::slice::Iter<'a, Value>>,
        fn((usize, &'a Value)) -> ObjectRef<'a, I>,
    >;
    fn into_iter(self) -> Self::IntoIter {
        fn make<I: From<usize> + Into<usize> + Copy>(
            (index, value): (usize, &Value),
        ) -> ObjectRef<'_, I> {
            ObjectRef {
                index: I::from(index),
                value,
            }
        }
        self.values.iter().enumerate().map(make::<I>)
    }
}

macro_rules! index_conversions { ($($name:ident),+ $(,)?) => { $(impl From<usize> for $name { fn from(value: usize) -> Self { Self(value) } } impl From<$name> for usize { fn from(value: $name) -> Self { value.0 } })+ }; }
index_conversions!(
    AccessorIndex,
    AnimationIndex,
    BufferIndex,
    BufferViewIndex,
    CameraIndex,
    ExternalAssetIndex,
    FileIndex,
    ImageIndex,
    MaterialIndex,
    MeshIndex,
    NodeIndex,
    SamplerIndex,
    SceneIndex,
    ShapeIndex,
    SkinIndex,
    TextureIndex
);

/// Stable reference to a primitive in the lossless document.
#[derive(Clone, Copy)]
pub struct PrimitiveRef<'a> {
    document: &'a Document,
    mesh: MeshIndex,
    primitive: usize,
}
impl<'a> PrimitiveRef<'a> {
    /// Returns the mesh containing this primitive.
    pub fn mesh_index(self) -> MeshIndex {
        self.mesh
    }
    /// Returns the primitive's zero-based position within its mesh.
    pub fn primitive_index(self) -> usize {
        self.primitive
    }
    /// Returns the primitive's lossless JSON object.
    pub fn value(self) -> &'a Value {
        &self.document.as_value()["meshes"][self.mesh.0]["primitives"][self.primitive]
    }
    /// Returns the primitive attribute map.
    pub fn attributes(self) -> Option<&'a [(String, Value)]> {
        self.value().get("attributes").and_then(Value::as_object)
    }
    /// Iterates over named primitive attribute accessor indexes.
    pub fn attribute_indices(self) -> impl Iterator<Item = (&'a str, AccessorIndex)> + 'a {
        self.attributes()
            .unwrap_or(&[])
            .iter()
            .filter_map(|(semantic, value)| {
                value
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .map(|index| (semantic.as_str(), AccessorIndex(index)))
            })
    }
    /// Returns the optional index accessor.
    pub fn indices(self) -> Option<AccessorIndex> {
        index_value(self.value(), "indices").map(AccessorIndex)
    }
    /// Returns the optional material index.
    pub fn material(self) -> Option<MaterialIndex> {
        index_value(self.value(), "material").map(MaterialIndex)
    }
    /// Returns the primitive mode, defaulting to TRIANGLES (4).
    pub fn mode(self) -> u32 {
        self.value()
            .get("mode")
            .and_then(Value::as_u64)
            .and_then(|mode| u32::try_from(mode).ok())
            .unwrap_or(4)
    }
    /// Iterates over morph-target attribute maps.
    pub fn morph_targets(self) -> impl Iterator<Item = &'a [(String, Value)]> + 'a {
        self.value()
            .get("targets")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(Value::as_object)
    }
    /// Returns a named primitive extension payload.
    pub fn extension(self, name: &str) -> Option<&'a Value> {
        self.value().get("extensions")?.get(name)
    }
}
