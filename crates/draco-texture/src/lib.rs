//! Reading the texture containers a glTF file can point at.
//!
//! Geometry codecs live in `draco-core` and `draco-io`; this crate exists for
//! the one thing neither of them covers, which is the image side of
//! `KHR_texture_basisu`. A KTX2 file carrying Basis Universal data cannot be
//! handed to a browser's image decoder — it has to be transcoded first — so a
//! converter that only carries those bytes through can export the file
//! faithfully and still show the model untextured.
//!
//! Deliberately separate from `draco-io`, which is scoped to Draco's own
//! container and accessor work: nothing here is about Draco, and none of it
//! belongs in a published crate's API surface.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

/// Basis Universal ETC1S decoding.
pub mod etc1s;
mod huffman;
/// KTX2 container reading.
pub mod ktx2;
/// Decoding a KTX2 file's payload into pixels.
pub mod transcode;
/// Basis Universal UASTC LDR decoding.
pub mod uastc;
