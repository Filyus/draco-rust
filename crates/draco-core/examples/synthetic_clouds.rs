//! Write the synthetic point clouds the crate's tests take through every
//! decode and encode path as binary PLY, for timing on shapes no capture at
//! hand has, and print what each compresses to.
//!
//! ```text
//! cargo run --release --example synthetic_clouds -- <out-dir> [points] [seed]
//! ```
//!
//! Every shape is written in its own order and shuffled. Each file is read
//! back with `draco-io` before it is reported, so a file that does not parse
//! into the attributes it was written from stops the run.

// `DataType`, `PointAttribute` and `PointCloud` are what the generator names
// through `crate::`, which is this example when it is included here.
use draco_core::{
    DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute, PointCloud,
    PointCloudEncoder,
};
use std::io::Write;
use std::path::Path;

#[path = "../src/synthetic_cloud.rs"]
#[allow(dead_code)]
mod synthetic_cloud;

use synthetic_cloud::{Cloud, Values};

/// The PLY type of a value and its size in bytes.
fn ply_type(values: &Values) -> (&'static str, usize) {
    match values {
        Values::F32(_) => ("float", 4),
        Values::F64(_) => ("double", 8),
        Values::U8(_) => ("uchar", 1),
        Values::U16(_) => ("ushort", 2),
        Values::I32(_) => ("int", 4),
    }
}

/// The PLY property names of a column's components.
fn property_names(column: &synthetic_cloud::Column) -> Vec<String> {
    let names: &[&str] = match column.kind {
        GeometryAttributeType::Position => &["x", "y", "z"],
        GeometryAttributeType::Normal => &["nx", "ny", "nz"],
        GeometryAttributeType::Color => &["red", "green", "blue"],
        _ => return vec![column.name.to_string()],
    };
    names.iter().map(|name| name.to_string()).collect()
}

fn write_ply(cloud: &Cloud, path: &Path) -> std::io::Result<()> {
    let mut out = Vec::new();
    writeln!(out, "ply\nformat binary_little_endian 1.0")?;
    writeln!(out, "element vertex {}", cloud.points)?;
    for column in &cloud.columns {
        let (type_name, _) = ply_type(&column.values);
        for name in property_names(column) {
            writeln!(out, "property {type_name} {name}")?;
        }
    }
    writeln!(out, "end_header")?;
    let columns: Vec<(Vec<u8>, usize)> = cloud
        .columns
        .iter()
        .map(|column| {
            let (_, width) = ply_type(&column.values);
            (column.values.bytes(), width * column.components)
        })
        .collect();
    for point in 0..cloud.points {
        for (bytes, stride) in &columns {
            out.extend_from_slice(&bytes[point * stride..(point + 1) * stride]);
        }
    }
    std::fs::write(path, out)
}

fn encoded_size(cloud: &Cloud) -> Result<usize, String> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0);
    for (id, column) in cloud.columns.iter().enumerate() {
        if column.quantization_bits > 0 {
            options.set_attribute_int(id as i32, "quantization_bits", column.quantization_bits);
        }
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.to_point_cloud());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .map_err(|e| e.to_string())?;
    Ok(buffer.data().len())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(dir) = args.get(1) else {
        eprintln!("usage: synthetic_clouds <out-dir> [points] [seed]");
        std::process::exit(2);
    };
    let points: usize = args
        .get(2)
        .map_or(1_000_000, |v| v.parse().expect("points"));
    let seed: u64 = args.get(3).map_or(1, |v| v.parse().expect("seed"));
    std::fs::create_dir_all(dir).expect("creates the output directory");

    println!(
        "{:<22} {:>10} {:>10} {:>12} {:>9}",
        "cloud", "points", "PLY MB", "encoded", "bits/pt"
    );
    for cloud in synthetic_cloud::all(points, seed) {
        let shuffled = cloud.shuffled(seed ^ 0x5eed);
        for cloud in [cloud, shuffled] {
            let path = Path::new(dir).join(format!("{}.ply", cloud.name));
            write_ply(&cloud, &path).expect("writes the PLY");
            let read = draco_io::ply_reader::PlyReader::from_bytes(
                std::fs::read(&path).expect("reads the PLY back"),
            )
            .with_generic_attributes(true)
            .read_mesh()
            .expect("parses the PLY back")
            .into_point_cloud();
            assert_eq!(read.num_points(), cloud.points, "{}", cloud.name);
            assert_eq!(
                read.num_attributes() as usize,
                cloud.columns.len(),
                "{}: attributes read back",
                cloud.name
            );
            let ply_mb = std::fs::metadata(&path).map_or(0, |m| m.len()) as f64 / 1e6;
            match encoded_size(&cloud) {
                Ok(bytes) => println!(
                    "{:<22} {:>10} {:>10.1} {:>12} {:>9.2}",
                    cloud.name,
                    cloud.points,
                    ply_mb,
                    bytes,
                    bytes as f64 * 8.0 / cloud.points.max(1) as f64
                ),
                Err(e) => println!(
                    "{:<22} {:>10} {ply_mb:>10.1}  {e}",
                    cloud.name, cloud.points
                ),
            }
        }
    }
}
