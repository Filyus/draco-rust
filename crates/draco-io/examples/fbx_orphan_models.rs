//! Report which documents under a directory hold Models that reach neither
//! the document root nor a parent Model by OO connection.
//!
//! The reader drops such Models from the scene graph, which is only right
//! because the corpus was checked first: this is the tool that did it, and
//! the one to re-run before ever revisiting that rule. Per file, it prints
//! every such Model with whether it carries geometry and how many Models
//! parent under it -- the two things a rooting change would strand.
//!
//! ```text
//! cargo run --example fbx_orphan_models -- <dir>
//! ```

use draco_io::fbx_node::FbxProperty;
use draco_io::FbxReader;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn object_id(node: &draco_io::fbx_reader::FbxNode) -> Option<i64> {
    property_id(node.properties.first()?)
}

fn property_id(property: &FbxProperty) -> Option<i64> {
    match property {
        FbxProperty::I64(v) => Some(*v),
        FbxProperty::I32(v) => Some(i64::from(*v)),
        _ => None,
    }
}

fn object_name(node: &draco_io::fbx_reader::FbxNode) -> String {
    match node.properties.get(1) {
        Some(FbxProperty::String(v)) => v.split('\0').next().unwrap_or("?").to_string(),
        _ => "?".to_string(),
    }
}

fn find_all<'a>(
    node: &'a draco_io::fbx_reader::FbxNode,
    name: &str,
    out: &mut Vec<&'a draco_io::fbx_reader::FbxNode>,
) {
    if node.name == name {
        out.push(node);
    }
    for child in &node.children {
        find_all(child, name, out);
    }
}

fn collect_fbx(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fbx(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fbx"))
        {
            out.push(path);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .ok_or("usage: fbx_orphan_models <dir>")?
        .into();
    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();
    println!("{} files", files.len());

    let mut files_with_orphans = 0;
    let mut orphans_with_geometry = 0;
    for path in &files {
        let Ok(mut reader) = FbxReader::open(path) else {
            continue;
        };
        let Ok(roots) = reader.read_nodes() else {
            continue;
        };
        let mut objects = Vec::new();
        let mut connections = Vec::new();
        for root in &roots {
            find_all(root, "Objects", &mut objects);
            find_all(root, "Connections", &mut connections);
        }
        let mut models: Vec<(i64, &draco_io::fbx_reader::FbxNode)> = Vec::new();
        let mut geometries: BTreeSet<i64> = BTreeSet::new();
        for objects in &objects {
            let mut found = Vec::new();
            find_all(objects, "Model", &mut found);
            for model in found {
                if let Some(id) = object_id(model) {
                    models.push((id, model));
                }
            }
            let mut geoms = Vec::new();
            find_all(objects, "Geometry", &mut geoms);
            for geom in geoms {
                if let Some(id) = object_id(geom) {
                    geometries.insert(id);
                }
            }
        }
        let model_ids: BTreeSet<i64> = models.iter().map(|(id, _)| *id).collect();

        // OO connections child -> parent.
        let mut oo_parent_of_model = std::collections::HashMap::new();
        let mut geometry_of_model = std::collections::HashMap::new();
        let mut child_models_of = std::collections::HashMap::new();
        for connections in &connections {
            let mut cs = Vec::new();
            find_all(connections, "C", &mut cs);
            for c in cs {
                let (Some(FbxProperty::String(kind)), Some(child), Some(parent)) = (
                    c.properties.first(),
                    c.properties.get(1).and_then(property_id),
                    c.properties.get(2).and_then(property_id),
                ) else {
                    continue;
                };
                let (child, parent) = (child, parent);
                if kind != "OO" {
                    continue;
                }
                if model_ids.contains(&child) && model_ids.contains(&parent) {
                    oo_parent_of_model.insert(child, parent);
                    child_models_of
                        .entry(parent)
                        .or_insert_with(Vec::new)
                        .push(child);
                }
                if geometries.contains(&child) && model_ids.contains(&parent) {
                    geometry_of_model.insert(parent, child);
                }
                if model_ids.contains(&child) && parent == 0 {
                    oo_parent_of_model.insert(child, 0);
                }
            }
        }

        let orphans: Vec<_> = models
            .iter()
            .filter(|(id, _)| !oo_parent_of_model.contains_key(id))
            .collect();
        if orphans.is_empty() {
            continue;
        }
        files_with_orphans += 1;
        println!("== {} ==", path.display());
        for (id, node) in orphans {
            let has_geometry = geometry_of_model.contains_key(id);
            let children = child_models_of.get(id).map(Vec::len).unwrap_or(0);
            if has_geometry {
                orphans_with_geometry += 1;
            }
            println!(
                "  Model {id} {:?} geometry={} child_models={}",
                object_name(node),
                has_geometry,
                children
            );
        }
    }
    println!(
        "files with orphan models: {}, orphans carrying geometry: {}",
        files_with_orphans, orphans_with_geometry
    );
    Ok(())
}
