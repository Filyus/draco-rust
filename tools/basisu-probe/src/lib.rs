//! The same job as `web/ktx2-wasm`, on the `basisu` crate, for measuring only.

use basisu::{DecodeFlags, TargetFormat, Transcoder};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Ktx2File {
    data: Vec<u8>,
}

#[wasm_bindgen]
impl Ktx2File {
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<Ktx2File, JsError> {
        Transcoder::new(&data).map_err(|error| JsError::new(&format!("{error:?}")))?;
        Ok(Ktx2File { data })
    }

    #[wasm_bindgen(getter)]
    pub fn levels(&self) -> u32 {
        Transcoder::new(&self.data)
            .map(|file| file.level_count())
            .unwrap_or(0)
    }

    pub fn decode(&self, level: u32, target: &str) -> Result<Vec<u8>, JsError> {
        let target = match target {
            "rgba8" => TargetFormat::Rgba32,
            "bc1" => TargetFormat::Bc1Rgb,
            "bc3" => TargetFormat::Bc3Rgba,
            "bc7" => TargetFormat::Bc7Rgba,
            "etc1" => TargetFormat::Etc1Rgb,
            "etc2" => TargetFormat::Etc2Rgba,
            "astc" => TargetFormat::Astc4x4Rgba,
            other => return Err(JsError::new(other)),
        };
        let file =
            Transcoder::new(&self.data).map_err(|error| JsError::new(&format!("{error:?}")))?;
        file.transcode(level, target, DecodeFlags::default())
            .map_err(|error| JsError::new(&format!("{error:?}")))
    }
}
