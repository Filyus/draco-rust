//! What a real Gaussian splat scene costs as a Draco point cloud.
//!
//! The synthetic probe next to this one established that Draco lands on SPZ's
//! packed size when given SPZ's bit budget, and left one question open: SPZ's
//! published ratio is reached only after its packed stream is deflated, and how
//! much that wins depends on how the coefficients of a real scene are
//! distributed. Synthetic values cannot answer that -- quantization normalizes
//! each attribute to its own range, so a modelled bell costs nearly what
//! uniform noise costs, and the real concentration comes from outliers setting
//! a range most values sit far inside.
//!
//! So this reads a real scene. It needs one, and a scene is 180MB and more, so
//! it is skipped unless `DRACO_SPLAT_PLY` names a file:
//!
//! ```text
//! DRACO_SPLAT_PLY=/path/to/point_cloud.ply \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --test splat_real_scene_probe -- --ignored --nocapture
//! ```
//!
//! # How the entropy coders are compared
//!
//! Draco quantizes floats internally and SPZ quantizes them its own way, so
//! comparing the two end to end would compare two quantizers as much as two
//! coders. Instead this quantizes once, in `quantize`, and hands the **same
//! integers** to both: Draco encodes them as integer attributes, where no
//! further quantization happens, and the identical integers are written planar
//! to a file for an external `gzip` -- which is the stage SPZ uses. Whatever
//! separates the two numbers is the coder and nothing else.
//!
//! Planar rather than interleaved because that is SPZ's layout: it stores each
//! field as its own array, and a byte-level compressor cares a great deal.

#![cfg(all(feature = "encoder", feature = "decoder"))]

use std::path::PathBuf;

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder,
};

/// Sequential, the encoder that codes each attribute on its own. The kd-tree
/// coder pools every component into one cube at a single bit depth, so a
/// per-attribute budget does not reach it; see `splat_pointcloud_probe`.
const SEQUENTIAL: i32 = 0;

/// SPZ's allocation: 3*24 position + 3*8 scale + 3*8 rotation + 8 alpha
/// + 3*8 colour + 45*8 harmonics, in bytes per point.
const SPZ_PACKED_BYTES_PER_POINT: f32 = 64.0;

fn scene_path() -> Option<PathBuf> {
    // A path outside the repository is configuration, never a default in code:
    // a test that reads an untracked file passes here and fails everywhere.
    std::env::var_os("DRACO_SPLAT_PLY").map(PathBuf::from)
}

/// The bit budget for one attribute, by what the PLY called it.
fn spz_bits(kind: GeometryAttributeType, name: Option<&str>, sh_bits: i32) -> i32 {
    match kind {
        GeometryAttributeType::Position => 24,
        // 3DGS writes nx/ny/nz and fills them with zeros. They are budgeted
        // rather than dropped so that the totals stay comparable to the file.
        GeometryAttributeType::Normal => 8,
        _ => match name {
            Some(name) if name.starts_with("f_rest_") => sh_bits,
            _ => 8,
        },
    }
}

/// Names of the attributes that carry one, by attribute id.
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

fn read_f32(attribute: &PointAttribute, point: usize, component: usize) -> f32 {
    let stride = attribute.byte_stride() as usize;
    let offset = point * stride + component * 4;
    let mut bytes = [0u8; 4];
    attribute.buffer().read(offset, &mut bytes);
    f32::from_le_bytes(bytes)
}

/// One attribute's floats mapped onto `bits` integers.
///
/// Draco's own transform normalizes an attribute by a single range shared
/// across its components, which keeps a vector attribute's directions
/// comparable; this does the same, so the integers here are the integers Draco
/// would have produced.
fn quantize(attribute: &PointAttribute, num_points: usize, bits: i32) -> Vec<u32> {
    let components = attribute.num_components() as usize;
    let mut min = vec![f32::INFINITY; components];
    let mut max = vec![f32::NEG_INFINITY; components];
    for point in 0..num_points {
        for c in 0..components {
            let value = read_f32(attribute, point, c);
            min[c] = min[c].min(value);
            max[c] = max[c].max(value);
        }
    }
    let range = (0..components)
        .map(|c| max[c] - min[c])
        .fold(0.0f32, f32::max);
    let levels = ((1u64 << bits) - 1) as f32;
    // A constant attribute -- 3DGS's normals are exactly this -- has no range
    // to divide by, and every value is level zero.
    let scale = if range > 0.0 { levels / range } else { 0.0 };

    let mut out = Vec::with_capacity(num_points * components);
    for point in 0..num_points {
        for c in 0..components {
            let value = read_f32(attribute, point, c);
            out.push((((value - min[c]) * scale) + 0.5) as u32);
        }
    }
    out
}

/// A point cloud of integer attributes, mirroring `source`'s layout.
///
/// Integers pass through Draco untouched, so what it costs here is the cost of
/// coding these exact values -- the same ones written out for gzip.
fn quantized_cloud(source: &PointCloud, budgets: &[i32]) -> (PointCloud, Vec<Vec<u32>>) {
    let num_points = source.num_points();
    let mut cloud = PointCloud::new();
    cloud.set_num_points(num_points);
    let mut planes = Vec::new();

    for id in 0..source.num_attributes() {
        let source_attribute = source.attribute(id);
        let components = source_attribute.num_components();
        let bits = budgets[id as usize];
        let values = quantize(source_attribute, num_points, bits);

        // Uint8 where the budget fits a byte, as SPZ stores it; Uint32 for the
        // 24-bit positions, which is the narrowest type Draco offers that
        // holds them.
        let data_type = if bits <= 8 {
            DataType::Uint8
        } else {
            DataType::Uint32
        };
        let mut attribute = PointAttribute::new();
        attribute.init(
            source_attribute.attribute_type(),
            components,
            data_type,
            false,
            num_points,
        );
        let width = data_type.byte_length();
        let buffer = attribute.buffer_mut();
        for (index, value) in values.iter().enumerate() {
            let bytes = value.to_le_bytes();
            buffer.write(index * width, &bytes[..width]);
        }
        cloud.add_attribute(attribute);
        planes.push(values);
    }
    (cloud, planes)
}

fn encode(cloud: &PointCloud, budgets: &[i32], method: Option<i32>) -> usize {
    let mut options = EncoderOptions::new();
    if let Some(method) = method {
        options.set_encoding_method(method);
    }
    for (id, bits) in budgets.iter().enumerate() {
        options.set_attribute_int(id as i32, "quantization_bits", *bits);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("the scene encodes");
    let bytes = buffer.data().to_vec();

    // Decoding is not free at this size, but an encoder that produces a stream
    // nothing reads has measured nothing.
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(&bytes), &mut decoded)
        .expect("what it wrote, it reads");
    assert_eq!(decoded.num_points(), cloud.num_points());
    bytes.len()
}

#[test]
#[ignore = "needs a real scene in DRACO_SPLAT_PLY: run with --ignored --nocapture"]
fn a_real_scene_under_the_spz_bit_budget() {
    let Some(path) = scene_path() else {
        println!("DRACO_SPLAT_PLY is not set; nothing to measure");
        return;
    };
    let file_bytes = std::fs::metadata(&path).expect("the scene exists").len();
    println!("scene: {} ({:.1} MB)", path.display(), file_bytes as f64 / 1e6);

    let source = std::fs::read(&path).expect("the scene reads");
    let (mesh, report) = draco_io::ply_reader::PlyReader::from_bytes(source)
        .with_generic_attributes(true)
        .read_mesh_reporting_loss()
        .expect("a splat PLY parses");
    assert!(
        report.is_lossless(),
        "the scene is carried whole: {:?}",
        report.dropped()
    );
    let cloud = mesh.into_point_cloud();
    let num_points = cloud.num_points();
    let names = attribute_names(&cloud);
    let total_components: usize = (0..cloud.num_attributes())
        .map(|id| cloud.attribute(id).num_components() as usize)
        .sum();
    let raw_bytes_per_point = total_components as f32 * 4.0;
    println!(
        "{num_points} points, {} attributes, {total_components} components; \
         raw {raw_bytes_per_point} B/point, file {:.1} B/point",
        cloud.num_attributes(),
        file_bytes as f32 / num_points as f32,
    );

    let budgets = |sh_bits: i32| -> Vec<i32> {
        (0..cloud.num_attributes())
            .map(|id| {
                spz_bits(
                    cloud.attribute(id).attribute_type(),
                    names[id as usize].as_deref(),
                    sh_bits,
                )
            })
            .collect()
    };

    // ---------------------------------------------------------------------
    // What a .drc of this scene costs, quantizing floats the ordinary way.
    // ---------------------------------------------------------------------
    println!();
    println!("=== float attributes, quantized by Draco ===");
    println!(
        "{:<18} {:>10} {:>11} {:>10} {:>11}",
        "arm", "auto B/pt", "sequential", "x vs raw", "x vs file"
    );
    let uniform14: Vec<i32> = vec![14; cloud.num_attributes() as usize];
    for (label, budget) in [
        ("uniform 14", &uniform14),
        ("spz budget", &budgets(8)),
        ("spz, sh 6 bits", &budgets(6)),
        ("spz, sh 4 bits", &budgets(4)),
    ] {
        let auto = encode(&cloud, budget, None) as f32 / num_points as f32;
        let sequential = encode(&cloud, budget, Some(SEQUENTIAL)) as f32 / num_points as f32;
        println!(
            "{label:<18} {auto:>10.2} {sequential:>11.2} {:>9.2}x {:>10.2}x",
            raw_bytes_per_point / sequential,
            (file_bytes as f32 / num_points as f32) / sequential,
        );
    }

    // ---------------------------------------------------------------------
    // The coder comparison: identical integers to Draco and to gzip.
    // ---------------------------------------------------------------------
    println!();
    println!("=== the same quantized integers, two entropy coders ===");
    let budget = budgets(8);
    let (integer_cloud, planes) = quantized_cloud(&cloud, &budget);
    // Integers carry no quantization; an empty budget says so.
    let no_budget: Vec<i32> = Vec::new();
    let draco = encode(&integer_cloud, &no_budget, Some(SEQUENTIAL));

    let packed_path = std::env::temp_dir().join("draco_splat_spz_packed.bin");
    let mut packed = Vec::with_capacity(num_points * 64);
    for (id, values) in planes.iter().enumerate() {
        let width = if budget[id] <= 8 { 1 } else { 3 };
        for value in values {
            packed.extend_from_slice(&value.to_le_bytes()[..width]);
        }
    }
    std::fs::write(&packed_path, &packed).expect("the packed stream writes");

    let per_point = |bytes: usize| bytes as f32 / num_points as f32;
    println!(
        "  packed planar     {:>10.2} B/point  ({} bytes)",
        per_point(packed.len()),
        packed.len()
    );
    println!(
        "  draco sequential  {:>10.2} B/point  ({draco} bytes)",
        per_point(draco)
    );
    println!(
        "  SPZ's allocation  {SPZ_PACKED_BYTES_PER_POINT:>10.2} B/point  \
         (its own packed size; this stream is wider by the normals 3DGS writes \
         and by rotation's fourth component)"
    );

    match gzip_size(&packed_path) {
        Some(gzipped) => {
            println!(
                "  gzip -9           {:>10.2} B/point  ({gzipped} bytes)",
                per_point(gzipped)
            );
            println!();
            println!(
                "  the coders, on identical integers: draco is {:.3}x gzip",
                draco as f32 / gzipped as f32
            );
        }
        None => println!("  gzip -9           (no gzip on PATH; skipped)"),
    }

    // ---------------------------------------------------------------------
    // Where the bytes are. Lowering only the harmonics' budget changes the
    // output by whatever the harmonics cost, so the slope across those arms
    // says what a bit of harmonic costs to store -- and whether anything
    // compresses it at all.
    // ---------------------------------------------------------------------
    println!();
    println!("=== what the harmonics cost ===");
    let sh_components: usize = (0..cloud.num_attributes())
        .filter(|id| {
            names[*id as usize]
                .as_deref()
                .is_some_and(|name| name.starts_with("f_rest_"))
        })
        .map(|id| cloud.attribute(id).num_components() as usize)
        .sum();
    let at = |sh_bits: i32| encode(&cloud, &budgets(sh_bits), Some(SEQUENTIAL)) as f32;
    let (eight, four) = (at(8), at(4));
    let measured = (eight - four) / num_points as f32;
    let budgeted = sh_components as f32 * 4.0 / 8.0;
    println!("  {sh_components} harmonic components per point");
    println!(
        "  dropping them from 8 bits to 4 saves {measured:.2} B/point, against \
         {budgeted:.2} B/point of budget"
    );
    println!(
        "  so a harmonic bit costs {:.3} bits to store: at 1.0 nothing is \
         compressing them",
        measured / budgeted
    );
}

/// What `gzip -9` makes of a file, or `None` where there is no gzip.
///
/// The external binary rather than a crate because a dev-dependency added for
/// one probe reaches every feature combination CI builds, and this needs the
/// same deflate SPZ uses rather than a particular implementation of it.
fn gzip_size(path: &std::path::Path) -> Option<usize> {
    let output = std::process::Command::new("gzip")
        .arg("-9")
        .arg("-c")
        .arg(path)
        .output()
        .ok()?;
    output.status.success().then(|| output.stdout.len())
}
