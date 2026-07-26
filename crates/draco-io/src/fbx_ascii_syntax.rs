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
//! # What the ASCII writer owes this reader
//!
//! [`crate::fbx_ascii_writer`] prints the same tree the binary encoder writes,
//! so each obligation below is stated against the reader assumption it serves.
//! Both halves live here so that neither can drift from the other:
//!
//! * An `F64` is printed with a `.`, `e` or `E`. Without one it comes back as
//!   an `I32` -- see [`number_property`] and its inverse [`format_f64`].
//! * `-0.0` survives, though not by the writer's doing: [`format_f64`]'s
//!   decimal point would be enough, but [`number_property`] already carries a
//!   case for the bare `-0` that authored files contain. Either half alone
//!   keeps the sign, which is why no test here pins the writer's share of it.
//! * A `KeyAttrDataFloat` array is printed as integer bit patterns, not as
//!   decimals; see [`array_element_type`]. Printing the values themselves
//!   produces a file this reader would misread by orders of magnitude.
//! * An array node's shape is exactly `Name: *N { a: v,v,v }`, with `N`
//!   matching the element count.
//! * A `Content` node's payload is base64 ([`is_base64_node`], written by
//!   [`encode_base64`]).
//! * An object's name is written `"Class::Name"` by [`ascii_object_name`],
//!   inverting [`normalize_object_name`], with `"` escaped as `&quot;`. That
//!   escape is **not** reversible: one corpus file holds an object named `"`
//!   and another named literally `&quot;`, and ASCII spells both the same way.
//!   The writer cannot fix this; it only avoids pretending otherwise.
//! * A `Properties70` `P:` record carries its FBX type name in field 1, the
//!   table [`properties70_type_is_integral`] reads back.

// Items used by only one direction stay gated to it, so a single-feature build
// does not carry the other half as dead code. Widening a gate is the visible
// step that says a convention became shared.
#[cfg(feature = "fbx-reader")]
use crate::fbx_node::FbxProperty;

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
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
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
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
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
#[cfg(any(feature = "fbx-reader", feature = "fbx-writer"))]
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

/// Rewrites a binary `"Name\0\x01Class"` into the ASCII `"Class::Name"`.
///
/// The inverse of [`normalize_object_name`], including the `&quot;` escape: a
/// bare `"` would end the string, so escaping it is what keeps the document
/// parsable at all, not merely faithful. Escaping after the join is what lets
/// a `"` in the name survive, since the reader unescapes before it splits.
///
/// Applied to every string, not only to object names, because the reader
/// applies its inverse to every string. That symmetry has one cost it cannot
/// avoid: a string that merely happens to contain `::` -- a texture path, say
/// -- comes back split into a name and a class. The container has no way to
/// say "this one is not an object name", and ufbx reads it the same way.
#[cfg(feature = "fbx-writer")]
pub(crate) fn ascii_object_name(raw: &str) -> String {
    let joined = match raw.split_once(NAME_CLASS_SEPARATOR) {
        Some((name, class)) => format!("{class}::{name}"),
        None => raw.to_string(),
    };
    if joined.contains('"') {
        return joined.replace('"', "&quot;");
    }
    joined
}

/// Prints an `F64` so that [`number_property`] reads it back unchanged.
///
/// `{}` gives the shortest decimal that round-trips, so the only thing to add
/// is the mark that keeps it a float: without a `.`, `e` or `E` the reader
/// types it as an integer. That same mark preserves `-0.0`, which Rust prints
/// as `-0` and an integer parse would strip the sign from.
///
/// Returns `None` for a non-finite value. ASCII FBX has no spelling for one --
/// every candidate (`nan`, `inf`, Autodesk's `1.#INF`) reads back as something
/// else or as a syntax error -- so the caller reports which record could not be
/// written rather than emitting a number that lies.
#[cfg(feature = "fbx-writer")]
pub(crate) fn format_f64(value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    let mut text = format!("{value}");
    if !text.contains(['.', 'e', 'E']) {
        text.push_str(".0");
    }
    Some(text)
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

/// Encodes embedded media as the base64 ASCII FBX carries it.
///
/// The inverse of [`decode_base64`]: standard alphabet, `=` padding, no line
/// breaks. The reader strips whitespace, so wrapping would be legal, but a
/// single run is what makes the payload one diffable line.
#[cfg(feature = "fbx-writer")]
pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut packed = 0u32;
        for index in 0..3 {
            packed = (packed << 8) | u32::from(chunk.get(index).copied().unwrap_or(0));
        }
        for index in 0..4 {
            // A chunk of one byte spells two sextets, of two bytes three; the
            // rest is padding, which `decode_base64` reads back as absent.
            if index <= chunk.len() {
                let sextet = (packed >> (18 - index * 6)) & 0x3f;
                out.push(char::from(ALPHABET[sextet as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
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
