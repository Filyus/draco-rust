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
//!
//! # Features
//!
//! Two axes. `etc1s` and `uastc` are the codecs a KTX2 file can hold; `bc`,
//! `etc` and `astc` are the families of hardware a decoded block can be
//! written out for, and `block-formats` is all three at once. The container
//! itself is never optional.
//!
//! The second axis is by hardware rather than by target because that is what
//! goes out of date: a target is worth carrying for as long as machines that
//! take it are, and every one of these will one day stop being.
//!
//! Over those sit `modern` and `legacy`, which say not what a target is
//! but why it is still here. `modern` is what hardware sold today takes, both
//! `bc` and `astc`, since being current is not the same as being one kind of
//! machine. `legacy` is what is carried only for hardware with no current
//! alternative, today `etc` and only `etc` — every machine with ASTC also has
//! ETC, so it serves the devices that have one and not the other.
//!
//! `legacy` is in the default set rather than subtracted from it, because
//! Cargo features add and never subtract; retiring a family is deleting the
//! word from that line, and a CI slice builds without it so that edit stays a
//! one-liner.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

/// The fixed tables ASTC packing reads.
#[cfg(all(feature = "uastc", feature = "astc"))]
mod astc_tables;
/// Packing a BC7 block, and the tables it and the UASTC mapping share.
#[cfg(all(feature = "uastc", feature = "bc"))]
pub mod bc7;
#[cfg(all(feature = "uastc", feature = "bc"))]
mod bc7_tables;
/// Basis Universal ETC1S decoding.
#[cfg(feature = "etc1s")]
pub mod etc1s;
/// Turning ETC1S blocks into BC1, for a GPU that takes block formats.
#[cfg(all(feature = "etc1s", feature = "bc"))]
pub mod etc1s_to_bc1;
/// Turning an ETC1S alpha slice into BC4, the alpha half of BC3.
#[cfg(all(feature = "etc1s", feature = "bc"))]
pub mod etc1s_to_bc4;
/// Turning ETC1S blocks into ETC1 and ETC2.
#[cfg(all(feature = "etc1s", feature = "etc"))]
pub mod etc1s_to_etc;
#[cfg(feature = "etc1s")]
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
/// Restating a UASTC block as an ASTC block.
#[cfg(all(feature = "uastc", feature = "astc"))]
pub mod uastc_to_astc;
/// Restating a UASTC block as a BC7 block.
#[cfg(all(feature = "uastc", feature = "bc"))]
pub mod uastc_to_bc7;
