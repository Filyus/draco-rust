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
        for (c, (low, high)) in min.iter_mut().zip(max.iter_mut()).enumerate() {
            let value = read_f32(attribute, point, c);
            *low = low.min(value);
            *high = high.max(value);
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
        for (c, low) in min.iter().enumerate() {
            let value = read_f32(attribute, point, c);
            out.push((((value - low) * scale) + 0.5) as u32);
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
    encode_predicting(cloud, budgets, method, None)
}

/// Encodes with `set_prediction_search`, the option this all became.
fn encode_searching(cloud: &PointCloud, budgets: &[i32]) -> usize {
    encode_with_options(cloud, budgets, true, false)
}

/// Encodes with the two shipped options, in whatever combination.
fn encode_with_options(cloud: &PointCloud, budgets: &[i32], search: bool, spatial: bool) -> usize {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(search);
    options.set_spatial_point_order(spatial);
    for (id, bits) in budgets.iter().enumerate() {
        options.set_attribute_int(id as i32, "quantization_bits", *bits);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().len()
}

/// One prediction scheme per attribute, `-1` where the encoder should decide.
fn encode_per_attribute(
    cloud: &PointCloud,
    budgets: &[i32],
    method: Option<i32>,
    predictions: &[i32],
) -> usize {
    encode_inner(cloud, budgets, method, None, predictions)
}

/// Draco's `PREDICTION_NONE`: code the values themselves rather than their
/// differences. It has been in the bitstream since version 1.1, so choosing it
/// is a choice an ordinary decoder reads, not an extension.
const PREDICTION_NONE: i32 = -2;

fn encode_predicting(
    cloud: &PointCloud,
    budgets: &[i32],
    method: Option<i32>,
    prediction: Option<i32>,
) -> usize {
    encode_inner(cloud, budgets, method, prediction, &[])
}

fn encode_inner(
    cloud: &PointCloud,
    budgets: &[i32],
    method: Option<i32>,
    prediction: Option<i32>,
    predictions: &[i32],
) -> usize {
    let mut options = EncoderOptions::new();
    if let Some(method) = method {
        options.set_encoding_method(method);
    }
    if let Some(prediction) = prediction {
        for id in 0..cloud.num_attributes() {
            options.set_attribute_int(id, "prediction_scheme", prediction);
        }
    }
    for (id, scheme) in predictions.iter().enumerate() {
        // -1 is Draco's "undefined": leave the encoder's own choice alone.
        if *scheme != -1 {
            options.set_attribute_int(id as i32, "prediction_scheme", *scheme);
        }
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
    println!(
        "scene: {} ({:.1} MB)",
        path.display(),
        file_bytes as f64 / 1e6
    );

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
    // What the harmonics cost, measured on the harmonics.
    //
    // The tempting shortcut is the slope: lower only their budget, and the
    // change in the total is what they cost. It is wrong, and quietly. The
    // difference between two arms is a difference of two *compressed* costs,
    // and the coder does not compress a 4-bit plane by the same fraction as an
    // 8-bit one -- so the slope can equal the budget exactly while the coder is
    // compressing at both ends. It does here. Reading that slope as a cost said
    // nothing compresses the harmonics, which the direct measurement below
    // contradicts by nine bytes a point.
    // ---------------------------------------------------------------------
    println!();
    println!("=== what the harmonics cost ===");
    let harmonic_ids: Vec<i32> = (0..cloud.num_attributes())
        .filter(|id| {
            names[*id as usize]
                .as_deref()
                .is_some_and(|name| name.starts_with("f_rest_"))
        })
        .collect();
    let sh_components: usize = harmonic_ids
        .iter()
        .map(|id| cloud.attribute(*id).num_components() as usize)
        .sum();

    let mut harmonics_only = PointCloud::new();
    harmonics_only.set_num_points(num_points);
    for id in &harmonic_ids {
        let source_attribute = cloud.attribute(*id);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            source_attribute.num_components(),
            DataType::Float32,
            false,
            num_points,
        );
        let width = source_attribute.num_components() as usize * 4;
        let stride = source_attribute.byte_stride() as usize;
        let mut scratch = vec![0u8; width];
        let buffer_out = attribute.buffer_mut();
        for point in 0..num_points {
            source_attribute.buffer().read(point * stride, &mut scratch);
            buffer_out.write(point * width, &scratch);
        }
        harmonics_only.add_attribute(attribute);
    }
    let eight_budget = vec![8; harmonic_ids.len()];
    let alone = encode(&harmonics_only, &eight_budget, Some(SEQUENTIAL)) as f32 / num_points as f32;
    let budgeted = sh_components as f32 * 8.0 / 8.0;
    println!("  {sh_components} harmonic components per point");
    println!("  budget                {budgeted:>7.2} B/point");
    println!("  encoded on their own  {alone:>7.2} B/point");
    println!(
        "  so the coder takes {:.1}% off them, at {:.2} bits per 8-bit value",
        (1.0 - alone / budgeted) * 100.0,
        alone * 8.0 / sh_components as f32,
    );

    // The sequential coder difference-predicts every attribute, and
    // `splat_entropy_probe` measured that differenced harmonics cost *more*
    // than the values themselves. If that is what is happening, then the gap to
    // the entropy floor is not the coder falling short of its data -- it is the
    // coder sitting exactly on the entropy of the wrong sequence, and the fix
    // is a flag the bitstream has always carried.
    let unpredicted = encode_predicting(
        &harmonics_only,
        &eight_budget,
        Some(SEQUENTIAL),
        Some(PREDICTION_NONE),
    ) as f32
        / num_points as f32;
    println!("  with PREDICTION_NONE  {unpredicted:>7.2} B/point");
    println!(
        "  predicting them costs {:.2} B/point, at {:.2} bits per value unpredicted",
        alone - unpredicted,
        unpredicted * 8.0 / sh_components as f32,
    );

    // ---------------------------------------------------------------------
    // Positions on their own, under both encoders.
    //
    // This is the other half of the same question. The kd-tree coder is what
    // exploits where the points are, and the sequential coder is what a
    // per-attribute budget reaches; a splat needs both and the format makes
    // them exclusive. What that exclusivity costs is this pair of numbers.
    // ---------------------------------------------------------------------
    println!();
    println!("=== positions, under each encoder ===");
    // Used by this section and by the whole-scene arms below.
    let sorted = morton_sorted(&cloud, position_id_of(&cloud));
    let position_id = (0..cloud.num_attributes())
        .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)
        .expect("a splat has positions");
    let mut positions_only = PointCloud::new();
    positions_only.set_num_points(num_points);
    {
        let source_attribute = cloud.attribute(position_id);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            source_attribute.num_components(),
            DataType::Float32,
            false,
            num_points,
        );
        let width = source_attribute.num_components() as usize * 4;
        let stride = source_attribute.byte_stride() as usize;
        let mut scratch = vec![0u8; width];
        let buffer_out = attribute.buffer_mut();
        for point in 0..num_points {
            source_attribute.buffer().read(point * stride, &mut scratch);
            buffer_out.write(point * width, &scratch);
        }
        positions_only.add_attribute(attribute);
    }
    let twenty_four = vec![24];
    let kd = encode(&positions_only, &twenty_four, None) as f32 / num_points as f32;
    let seq = encode(&positions_only, &twenty_four, Some(SEQUENTIAL)) as f32 / num_points as f32;
    // And the same positions after the points are sorted. The kd-tree coder
    // wins by using where the points are; a difference predictor can use the
    // same thing once the order puts neighbours next to each other, so how much
    // of the gap is really the coder and how much was only the ordering is a
    // question the sorted cloud answers.
    let sorted_positions = {
        let mut only = PointCloud::new();
        only.set_num_points(num_points);
        let source_attribute = sorted.attribute(position_id);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            source_attribute.num_components(),
            DataType::Float32,
            false,
            num_points,
        );
        let width = source_attribute.num_components() as usize * 4;
        let stride = source_attribute.byte_stride() as usize;
        let mut scratch = vec![0u8; width];
        let buffer_out = attribute.buffer_mut();
        for point in 0..num_points {
            source_attribute.buffer().read(point * stride, &mut scratch);
            buffer_out.write(point * width, &scratch);
        }
        only.add_attribute(attribute);
        only
    };
    let seq_sorted =
        encode(&sorted_positions, &twenty_four, Some(SEQUENTIAL)) as f32 / num_points as f32;
    println!("  budget                {:>7.2} B/point (3 x 24 bits)", 9.0);
    println!("  kd-tree               {kd:>7.2} B/point");
    println!("  sequential, file order{seq:>7.2} B/point");
    println!("  sequential, Morton    {seq_sorted:>7.2} B/point");
    println!(
        "  the exclusive choice costs {:.2} B/point in file order and {:.2} once sorted",
        seq - kd,
        seq_sorted - kd,
    );

    // ---------------------------------------------------------------------
    // Both fixes together, on the whole scene.
    //
    // Neither needs anything the bitstream does not already carry. The
    // prediction scheme is per attribute and has been readable since version
    // 1.1; the point order of a point cloud is not semantic, so sorting before
    // encoding is a choice inside the encoder that no decoder has to know
    // about. What the sort buys is a difference predictor on positions that
    // predicts from a spatial neighbour rather than from whatever the exporter
    // happened to write next.
    // ---------------------------------------------------------------------
    println!();
    println!("=== the whole scene, with the predictor chosen per attribute ===");
    let per_attribute: Vec<i32> = (0..cloud.num_attributes())
        .map(|id| {
            if names[id as usize]
                .as_deref()
                .is_some_and(|name| name.starts_with("f_rest_"))
            {
                PREDICTION_NONE
            } else {
                -1
            }
        })
        .collect();
    let budget = budgets(8);
    let base = encode(&cloud, &budget, Some(SEQUENTIAL)) as f32 / num_points as f32;
    let sorted_only = encode(&sorted, &budget, Some(SEQUENTIAL)) as f32 / num_points as f32;
    let predicted_only = encode_per_attribute(&cloud, &budget, Some(SEQUENTIAL), &per_attribute)
        as f32
        / num_points as f32;
    let both = encode_per_attribute(&sorted, &budget, Some(SEQUENTIAL), &per_attribute) as f32
        / num_points as f32;
    // The shipped option, doing by measurement what the hand-written list above
    // does by knowing which attributes are harmonics. It should land on the
    // same place or better -- better, where it finds an attribute the list did
    // not think to name.
    let searched = encode_searching(&cloud, &budget) as f32 / num_points as f32;
    let searched_sorted = encode_searching(&sorted, &budget) as f32 / num_points as f32;

    println!("  as encoded today              {base:>7.2} B/point");
    println!("  Morton order only             {sorted_only:>7.2} B/point");
    println!("  per-attribute prediction only {predicted_only:>7.2} B/point");
    println!("  both                          {both:>7.2} B/point");
    println!("  set_prediction_search(true)   {searched:>7.2} B/point");
    println!("  the same, Morton order        {searched_sorted:>7.2} B/point");
    // Both shipped options, doing on their own what the arms above did with a
    // hand-sorted cloud and a hand-written list of attributes.
    let spatial_only = encode_with_options(&cloud, &budget, false, true) as f32 / num_points as f32;
    let both_options = encode_with_options(&cloud, &budget, true, true) as f32 / num_points as f32;
    println!("  set_spatial_point_order(true) {spatial_only:>7.2} B/point");
    println!("  both options                  {both_options:>7.2} B/point");
    println!(
        "  together {:.2} B/point, {:.1}% off, and {:.2}x the raw floats",
        base - both,
        (1.0 - both / base) * 100.0,
        raw_bytes_per_point / both,
    );

    // The two fixes are far from additive -- 3.53 and 6.00 apart, 6.36
    // together -- and the reason matters, because "they overlap" and "one of
    // them stopped working" look identical in a total. So the harmonics are
    // measured again in the sorted cloud: if sorting is what makes their
    // difference predictor stop hurting, that shows up here as the gap between
    // these two numbers closing.
    let mut sorted_harmonics = PointCloud::new();
    sorted_harmonics.set_num_points(num_points);
    for id in &harmonic_ids {
        let source_attribute = sorted.attribute(*id);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            source_attribute.num_components(),
            DataType::Float32,
            false,
            num_points,
        );
        let width = source_attribute.num_components() as usize * 4;
        let stride = source_attribute.byte_stride() as usize;
        let mut scratch = vec![0u8; width];
        let buffer_out = attribute.buffer_mut();
        for point in 0..num_points {
            source_attribute.buffer().read(point * stride, &mut scratch);
            buffer_out.write(point * width, &scratch);
        }
        sorted_harmonics.add_attribute(attribute);
    }
    let sorted_predicted =
        encode(&sorted_harmonics, &eight_budget, Some(SEQUENTIAL)) as f32 / num_points as f32;
    let sorted_unpredicted = encode_predicting(
        &sorted_harmonics,
        &eight_budget,
        Some(SEQUENTIAL),
        Some(PREDICTION_NONE),
    ) as f32
        / num_points as f32;
    println!();
    println!("  harmonics alone, in file order:  {alone:>6.2} predicted, {unpredicted:>6.2} not");
    println!(
        "  harmonics alone, in Morton order:{sorted_predicted:>7.2} predicted, \
         {sorted_unpredicted:>6.2} not"
    );
    println!(
        "  so sorting takes {:.2} B/point off the predictor's penalty, and what \
         is left for PREDICTION_NONE to save is {:.2}",
        (alone - unpredicted) - (sorted_predicted - sorted_unpredicted),
        sorted_predicted - sorted_unpredicted,
    );

    // Kept only to show what it is not: this is the slope, printed next to the
    // cost it would have been mistaken for.
    let slope = (encode(&cloud, &budgets(8), Some(SEQUENTIAL)) as f32
        - encode(&cloud, &budgets(4), Some(SEQUENTIAL)) as f32)
        / num_points as f32;
    println!(
        "  (the 8-to-4-bit slope is {slope:.2} B/point, which is the difference \
         of two compressed costs and not a cost)"
    );
}

/// The id of the cloud's position attribute.
fn position_id_of(cloud: &PointCloud) -> i32 {
    (0..cloud.num_attributes())
        .find(|id| cloud.attribute(*id).attribute_type() == GeometryAttributeType::Position)
        .expect("a splat has positions")
}

/// The same cloud with its points in Morton order.
///
/// A point cloud's point order carries no meaning -- there is no connectivity
/// referring to it and a splat renderer sorts by depth anyway -- so an encoder
/// may choose it freely. Every attribute is permuted together, which is what
/// keeps a point a point.
fn morton_sorted(cloud: &PointCloud, position_id: i32) -> PointCloud {
    let num_points = cloud.num_points();
    let positions = cloud.attribute(position_id);
    let stride = positions.byte_stride() as usize;
    let read = |point: usize, component: usize| -> f32 {
        let mut bytes = [0u8; 4];
        positions
            .buffer()
            .read(point * stride + component * 4, &mut bytes);
        f32::from_le_bytes(bytes)
    };

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for point in 0..num_points {
        for (c, (low, high)) in min.iter_mut().zip(max.iter_mut()).enumerate() {
            let value = read(point, c);
            *low = low.min(value);
            *high = high.max(value);
        }
    }
    // Ten bits an axis interleave into 30 and fit a u32 key. At three quarters
    // of a million points that separates neighbours far more finely than the
    // difference predictor can use.
    let spread = |v: u32| -> u32 {
        let mut x = v & 0x3FF;
        x = (x | (x << 16)) & 0x030000FF;
        x = (x | (x << 8)) & 0x0300F00F;
        x = (x | (x << 4)) & 0x030C30C3;
        x = (x | (x << 2)) & 0x09249249;
        x
    };
    let mut keys: Vec<(u32, u32)> = (0..num_points)
        .map(|point| {
            let mut axis = [0u32; 3];
            for c in 0..3 {
                let range = max[c] - min[c];
                let normalized = if range > 0.0 {
                    (read(point, c) - min[c]) / range
                } else {
                    0.0
                };
                axis[c] = (normalized * 1023.0) as u32;
            }
            let key = spread(axis[0]) | (spread(axis[1]) << 1) | (spread(axis[2]) << 2);
            (key, point as u32)
        })
        .collect();
    keys.sort_unstable();

    let mut sorted = PointCloud::new();
    sorted.set_num_points(num_points);
    for id in 0..cloud.num_attributes() {
        let source_attribute = cloud.attribute(id);
        let components = source_attribute.num_components();
        let mut attribute = PointAttribute::new();
        attribute.init(
            source_attribute.attribute_type(),
            components,
            source_attribute.data_type(),
            source_attribute.normalized(),
            num_points,
        );
        let width = components as usize * source_attribute.data_type().byte_length();
        let source_stride = source_attribute.byte_stride() as usize;
        let mut scratch = vec![0u8; width];
        let buffer_out = attribute.buffer_mut();
        for (destination, (_, source)) in keys.iter().enumerate() {
            source_attribute
                .buffer()
                .read(*source as usize * source_stride, &mut scratch);
            buffer_out.write(destination * width, &scratch);
        }
        let new_id = sorted.add_attribute(attribute);
        // Names travel with the values; without them a reader cannot tell
        // which harmonic it is holding.
        let unique_id = cloud.attribute(id).unique_id();
        if let Some(metadata) = cloud.attribute_metadata_by_unique_id(unique_id) {
            let carried = metadata.metadata().clone();
            let new_unique_id = sorted.attribute(new_id).unique_id();
            sorted
                .metadata_or_insert()
                .set_attribute_metadata(new_unique_id, carried);
        }
    }
    sorted
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
    output.status.success().then_some(output.stdout.len())
}
