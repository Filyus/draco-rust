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
}
