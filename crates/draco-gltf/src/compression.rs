use crate::{Error, NativeImport, Result};
use draco_core::{
    encoder_buffer::EncoderBuffer, encoder_options::EncoderOptions, mesh_encoder::MeshEncoder,
};

#[derive(Clone, Copy, Debug)]
pub struct CompressionOptions {
    pub encoding_speed: u8,
    pub decoding_speed: u8,
}
impl Default for CompressionOptions {
    fn default() -> Self {
        Self {
            encoding_speed: 5,
            decoding_speed: 5,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct CompressionReport {
    pub compressed_primitives: usize,
    pub encoded_bytes: usize,
}

impl NativeImport {
    /// Encodes an already decoded mesh to a raw Draco payload using native options.
    pub fn encode_draco_mesh(
        &self,
        mesh: draco_core::Mesh,
        options: CompressionOptions,
    ) -> Result<Vec<u8>> {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut settings = EncoderOptions::new();
        settings.set_global_int("encoding_speed", options.encoding_speed as i32);
        settings.set_global_int("decoding_speed", options.decoding_speed as i32);
        let mut output = EncoderBuffer::new();
        encoder
            .encode(&settings, &mut output)
            .map_err(|error| Error::Extension(error.to_string()))?;
        Ok(output.data().to_vec())
    }

    pub fn compress_primitive(
        &mut self,
        mesh: crate::MeshIndex,
        primitive: usize,
        options: CompressionOptions,
    ) -> Result<CompressionReport> {
        let reference = self.document.primitive(mesh, primitive).ok_or_else(|| Error::Extension("primitive out of range".into()))?;
        let (geometry, mapping) = self.decode_geometry_primitive(reference)?;
        let bytes = self.encode_draco_mesh(geometry, options)?;
        let buffer = self.resources.buffers.len();
        let view;
        {
            let root = self.document.as_value_mut();
            let buffers = root["buffers"].as_array_mut().ok_or_else(|| Error::Extension("buffers is not an array".into()))?;
            buffers.push(crate::JsonValue::object([("byteLength", crate::JsonValue::from(bytes.len()))]));
            let views = root["bufferViews"].as_array_mut().ok_or_else(|| Error::Extension("bufferViews is not an array".into()))?;
            view = views.len();
            views.push(crate::JsonValue::object([("buffer", crate::JsonValue::from(buffer)), ("byteLength", crate::JsonValue::from(bytes.len()))]));
            let attributes = crate::JsonValue::Object(mapping.into_iter().map(|(name, id)| (name, crate::JsonValue::from(id as u64))).collect());
            root["meshes"][mesh.0]["primitives"][primitive]["extensions"][crate::KHR_DRACO_MESH_COMPRESSION] = crate::JsonValue::object([("bufferView", crate::JsonValue::from(view)), ("attributes", attributes)]);
            for name in ["extensionsUsed", "extensionsRequired"] {
                if root.get(name).is_none() { root[name] = crate::JsonValue::Array(Vec::new()); }
                let list = root[name].as_array_mut().unwrap();
                if !list.iter().any(|value| value.as_str() == Some(crate::KHR_DRACO_MESH_COMPRESSION)) { list.push(crate::JsonValue::from(crate::KHR_DRACO_MESH_COMPRESSION)); }
            }
        }
        self.resources.buffers.push(bytes.clone());
        Ok(CompressionReport { compressed_primitives: 1, encoded_bytes: bytes.len() })
    }
}
