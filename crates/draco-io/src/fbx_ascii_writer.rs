//! Prints an FBX document as ASCII text.
//!
//! This is the second backend over the tree [`crate::fbx_writer::FbxWriter`]
//! builds, not a second document builder: what records a file contains is
//! decided once, and this module only decides how one is spelled. The binary
//! spelling lives in [`crate::fbx_encoder`], and the conventions the two
//! containers disagree about live in [`crate::fbx_ascii_syntax`], next to the
//! reader code that inverts them.
//!
//! ASCII records less than the binary container does, so three of the fourteen
//! property variants come back as a wider type. None of them is a value this
//! crate writes -- [`crate::fbx_writer`] emits neither `U8`, `I16` nor a scalar
//! `F32` -- but the printer is total, so they are stated rather than left to be
//! discovered:
//!
//! | written | read back |
//! |---|---|
//! | `U8`, `I16` | `I32` |
//! | scalar `F32` | `F64` |
//! | `F32Array` | `F64Array`, except where the node name types it as bits |
//! | `BoolArray` | `F64Array` of `1.0`/`0.0` |
//!
//! Three shapes have no ASCII spelling at all and are reported as errors
//! rather than written wrong: a node carrying two array properties, a
//! non-finite float, and raw bytes on a node the reader will not decode as
//! base64.

use std::fmt::Write as _;
use std::io;

use crate::fbx_ascii_syntax::{
    array_element_type, ascii_object_name, encode_base64, format_f64, is_base64_node, ArrayElement,
    FBX_VERSION,
};
use crate::fbx_node::{FbxNode, FbxProperty};

/// Column an array line is broken at.
///
/// The reader skips whitespace between elements, so this is free to pick any
/// width. A mesh's index array is one line of hundreds of kilobytes otherwise,
/// which defeats the only reason to write ASCII at all.
const ARRAY_LINE_BUDGET: usize = 2000;

/// How one property is spelled: in the node's property list, or as the array
/// that occupies its block.
enum Spelling {
    /// Text that goes in the property list, after the node's colon.
    Scalar(String),
    /// One element text per value. A node has one block, so it has at most one
    /// of these.
    Array(Vec<String>),
}

/// Prints a whole document, header comment included.
pub(crate) fn print_document(nodes: &[FbxNode]) -> io::Result<Vec<u8>> {
    let mut out = String::new();
    // `is_ascii_fbx` sniffs for this line, and Autodesk's own files open with
    // it, so it is both the format marker and the version in readable form.
    let _ = writeln!(
        out,
        "; FBX {}.{}.{} project file",
        FBX_VERSION / 1000,
        (FBX_VERSION / 100) % 10,
        (FBX_VERSION / 10) % 10,
    );
    out.push_str("; ----------------------------------------------------\n");
    for node in nodes {
        out.push('\n');
        print_node(&mut out, node, 0)?;
    }
    Ok(out.into_bytes())
}

fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push('\t');
    }
}

fn print_node(out: &mut String, node: &FbxNode, depth: usize) -> io::Result<()> {
    let mut scalars = Vec::new();
    let mut array: Option<Vec<String>> = None;
    for property in &node.properties {
        match spell(&node.name, property)? {
            Spelling::Scalar(text) => scalars.push(text),
            Spelling::Array(texts) => {
                if array.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "FBX: node '{}' carries two array properties, which ASCII cannot \
                             spell: an array lives in the node's block, and a node has one block",
                            node.name
                        ),
                    ));
                }
                // The count is declared before the block and the reader checks
                // it against what the block lists, so it is written from the
                // same vector rather than from the property.
                scalars.push(format!("*{}", texts.len()));
                array = Some(texts);
            }
        }
    }

    push_indent(out, depth);
    out.push_str(&node.name);
    out.push(':');
    let properties = scalars.join(", ");
    // A space always follows the colon and another always precedes the brace,
    // so a node with a block and no properties reads `Name:  {`, which is how
    // Autodesk spells it.
    let has_block = array.is_some() || !node.children.is_empty();
    if has_block {
        let _ = writeln!(out, " {properties} {{");
    } else if properties.is_empty() {
        out.push('\n');
    } else {
        let _ = writeln!(out, " {properties}");
    }

    if let Some(texts) = array {
        print_array(out, &texts, depth + 1);
    }
    for child in &node.children {
        print_node(out, child, depth + 1)?;
    }
    if has_block {
        push_indent(out, depth);
        out.push_str("}\n");
    }
    Ok(())
}

/// Writes an array's values as the `a:` child the reader folds back onto its
/// parent.
fn print_array(out: &mut String, texts: &[String], depth: usize) {
    push_indent(out, depth);
    out.push_str("a:");
    let mut column = depth + 2;
    for (index, text) in texts.iter().enumerate() {
        if index == 0 {
            out.push(' ');
            column += 1;
        } else {
            out.push(',');
            column += 1;
            if column + text.len() > ARRAY_LINE_BUDGET {
                out.push('\n');
                push_indent(out, depth + 1);
                column = depth + 1;
            }
        }
        out.push_str(text);
        column += text.len();
    }
    out.push('\n');
}

fn spell(node_name: &str, property: &FbxProperty) -> io::Result<Spelling> {
    let floats = |values: &[f64]| -> io::Result<Vec<String>> {
        values
            .iter()
            .map(|value| float(node_name, *value))
            .collect()
    };
    Ok(match property {
        FbxProperty::Bool(value) => Spelling::Scalar(bool_text(*value).to_string()),
        FbxProperty::U8(value) => Spelling::Scalar(value.to_string()),
        FbxProperty::I16(value) => Spelling::Scalar(value.to_string()),
        FbxProperty::I32(value) => Spelling::Scalar(value.to_string()),
        FbxProperty::I64(value) => Spelling::Scalar(value.to_string()),
        FbxProperty::F32(value) => Spelling::Scalar(float(node_name, f64::from(*value))?),
        FbxProperty::F64(value) => Spelling::Scalar(float(node_name, *value)?),
        FbxProperty::String(text) => Spelling::Scalar(format!("\"{}\"", ascii_object_name(text))),
        FbxProperty::Raw(bytes) => {
            if !is_base64_node(node_name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "FBX: node '{node_name}' holds {} raw bytes, which ASCII spells as base64 \
                         only where the reader knows to decode it",
                        bytes.len()
                    ),
                ));
            }
            Spelling::Scalar(format!("\"{}\"", encode_base64(bytes)))
        }
        // ASCII has no boolean array: the reader types an array by its node
        // name, and no name in the format denotes one.
        FbxProperty::BoolArray(values) => Spelling::Array(
            values
                .iter()
                .map(|value| u8::from(*value).to_string())
                .collect(),
        ),
        FbxProperty::I32Array(values) => {
            Spelling::Array(values.iter().map(i32::to_string).collect())
        }
        FbxProperty::I64Array(values) => {
            Spelling::Array(values.iter().map(i64::to_string).collect())
        }
        FbxProperty::F32Array(values) => {
            Spelling::Array(if array_element_type(node_name) == ArrayElement::F32Bits {
                // FBX packs a cubic key's two weights into the bits of one
                // float and ASCII prints that float's integer bit pattern.
                // Printing the value instead yields a file this crate's own
                // reader misreads by orders of magnitude.
                values
                    .iter()
                    .map(|value| value.to_bits().to_string())
                    .collect()
            } else {
                // Widened before printing, not after: the shortest decimal
                // that round-trips an `f32` is not the one that round-trips
                // the `f64` the reader builds from it, so printing the `f32`
                // spelling would shift every value by a few last bits.
                floats(
                    &values
                        .iter()
                        .map(|value| f64::from(*value))
                        .collect::<Vec<_>>(),
                )?
            })
        }
        FbxProperty::F64Array(values) => Spelling::Array(floats(values)?),
    })
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "T"
    } else {
        "F"
    }
}

fn float(node_name: &str, value: f64) -> io::Result<String> {
    format_f64(value).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("FBX: node '{node_name}' holds {value}, which ASCII FBX cannot spell"),
        )
    })
}

// Every test here prints a tree and reads it back with the ASCII reader, which
// is what the printer exists to satisfy. Comparing against a fixed string
// instead would pin the layout and prove nothing about the round trip, so the
// only assertions on the text itself are the two where the text is the point.
#[cfg(all(test, feature = "fbx-reader"))]
mod tests {
    use super::*;
    use crate::fbx_ascii::parse_ascii_nodes;
    use crate::fbx_ascii_syntax::NAME_CLASS_SEPARATOR;
    use crate::fbx_options::FbxReadOptions;

    fn round_trip(nodes: Vec<FbxNode>) -> Vec<FbxNode> {
        let text = print_document(&nodes).expect("printable");
        parse_ascii_nodes(&text, &FbxReadOptions::default()).unwrap_or_else(|error| {
            panic!(
                "printed document did not parse: {error}\n{}",
                String::from_utf8_lossy(&text)
            )
        })
    }

    fn node(name: &str, properties: Vec<FbxProperty>) -> FbxNode {
        FbxNode {
            name: name.to_string(),
            properties,
            children: Vec::new(),
        }
    }

    #[test]
    fn a_float_that_happens_to_be_whole_stays_a_float() {
        let read = round_trip(vec![node("Weight", vec![FbxProperty::F64(1.0)])]);
        // Without the decimal point the reader types this as an `I32`, and a
        // geometry reader looking for floats finds nothing.
        assert!(
            matches!(read[0].properties[0], FbxProperty::F64(value) if value == 1.0),
            "{:?}",
            read[0].properties[0]
        );
    }

    // Guarded on both sides -- the printer's decimal point and the reader's
    // case for the bare `-0` an authored file contains -- so this pins the
    // round trip rather than either half of it.
    #[test]
    fn negative_zero_keeps_its_sign() {
        let read = round_trip(vec![node("Weight", vec![FbxProperty::F64(-0.0)])]);
        let FbxProperty::F64(value) = read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert!(value == 0.0 && value.is_sign_negative(), "{value}");
    }

    #[test]
    fn a_non_finite_value_is_refused_rather_than_spelled_wrong() {
        let error = print_document(&[node("Weight", vec![FbxProperty::F64(f64::NAN)])])
            .expect_err("NaN has no ASCII spelling");
        assert!(error.to_string().contains("Weight"), "{error}");
    }

    #[test]
    fn an_animation_curve_widens_its_values_without_shifting_them() {
        // Widened before printing, not after. Printing the `f32`'s own
        // shortest decimal spells `0.1`, which the reader parses to a `f64`
        // that is not the one this `f32` widens to -- every key would land a
        // few last bits away from where it started.
        let values = vec![0.1f32, -2.5, 1e-8, 0.3];
        let read = round_trip(vec![node(
            "KeyValueFloat",
            vec![FbxProperty::F32Array(values.clone())],
        )]);
        let FbxProperty::F64Array(read_values) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        let widened: Vec<f64> = values.iter().map(|value| f64::from(*value)).collect();
        assert_eq!(read_values, &widened);
    }

    #[test]
    fn a_curve_tangent_is_printed_as_the_bit_pattern_it_is() {
        // This one is asserted on the text, because the text is the only place
        // it shows. FBX packs a cubic key's two weights into the bits of one
        // float and ASCII prints that float's integer bit pattern; printing
        // the value itself yields `2.1843482e8`-sized nonsense in every other
        // reader. This crate's own reader takes either spelling -- `fold_array`
        // reads a decimal point as "this is a value" -- so a round trip agrees
        // with itself no matter which is written, and cannot pin this at all.
        let tangents = vec![f32::from_bits(218_434_821), f32::from_bits(0)];
        let text = print_document(&[node(
            "KeyAttrDataFloat",
            vec![FbxProperty::F32Array(tangents.clone())],
        )])
        .expect("printable");
        let text = String::from_utf8_lossy(&text);
        let line = text
            .lines()
            .find(|line| line.trim_start().starts_with("a:"))
            .expect("the array line");
        assert_eq!(line.trim(), "a: 218434821,0", "{text}");

        // The round trip still has to work, decimal-tolerant reader and all.
        let read = round_trip(vec![node(
            "KeyAttrDataFloat",
            vec![FbxProperty::F32Array(tangents.clone())],
        )]);
        let FbxProperty::F32Array(read_tangents) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert_eq!(
            read_tangents
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            tangents.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_object_name_is_written_class_first_and_read_back_name_first() {
        let stored = format!("Cube{NAME_CLASS_SEPARATOR}Model");
        let text = print_document(&[node("Model", vec![FbxProperty::String(stored.clone())])])
            .expect("printable");
        assert!(
            String::from_utf8_lossy(&text).contains("\"Model::Cube\""),
            "{}",
            String::from_utf8_lossy(&text)
        );

        let read = round_trip(vec![node("Model", vec![FbxProperty::String(stored)])]);
        let FbxProperty::String(name) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert_eq!(name, &format!("Cube{NAME_CLASS_SEPARATOR}Model"));
    }

    #[test]
    fn a_name_containing_the_ascii_separator_survives() {
        // The reader splits at the first `::`, so the rest stays in the name.
        let stored = format!("a::b{NAME_CLASS_SEPARATOR}Model");
        let read = round_trip(vec![node("Model", vec![FbxProperty::String(stored)])]);
        let FbxProperty::String(name) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert_eq!(name, &format!("a::b{NAME_CLASS_SEPARATOR}Model"));
    }

    #[test]
    fn a_quote_in_a_name_survives_but_cannot_be_told_from_its_escape() {
        let quote = format!("\"{NAME_CLASS_SEPARATOR}Model");
        let read = round_trip(vec![node(
            "Model",
            vec![FbxProperty::String(quote.clone())],
        )]);
        let FbxProperty::String(name) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert_eq!(name, &quote);

        // ...and this is the case that cannot work. An object named `"` and an
        // object named literally `&quot;` are spelled the same way, so the
        // second comes back as the first. `max_quote_7500_binary.fbx` holds
        // both. Nothing the printer does can separate them; asserting it here
        // keeps the limit stated rather than discovered.
        let escaped = format!("&quot;{NAME_CLASS_SEPARATOR}Model");
        let read = round_trip(vec![node("Model", vec![FbxProperty::String(escaped)])]);
        let FbxProperty::String(name) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert_eq!(name, &quote);
    }

    #[test]
    fn embedded_media_comes_back_as_the_same_bytes() {
        for length in 0..8usize {
            let bytes: Vec<u8> = (0..length).map(|index| (index * 37 + 11) as u8).collect();
            let read = round_trip(vec![node("Content", vec![FbxProperty::Raw(bytes.clone())])]);
            if bytes.is_empty() {
                // Empty base64 is not base64, so the reader leaves the string.
                assert!(
                    matches!(&read[0].properties[0], FbxProperty::String(text) if text.is_empty())
                );
                continue;
            }
            let FbxProperty::Raw(read_bytes) = &read[0].properties[0] else {
                panic!("length {length}: {:?}", read[0].properties[0]);
            };
            assert_eq!(read_bytes, &bytes, "length {length}");
        }
    }

    #[test]
    fn raw_bytes_the_reader_would_not_decode_are_refused() {
        let error = print_document(&[node("Thumbnail", vec![FbxProperty::Raw(vec![1, 2, 3])])])
            .expect_err("only a Content node is read as base64");
        assert!(error.to_string().contains("Thumbnail"), "{error}");
    }

    #[test]
    fn two_array_properties_on_one_node_are_refused() {
        let error = print_document(&[node(
            "Vertices",
            vec![
                FbxProperty::F64Array(vec![1.0]),
                FbxProperty::F64Array(vec![2.0]),
            ],
        )])
        .expect_err("a node has one block, so it has one array");
        assert!(error.to_string().contains("Vertices"), "{error}");
    }

    #[test]
    fn a_long_array_is_broken_across_lines_and_still_reads_back() {
        let values: Vec<i32> = (0..5000).collect();
        let read = round_trip(vec![node(
            "PolygonVertexIndex",
            vec![FbxProperty::I32Array(values.clone())],
        )]);
        let FbxProperty::I32Array(read_values) = &read[0].properties[0] else {
            panic!("{:?}", read[0].properties[0]);
        };
        assert_eq!(read_values, &values);

        let text = print_document(&[node(
            "PolygonVertexIndex",
            vec![FbxProperty::I32Array(values)],
        )])
        .expect("printable");
        let longest = String::from_utf8_lossy(&text)
            .lines()
            .map(str::len)
            .max()
            .unwrap_or(0);
        assert!(longest <= ARRAY_LINE_BUDGET, "longest line {longest}");
    }

    #[test]
    fn a_node_keeps_its_children_alongside_its_array() {
        let mut parent = node("Vertices", vec![FbxProperty::F64Array(vec![1.5, 2.5])]);
        parent
            .children
            .push(node("Version", vec![FbxProperty::I32(7)]));
        let read = round_trip(vec![parent]);
        assert!(matches!(
            &read[0].properties[0],
            FbxProperty::F64Array(values) if values == &[1.5, 2.5]
        ));
        assert_eq!(read[0].children.len(), 1);
        assert_eq!(read[0].children[0].name, "Version");
    }

    #[test]
    fn an_empty_array_declares_zero_and_reads_back_empty() {
        let read = round_trip(vec![node("Edges", vec![FbxProperty::I32Array(Vec::new())])]);
        assert!(matches!(
            &read[0].properties[0],
            FbxProperty::I32Array(values) if values.is_empty()
        ));
    }
}
