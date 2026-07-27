//! KTX2 reading and Basis Universal transcoding WASM module.
//!
//! Loaded on demand rather than at startup: KTX2 is rare among the files this
//! converter opens, and a page that never meets one should not pay for the
//! transcoder. Everything real lives in `draco-texture`; this is the thin
//! boundary that hands a decoded image to JavaScript.

use wasm_bindgen::prelude::*;

use draco_texture::ktx2::{Ktx2, Ktx2Format};
use draco_texture::transcode::{Target, Transcoder};

/// Initialize panic hook for better error messages in browser console.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Get the version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// One decoded mip level, as pixels or as GPU-ready blocks.
#[wasm_bindgen]
pub struct Ktx2Image {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

#[wasm_bindgen]
impl Ktx2Image {
    /// Width in pixels.
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The decoded bytes: pixels in raster order, or blocks in raster order,
    /// depending on the target the caller asked for.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

/// A KTX2 file opened for decoding.
///
/// Holds the file's bytes because a mip level is not self-contained: an ETC1S
/// file keeps the codebooks every level indexes into once, in its global data.
/// Opening reads those, so decoding eleven levels reads them once rather than
/// eleven times.
#[wasm_bindgen]
pub struct Ktx2File {
    data: Vec<u8>,
    transcoder: Transcoder,
    width: u32,
    height: u32,
    levels: u32,
    has_alpha: bool,
    codec: &'static str,
}

#[wasm_bindgen]
impl Ktx2File {
    /// Open a KTX2 file, failing if it is not one or holds no Basis payload.
    ///
    /// Failing here rather than at the first decode is deliberate: the caller
    /// can then fall back before it has built anything around the image.
    #[wasm_bindgen(constructor)]
    pub fn new(data: Vec<u8>) -> Result<Ktx2File, JsError> {
        let file = Ktx2::parse(&data).map_err(to_js)?;
        let transcoder = Transcoder::new(&file).map_err(to_js)?;
        let (has_alpha, codec) = match file.format() {
            Ktx2Format::Etc1s { has_alpha } => (has_alpha, "etc1s"),
            Ktx2Format::UastcLdr4x4 { has_alpha } => (has_alpha, "uastc"),
            Ktx2Format::Plain { .. } => (false, "plain"),
        };
        let (width, height, levels) = (file.width(), file.height(), file.level_count());
        drop(file);
        Ok(Ktx2File {
            data,
            transcoder,
            width,
            height,
            levels,
            has_alpha,
            codec,
        })
    }

    /// Width of mip level 0.
    #[wasm_bindgen(getter)]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of mip level 0.
    #[wasm_bindgen(getter)]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// How many mip levels the file stores.
    #[wasm_bindgen(getter)]
    pub fn levels(&self) -> u32 {
        self.levels
    }

    /// Whether the image carries meaningful alpha.
    #[wasm_bindgen(getter, js_name = hasAlpha)]
    pub fn has_alpha(&self) -> bool {
        self.has_alpha
    }

    /// Which Basis codec the file holds: `"etc1s"` or `"uastc"`.
    #[wasm_bindgen(getter)]
    pub fn codec(&self) -> String {
        self.codec.to_string()
    }

    /// Decode one mip level to RGBA8.
    #[wasm_bindgen(js_name = decodeRgba)]
    pub fn decode_rgba(&self, level: u32) -> Result<Ktx2Image, JsError> {
        self.decode(level, "rgba8")
    }

    /// Decode one mip level into a named target: `"rgba8"` or `"bc1"`.
    ///
    /// Named rather than numbered because the caller picks the target from
    /// what the GL context reports, and a string survives that round trip
    /// without either side owning an enum the other has to mirror.
    #[wasm_bindgen(js_name = decode)]
    pub fn decode(&self, level: u32, target: &str) -> Result<Ktx2Image, JsError> {
        let target = match target {
            "rgba8" => Target::Rgba8,
            "bc1" => Target::Bc1,
            other => return Err(JsError::new(&format!("unknown target format {other}"))),
        };
        let file = Ktx2::parse(&self.data).map_err(to_js)?;
        let decoded = self
            .transcoder
            .decode(&file, level, 0, 0, target)
            .map_err(to_js)?;
        Ok(Ktx2Image {
            width: decoded.width,
            height: decoded.height,
            bytes: decoded.bytes,
        })
    }
}

fn to_js<E: std::fmt::Display>(error: E) -> JsError {
    JsError::new(&error.to_string())
}
