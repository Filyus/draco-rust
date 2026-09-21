//! Encode a splat scene several ways and write each back as a PLY, so that a
//! renderer can be asked what the difference looks like.
//!
//! Every other probe in this area reports bytes. Bytes are half of a lossy
//! decision and the cheap half: `splat_colour_scale_domain_probe` and
//! `splat_opacity_domain_probe` both end at "and whether that is worth doing
//! is a rendering question", and the research note has an open item saying the
//! same about the largest lever of all, the harmonics' bit budget. This is the
//! part that makes those questions answerable: the same scene, encoded and
//! decoded under each setting, handed back in a form a viewer reads.
//!
//! The reference arm is the source cloud written by this same writer rather
//! than the input file. Comparing against the input would fold whatever this
//! writer does differently into every measurement; comparing against a written
//! copy of the source leaves only the encode and decode.
//!
//! ```text
//! DRACO_SPLAT_PLY=../../dev/splat-corpus/train_7000.ply DRACO_PROBE_OUT=/tmp/arms \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test splat_render_arms_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::io::Write as _;
use std::path::{Path, PathBuf};

use draco_core::{
    DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;

/// What an arm does to the scene before it is written back.
struct Arm {
    name: &'static str,
    /// `None` leaves the scene alone: the reference every other arm is read
    /// against, and the one that proves the writer is not the difference.
    encode: Option<Budget>,
}

#[derive(Clone, Copy)]
struct Budget {
    positions: i32,
    harmonics: i32,
    /// Everything that is neither a position nor a harmonic.
    rest: i32,
    search: bool,
    spatial: bool,
}

fn attribute_names(cloud: &PointCloud) -> Vec<Option<String>> {
    (0..cloud.num_attributes())
        .map(|id| {
            let unique_id = cloud.attribute(id).unique_id();
            cloud
                .attribute_metadata_by_unique_id(unique_id)?
                .metadata()
                .get_string("name")
                .map(str::to_string)
        })
        .collect()
}

fn read_f32(attribute: &PointAttribute, value: usize, component: usize) -> f32 {
    let stride = attribute.byte_stride() as usize;
    let mut bytes = [0u8; 4];
    attribute
        .buffer()
        .read(value * stride + component * 4, &mut bytes);
    f32::from_le_bytes(bytes)
}

/// The cloud as a binary little-endian PLY of float properties.
///
/// Written here rather than with `PlyWriter` because that one carries the
/// named types a mesh has -- position, normal, colour, texture coordinate --
/// and a splat's payload is none of those. The reader has carried arbitrary
/// named properties since `with_generic_attributes`; the writer has no matching
/// half, which is a real gap in `draco-io` and is worked around rather than
/// fixed from a probe.
fn write_ply(path: &Path, cloud: &PointCloud, names: &[Option<String>]) -> std::io::Result<()> {
    let num_points = cloud.num_points();

    // One column per component, in the order the properties were declared.
    let mut columns: Vec<(String, i32, usize)> = Vec::new();
    for id in 0..cloud.num_attributes() {
        let attribute = cloud.attribute(id);
        let components = attribute.num_components() as usize;
        match attribute.attribute_type() {
            GeometryAttributeType::Position => {
                for (component, axis) in ["x", "y", "z"].iter().enumerate() {
                    columns.push(((*axis).to_string(), id, component));
                }
            }
            GeometryAttributeType::Normal => {
                for (component, axis) in ["nx", "ny", "nz"].iter().enumerate() {
                    columns.push(((*axis).to_string(), id, component));
                }
            }
            _ => {
                let base = names[id as usize]
                    .clone()
                    .unwrap_or_else(|| format!("attribute_{id}"));
                for component in 0..components {
                    let name = if components == 1 {
                        base.clone()
                    } else {
                        format!("{base}_{component}")
                    };
                    columns.push((name, id, component));
                }
            }
        }
    }

    let mut header = String::from("ply\nformat binary_little_endian 1.0\n");
    header.push_str(&format!("element vertex {num_points}\n"));
    for (name, _, _) in &columns {
        header.push_str(&format!("property float {name}\n"));
    }
    header.push_str("end_header\n");

    let mut bytes = Vec::with_capacity(header.len() + num_points * columns.len() * 4);
    bytes.extend_from_slice(header.as_bytes());
    for point in 0..num_points {
        for (_, id, component) in &columns {
            let attribute = cloud.attribute(*id);
            let value_index = attribute.mapped_index(PointIndex(point as u32));
            let value = read_f32(attribute, value_index.0 as usize, *component);
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    // Written and synced through the one handle: a renderer reads this next,
    // and on Windows a reopened read-only handle cannot be synced.
    let mut file = std::fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// The scene through the encoder and back, at the given budget.
fn round_trip(cloud: &PointCloud, names: &[Option<String>], budget: Budget) -> (PointCloud, usize) {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(budget.search);
    options.set_spatial_point_order(budget.spatial);
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => budget.positions,
            _ => {
                let harmonic = names[id as usize]
                    .as_deref()
                    .is_some_and(|name| name.starts_with("f_rest_"));
                if harmonic {
                    budget.harmonics
                } else {
                    budget.rest
                }
            }
        };
        options.set_attribute_int(id, "quantization_bits", bits);
    }

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    let bytes = buffer.data().len();

    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("a stream this crate wrote must decode");
    (decoded, bytes)
}

#[test]
#[ignore = "writes files for a renderer: run with --release --ignored --nocapture"]
fn write_the_arms() {
    let Some(path) = std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from) else {
        println!("DRACO_SPLAT_PLY is not set; nothing to write");
        return;
    };
    let Some(out) = std::env::var_os("DRACO_PROBE_OUT").map(PathBuf::from) else {
        println!("DRACO_PROBE_OUT is not set; nowhere to write");
        return;
    };
    std::fs::create_dir_all(&out).expect("the output directory");

    let source = std::fs::read(&path).expect("the scene reads");
    let mesh = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh()
        .expect("the scene parses");
    let cloud = mesh.into_point_cloud();
    let names = attribute_names(&cloud);
    let num_points = cloud.num_points();
    println!(
        "scene: {} ({num_points} splats, {} attributes)",
        path.display(),
        cloud.num_attributes()
    );

    let full = Budget {
        positions: 16,
        harmonics: 8,
        rest: 8,
        search: true,
        spatial: true,
    };
    let arms = [
        Arm {
            name: "source",
            encode: None,
        },
        Arm {
            name: "budget",
            encode: Some(full),
        },
        Arm {
            name: "harmonics6",
            encode: Some(Budget {
                harmonics: 6,
                ..full
            }),
        },
        Arm {
            name: "harmonics4",
            encode: Some(Budget {
                harmonics: 4,
                ..full
            }),
        },
        Arm {
            name: "positions12",
            encode: Some(Budget {
                positions: 12,
                ..full
            }),
        },
    ];

    println!();
    println!("{:<14} {:>14} {:>12}", "arm", "encoded bytes", "B/point");
    for arm in &arms {
        let file = out.join(format!("{}.ply", arm.name));
        match arm.encode {
            None => {
                write_ply(&file, &cloud, &names).expect("writes");
                println!("{:<14} {:>14} {:>12}", arm.name, "-", "-");
            }
            Some(budget) => {
                let (decoded, bytes) = round_trip(&cloud, &names, budget);
                // The decoded cloud carries the names through its own metadata.
                let decoded_names = attribute_names(&decoded);
                write_ply(&file, &decoded, &decoded_names).expect("writes");
                println!(
                    "{:<14} {bytes:>14} {:>12.3}",
                    arm.name,
                    bytes as f64 / num_points as f64
                );
            }
        }
    }

    println!();
    println!("  written to {}", out.display());
    println!("  the `source` arm went through this writer and nothing else, so");
    println!("  a difference against it is the encode and decode alone");
}
