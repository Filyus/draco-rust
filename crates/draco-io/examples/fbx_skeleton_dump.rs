//! Dump the skeleton-relevant records of an FBX document: limb Models, their
//! NodeAttributes, the Deformer tree, the Connections between them, and the
//! `Definitions` counts an importer cross-checks.
//!
//! Companion to `fbx_inspect`, which prints the whole tree: this one exists
//! for diffing what a round trip did to a rig, where the interesting question
//! is not the tree shape but whether a joint still reads as a joint.
//!
//! ```text
//! cargo run --example fbx_skeleton_dump -- file.fbx
//! ```

use draco_io::fbx_reader::FbxNode;
use draco_io::FbxReader;
use std::collections::BTreeSet;

fn as_i64(node: &FbxNode, index: usize) -> Option<i64> {
    match node.properties.get(index)? {
        draco_io::fbx_node::FbxProperty::I64(v) => Some(*v),
        _ => None,
    }
}

fn as_str(node: &FbxNode, index: usize) -> Option<&str> {
    match node.properties.get(index)? {
        draco_io::fbx_node::FbxProperty::String(v) => Some(v),
        _ => None,
    }
}

fn find_all<'a>(node: &'a FbxNode, name: &str, out: &mut Vec<&'a FbxNode>) {
    if node.name == name {
        out.push(node);
    }
    for child in &node.children {
        find_all(child, name, out);
    }
}

fn property_numbers(p: &draco_io::fbx_node::FbxProperty) -> Option<Vec<f64>> {
    use draco_io::fbx_node::FbxProperty::*;
    Some(match p {
        F64(v) => vec![*v],
        F32(v) => vec![f64::from(*v)],
        I32(v) => vec![f64::from(*v)],
        I64(v) => vec![*v as f64],
        F64Array(v) => v.to_vec(),
        F32Array(v) => v.iter().map(|&x| f64::from(x)).collect(),
        I32Array(v) => v.iter().map(|&x| f64::from(x)).collect(),
        _ => return None,
    })
}

fn named_properties(model: &FbxNode) -> Vec<(&str, Vec<f64>)> {
    let mut out = Vec::new();
    for props in model.children.iter().filter(|c| c.name == "Properties70") {
        for p in props.children.iter().filter(|c| c.name == "P") {
            let Some(name) = as_str(p, 0) else {
                continue;
            };
            for value in p.properties.iter().skip(4) {
                if let Some(numbers) = property_numbers(value) {
                    out.push((name, numbers));
                    break;
                }
            }
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: fbx_skeleton_dump <file.fbx>")?;
    let mut reader = FbxReader::open(&path)?;
    let roots = reader.read_nodes()?;

    let mut objects = Vec::new();
    let mut connections = Vec::new();
    let mut definitions = Vec::new();
    for root in &roots {
        find_all(root, "Objects", &mut objects);
        find_all(root, "Connections", &mut connections);
        find_all(root, "Definitions", &mut definitions);
    }

    let mut limb_ids: BTreeSet<i64> = BTreeSet::new();
    let mut attr_ids: BTreeSet<i64> = BTreeSet::new();

    println!("== {path} ==");
    println!("-- Models (all, class noted; limb-ish marked) --");
    for objects in &objects {
        let mut models = Vec::new();
        find_all(objects, "Model", &mut models);
        for model in models {
            let (Some(id), Some(name)) = (as_i64(model, 0), as_str(model, 1)) else {
                continue;
            };
            let class = as_str(model, 2).unwrap_or("?");
            let limbish = ["LimbNode", "Root", "Null", "Limb"]
                .iter()
                .any(|k| class.contains(k));
            if !limbish && !name.contains("_end") && !name.contains("Armature") {
                continue;
            }
            if limbish {
                limb_ids.insert(id);
            }
            let interesting: Vec<_> = named_properties(model)
                .into_iter()
                .filter(|(n, _)| {
                    matches!(
                        *n,
                        "LclTranslation" | "LclRotation" | "LimbLength" | "Size" | "Translation"
                    )
                })
                .collect();
            print!("  Model {id} {name:?} class={class}");
            for (n, v) in interesting {
                print!(" {n}={:?}", v);
            }
            println!();
        }
    }

    println!("-- NodeAttributes --");
    for objects in &objects {
        let mut attrs = Vec::new();
        find_all(objects, "NodeAttribute", &mut attrs);
        for attr in attrs {
            let id = as_i64(attr, 0).unwrap_or(0);
            let (name, class) = (
                as_str(attr, 1).unwrap_or("?"),
                as_str(attr, 2).unwrap_or("?"),
            );
            attr_ids.insert(id);
            let type_flags = attr
                .children
                .iter()
                .find(|c| c.name == "TypeFlags")
                .and_then(|c| as_str(c, 0).map(str::to_string));
            println!("  NodeAttribute {id} {name:?} class={class} TypeFlags={type_flags:?}");
        }
    }

    println!("-- Deformers --");
    for objects in &objects {
        let mut deformers = Vec::new();
        find_all(objects, "Deformer", &mut deformers);
        for d in deformers {
            println!(
                "  Deformer {} {:?} class={}",
                as_i64(d, 0).unwrap_or(0),
                as_str(d, 1).unwrap_or("?"),
                as_str(d, 2).unwrap_or("?")
            );
        }
    }

    println!("-- Connections touching limbs or attributes --");
    for connections in &connections {
        let mut cs = Vec::new();
        find_all(connections, "C", &mut cs);
        for c in cs {
            let (Some(kind), Some(a), Some(b)) = (as_str(c, 0), as_i64(c, 1), as_i64(c, 2)) else {
                continue;
            };
            if limb_ids.contains(&a)
                || limb_ids.contains(&b)
                || attr_ids.contains(&a)
                || attr_ids.contains(&b)
            {
                let extra = as_str(c, 3)
                    .map(|s| format!(" prop={s:?}"))
                    .unwrap_or_default();
                println!("  {kind} {a} -> {b}{extra}");
            }
        }
    }

    println!("-- Definitions counts --");
    for definitions in &definitions {
        let mut counts = Vec::new();
        find_all(definitions, "ObjectType", &mut counts);
        for ot in counts {
            let Some(kind) = as_str(ot, 0) else { continue };
            if !matches!(kind, "Model" | "NodeAttribute" | "Deformer" | "Pose") {
                continue;
            }
            let count = ot
                .children
                .iter()
                .find(|c| c.name == "Count")
                .and_then(|c| match c.properties.first() {
                    Some(draco_io::fbx_node::FbxProperty::I32(v)) => Some(*v),
                    _ => None,
                });
            println!("  ObjectType {kind} Count={count:?}");
        }
    }

    Ok(())
}
