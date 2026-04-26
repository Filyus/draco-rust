//! Inspect an FBX file using FbxReader and print node names
//!
//! Run with: cargo run --example fbx_inspect -- output/triangle.fbx

use draco_io::FbxReader;
use std::env;

fn print_node(node: &draco_io::fbx_reader::FbxNode, indent: usize) {
    let pad = " ".repeat(indent);
    println!(
        "{}Node: {} (props: {}, children: {})",
        pad,
        node.name,
        node.properties.len(),
        node.children.len()
    );
    for child in &node.children {
        print_node(child, indent + 2);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let path = if args.len() > 1 {
        &args[1]
    } else {
        "crates/draco-io/input/scene.fbx"
    }; // Default to a known existing file if any
    let mut reader = FbxReader::open(path)?;
    let nodes = reader.read_nodes()?;
    println!("Top-level nodes: {}", nodes.len());
    for n in &nodes {
        print_node(n, 0);
    }
    Ok(())
}
