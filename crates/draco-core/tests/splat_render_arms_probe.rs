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
//! Both paths absolute: cargo runs a test from its own crate's directory, so a
//! relative one resolves somewhere other than where it was typed.
//!
//! `DRACO_PROBE_ARMS=budget,raiseall` writes only the arms it names, plus
//! `source`.
//!
//! ```text
//! DRACO_SPLAT_PLY=<scene.ply> DRACO_PROBE_OUT=<output directory> \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --features encoder,decoder --test splat_render_arms_probe -- --ignored --nocapture
//! ```

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::io::Write as _;
use std::path::{Path, PathBuf};

use draco_core::mesh::Mesh;
use draco_core::{
    DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder, PointIndex,
};
use draco_io::{PlyWriter, Writer};

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
    /// Overrides by group: a property-name prefix, or `position`, and its
    /// bits. Raising one group while the rest stay at the budget finds which
    /// of them caps the picture.
    raise: &'static [(&'static str, i32)],
    search: bool,
    spatial: bool,
    /// What this arm does to the values before they are encoded, beyond the
    /// bit budget above.
    change: Change,
}

/// A change to the scene itself rather than to how many bits describe it.
///
/// Every one of these was measured in bytes by another probe here, and every
/// one of those measurements ended at "and whether that is worth doing is a
/// rendering question". These are the arms that ask a renderer.
#[derive(Clone, Copy)]
enum Change {
    /// The bit budget and nothing else.
    None,
    /// Quantize `sigmoid(opacity)` at this many bits instead of the logit at
    /// `rest`. From `splat_opacity_domain_probe`.
    Alpha(i32),
    /// Drop every gaussian whose alpha is below this. From
    /// `splat_invisible_splats_probe` -- the one lever that removes points
    /// rather than bits, so the arm's cost is a point count as well as a
    /// picture.
    Prune(f32),
    /// Clamp `f_dc_*` to this window before encoding. SPZ's is
    /// `[-3.33, 3.33]`; the reason to ask a renderer is that a colour outside
    /// it is not necessarily wrong, being what the higher bands correct.
    ClampColour(f32, f32),
    /// Clamp `scale_*` to this window. SPZ's is `[-10, 5.94]` in the log.
    ClampScale(f32, f32),
    /// Clip colour and scale to their own two-sided percentile window.
    /// Worth about 2% of the file, at a displacement the sizes cannot judge.
    Percentile(f64),
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// The inverse, with the ends pulled in.
///
/// Quantizing alpha puts values at exactly 0 and 1, whose logits are infinite,
/// and a PLY of infinities renders nothing. The clamp is a quarter of a step
/// of a six-bit alpha from each end -- far finer than the quantizer that
/// produced the value, so it cannot be what the arm is measuring.
fn logit(a: f32) -> f32 {
    let a = a.clamp(1.0 / 256.0, 1.0 - 1.0 / 256.0);
    (a / (1.0 - a)).ln()
}

/// Which attributes carry properties whose names begin with `prefix`.
fn attributes_named(names: &[Option<String>], prefix: &str) -> Vec<i32> {
    names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.as_deref().is_some_and(|name| name.starts_with(prefix)))
        .map(|(id, _)| id as i32)
        .collect()
}

/// The two-sided `fraction` percentile of everything in `targets`, together.
///
/// Together rather than per component, because the three components of a
/// colour share a scale and clipping them to different windows would tint.
fn percentile_window(cloud: &PointCloud, targets: &[i32], fraction: f64) -> (f32, f32) {
    let mut values: Vec<f32> = Vec::new();
    for &id in targets {
        let attribute = cloud.attribute(id);
        for point in 0..cloud.num_points() {
            let value = attribute.mapped_index(PointIndex(point as u32));
            for component in 0..attribute.num_components() as usize {
                values.push(read_f32(attribute, value.0 as usize, component));
            }
        }
    }
    values.sort_by(f32::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    (at(fraction), at(1.0 - fraction))
}

/// The cloud with `keep` applied to every point, and the rest gone.
fn select(cloud: &PointCloud, keep: &[bool]) -> PointCloud {
    let names = attribute_names(cloud);
    let kept: Vec<usize> = (0..cloud.num_points())
        .filter(|&point| keep[point])
        .collect();
    let mut out = PointCloud::new();
    out.set_num_points(kept.len());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components() as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            kept.len(),
        );
        let buffer = attribute.buffer_mut();
        for (at, &point) in kept.iter().enumerate() {
            let value_index = source.mapped_index(PointIndex(point as u32));
            for component in 0..components {
                let value = read_f32(source, value_index.0 as usize, component);
                buffer.write((at * components + component) * 4, &value.to_le_bytes());
            }
        }
        let new_id = out.add_attribute(attribute);
        if let Some(name) = names[id as usize].clone() {
            let unique_id = out.attribute(new_id).unique_id();
            let mut metadata = Metadata::new();
            metadata.set_string("name", name).expect("string entry");
            out.metadata_or_insert()
                .set_attribute_metadata(unique_id, metadata);
        }
    }
    out
}

/// The cloud with every value of `targets` put through `transform`.
fn mapped(cloud: &PointCloud, targets: &[i32], transform: impl Fn(f32) -> f32) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(cloud.num_points());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components() as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            cloud.num_points(),
        );
        let buffer = attribute.buffer_mut();
        let touched = targets.contains(&id);
        #[allow(clippy::needless_range_loop)]
        for point in 0..cloud.num_points() {
            let value_index = source.mapped_index(PointIndex(point as u32));
            for component in 0..components {
                let value = read_f32(source, value_index.0 as usize, component);
                let value = if touched { transform(value) } else { value };
                buffer.write((point * components + component) * 4, &value.to_le_bytes());
            }
        }
        let new_id = out.add_attribute(attribute);
        if let Some(name) = names[id as usize].clone() {
            let unique_id = out.attribute(new_id).unique_id();
            let mut metadata = Metadata::new();
            metadata.set_string("name", name).expect("string entry");
            out.metadata_or_insert()
                .set_attribute_metadata(unique_id, metadata);
        }
    }
    out
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

/// The cloud with one attribute's values replaced, everything else copied.
fn with_values(cloud: &PointCloud, target: i32, values: &[f32]) -> PointCloud {
    let names = attribute_names(cloud);
    let mut out = PointCloud::new();
    out.set_num_points(cloud.num_points());
    for id in 0..cloud.num_attributes() {
        let source = cloud.attribute(id);
        let components = source.num_components() as usize;
        let mut attribute = PointAttribute::new();
        attribute.init(
            source.attribute_type(),
            source.num_components(),
            source.data_type(),
            source.normalized(),
            cloud.num_points(),
        );
        let buffer = attribute.buffer_mut();
        // The index addresses three different things -- the replacement
        // values, the source's point mapping, and the write offset -- so it
        // stays an index.
        #[allow(clippy::needless_range_loop)]
        for point in 0..cloud.num_points() {
            let value_index = source.mapped_index(PointIndex(point as u32));
            for component in 0..components {
                let value = if id == target {
                    values[point]
                } else {
                    read_f32(source, value_index.0 as usize, component)
                };
                buffer.write((point * components + component) * 4, &value.to_le_bytes());
            }
        }
        let new_id = out.add_attribute(attribute);
        if let Some(name) = names[id as usize].clone() {
            let unique_id = out.attribute(new_id).unique_id();
            let mut metadata = Metadata::new();
            metadata.set_string("name", name).expect("string entry");
            out.metadata_or_insert()
                .set_attribute_metadata(unique_id, metadata);
        }
    }
    out
}

/// Which attribute carries `opacity`, if any.
fn opacity_id(names: &[Option<String>]) -> Option<i32> {
    names
        .iter()
        .position(|name| name.as_deref() == Some("opacity"))
        .map(|id| id as i32)
}

/// The cloud as a binary little-endian PLY, every named property carried.
///
/// Through `PlyWriter` with generic attributes on, which writes each named
/// generic back under its own name and type: the writing half of the
/// reader's `with_generic_attributes`.
fn write_ply(path: &Path, cloud: &PointCloud) -> std::io::Result<()> {
    // The writer takes a mesh, and a point cloud is a mesh without faces. The
    // attributes keep their ids, because that is what their names are keyed
    // by in the metadata.
    let mut mesh = Mesh::new();
    mesh.set_num_points(cloud.num_points());
    for id in 0..cloud.num_attributes() {
        let attribute = cloud.attribute(id);
        let unique_id = attribute.unique_id();
        mesh.add_attribute_preserve_unique_id(attribute.clone());
        if let Some(metadata) = cloud.attribute_metadata_by_unique_id(unique_id) {
            mesh.metadata_or_insert()
                .set_attribute_metadata(unique_id, metadata.metadata().clone());
        }
    }

    let mut writer = PlyWriter::new()
        .with_binary_little_endian()
        .with_generic_attributes(true);
    Writer::add_mesh(&mut writer, &mesh, None)?;
    let bytes = writer.write_to_vec()?;

    // Written and synced through the one handle: a renderer reads this next,
    // and on Windows a reopened read-only handle cannot be synced.
    let mut file = std::fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

/// The scene through the encoder and back, at the given budget.
fn round_trip(
    cloud: &PointCloud,
    names: &[Option<String>],
    budget: Budget,
) -> (PointCloud, usize, usize, Vec<u8>) {
    // Every change happens around the encode, not inside the file. What is
    // written back out is what the dialect says the property holds -- a logit
    // in `opacity`, a log in `scale_*` -- because that is what a renderer will
    // apply its activation to. An arm that handed the viewer alpha in
    // `opacity` would be measuring a second sigmoid, not a bit budget.
    let alpha_id = match budget.change {
        Change::Alpha(_) => opacity_id(names),
        _ => None,
    };
    let owned;
    let cloud = match budget.change {
        Change::None => cloud,
        Change::Alpha(_) => {
            let id = alpha_id.expect("an alpha arm needs an opacity attribute");
            let attribute = cloud.attribute(id);
            let alphas: Vec<f32> = (0..cloud.num_points())
                .map(|point| {
                    let value = attribute.mapped_index(PointIndex(point as u32));
                    sigmoid(read_f32(attribute, value.0 as usize, 0))
                })
                .collect();
            owned = with_values(cloud, id, &alphas);
            &owned
        }
        Change::Prune(threshold) => {
            let id = opacity_id(names).expect("a prune arm needs an opacity attribute");
            let attribute = cloud.attribute(id);
            let keep: Vec<bool> = (0..cloud.num_points())
                .map(|point| {
                    let value = attribute.mapped_index(PointIndex(point as u32));
                    sigmoid(read_f32(attribute, value.0 as usize, 0)) >= threshold
                })
                .collect();
            owned = select(cloud, &keep);
            &owned
        }
        Change::ClampColour(low, high) => {
            owned = mapped(cloud, &attributes_named(names, "f_dc_"), |v| {
                v.clamp(low, high)
            });
            &owned
        }
        Change::ClampScale(low, high) => {
            owned = mapped(cloud, &attributes_named(names, "scale_"), |v| {
                v.clamp(low, high)
            });
            &owned
        }
        Change::Percentile(fraction) => {
            // Colour and scale take their own windows: they are different
            // quantities and share nothing but the idea.
            let colour = attributes_named(names, "f_dc_");
            let scale = attributes_named(names, "scale_");
            let (colour_low, colour_high) = percentile_window(cloud, &colour, fraction);
            let (scale_low, scale_high) = percentile_window(cloud, &scale, fraction);
            let clipped = mapped(cloud, &colour, |v| v.clamp(colour_low, colour_high));
            owned = mapped(&clipped, &scale, |v| v.clamp(scale_low, scale_high));
            &owned
        }
    };
    let names = &attribute_names(cloud);
    let points = cloud.num_points();

    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(budget.search);
    options.set_spatial_point_order(budget.spatial);
    for id in 0..cloud.num_attributes() {
        let bits = match cloud.attribute(id).attribute_type() {
            GeometryAttributeType::Position => budget.positions,
            _ if alpha_id.is_some() && names[id as usize].as_deref() == Some("opacity") => {
                match budget.change {
                    Change::Alpha(bits) => bits,
                    _ => unreachable!("alpha_id is set only for an alpha arm"),
                }
            }
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
        let is_position = cloud.attribute(id).attribute_type() == GeometryAttributeType::Position;
        let name = names[id as usize].as_deref().unwrap_or("");
        let bits = budget
            .raise
            .iter()
            .find(|(group, _)| {
                if *group == "position" {
                    is_position
                } else {
                    !is_position && name.starts_with(group)
                }
            })
            .map_or(bits, |(_, raised)| *raised);
        options.set_attribute_int(id, "quantization_bits", bits);
    }

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    let bytes = buffer.data().len();

    let stream = buffer.data().to_vec();

    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("a stream this crate wrote must decode");

    let decoded = match alpha_id.and_then(|_| opacity_id(&attribute_names(&decoded))) {
        None => decoded,
        // The alpha arm alone comes back in the wrong domain, and goes home.
        Some(id) => {
            let attribute = decoded.attribute(id);
            let logits: Vec<f32> = (0..decoded.num_points())
                .map(|point| {
                    let value = attribute.mapped_index(PointIndex(point as u32));
                    logit(read_f32(attribute, value.0 as usize, 0))
                })
                .collect();
            with_values(&decoded, id, &logits)
        }
    };
    (decoded, bytes, points, stream)
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
        change: Change::None,
        raise: &[],
    };
    let raised = |name, raise| Arm {
        name,
        encode: Some(Budget { raise, ..full }),
    };
    let arm = |name, change| Arm {
        name,
        encode: Some(Budget { change, ..full }),
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
        arm("alpha6", Change::Alpha(6)),
        // The one lever that removes points. 1/255 is where this trainer
        // already pruned, so the arms above it are the ones with anything to
        // take: 0.05 is what the size measurement found worth asking about.
        arm("prune005", Change::Prune(0.05)),
        arm("prune01", Change::Prune(0.1)),
        // SPZ's windows, in this crate's quantizer rather than theirs.
        arm("colourspz", Change::ClampColour(-3.33, 3.33)),
        arm("scalespz", Change::ClampScale(-10.0, 5.94)),
        arm("percentile2", Change::Percentile(0.02)),
        // One group raised at a time, the rest at the budget: whichever lifts
        // the picture toward `raiseall` is what caps it.
        raised("raiseposition", &[("position", 20)]),
        raised("raiserotation", &[("rot_", 12)]),
        raised("raisescale", &[("scale_", 12)]),
        raised("raiseopacity", &[("opacity", 12)]),
        raised("raisecolour", &[("f_dc_", 12)]),
        raised("raiseharmonics", &[("f_rest_", 12)]),
        raised(
            "raiseall",
            &[
                ("position", 20),
                ("rot_", 12),
                ("scale_", 12),
                ("opacity", 12),
                ("f_dc_", 12),
                ("f_rest_", 12),
            ],
        ),
        // The harmonics saturate at 8 bits while scale, colour and opacity
        // carry most of the error for a byte or two each, so these move the
        // bits from the first to the others.
        Arm {
            name: "rebalance6",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[("scale_", 12), ("f_dc_", 12), ("opacity", 12)],
                ..full
            }),
        },
        Arm {
            name: "rebalance6at10",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[("scale_", 10), ("f_dc_", 10), ("opacity", 10)],
                ..full
            }),
        },
        // Position and rotation, each on its own, over the rebalance at ten
        // bits: raised together they bought a street scene 6.6 dB.
        Arm {
            name: "rebalance6at10pos18",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[
                    ("position", 18),
                    ("scale_", 10),
                    ("f_dc_", 10),
                    ("opacity", 10),
                ],
                ..full
            }),
        },
        Arm {
            name: "rebalance6at10pos20",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[
                    ("position", 20),
                    ("scale_", 10),
                    ("f_dc_", 10),
                    ("opacity", 10),
                ],
                ..full
            }),
        },
        Arm {
            name: "rebalance6at10rot12",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[("rot_", 12), ("scale_", 10), ("f_dc_", 10), ("opacity", 10)],
                ..full
            }),
        },
        Arm {
            name: "rebalance6at10both",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[
                    ("position", 20),
                    ("rot_", 12),
                    ("scale_", 10),
                    ("f_dc_", 10),
                    ("opacity", 10),
                ],
                ..full
            }),
        },
        Arm {
            name: "rebalance7",
            encode: Some(Budget {
                harmonics: 7,
                raise: &[("scale_", 12), ("f_dc_", 12), ("opacity", 12)],
                ..full
            }),
        },
        Arm {
            name: "rebalance6all",
            encode: Some(Budget {
                harmonics: 6,
                raise: &[
                    ("position", 20),
                    ("rot_", 12),
                    ("scale_", 12),
                    ("f_dc_", 12),
                    ("opacity", 12),
                ],
                ..full
            }),
        },
        Arm {
            name: "rebalance4",
            encode: Some(Budget {
                harmonics: 4,
                raise: &[("scale_", 12), ("f_dc_", 12), ("opacity", 12)],
                ..full
            }),
        },
    ];
    // A comma-separated list writes only those arms, since the renderer shoots
    // every one it finds. `source` is always written: it is the reference.
    let wanted: Option<Vec<String>> = std::env::var("DRACO_PROBE_ARMS")
        .ok()
        .map(|list| list.split(',').map(|arm| arm.trim().to_string()).collect());
    let arms: Vec<Arm> = arms
        .into_iter()
        .filter(|arm| {
            arm.encode.is_none()
                || wanted
                    .as_ref()
                    .is_none_or(|wanted| wanted.iter().any(|name| name == arm.name))
        })
        .collect();

    println!();
    println!(
        "{:<14} {:>14} {:>12} {:>12}",
        "arm", "encoded bytes", "B/point", "splats"
    );
    for arm in &arms {
        let file = out.join(format!("{}.ply", arm.name));
        match arm.encode {
            None => {
                write_ply(&file, &cloud).expect("writes");
                println!("{:<14} {:>14} {:>12} {num_points:>12}", arm.name, "-", "-");
            }
            Some(budget) => {
                let (decoded, bytes, points, stream) = round_trip(&cloud, &names, budget);
                // The decoded cloud carries the names through its own metadata.
                write_ply(&file, &decoded).expect("writes");
                // And the stream itself, so a consumer can be handed the
                // product rather than a PLY rewritten from it.
                //
                // Not for an alpha arm. Its stream holds alpha in `opacity`,
                // where the dialect says a logit lives, so any reader that
                // follows the dialect puts a second sigmoid through it. The
                // domain change is not expressible in the file, which is a
                // real limit of that arm and not a gap here: its PLY is its
                // renderable form, and the missing `.drc` is what says so.
                if !matches!(budget.change, Change::Alpha(_)) {
                    std::fs::write(out.join(format!("{}.drc", arm.name)), &stream).expect("writes");
                }
                // Bytes per point is the wrong ruler for an arm that removes
                // points -- it can rise while the file shrinks -- so the count
                // is printed beside it and the total is what to read.
                println!(
                    "{:<14} {bytes:>14} {:>12.3} {points:>12}",
                    arm.name,
                    bytes as f64 / points as f64
                );
            }
        }
    }

    println!();
    println!("  written to {}", out.display());
    println!("  the `source` arm went through this writer and nothing else, so");
    println!("  a difference against it is the encode and decode alone");
}
