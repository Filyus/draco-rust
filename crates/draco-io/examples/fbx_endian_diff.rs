//! Compare a big-endian FBX against its little-endian twin.
//!
//! The two files describe the same scene, so every decoded value must match
//! except the version field. Run with:
//!
//! ```text
//! cargo run --example fbx_endian_diff -- little.fbx big.fbx
//! ```

use draco_io::fbx_reader::{FbxNode, FbxProperty};
use draco_io::FbxReader;
use std::env;

fn summarize(node: &FbxNode, depth: usize, out: &mut Vec<String>) {
    let props: Vec<String> = node
        .properties
        .iter()
        .map(|p| match p {
            FbxProperty::Bool(v) => format!("B{v}"),
            FbxProperty::U8(v) => format!("Z{v}"),
            FbxProperty::I16(v) => format!("Y{v}"),
            FbxProperty::I32(v) => format!("I{v}"),
            FbxProperty::I64(v) => format!("L{v}"),
            FbxProperty::F32(v) => format!("F{v:.6}"),
            FbxProperty::F64(v) => format!("D{v:.9}"),
            FbxProperty::String(v) => format!("S{v}"),
            FbxProperty::Raw(v) => format!("R[{}]", v.len()),
            FbxProperty::BoolArray(v) => format!("b{:?}", &v[..v.len().min(4)]),
            FbxProperty::I32Array(v) => format!("i[{}]{:?}", v.len(), &v[..v.len().min(4)]),
            FbxProperty::I64Array(v) => format!("l[{}]{:?}", v.len(), &v[..v.len().min(4)]),
            FbxProperty::F32Array(v) => format!("f[{}]{:?}", v.len(), &v[..v.len().min(4)]),
            FbxProperty::F64Array(v) => format!("d[{}]{:?}", v.len(), &v[..v.len().min(4)]),
        })
        .collect();
    out.push(format!(
        "{}{} {}",
        "  ".repeat(depth),
        node.name,
        props.join(",")
    ));
    for child in &node.children {
        summarize(child, depth + 1, out);
    }
}

fn load(path: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut reader = FbxReader::open(path)?;
    println!(
        "{path}: version={} order={:?}",
        reader.version(),
        reader.byte_order()
    );
    let mut out = Vec::new();
    for node in reader.read_nodes()? {
        summarize(&node, 0, &mut out);
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        return Err("usage: fbx_endian_diff <little.fbx> <big.fbx>".into());
    }
    let little = load(&args[1])?;
    let big = load(&args[2])?;

    if little.len() != big.len() {
        println!("MISMATCH: {} lines vs {} lines", little.len(), big.len());
    }
    let mut differences = 0;
    for (index, (l, b)) in little.iter().zip(big.iter()).enumerate() {
        // The version node legitimately differs between the two profiles.
        if l != b {
            differences += 1;
            if differences <= 10 {
                println!("line {index}:\n  LE {l}\n  BE {b}");
            }
        }
    }
    println!(
        "compared {} lines, {differences} differing",
        little.len().min(big.len())
    );
    if differences == 0 && little.len() == big.len() {
        println!("IDENTICAL");
    }
    Ok(())
}
