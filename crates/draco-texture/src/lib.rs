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

/// Packing a BC7 block, and the tables it and the UASTC mapping share.
#[cfg(all(feature = "uastc", feature = "block-formats"))]
pub mod bc7;
#[cfg(all(feature = "uastc", feature = "block-formats"))]
mod bc7_tables;
/// Basis Universal ETC1S decoding.
#[cfg(feature = "etc1s")]
pub mod etc1s;
/// Turning ETC1S blocks into BC1, for a GPU that takes block formats.
#[cfg(all(feature = "etc1s", feature = "block-formats"))]
pub mod etc1s_to_bc1;
/// Turning an ETC1S alpha slice into BC4, the alpha half of BC3.
#[cfg(all(feature = "etc1s", feature = "block-formats"))]
pub mod etc1s_to_bc4;
/// Turning ETC1S blocks into ETC1 and ETC2.
#[cfg(all(feature = "etc1s", feature = "block-formats"))]
pub mod etc1s_to_etc;
#[cfg(any(feature = "etc1s", feature = "uastc"))]
mod huffman;
/// KTX2 container reading.
pub mod ktx2;
/// Decoding a KTX2 file's payload into pixels.
///
/// Needs both codecs: its whole job is to answer "whatever this file holds",
/// and a build with one of them could not. A consumer that wants only one
/// reaches for that codec's module directly.
#[cfg(all(feature = "etc1s", feature = "uastc"))]
pub mod transcode;
/// Basis Universal UASTC LDR decoding.
#[cfg(feature = "uastc")]
pub mod uastc;
/// Restating a UASTC block as a BC7 block.
#[cfg(all(feature = "uastc", feature = "block-formats"))]
pub mod uastc_to_bc7;
