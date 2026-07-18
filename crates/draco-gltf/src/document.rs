//! Lossless glTF document model and typed object views.
//!
//! The JSON DOM is canonical: typed views deliberately never own a second
//! schema copy, so draft fields and unknown extensions survive edits.

use std::marker::PhantomData;

use crate::json::Value;

use crate::{Error, Result};

macro_rules! index {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);
        impl $name {
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
    I8 = 5120,
    U8 = 5121,
    I16 = 5122,
    U16 = 5123,
    U32 = 5125,
    F32 = 5126,
    I32 = 5127,
    F16 = 5131,
    F64 = 5132,
    I64 = 5133,
    U64 = 5134,
}

impl ComponentType {
    pub fn from_gltf(value: u64) -> Option<Self> {
        Some(match value {
            5120 => Self::I8,
            5121 => Self::U8,
            5122 => Self::I16,
            5123 => Self::U16,
            5125 => Self::U32,
            5126 => Self::F32,
            5127 => Self::I32,
            5131 => Self::F16,
            5132 => Self::F64,
            5133 => Self::I64,
            5134 => Self::U64,
            _ => return None,
        })
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

    /// Returns the complete canonical JSON value.
    pub fn as_value(&self) -> &Value {
        &self.root
    }

    /// Returns mutable JSON and marks the source representation dirty.
    pub fn as_value_mut(&mut self) -> &mut Value {
        self.original_json = None;
        &mut self.root
    }

    /// Serializes JSON, preserving original bytes when the document is untouched.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        match &self.original_json {
            Some(bytes) => Ok(bytes.clone()),
            None => Ok(self.root.to_vec()),
        }
    }

    /// Performs the core structural checks required before transforming a document.
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
            && (self.root.get("files").is_some() || self.root.get("shapes").is_some())
        {
            return Err(Error::Validation(vec![
                "glTF 2.1 fields require the draft profile".into(),
            ]));
        }
        validate_references(&self.root, profile)?;
        Ok(())
    }

    pub fn accessors(&self) -> Objects<'_, AccessorIndex> {
        self.objects("accessors")
    }
    pub fn accessor(&self, index: AccessorIndex) -> Option<Accessor<'_>> {
        self.accessors().get(index).map(Accessor)
    }
    pub fn animations(&self) -> Objects<'_, AnimationIndex> {
        self.objects("animations")
    }
    pub fn animation(&self, index: AnimationIndex) -> Option<Animation<'_>> {
        self.animations().get(index).map(Animation)
    }
    pub fn buffers(&self) -> Objects<'_, BufferIndex> {
        self.objects("buffers")
    }
    pub fn buffer(&self, index: BufferIndex) -> Option<Buffer<'_>> {
        self.buffers().get(index).map(Buffer)
    }
    pub fn buffer_views(&self) -> Objects<'_, BufferViewIndex> {
        self.objects("bufferViews")
    }
    pub fn buffer_view(&self, index: BufferViewIndex) -> Option<BufferView<'_>> {
        self.buffer_views().get(index).map(BufferView)
    }
    pub fn cameras(&self) -> Objects<'_, CameraIndex> {
        self.objects("cameras")
    }
    pub fn camera(&self, index: CameraIndex) -> Option<Camera<'_>> {
        self.cameras().get(index).map(Camera)
    }
    pub fn files(&self) -> Objects<'_, FileIndex> {
        self.objects("files")
    }
    pub fn file(&self, index: FileIndex) -> Option<File<'_>> {
        self.files().get(index).map(File)
    }
    pub fn images(&self) -> Objects<'_, ImageIndex> {
        self.objects("images")
    }
    pub fn image(&self, index: ImageIndex) -> Option<Image<'_>> {
        self.images().get(index).map(Image)
    }
    pub fn materials(&self) -> Objects<'_, MaterialIndex> {
        self.objects("materials")
    }
    pub fn material(&self, index: MaterialIndex) -> Option<Material<'_>> {
        self.materials().get(index).map(Material)
    }
    pub fn meshes(&self) -> Objects<'_, MeshIndex> {
        self.objects("meshes")
    }
    pub fn mesh(&self, index: MeshIndex) -> Option<Mesh<'_>> {
        self.meshes().get(index).map(Mesh)
    }
    pub fn nodes(&self) -> Objects<'_, NodeIndex> {
        self.objects("nodes")
    }
    pub fn node(&self, index: NodeIndex) -> Option<Node<'_>> {
        self.nodes().get(index).map(Node)
    }
    pub fn samplers(&self) -> Objects<'_, SamplerIndex> {
        self.objects("samplers")
    }
    pub fn sampler(&self, index: SamplerIndex) -> Option<Sampler<'_>> {
        self.samplers().get(index).map(Sampler)
    }
    pub fn scenes(&self) -> Objects<'_, SceneIndex> {
        self.objects("scenes")
    }
    pub fn scene(&self, index: SceneIndex) -> Option<Scene<'_>> {
        self.scenes().get(index).map(Scene)
    }
    pub fn shapes(&self) -> Objects<'_, ShapeIndex> {
        self.objects("shapes")
    }
    pub fn shape(&self, index: ShapeIndex) -> Option<Shape<'_>> {
        self.shapes().get(index).map(Shape)
    }
    pub fn skins(&self) -> Objects<'_, SkinIndex> {
        self.objects("skins")
    }
    pub fn skin(&self, index: SkinIndex) -> Option<Skin<'_>> {
        self.skins().get(index).map(Skin)
    }
    pub fn textures(&self) -> Objects<'_, TextureIndex> {
        self.objects("textures")
    }
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
            values: self
                .root
                .get(key)
                .and_then(Value::as_array)
                .map(|value| &value[..])
                .unwrap_or(&[]),
            marker: PhantomData,
        }
    }
}

fn validate_references(root: &Value, profile: ValidationProfile) -> Result<()> {
    let len = |name: &str| -> usize {
        root.get(name)
            .and_then(Value::as_array)
            .map_or(0, <[Value]>::len)
    };
    let check = |value: &Value, field: &str, target: &str| -> Result<()> {
        let index = value.get(field).and_then(Value::as_u64);
        if let Some(index) = index {
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
        let component = accessor.get("componentType").and_then(Value::as_u64);
        if let Some(component) = component {
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
        }
        if let Some(kind) = accessor.get("type").and_then(Value::as_str) {
            if !matches!(
                kind,
                "SCALAR" | "VEC2" | "VEC3" | "VEC4" | "MAT2" | "MAT3" | "MAT4"
            ) {
                return Err(Error::Validation(vec![format!(
                    "unsupported accessor type {kind:?}"
                )]));
            }
        }
    }
    for image in root.get("images").and_then(Value::as_array).unwrap_or(&[]) {
        check(image, "bufferView", "bufferViews")?;
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
    Ok(())
}

/// Typed view of a root-level glTF object.
#[derive(Clone, Copy)]
pub struct ObjectRef<'a, I> {
    index: I,
    value: &'a Value,
}
impl<'a, I: Copy> ObjectRef<'a, I> {
    pub fn index(self) -> I {
        self.index
    }
    pub fn value(self) -> &'a Value {
        self.value
    }
    pub fn name(self) -> Option<&'a str> {
        self.value.get("name").and_then(Value::as_str)
    }
    pub fn uid(self) -> Option<&'a str> {
        self.value.get("uid").and_then(Value::as_str)
    }
    pub fn extensions(self) -> Option<&'a [(String, Value)]> {
        self.value.get("extensions").and_then(Value::as_object)
    }
    pub fn extras(self) -> Option<&'a Value> {
        self.value.get("extras")
    }
}

macro_rules! typed_object {
    ($name:ident, $index:ident) => {
        #[derive(Clone, Copy)]
        pub struct $name<'a>(ObjectRef<'a, $index>);
        impl<'a> $name<'a> {
            pub fn index(self) -> $index {
                self.0.index()
            }
            pub fn value(self) -> &'a Value {
                self.0.value()
            }
            pub fn name(self) -> Option<&'a str> {
                self.0.name()
            }
            pub fn uid(self) -> Option<&'a str> {
                self.0.uid()
            }
            pub fn extras(self) -> Option<&'a Value> {
                self.0.extras()
            }
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
    pub fn byte_length(self) -> Option<u64> {
        self.value().get("byteLength").and_then(Value::as_u64)
    }
    pub fn uri(self) -> Option<&'a str> {
        self.value().get("uri").and_then(Value::as_str)
    }
}

impl<'a> BufferView<'a> {
    pub fn buffer(self) -> Option<BufferIndex> {
        index_value(self.value(), "buffer").map(BufferIndex)
    }
    pub fn byte_offset(self) -> u64 {
        self.value()
            .get("byteOffset")
            .and_then(Value::as_u64)
            .unwrap_or(0)
    }
    pub fn byte_length(self) -> Option<u64> {
        self.value().get("byteLength").and_then(Value::as_u64)
    }
    pub fn byte_stride(self) -> Option<u64> {
        self.value().get("byteStride").and_then(Value::as_u64)
    }
}

impl<'a> Accessor<'a> {
    pub fn buffer_view(self) -> Option<BufferViewIndex> {
        index_value(self.value(), "bufferView").map(BufferViewIndex)
    }
    pub fn count(self) -> Option<u64> {
        self.value().get("count").and_then(Value::as_u64)
    }
    pub fn component_type(self) -> Option<ComponentType> {
        self.value()
            .get("componentType")
            .and_then(Value::as_u64)
            .and_then(ComponentType::from_gltf)
    }
    pub fn accessor_type(self) -> Option<&'a str> {
        self.value().get("type").and_then(Value::as_str)
    }
    pub fn normalized(self) -> bool {
        matches!(self.value().get("normalized"), Some(Value::Bool(true)))
    }
}

impl<'a> Image<'a> {
    pub fn uri(self) -> Option<&'a str> {
        self.value().get("uri").and_then(Value::as_str)
    }
    pub fn buffer_view(self) -> Option<BufferViewIndex> {
        index_value(self.value(), "bufferView").map(BufferViewIndex)
    }
}

impl<'a> Texture<'a> {
    pub fn source(self) -> Option<ImageIndex> {
        index_value(self.value(), "source").map(ImageIndex)
    }
    pub fn sampler(self) -> Option<SamplerIndex> {
        index_value(self.value(), "sampler").map(SamplerIndex)
    }
}

impl<'a> Node<'a> {
    pub fn mesh(self) -> Option<MeshIndex> {
        index_value(self.value(), "mesh").map(MeshIndex)
    }
    pub fn camera(self) -> Option<CameraIndex> {
        index_value(self.value(), "camera").map(CameraIndex)
    }
    pub fn skin(self) -> Option<SkinIndex> {
        index_value(self.value(), "skin").map(SkinIndex)
    }
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
    pub fn uri(self) -> Option<&'a str> {
        self.value().get("uri").and_then(Value::as_str)
    }
}

impl<'a> Mesh<'a> {
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
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
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
    pub fn mesh_index(self) -> MeshIndex {
        self.mesh
    }
    pub fn primitive_index(self) -> usize {
        self.primitive
    }
    pub fn value(self) -> &'a Value {
        &self.document.as_value()["meshes"][self.mesh.0]["primitives"][self.primitive]
    }
    pub fn attributes(self) -> Option<&'a [(String, Value)]> {
        self.value().get("attributes").and_then(Value::as_object)
    }
    pub fn extension(self, name: &str) -> Option<&'a Value> {
        self.value().get("extensions")?.get(name)
    }
}
