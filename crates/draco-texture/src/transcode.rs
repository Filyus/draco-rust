//! Turning a KTX2 file's Basis Universal payload into pixels.
//!
//! The entry point everything outside this crate uses. It exists so a caller
//! never has to know which of the two Basis codecs a file holds: the container
//! says which, and both arrive here as the same RGBA8 image.

use crate::etc1s::{Etc1sDecoder, Etc1sError};
use crate::ktx2::{Ktx2, Ktx2Error, Ktx2Format};
use crate::uastc::{self, UastcError};

/// One decoded mip level, in raster order, four bytes per pixel.
#[derive(Debug, Clone)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, R, G, B, A.
    pub rgba: Vec<u8>,
}

/// What the caller wants the image in.
///
/// A GPU samples a block format directly, so transcoding into one keeps the
/// texture compressed in video memory instead of expanding it eightfold. Which
/// of these a machine can take depends on its extensions, so the caller picks
/// and this says whether the file's codec can reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Eight bits per channel, in raster order. Every codec reaches this.
    Rgba8,
    /// BC1, eight bytes per 4x4 block, no alpha.
    #[cfg(feature = "block-formats")]
    Bc1,
}

/// One decoded image, either as pixels or as GPU-ready blocks.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The format the bytes are in.
    pub target: Target,
    /// Pixels in raster order, or blocks in raster order.
    pub bytes: Vec<u8>,
}

/// Anything that can stop a file from being turned into pixels.
#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    /// The container could not be read.
    #[error(transparent)]
    Container(#[from] Ktx2Error),
    /// The ETC1S payload could not be decoded.
    #[error(transparent)]
    Etc1s(#[from] Etc1sError),
    /// The UASTC payload could not be decoded.
    #[error(transparent)]
    Uastc(#[from] UastcError),
    /// The file holds something no codec here reads.
    #[error("this KTX2 file cannot be transcoded: {0}")]
    Unsupported(String),
    /// The file's codec cannot reach the format that was asked for.
    #[error("a {codec} file cannot be transcoded to {target:?}")]
    NoSuchTarget {
        /// Which Basis codec the file holds.
        codec: &'static str,
        /// The format that was asked for.
        target: Target,
    },
    /// A level, layer or face that the file does not have.
    #[error("no such image: level {level}, layer {layer}, face {face}")]
    NoSuchImage {
        /// Requested mip level.
        level: u32,
        /// Requested array layer.
        layer: u32,
        /// Requested cubemap face.
        face: u32,
    },
}

/// Whatever a file's codec shares between its images.
///
/// Built once per file rather than once per level, because ETC1S keeps its
/// codebooks in the global data: decoding eleven mip levels one at a time
/// would otherwise mean reading the same codebooks eleven times. It borrows
/// nothing, so a caller can keep it beside the bytes it came from and hand
/// both back for each level — which is what the WASM boundary needs, since it
/// cannot hold a value that borrows from its neighbour.
pub struct Transcoder {
    etc1s: Option<Etc1sDecoder>,
}

impl Transcoder {
    /// Read whatever has to be read before any image can be decoded.
    pub fn new(file: &Ktx2<'_>) -> Result<Self, TranscodeError> {
        // Video files reuse the previous frame's block indices across levels,
        // which makes a level undecodable on its own. Nothing in glTF is a
        // video texture, so this says so rather than decoding it wrongly.
        if file.key_value("KTXanimData").is_some() {
            return Err(TranscodeError::Unsupported("it is a video".into()));
        }

        let etc1s = match file.format() {
            Ktx2Format::Etc1s { .. } => {
                let image_count = file.level_count() as usize
                    * file.layer_count() as usize
                    * file.face_count() as usize;
                Some(Etc1sDecoder::new(file.global_data(), image_count)?)
            }
            _ => None,
        };
        Ok(Transcoder { etc1s })
    }

    /// Decode one image to RGBA8.
    ///
    /// `file` must be the one this was built from; handing over a different
    /// file decodes its levels against the wrong codebooks.
    pub fn decode_rgba(
        &self,
        file: &Ktx2<'_>,
        level: u32,
        layer: u32,
        face: u32,
    ) -> Result<Image, TranscodeError> {
        let decoded = self.decode(file, level, layer, face, Target::Rgba8)?;
        Ok(Image {
            width: decoded.width,
            height: decoded.height,
            rgba: decoded.bytes,
        })
    }

    /// Decode one image into `target`.
    pub fn decode(
        &self,
        file: &Ktx2<'_>,
        level: u32,
        layer: u32,
        face: u32,
        target: Target,
    ) -> Result<Decoded, TranscodeError> {
        if level >= file.level_count() || layer >= file.layer_count() || face >= file.face_count() {
            return Err(TranscodeError::NoSuchImage { level, layer, face });
        }
        let (width, height) = file.level_dimensions(level);

        match file.format() {
            Ktx2Format::Etc1s { .. } => {
                let decoder = self.etc1s.as_ref().ok_or_else(|| {
                    TranscodeError::Unsupported("its codebooks were never read".into())
                })?;
                let index = image_index(file, level, layer, face);
                let desc = decoder
                    .image_desc(index)
                    .ok_or(TranscodeError::NoSuchImage { level, layer, face })?;
                let level_data = file.level_bytes(level)?;
                let bytes = match target {
                    Target::Rgba8 => decoder.decode_rgba(&level_data, desc, width, height)?,
                    #[cfg(feature = "block-formats")]
                    Target::Bc1 => decoder.decode_bc1(&level_data, desc, width, height, true)?,
                };
                Ok(Decoded {
                    width,
                    height,
                    target,
                    bytes,
                })
            }
            Ktx2Format::UastcLdr4x4 { .. } => {
                let level_data = file.level_bytes(level)?;
                // A UASTC level holds every layer and face back to back, all
                // the same size, so the image's own blocks are one slice of it.
                let image_size = (width.div_ceil(4) as usize)
                    * (height.div_ceil(4) as usize)
                    * uastc::BLOCK_SIZE;
                let start = image_index(file, 0, layer, face) * image_size;
                let image_data = level_data
                    .get(start..start + image_size)
                    .ok_or(TranscodeError::NoSuchImage { level, layer, face })?;
                let bytes = match target {
                    Target::Rgba8 => uastc::decode_rgba(image_data, width, height)?,
                    #[allow(unreachable_patterns)]
                    other => {
                        return Err(TranscodeError::NoSuchTarget {
                            codec: "UASTC",
                            target: other,
                        })
                    }
                };
                Ok(Decoded {
                    width,
                    height,
                    target,
                    bytes,
                })
            }
            Ktx2Format::Plain { vk_format, .. } => Err(TranscodeError::Unsupported(format!(
                "it holds plain vkFormat {vk_format} rather than Basis Universal"
            ))),
        }
    }
}

/// Where one image sits in the file's flat per-image tables.
fn image_index(file: &Ktx2<'_>, level: u32, layer: u32, face: u32) -> usize {
    let faces = file.face_count() as usize;
    let layers = file.layer_count() as usize;
    level as usize * layers * faces + layer as usize * faces + face as usize
}
