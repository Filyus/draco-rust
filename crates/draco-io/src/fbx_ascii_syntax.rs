//! Conventions the ASCII FBX container imposes, independent of direction.
//!
//! ASCII FBX carries the same node tree as the binary container but records
//! less about it: values have no type tags, an object's name and class are
//! joined differently, and embedded media is text. Recovering the difference is
//! not parsing -- it is knowledge about the format that a reader and a writer
//! need identically, one inverted from the other.
//!
//! It lives here rather than in [`crate::fbx_ascii`] because that module is
//! gated on `fbx-reader` while [`crate::fbx_writer`] is gated on `fbx-writer`,
//! and the two features are independent. The binary writer already needs
//! [`NAME_CLASS_SEPARATOR`], which it had spelled out sixteen times.
//!
//! # What an ASCII writer would owe this reader
//!
//! Nothing here writes ASCII yet. These are the obligations a writer would
//! inherit, each stated against the reader assumption it serves, so that
//! adding one does not mean rediscovering them:
//!
//! * An `F64` must be printed with a `.`, `e` or `E`. Without one it comes
//!   back as an `I32` -- see [`number_property`].
//! * `-0.0` must not be printed as `-0`. The sign is preserved by the binary
//!   container and would be dropped by an integer parse.
//! * A `KeyAttrDataFloat` array must be printed as integer bit patterns, not
//!   as decimals; see [`array_element_type`]. Printing the values themselves
//!   produces a file this reader would misread by orders of magnitude.
//! * An array node's shape is exactly `Name: *N { a: v,v,v }`, with `N`
//!   matching the element count.
//! * A `Content` node's payload is base64 ([`is_base64_node`]).
//! * An object's name is written `"Class::Name"`, inverting
//!   [`normalize_object_name`], with `"` escaped as `&quot;`. That escape is
//!   **not** reversible: one corpus file holds an object named `"` and another
//!   named literally `&quot;`, and ASCII spells both the same way. A writer
//!   cannot fix this; it can only avoid pretending otherwise.
//! * A `Properties70` `P:` record must carry its FBX type name in field 1, the
//!   table [`properties70_type_is_integral`] reads back.

// Every item below the two constants describes how a value is spelled, and
// only the reader spells values today. They are gated so a `fbx-writer`-only
// build does not carry them as dead code -- and so that the day an ASCII
// writer arrives, widening a gate is the visible step that says which
// convention it took on.
#[cfg(feature = "fbx-reader")]
use crate::fbx_container::FbxProperty;

/// Separator the binary container puts between an object's name and its class.
///
/// Binary stores `"Name\0\x01Class"`; ASCII writes `"Class::Name"` -- reversed
/// order, different separator. Everything above the node tree reads the name as
/// the text before this pair.
pub(crate) const NAME_CLASS_SEPARATOR: &str = "\u{0}\u{1}";

/// Joins an object's name and class as the binary container stores them.
#[cfg(feature = "fbx-writer")]
pub(crate) fn name_class(name: &str, class: &str) -> String {
    format!("{name}{NAME_CLASS_SEPARATOR}{class}")
}

/// Version this crate writes.
#[cfg(feature = "fbx-writer")]
pub(crate) const FBX_VERSION: u32 = 7500;

/// Element type of an array node's payload.
#[cfg(feature = "fbx-reader")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArrayElement {
    /// 32-bit signed integers.
    I32,
    /// 64-bit signed integers.
    I64,
    /// 64-bit floats.
    F64,
    /// Single-precision floats whose integer spellings are bit patterns.
    F32Bits,
}

/// Recovers the element type ASCII does not record.
///
/// In FBX the type is fixed by the node's schema, not by how the values happen
/// to be written, so the node name is what recovers it. Inferring from the text
/// instead would type a mesh whose coordinates are all whole numbers as an
/// integer array -- 27 of the 369 `Vertices` arrays in the ufbx ASCII corpus --
/// and the geometry reader, which looks for floats, would find nothing.
///
/// The integer names are exactly those this crate reads as integers; anything
/// else is floating point, which is the safe default because a float array with
/// whole values still reads correctly while the reverse does not.
#[cfg(feature = "fbx-reader")]
pub(crate) fn array_element_type(owner: &str) -> ArrayElement {
    match owner {
        "PolygonVertexIndex" | "Edges" | "Indexes" | "Materials" | "Smoothing" | "UVIndex"
        | "NormalsIndex" | "NormalIndex" | "ColorIndex" | "TangentIndex" | "BinormalIndex"
        | "KeyAttrFlags" | "KeyAttrRefCount" => ArrayElement::I32,
        "KeyTime" => ArrayElement::I64,
        // FBX packs a cubic key's two weights into the bits of one float and
        // ASCII prints that float's integer bit pattern, where the binary
        // container stores the four bytes directly. Converting the number
        // instead of reinterpreting it yields a value off by many orders of
        // magnitude -- `218434821` rather than the weights it encodes.
        "KeyAttrDataFloat" => ArrayElement::F32Bits,
        _ => ArrayElement::F64,
    }
}

/// Whether a node's string payload is base64 rather than text.
///
/// Embedded media is the only case: ASCII writes it as base64 where the binary
/// container has a raw blob. Decoding it is what lets an embedded texture reach
/// `FbxTexture::content` from either container.
#[cfg(feature = "fbx-reader")]
pub(crate) fn is_base64_node(node_name: &str) -> bool {
    node_name == "Content"
}

/// Whether a `Properties70` type name denotes an integral value.
///
/// A `P` record names its own FBX type in field 1, so unlike a bare array this
/// needs no guessing. `Lcl Translation`, `FieldOfView`, `Vector3D` and friends
/// are all floating point; the integral set is short and closed.
#[cfg(feature = "fbx-reader")]
pub(crate) fn properties70_type_is_integral(type_name: &str) -> bool {
    matches!(
        type_name,
        "int" | "Integer" | "enum" | "bool" | "Bool" | "Visibility" | "KTime"
    )
}

/// Reads the boolean spelling ASCII uses, where the binary container has a
/// typed byte.
///
/// Returns `None` for any other bare word, which is an enum token such as the
/// `Type: A` of an extrapolation block.
#[cfg(feature = "fbx-reader")]
pub(crate) fn parse_ascii_bool(word: &str) -> Option<bool> {
    match word {
        "T" | "Y" => Some(true),
        "F" | "N" => Some(false),
        _ => None,
    }
}

/// Rewrites an ASCII `"Class::Name"` into the binary `"Name\0\x01Class"`.
///
/// Everything above the node tree reads an object's name as the text before the
/// separator, so leaving the ASCII spelling in place would name every object
/// after its class. `ufbx` applies the same rule; a string with no `::` is
/// returned unchanged.
#[cfg(feature = "fbx-reader")]
pub(crate) fn normalize_object_name(raw: &[u8]) -> String {
    // A quoted string cannot hold a bare quote, so ASCII writes `&quot;` where
    // the binary container stores the character itself.
    //
    // The escape is not reversible: `max_quote_7500_binary.fbx` contains one
    // object named `"` and another named literally `&quot;`, and ASCII spells
    // both the same way. Decoding is still the better reading -- it recovers
    // the common case -- but that one file cannot round-trip through the ASCII
    // container no matter what this does.
    let unescaped;
    let raw = if raw.windows(6).any(|window| window == b"&quot;") {
        unescaped = String::from_utf8_lossy(raw)
            .replace("&quot;", "\"")
            .into_bytes();
        unescaped.as_slice()
    } else {
        raw
    };
    let Some(split) = raw.windows(2).position(|pair| pair == b"::") else {
        return String::from_utf8_lossy(raw).into_owned();
    };
    let class = &raw[..split];
    let name = &raw[split + 2..];
    let mut out = Vec::with_capacity(raw.len());
    out.extend_from_slice(name);
    out.extend_from_slice(NAME_CLASS_SEPARATOR.as_bytes());
    out.extend_from_slice(class);
    String::from_utf8_lossy(&out).into_owned()
}

/// Types a bare number by how it is written, which is all ASCII FBX offers.
#[cfg(feature = "fbx-reader")]
pub(crate) fn number_property(text: &str) -> FbxProperty {
    let integral = !text.contains(['.', 'e', 'E']);
    // `-0` is written without a decimal point but is not the integer zero:
    // parsing it as one drops the sign, and the binary container preserves it.
    if integral && text.starts_with('-') && text.trim_start_matches(['-', '0']).is_empty() {
        return FbxProperty::F64(-0.0);
    }
    if integral {
        if let Ok(value) = text.parse::<i32>() {
            return FbxProperty::I32(value);
        }
        if let Ok(value) = text.parse::<i64>() {
            return FbxProperty::I64(value);
        }
    }
    FbxProperty::F64(text.parse::<f64>().unwrap_or(0.0))
}

/// Decodes the base64 ASCII FBX uses for embedded media.
///
/// Returns `None` for anything that is not valid base64, so a `Content` that
/// holds an ordinary string is left as one rather than silently emptied.
#[cfg(feature = "fbx-reader")]
pub(crate) fn decode_base64(text: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let body: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if body.is_empty() || !body.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3);
    for chunk in body.chunks_exact(4) {
        let padding = chunk.iter().filter(|byte| **byte == b'=').count();
        if padding > 2 {
            return None;
        }
        let mut packed = 0u32;
        for &byte in chunk {
            let value = if byte == b'=' { 0 } else { sextet(byte)? };
            packed = (packed << 6) | value;
        }
        let bytes = packed.to_be_bytes();
        out.push(bytes[1]);
        if padding < 2 {
            out.push(bytes[2]);
        }
        if padding < 1 {
            out.push(bytes[3]);
        }
    }
    Some(out)
}
