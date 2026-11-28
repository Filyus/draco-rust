use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use draco_core::geometry_indices::PointIndex;
use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder,
};
use draco_gltf::{
    ComponentType, CompressionMode, CompressionOptions, ExternalFilePolicy, Import, ImportOptions,
    MeshIndex, OutputFormat, PackedAttribute, PackedGeometry, PrimitiveIndex, QuantizationBits,
    ShQuantization,
};

#[path = "../../../crates/draco-core/src/synthetic_cloud.rs"]
#[allow(dead_code)]
mod synthetic_cloud;

#[derive(Clone)]
struct SourceAttribute {
    name: String,
    kind: GeometryAttributeType,
    components: usize,
    values: Vec<f32>,
}

struct QuantizedAttribute {
    kind: GeometryAttributeType,
    components: usize,
    bits: u8,
    width: usize,
    minimum: Vec<f32>,
    scale: f32,
    values: Vec<u32>,
}

struct QuantizedCloud {
    points: usize,
    attributes: Vec<QuantizedAttribute>,
    geometry: PointCloud,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("codec-ply") if args.len() == 6 => {
            let label = &args[2];
            let ply = Path::new(&args[3]);
            let bridge = Path::new(&args[4]);
            let work = Path::new(&args[5]);
            let source = read_ply(ply)?;
            run_codec_matrix(label, &source, bridge, work)?;
        }
        Some("codec-synthetic") if args.len() == 6 => {
            let profile = &args[2];
            let points: usize = args[3].parse()?;
            let bridge = Path::new(&args[4]);
            let work = Path::new(&args[5]);
            let source = synthetic_profile(profile, points)?;
            run_codec_matrix(profile, &source, bridge, work)?;
        }
        Some("integrate-splat") if args.len() == 7 => {
            integrate_splat(
                &args[2],
                Path::new(&args[3]),
                Path::new(&args[4]),
                Path::new(&args[5]),
                Path::new(&args[6]),
            )?;
        }
        Some("integrate-mesh") if args.len() == 5 => {
            integrate_mesh(
                Path::new(&args[2]),
                Path::new(&args[3]),
                Path::new(&args[4]),
            )?;
        }
        Some("prepare-mesh") if args.len() == 4 => {
            write_mesh_glb(Path::new(&args[2]), Path::new(&args[3]))?;
        }
        Some("summary") if args.len() == 3 => write_summary(Path::new(&args[2]))?,
        _ => {
            eprintln!(
                "usage:\n  meshopt-bench codec-ply <label> <input.ply> <meshopt-bridge.exe> <work-dir>\n  meshopt-bench codec-synthetic <xyz|xyz_rgb|xyz_normal_rgb> <points> <meshopt-bridge.exe> <work-dir>\n  meshopt-bench integrate-splat <label> <input.ply> <gltfpack-gs.exe> <output-dir> <work-dir>\n  meshopt-bench integrate-mesh <input.glb> <gltfpack.exe> <output-dir>"
            );
            std::process::exit(2);
        }
    }
    Ok(())
}

fn read_ply(path: &Path) -> Result<Vec<SourceAttribute>, Box<dyn Error>> {
    let mesh = read_ply_mesh(path)?;
    source_attributes(&mesh)
}

fn read_ply_mesh(path: &Path) -> Result<draco_core::Mesh, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let (mesh, report) = draco_io::ply_reader::PlyReader::from_bytes(bytes)
        .with_generic_attributes(true)
        .read_mesh_reporting_loss()?;
    if !report.is_lossless() {
        return Err(format!(
            "{} dropped PLY properties: {:?}",
            path.display(),
            report.dropped()
        )
        .into());
    }
    Ok(mesh)
}

fn write_mesh_glb(input_ply: &Path, output_glb: &Path) -> Result<(), Box<dyn Error>> {
    let mesh = read_ply_mesh(input_ply)?;
    let attributes = source_attributes(&mesh)?;
    let points = mesh.num_points();
    let mut binary = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut gltf_attributes = serde_json::Map::new();
    let mut position_min = vec![f32::INFINITY; 3];
    let mut position_max = vec![f32::NEG_INFINITY; 3];

    for attribute in &attributes {
        let (semantic, type_name) = match attribute.kind {
            GeometryAttributeType::Position if attribute.components == 3 => ("POSITION", "VEC3"),
            GeometryAttributeType::Normal if attribute.components == 3 => ("NORMAL", "VEC3"),
            GeometryAttributeType::Color if attribute.components == 3 => ("COLOR_0", "VEC3"),
            _ => continue,
        };
        while binary.len() % 4 != 0 {
            binary.push(0);
        }
        let offset = binary.len();
        let mut min = vec![f32::INFINITY; attribute.components];
        let mut max = vec![f32::NEG_INFINITY; attribute.components];
        for point in 0..points {
            for component in 0..attribute.components {
                let value = attribute.values[point * attribute.components + component];
                binary.extend_from_slice(&value.to_le_bytes());
                min[component] = min[component].min(value);
                max[component] = max[component].max(value);
            }
        }
        let accessor = accessors.len();
        buffer_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": binary.len() - offset,
            "target": 34962
        }));
        let mut accessor_value = serde_json::json!({
            "bufferView": buffer_views.len() - 1,
            "componentType": 5126,
            "count": points,
            "type": type_name
        });
        if semantic == "POSITION" {
            position_min.clone_from(&min);
            position_max.clone_from(&max);
            accessor_value["min"] = serde_json::json!(min);
            accessor_value["max"] = serde_json::json!(max);
        }
        accessors.push(accessor_value);
        gltf_attributes.insert(semantic.into(), serde_json::json!(accessor));
    }
    if !gltf_attributes.contains_key("POSITION") {
        return Err("bun PLY does not have a three-component POSITION".into());
    }

    while binary.len() % 4 != 0 {
        binary.push(0);
    }
    let indices_offset = binary.len();
    for face in mesh.faces() {
        for point in face {
            binary.extend_from_slice(&point.0.to_le_bytes());
        }
    }
    let indices_count = mesh.num_faces() * 3;
    buffer_views.push(serde_json::json!({
        "buffer": 0,
        "byteOffset": indices_offset,
        "byteLength": binary.len() - indices_offset,
        "target": 34963
    }));
    let indices_accessor = accessors.len();
    accessors.push(serde_json::json!({
        "bufferView": buffer_views.len() - 1,
        "componentType": 5125,
        "count": indices_count,
        "type": "SCALAR"
    }));
    let document = serde_json::json!({
        "asset": {"version": "2.0", "generator": "draco-rust meshopt-bench"},
        "buffers": [{"byteLength": binary.len()}],
        "bufferViews": buffer_views,
        "accessors": accessors,
        "meshes": [{"primitives": [{"attributes": gltf_attributes, "indices": indices_accessor, "mode": 4}]}],
        "nodes": [{"mesh": 0}],
        "scenes": [{"nodes": [0]}],
        "scene": 0
    });
    let mut json = serde_json::to_vec(&document)?;
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    while binary.len() % 4 != 0 {
        binary.push(0);
    }
    let total_length = 12 + 8 + json.len() + 8 + binary.len();
    let mut glb = Vec::with_capacity(total_length);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total_length as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(binary.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&binary);
    fs::write(output_glb, glb)?;
    println!(
        "{{\"kind\":\"mesh_source\",\"source_ply\":{},\"path\":{},\"points\":{},\"faces\":{},\"indices\":{},\"position_bounds\":{{\"min\":{:?},\"max\":{:?}}}}}",
        serde_json::to_string(&input_ply.display().to_string())?,
        serde_json::to_string(&output_glb.display().to_string())?,
        points,
        mesh.num_faces(),
        indices_count,
        position_min,
        position_max
    );
    Ok(())
}

fn source_attributes(cloud: &PointCloud) -> Result<Vec<SourceAttribute>, Box<dyn Error>> {
    let mut output = Vec::with_capacity(cloud.num_attributes() as usize);
    for id in 0..cloud.num_attributes() {
        let attribute = cloud.attribute(id);
        let name = cloud
            .attribute_metadata_by_unique_id(attribute.unique_id())
            .and_then(|metadata| metadata.metadata().get_string("name"))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("attribute_{id}"));
        let components = attribute.num_components() as usize;
        let mut values = Vec::with_capacity(cloud.num_points() * components);
        for point in 0..cloud.num_points() {
            let value_index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            let stride = attribute.byte_stride() as usize;
            let scalar_width = attribute.data_type().byte_length();
            for component in 0..components {
                let offset = value_index * stride + component * scalar_width;
                values.push(read_scalar_f32(attribute, offset)?);
            }
        }
        output.push(SourceAttribute {
            name,
            kind: attribute.attribute_type(),
            components,
            values,
        });
    }
    Ok(output)
}

fn read_scalar_f32(attribute: &PointAttribute, offset: usize) -> Result<f32, Box<dyn Error>> {
    let mut bytes = [0u8; 8];
    let width = attribute.data_type().byte_length();
    if width > bytes.len() {
        return Err("attribute scalar is wider than eight bytes".into());
    }
    attribute.buffer().read(offset, &mut bytes[..width]);
    let value = match attribute.data_type() {
        DataType::Int8 => i8::from_le_bytes([bytes[0]]) as f32,
        DataType::Uint8 | DataType::Bool => bytes[0] as f32,
        DataType::Int16 => i16::from_le_bytes(bytes[..2].try_into()?) as f32,
        DataType::Uint16 => u16::from_le_bytes(bytes[..2].try_into()?) as f32,
        DataType::Int32 => i32::from_le_bytes(bytes[..4].try_into()?) as f32,
        DataType::Uint32 => u32::from_le_bytes(bytes[..4].try_into()?) as f32,
        DataType::Int64 => i64::from_le_bytes(bytes) as f32,
        DataType::Uint64 => u64::from_le_bytes(bytes) as f32,
        DataType::Float32 => f32::from_le_bytes(bytes[..4].try_into()?),
        DataType::Float64 => f64::from_le_bytes(bytes) as f32,
        DataType::Invalid => return Err("PLY contains an invalid attribute type".into()),
    };
    if !value.is_finite() {
        return Err("PLY contains a non-finite attribute value".into());
    }
    Ok(value)
}

fn synthetic_profile(profile: &str, points: usize) -> Result<Vec<SourceAttribute>, Box<dyn Error>> {
    let seed = 0x6d65_7368_6f70_7431;
    let selected = match profile {
        "xyz" => synthetic_cloud::uniform(points, seed),
        "xyz_rgb" => {
            let full = synthetic_cloud::terrestrial(points, seed);
            let columns = full
                .columns
                .into_iter()
                .filter(|column| {
                    matches!(
                        column.kind,
                        GeometryAttributeType::Position | GeometryAttributeType::Color
                    )
                })
                .collect();
            synthetic_cloud::Cloud {
                name: "synthetic_xyz_rgb".into(),
                points,
                columns,
            }
        }
        "xyz_normal_rgb" => synthetic_cloud::terrestrial(points, seed),
        _ => return Err(format!("unknown synthetic profile {profile}").into()),
    };
    let mut attributes = Vec::new();
    for column in selected.columns {
        if profile == "xyz" && column.kind != GeometryAttributeType::Position {
            continue;
        }
        let values = match column.values {
            synthetic_cloud::Values::F32(values) => values,
            synthetic_cloud::Values::F64(values) => {
                values.into_iter().map(|value| value as f32).collect()
            }
            synthetic_cloud::Values::U8(values) => values.into_iter().map(f32::from).collect(),
            synthetic_cloud::Values::U16(values) => values.into_iter().map(f32::from).collect(),
            synthetic_cloud::Values::I32(values) => {
                values.into_iter().map(|value| value as f32).collect()
            }
        };
        attributes.push(SourceAttribute {
            name: column.name.to_owned(),
            kind: column.kind,
            components: column.components,
            values,
        });
    }
    Ok(attributes)
}

fn quantize(
    source: &[SourceAttribute],
    points: usize,
    bits: u8,
) -> Result<QuantizedCloud, Box<dyn Error>> {
    let mut attributes = Vec::with_capacity(source.len());
    for input in source {
        let mut minimum = vec![f32::INFINITY; input.components];
        let mut maximum = vec![f32::NEG_INFINITY; input.components];
        for point in 0..points {
            for component in 0..input.components {
                let value = input.values[point * input.components + component];
                minimum[component] = minimum[component].min(value);
                maximum[component] = maximum[component].max(value);
            }
        }
        let range = maximum
            .iter()
            .zip(&minimum)
            .map(|(high, low)| high - low)
            .fold(0.0f32, f32::max);
        let levels = ((1u64 << bits) - 1) as f32;
        let scale = if range > 0.0 { levels / range } else { 0.0 };
        let mut values = Vec::with_capacity(points * input.components);
        for point in 0..points {
            for component in 0..input.components {
                let value = input.values[point * input.components + component];
                let q = if scale > 0.0 {
                    (((value - minimum[component]) * scale) + 0.5).clamp(0.0, levels) as u32
                } else {
                    0
                };
                values.push(q);
            }
        }
        attributes.push(QuantizedAttribute {
            kind: input.kind,
            components: input.components,
            bits,
            width: if bits <= 8 {
                1
            } else if bits <= 16 {
                2
            } else {
                4
            },
            minimum,
            scale,
            values,
        });
    }
    let geometry = build_geometry(points, &attributes)?;
    Ok(QuantizedCloud {
        points,
        attributes,
        geometry,
    })
}

fn build_geometry(
    points: usize,
    attributes: &[QuantizedAttribute],
) -> Result<PointCloud, Box<dyn Error>> {
    let mut geometry = PointCloud::new();
    geometry.set_num_points(points);
    for input in attributes {
        let data_type = match input.width {
            1 => DataType::Uint8,
            2 => DataType::Uint16,
            _ => DataType::Uint32,
        };
        let mut attribute = PointAttribute::new();
        attribute.init(input.kind, input.components as u8, data_type, false, points);
        for (index, value) in input.values.iter().enumerate() {
            let bytes = value.to_le_bytes();
            attribute
                .buffer_mut()
                .write(index * input.width, &bytes[..input.width]);
        }
        geometry.add_attribute(attribute);
    }
    Ok(geometry)
}

fn pack_records(cloud: &QuantizedCloud) -> Result<(Vec<u8>, usize, Vec<u8>), Box<dyn Error>> {
    let row_bytes: usize = cloud
        .attributes
        .iter()
        .map(|attribute| attribute.components * attribute.width)
        .sum();
    let stride = row_bytes.div_ceil(4) * 4;
    if stride > 256 {
        return Err(format!("Meshopt vertex codec stride {stride} exceeds 256 bytes").into());
    }
    let mut records = vec![0u8; cloud.points * stride];
    for point in 0..cloud.points {
        let mut offset = point * stride;
        for attribute in &cloud.attributes {
            for component in 0..attribute.components {
                let value = attribute.values[point * attribute.components + component];
                let bytes = value.to_le_bytes();
                records[offset..offset + attribute.width]
                    .copy_from_slice(&bytes[..attribute.width]);
                offset += attribute.width;
            }
        }
    }
    let position = cloud
        .attributes
        .iter()
        .find(|attribute| {
            attribute.kind == GeometryAttributeType::Position && attribute.components >= 3
        })
        .ok_or("input has no three-component position attribute")?;
    let mut positions = Vec::with_capacity(cloud.points * 12);
    for point in 0..cloud.points {
        for component in 0..3 {
            positions.extend_from_slice(
                &position.values[point * position.components + component].to_le_bytes(),
            );
        }
    }
    Ok((records, stride, positions))
}

fn geometry_from_records(
    points: usize,
    attributes: &[QuantizedAttribute],
    stride: usize,
    records: &[u8],
) -> Result<PointCloud, Box<dyn Error>> {
    if records.len() != points * stride {
        return Err("Meshopt prepared buffer has an unexpected length".into());
    }
    let mut geometry = PointCloud::new();
    geometry.set_num_points(points);
    let mut attribute_offset = 0usize;
    for descriptor in attributes {
        let mut attribute = PointAttribute::new();
        let data_type = match descriptor.width {
            1 => DataType::Uint8,
            2 => DataType::Uint16,
            _ => DataType::Uint32,
        };
        attribute.init(
            descriptor.kind,
            descriptor.components as u8,
            data_type,
            false,
            points,
        );
        for point in 0..points {
            for component in 0..descriptor.components {
                let src = point * stride + attribute_offset + component * descriptor.width;
                let dst = (point * descriptor.components + component) * descriptor.width;
                attribute
                    .buffer_mut()
                    .write(dst, &records[src..src + descriptor.width]);
            }
        }
        geometry.add_attribute(attribute);
        attribute_offset += descriptor.components * descriptor.width;
    }
    Ok(geometry)
}

fn run_codec_matrix(
    label: &str,
    source: &[SourceAttribute],
    bridge: &Path,
    work: &Path,
) -> Result<(), Box<dyn Error>> {
    let points = source
        .first()
        .map_or(0, |attribute| attribute.values.len() / attribute.components);
    if points == 0 {
        return Err(format!("{label}: empty point set").into());
    }
    fs::create_dir_all(work)?;
    let input_path = work.join("codec-input.bin");
    let positions_path = work.join("codec-position.bin");
    let prepared_path = work.join("meshopt-prepared.bin");
    let encoded_path = work.join("meshopt-encoded.bin");
    let decoded_path = work.join("meshopt-decoded.bin");
    let mut rows = Vec::new();

    for bits in [8u8, 12, 16] {
        let quantize_start = Instant::now();
        let quantized = quantize(source, points, bits)?;
        let quantize_ms = quantize_start.elapsed().as_secs_f64() * 1000.0;
        let (records, stride, positions) = pack_records(&quantized)?;
        fs::write(&input_path, records)?;
        fs::write(&positions_path, positions)?;
        let mut meshopt_runs = Vec::new();
        for level in [2, 3] {
            let bridge_output = Command::new(bridge)
                .arg(&input_path)
                .arg(&positions_path)
                .arg(points.to_string())
                .arg(stride.to_string())
                .arg(&prepared_path)
                .arg(&encoded_path)
                .arg(&decoded_path)
                .arg(level.to_string())
                .output()?;
            if !bridge_output.status.success() {
                return Err(format!(
                    "Meshopt bridge failed for {label} at {bits} bits, level {level}: {}",
                    String::from_utf8_lossy(&bridge_output.stderr)
                )
                .into());
            }
            let meshopt: serde_json::Value = serde_json::from_slice(&bridge_output.stdout)?;
            let prepared = fs::read(&prepared_path)?;
            let meshopt_decoded = fs::read(&decoded_path)?;
            if prepared != meshopt_decoded || meshopt["roundtrip_exact"] != true {
                return Err(
                    format!("Meshopt round-trip mismatch for {label} at {bits} bits").into(),
                );
            }
            meshopt_runs.push((level, meshopt));
        }
        let prepared = fs::read(&prepared_path)?;
        let sorted_geometry =
            geometry_from_records(points, &quantized.attributes, stride, &prepared)?;
        let (draco_bytes, draco_encode_ms) = encode_draco(&sorted_geometry, 0, false, false, 3)?;
        let (draco_decoded, draco_decode_ms) = decode_draco(&draco_bytes)?;
        let draco_exact = same_point_set(&sorted_geometry, &draco_decoded)?;
        if !draco_exact {
            return Err(format!(
                "Draco sequential changed quantized point records for {label} at {bits} bits"
            )
            .into());
        }

        let fidelity = fidelity_metrics(source, &quantized.attributes, points);
        for (level, meshopt) in meshopt_runs {
            rows.push(serde_json::json!({
                "kind": "codec_curve",
                "dataset": label,
                "points": points,
                "attribute_count": source.len(),
                "component_count": source.iter().map(|attribute| attribute.components).sum::<usize>(),
                "quality_bits": bits,
                "quantize_ms": quantize_ms,
                "meshopt": {
                    "version": meshopt["meshopt_version"],
                    "mode": format!("spatial_sort_plus_vertex_codec_level_{level}"),
                    "stride": stride,
                    "raw_quantized_bytes": meshopt["prepared_bytes"],
                    "encoded_bytes": meshopt["encoded_bytes"],
                    "prepare_ms": meshopt["prepare_ms"],
                    "encode_ms_median": meshopt["encode_ms_median"],
                    "decode_ms_median": meshopt["decode_ms_median"],
                    "peak_working_set_bytes": meshopt["peak_working_set_bytes"],
                    "roundtrip_exact": true
                },
                "draco_core": {
                    "mode": "sequential_on_same_spatially_sorted_quantized_records",
                    "encoded_bytes": draco_bytes.len(),
                    "encode_ms_median_3_runs": draco_encode_ms,
                    "decode_ms_median_3_runs": draco_decode_ms,
                    "decode_runs": 3,
                    "roundtrip_point_set_exact": draco_exact
                },
                "fidelity": fidelity
            }));
        }

        if bits == 12 {
            for (method, search, name) in [
                (0, false, "sequential_input_order"),
                (1, false, "kd_tree"),
                (0, true, "point_order_search"),
            ] {
                match encode_draco(&quantized.geometry, method, search, false, 1) {
                    Ok((bytes, encode_ms)) => {
                        let (decoded, decode_ms) = decode_draco(&bytes)?;
                        let exact = same_point_set(&quantized.geometry, &decoded)?;
                        rows.push(serde_json::json!({
                            "kind": "draco_mode",
                            "dataset": label,
                            "points": points,
                            "quality_bits": bits,
                            "mode": name,
                            "encoded_bytes": bytes.len(),
                            "encode_ms_single_run": encode_ms,
                            "decode_ms_median_3_runs": decode_ms,
                            "decode_runs": 3,
                            "threads": 1,
                            "roundtrip_point_set_exact": exact,
                            "fidelity_source": "same quantized point set as codec_curve"
                        }));
                        if !exact {
                            return Err(format!(
                                "Draco {name} changed quantized point records for {label}"
                            )
                            .into());
                        }
                    }
                    Err(error) => rows.push(serde_json::json!({
                        "kind": "draco_mode",
                        "dataset": label,
                        "points": points,
                        "quality_bits": bits,
                        "mode": name,
                        "success": false,
                        "error": error.to_string(),
                        "threads": 1
                    })),
                }
            }
        }
    }

    let peak = peak_working_set_bytes();
    for row in &mut rows {
        row["dataset_process_peak_working_set_bytes"] = serde_json::json!(peak);
        println!("{}", serde_json::to_string(row)?);
    }
    Ok(())
}

fn encode_draco(
    geometry: &PointCloud,
    method: i32,
    point_order_search: bool,
    spatial_order: bool,
    repeats: usize,
) -> Result<(Vec<u8>, f64), Box<dyn Error>> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(method);
    options.set_threads(1);
    options.set_point_order_search(point_order_search);
    options.set_spatial_point_order(spatial_order);
    let mut durations = Vec::with_capacity(repeats);
    let mut last = Vec::new();
    for _ in 0..repeats {
        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(geometry.clone());
        let mut buffer = EncoderBuffer::new();
        let start = Instant::now();
        encoder.encode(&options, &mut buffer)?;
        durations.push(start.elapsed().as_secs_f64() * 1000.0);
        last = buffer.data().to_vec();
    }
    durations.sort_by(f64::total_cmp);
    let median = durations[durations.len() / 2];
    Ok((last, median))
}

fn decode_draco(bytes: &[u8]) -> Result<(PointCloud, f64), Box<dyn Error>> {
    let mut durations = Vec::with_capacity(3);
    let mut decoded = PointCloud::new();
    for _ in 0..3 {
        let mut current = PointCloud::new();
        let mut input = DecoderBuffer::new(bytes);
        let start = Instant::now();
        PointCloudDecoder::new().decode(&mut input, &mut current)?;
        durations.push(start.elapsed().as_secs_f64() * 1000.0);
        decoded = current;
    }
    durations.sort_by(f64::total_cmp);
    Ok((decoded, durations[1]))
}

fn same_point_set(expected: &PointCloud, actual: &PointCloud) -> Result<bool, Box<dyn Error>> {
    if expected.num_points() != actual.num_points()
        || expected.num_attributes() != actual.num_attributes()
    {
        return Ok(false);
    }
    let expected_hashes = point_hashes(expected)?;
    let actual_hashes = point_hashes(actual)?;
    Ok(expected_hashes == actual_hashes)
}

fn point_hashes(geometry: &PointCloud) -> Result<Vec<(u64, u64)>, Box<dyn Error>> {
    let mut hashes =
        vec![(0xcbf2_9ce4_8422_2325u64, 0x9e37_79b9_7f4a_7c15u64); geometry.num_points()];
    for point in 0..geometry.num_points() {
        for id in 0..geometry.num_attributes() {
            let attribute = geometry.attribute(id);
            let index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            let width = attribute.data_type().byte_length();
            for component in 0..attribute.num_components() as usize {
                let mut bytes = [0u8; 8];
                attribute.buffer().read(
                    index * attribute.byte_stride() as usize + component * width,
                    &mut bytes[..width],
                );
                let (a, b) = &mut hashes[point];
                for byte in &bytes[..width] {
                    *a = (*a ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
                    *b = (*b ^ u64::from(*byte))
                        .rotate_left(13)
                        .wrapping_mul(0x9e37_79b1_85eb_ca87);
                }
                *b ^= (id as u64).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    hashes.sort_unstable();
    Ok(hashes)
}

fn fidelity_metrics(
    source: &[SourceAttribute],
    quantized: &[QuantizedAttribute],
    points: usize,
) -> serde_json::Value {
    let mut attribute_metrics = Vec::with_capacity(source.len());
    for (input, output) in source.iter().zip(quantized) {
        let mut sum_squared = 0.0f64;
        let mut max_absolute = 0.0f64;
        for (index, &value) in input.values.iter().enumerate() {
            let component = index % input.components;
            let reconstructed = dequantize(output, index / input.components, component);
            let error = f64::from((reconstructed - value).abs());
            sum_squared += error * error;
            max_absolute = max_absolute.max(error);
        }
        let count = input.values.len().max(1) as f64;
        attribute_metrics.push(serde_json::json!({
            "name": input.name,
            "geometry_type": format!("{:?}", input.kind),
            "components": input.components,
            "bits": output.bits,
            "rmse": (sum_squared / count).sqrt(),
            "max_absolute_error": max_absolute
        }));
    }

    let position_metrics = source
        .iter()
        .zip(quantized)
        .find(|(input, _)| input.kind == GeometryAttributeType::Position && input.components >= 3)
        .map(|(input, output)| {
            let mut squared = 0.0f64;
            let mut max_distance = 0.0f64;
            for point in 0..points {
                let mut distance_squared = 0.0f64;
                for component in 0..3 {
                    let a = f64::from(input.values[point * input.components + component]);
                    let b = f64::from(dequantize(output, point, component));
                    distance_squared += (a - b) * (a - b);
                }
                squared += distance_squared;
                max_distance = max_distance.max(distance_squared.sqrt());
            }
            serde_json::json!({
                "rmse_euclidean": (squared / points.max(1) as f64).sqrt(),
                "max_euclidean": max_distance
            })
        });

    let special = special_fidelity(source, quantized, points);
    serde_json::json!({
        "attributes": attribute_metrics,
        "position": position_metrics,
        "rotation_quaternion_angular_error_deg": special["rotation_quaternion_angular_error_deg"],
        "scale_exp_rmse": special["scale_exp_rmse"],
        "opacity_sigmoid_rmse": special["opacity_sigmoid_rmse"],
        "sh_view_direction_color_rmse": sh_color_metrics(source, quantized, points)
    })
}

fn dequantize(attribute: &QuantizedAttribute, point: usize, component: usize) -> f32 {
    if attribute.scale == 0.0 {
        attribute.minimum[component]
    } else {
        attribute.minimum[component]
            + attribute.values[point * attribute.components + component] as f32 / attribute.scale
    }
}

fn special_fidelity(
    source: &[SourceAttribute],
    quantized: &[QuantizedAttribute],
    points: usize,
) -> serde_json::Value {
    let lookup: HashMap<&str, usize> = source
        .iter()
        .enumerate()
        .map(|(index, attribute)| (attribute.name.as_str(), index))
        .collect();
    let rotation_names = ["rot_0", "rot_1", "rot_2", "rot_3"];
    let packed_rotation = lookup.get("rotation").copied();
    let rotation_indices: Option<Vec<(usize, usize)>> = if let Some(index) = packed_rotation {
        (source[index].components >= 4)
            .then(|| (0..4).map(|component| (index, component)).collect())
    } else {
        rotation_names
            .iter()
            .map(|name| lookup.get(name).copied().map(|index| (index, 0)))
            .collect()
    };
    let rotation = rotation_indices.map(|indices| {
        let mut sum = 0.0f64;
        let mut maximum = 0.0f64;
        for point in 0..points {
            let original: Vec<f64> = indices
                .iter()
                .map(|&(index, component)| {
                    f64::from(source[index].values[point * source[index].components + component])
                })
                .collect();
            let decoded: Vec<f64> = indices
                .iter()
                .map(|&(index, component)| {
                    f64::from(dequantize(&quantized[index], point, component))
                })
                .collect();
            let norm_a = original
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            let norm_b = decoded
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            if norm_a > 0.0 && norm_b > 0.0 {
                let dot = original
                    .iter()
                    .zip(&decoded)
                    .map(|(a, b)| a * b)
                    .sum::<f64>()
                    / (norm_a * norm_b);
                let angle = 2.0 * dot.abs().clamp(0.0, 1.0).acos().to_degrees();
                sum += angle * angle;
                maximum = maximum.max(angle);
            }
        }
        serde_json::json!({"rmse_deg": (sum / points.max(1) as f64).sqrt(), "max_deg": maximum})
    });

    let scale_indices: Vec<usize> = source
        .iter()
        .enumerate()
        .filter_map(|(index, attribute)| attribute.name.starts_with("scale_").then_some(index))
        .collect();
    let scale = (!scale_indices.is_empty()).then(|| {
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for index in scale_indices {
            for point in 0..points {
                let a = f64::from(source[index].values[point].exp());
                let b = f64::from(dequantize(&quantized[index], point, 0).exp());
                sum += (a - b) * (a - b);
                count += 1;
            }
        }
        (sum / count.max(1) as f64).sqrt()
    });

    let opacity = lookup.get("opacity").copied().map(|index| {
        let mut sum = 0.0f64;
        for point in 0..points {
            let a = f64::from(sigmoid(source[index].values[point]));
            let b = f64::from(sigmoid(dequantize(&quantized[index], point, 0)));
            sum += (a - b) * (a - b);
        }
        (sum / points.max(1) as f64).sqrt()
    });
    serde_json::json!({
        "rotation_quaternion_angular_error_deg": rotation,
        "scale_exp_rmse": scale,
        "opacity_sigmoid_rmse": opacity
    })
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

fn sh_color_metrics(
    source: &[SourceAttribute],
    quantized: &[QuantizedAttribute],
    points: usize,
) -> serde_json::Value {
    let lookup: HashMap<&str, usize> = source
        .iter()
        .enumerate()
        .map(|(index, attribute)| (attribute.name.as_str(), index))
        .collect();
    if !(0..3).all(|channel| lookup.contains_key(format!("f_dc_{channel}").as_str()))
        || !(0..45).all(|coefficient| lookup.contains_key(format!("f_rest_{coefficient}").as_str()))
    {
        return serde_json::Value::Null;
    }

    let directions: [(&str, [f32; 3]); 8] = [
        ("camera_front", [0.0, 0.0, 1.0]),
        ("camera_back", [0.0, 0.0, -1.0]),
        ("camera_right", [1.0, 0.0, 0.0]),
        ("camera_left", [-1.0, 0.0, 0.0]),
        ("camera_up", [0.0, 1.0, 0.0]),
        ("camera_down", [0.0, -1.0, 0.0]),
        (
            "camera_oblique_a",
            [0.577_350_26, 0.577_350_26, 0.577_350_26],
        ),
        (
            "camera_oblique_b",
            [-0.577_350_26, 0.577_350_26, -0.577_350_26],
        ),
    ];
    let stride = (points / 100_000).max(1);
    let mut result = serde_json::Map::new();
    for (direction_name, direction) in directions {
        let basis = sh_basis(direction);
        let mut squared = 0.0f64;
        let mut count = 0usize;
        for point in (0..points).step_by(stride) {
            for channel in 0..3 {
                let mut a = 0.5f32;
                let mut b = 0.5f32;
                let dc_index = lookup[format!("f_dc_{channel}").as_str()];
                a += source[dc_index].values[point] * basis[0];
                b += dequantize(&quantized[dc_index], point, 0) * basis[0];
                for coefficient in 0..15 {
                    let name = format!("f_rest_{}", channel * 15 + coefficient);
                    let index = lookup[name.as_str()];
                    a += source[index].values[point] * basis[coefficient + 1];
                    b += dequantize(&quantized[index], point, 0) * basis[coefficient + 1];
                }
                let error = f64::from((a.clamp(0.0, 1.0) - b.clamp(0.0, 1.0)).abs());
                squared += error * error;
                count += 1;
            }
        }
        result.insert(
            direction_name.to_owned(),
            serde_json::json!({"rmse": (squared / count.max(1) as f64).sqrt(), "sampled_splats": points.div_ceil(stride)}),
        );
    }
    serde_json::Value::Object(result)
}

fn sh_basis([x, y, z]: [f32; 3]) -> [f32; 16] {
    [
        0.282_094_8,
        -0.488_602_52 * y,
        0.488_602_52 * z,
        -0.488_602_52 * x,
        1.092_548_5 * x * y,
        -1.092_548_5 * y * z,
        0.315_391_57 * (2.0 * z * z - x * x - y * y),
        -1.092_548_5 * x * z,
        0.546_274_24 * (x * x - y * y),
        -0.590_043_6 * y * (3.0 * x * x - y * y),
        2.890_611_4 * x * y * z,
        -0.457_045_8 * y * (4.0 * z * z - x * x - y * y),
        0.373_176_34 * z * (2.0 * z * z - 3.0 * x * x - 3.0 * y * y),
        -0.457_045_8 * x * (4.0 * z * z - x * x - y * y),
        1.445_305_7 * z * (x * x - y * y),
        -0.590_043_6 * x * (x * x - 3.0 * y * y),
    ]
}

#[cfg(windows)]
#[repr(C)]
struct ProcessMemoryCountersEx {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn OpenProcess(
        desired_access: u32,
        inherit_handle: i32,
        process_id: u32,
    ) -> *mut std::ffi::c_void;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(
        process: *mut std::ffi::c_void,
        counters: *mut ProcessMemoryCountersEx,
        size: u32,
    ) -> i32;
}

#[cfg(windows)]
fn peak_working_set_bytes() -> u64 {
    // SAFETY: GetCurrentProcess has no arguments and returns this process's pseudo-handle.
    query_process_peak_working_set(unsafe { GetCurrentProcess() })
}

#[cfg(windows)]
fn query_process_peak_working_set(process: *mut std::ffi::c_void) -> u64 {
    let mut counters = ProcessMemoryCountersEx {
        cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
        private_usage: 0,
    };
    // SAFETY: GetCurrentProcess returns a pseudo-handle valid for this process; counters is a writable, correctly sized PROCESS_MEMORY_COUNTERS_EX.
    unsafe {
        if GetProcessMemoryInfo(
            process,
            &mut counters,
            std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
        ) != 0
        {
            counters.peak_working_set_size as u64
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
fn peak_working_set_bytes() -> u64 {
    0
}

#[cfg(windows)]
fn run_command_with_peak_working_set(
    command: &mut Command,
) -> Result<(Output, u64), Box<dyn Error>> {
    let mut child = command.spawn()?;
    // SAFETY: OpenProcess receives the live child PID returned by std::process and only requests query/read access.
    let process = unsafe { OpenProcess(0x0400 | 0x0010, 0, child.id()) };
    let mut peak = 0;
    let wait_result = loop {
        if !process.is_null() {
            peak = peak.max(query_process_peak_working_set(process));
        }
        match child.try_wait() {
            Ok(Some(_)) => break Ok(()),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => break Err(error),
        }
    };
    if !process.is_null() {
        peak = peak.max(query_process_peak_working_set(process));
        // SAFETY: `process` is the owned handle returned by OpenProcess above and is closed once here.
        unsafe { CloseHandle(process) };
    }
    wait_result?;
    let output = child.wait_with_output()?;
    Ok((output, peak))
}

#[cfg(not(windows))]
fn run_command_with_peak_working_set(
    command: &mut Command,
) -> Result<(Output, u64), Box<dyn Error>> {
    Ok((command.output()?, 0))
}

fn integrate_splat(
    label: &str,
    input_ply: &Path,
    gltfpack: &Path,
    output_dir: &Path,
    work_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(output_dir)?;
    fs::create_dir_all(work_dir)?;
    let source_path = input_ply.to_str().ok_or("input path is not UTF-8")?;
    let base_path = work_dir.join(format!("{label}-splat-base-noq.glb"));
    let base_start = Instant::now();
    let mut base_command = Command::new(gltfpack);
    base_command
        .args(["-i", source_path, "-o"])
        .arg(&base_path)
        .args(["-gsd", "3", "-noq"]);
    let (base_output, base_peak_working_set_bytes) =
        run_command_with_peak_working_set(&mut base_command)?;
    if !base_output.status.success() {
        return Err(format!(
            "gltfpack -noq failed for {label}: {}",
            String::from_utf8_lossy(&base_output.stderr)
        )
        .into());
    }
    let base_gltfpack_ms = base_start.elapsed().as_secs_f64() * 1000.0;
    let base_import = import_file(&base_path)?;
    let base_geometry = read_first_primitive(&base_import)?;
    let points = base_geometry.vertex_count();
    if points == 0 {
        return Err(format!("{label}: unquantized base GLB has no splats").into());
    }

    for (profile, bits) in [
        ("lossless", None),
        ("q8_matched_bits", Some(8u8)),
        ("q10_matched_bits", Some(10)),
        ("q12_matched_bits", Some(12)),
        ("q14_matched_bits", Some(14)),
        ("q16_matched_bits", Some(16)),
    ] {
        let meshopt_path = output_dir.join(format!("{label}-meshopt-{profile}.glb"));
        let mut meshopt_command = Command::new(gltfpack);
        meshopt_command
            .args(["-i", source_path, "-o"])
            .arg(&meshopt_path)
            .args(["-gsd", "3"]);
        if let Some(bits) = bits {
            meshopt_command
                .args(["-vp", &bits.to_string(), "-vt", &bits.to_string()])
                .args(["-vn", &(bits - 1).to_string(), "-vc", &bits.to_string()]);
        } else {
            meshopt_command.arg("-noq");
        }
        meshopt_command.args(["-c", "-ce", "khr"]);
        let meshopt_start = Instant::now();
        let (meshopt_output, meshopt_peak_working_set_bytes) =
            run_command_with_peak_working_set(&mut meshopt_command)?;
        if !meshopt_output.status.success() {
            return Err(format!(
                "gltfpack Meshopt {profile} failed for {label}: {}",
                String::from_utf8_lossy(&meshopt_output.stderr)
            )
            .into());
        }
        let meshopt_gltfpack_ms = meshopt_start.elapsed().as_secs_f64() * 1000.0;
        let meshopt_bytes = fs::read(&meshopt_path)?;
        let (meshopt_gzip_bytes, meshopt_brotli_bytes) = compressed_sizes(&meshopt_bytes)?;
        let (
            meshopt_import_parse_ms,
            meshopt_geometry_decode_ms,
            meshopt_file_to_geometry_ms,
            meshopt_geometry,
        ) = decode_glb_median(&meshopt_path, 3)?;
        ensure_geometry_counts(&base_geometry, &meshopt_geometry, "Meshopt", label, profile)?;
        let meshopt_quality = geometry_quality(&base_geometry, &meshopt_geometry)?;

        let mut import = import_file(&base_path)?;
        let mut options = CompressionOptions::default();
        options.mode = CompressionMode::DracoOnly;
        options.allow_points = true;
        options.encoding_method = 1;
        options.quantization = if let Some(bits) = bits {
            QuantizationBits {
                position: Some(bits),
                normal: None,
                tex_coord: None,
                color: None,
                generic: Some(bits),
            }
        } else {
            QuantizationBits::NONE
        };
        if let Some(bits) = bits {
            options.gaussian_splat_quantization.rotation_bits = Some(bits);
            options.gaussian_splat_quantization.scale_bits = Some(bits);
            options.gaussian_splat_quantization.opacity_bits = Some(bits);
            options.gaussian_splat_quantization.sh_degrees = [
                Some(ShQuantization::Bits(bits - 2)),
                Some(ShQuantization::Bits(bits - 2)),
                Some(ShQuantization::Bits(bits - 3)),
                Some(ShQuantization::Bits(bits - 3)),
            ];
        }
        let encode_start = Instant::now();
        let compression_report = import.compress_primitive(MeshIndex(0), 0, options)?;
        let draco_compress_ms = encode_start.elapsed().as_secs_f64() * 1000.0;
        let serialize_start = Instant::now();
        let draco_bytes = import.to_bytes(OutputFormat::GlbV2)?;
        let draco_serialize_ms = serialize_start.elapsed().as_secs_f64() * 1000.0;
        let draco_path = output_dir.join(format!("{label}-draco-{profile}.glb"));
        fs::write(&draco_path, &draco_bytes)?;
        let (draco_gzip_bytes, draco_brotli_bytes) = compressed_sizes(&draco_bytes)?;
        let (
            draco_import_parse_ms,
            draco_geometry_decode_ms,
            draco_file_to_geometry_ms,
            draco_geometry,
        ) = decode_glb_median(&draco_path, 3)?;
        ensure_geometry_counts(&base_geometry, &draco_geometry, "Draco", label, profile)?;
        let draco_quality = geometry_quality(&base_geometry, &draco_geometry)?;

        if bits.is_none()
            && (meshopt_quality["max_abs_error"].as_f64() != Some(0.0)
                || draco_quality["max_abs_error"].as_f64() != Some(0.0))
        {
            return Err(format!(
                "{label}: lossless GLB profile changed at least one source attribute"
            )
            .into());
        }

        let sh_bits = bits.map(|value| [value - 2, value - 2, value - 3, value - 3]);
        let profile_row = serde_json::json!({
            "kind": "splat_gltf_matched",
            "dataset": label,
            "profile": profile,
            "points": points,
            "attribute_count": base_geometry.attributes().len(),
            "source": "same PLY; Meshopt encodes directly, Draco reads the matching -noq KHR_gaussian_splatting GLB",
            "nominal_quantization_bits": {
                "position": bits,
                "rotation": bits,
                "scale": bits,
                "opacity": bits,
                "sh_degree_0": sh_bits.map(|value| value[0]),
                "sh_degree_1": sh_bits.map(|value| value[1]),
                "sh_degree_2": sh_bits.map(|value| value[2]),
                "sh_degree_3": sh_bits.map(|value| value[3]),
                "meshopt_flags": bits.map(|value| serde_json::json!({
                    "-vp": value,
                    "-vn": value - 1,
                    "-vt": value,
                    "-vc": value
                })),
                "note": "matched nominal bit allocation; codec quantization grids/filters differ, so decoded attribute errors are reported separately"
            },
            "decode_timing_note": "draco-gltf expands KHR_meshopt_compression bufferViews during import; Draco streams decode during read_primitives. Compare file_to_geometry_ms for end-to-end GLB latency, not the stage split.",
            "base": {
                "path": base_path,
                "gltfpack_noq_ms": base_gltfpack_ms,
                "gltfpack_peak_working_set_bytes": base_peak_working_set_bytes
            },
            "meshopt": {
                "path": meshopt_path,
                "glb_bytes": meshopt_bytes.len(),
                "gzip_bytes": meshopt_gzip_bytes,
                "brotli_bytes": meshopt_brotli_bytes,
                "gltfpack_total_ms": meshopt_gltfpack_ms,
                "gltfpack_peak_working_set_bytes": meshopt_peak_working_set_bytes,
                "import_and_meshopt_buffer_decode_ms_median_3_runs": meshopt_import_parse_ms,
                "geometry_read_ms_median_3_runs": meshopt_geometry_decode_ms,
                "file_to_geometry_ms_median_3_runs": meshopt_file_to_geometry_ms,
                "decoded_points": meshopt_geometry.vertex_count(),
                "quality_vs_unquantized_base": meshopt_quality
            },
            "draco_gltf": {
                "path": draco_path,
                "glb_bytes": draco_bytes.len(),
                "gzip_bytes": draco_gzip_bytes,
                "brotli_bytes": draco_brotli_bytes,
                "draco_payload_bytes": compression_report.encoded_bytes,
                "compress_ms_single_run": draco_compress_ms,
                "serialize_ms_single_run": draco_serialize_ms,
                "import_parse_ms_median_3_runs": draco_import_parse_ms,
                "draco_decode_and_geometry_read_ms_median_3_runs": draco_geometry_decode_ms,
                "file_to_geometry_ms_median_3_runs": draco_file_to_geometry_ms,
                "decoded_points": draco_geometry.vertex_count(),
                "quality_vs_unquantized_base": draco_quality,
                "standard_khr_draco_points_compatible": false
            },
            "external_compression": {
                "gzip": "flate2 Compression::best, applied to each complete GLB",
                "brotli": "quality 5, window 22, applied to each complete GLB"
            },
            "draco_mode": "sequential point cloud, point-order search disabled"
        });
        println!("{}", serde_json::to_string(&profile_row)?);
    }

    Ok(())
}

fn read_first_primitive(import: &Import) -> Result<PackedGeometry, Box<dyn Error>> {
    import
        .read_primitives(&[PrimitiveIndex::new(MeshIndex(0), 0)])?
        .into_iter()
        .next()
        .ok_or_else(|| "glTF has no first primitive".into())
}

fn decode_glb_median(
    path: &Path,
    runs: usize,
) -> Result<(f64, f64, f64, PackedGeometry), Box<dyn Error>> {
    if runs == 0 {
        return Err("decode run count must be positive".into());
    }
    let mut import_durations = Vec::with_capacity(runs);
    let mut decode_durations = Vec::with_capacity(runs);
    let mut total_durations = Vec::with_capacity(runs);
    // Prime the file cache on both paths. Report warm local reads, not cold I/O.
    drop(read_first_primitive(&import_file(path)?)?);
    let mut last_geometry = None;
    for _ in 0..runs {
        drop(last_geometry.take());
        let total_start = Instant::now();
        let import_start = Instant::now();
        let import = import_file(path)?;
        import_durations.push(import_start.elapsed().as_secs_f64() * 1000.0);
        let decode_start = Instant::now();
        let geometry = read_first_primitive(&import)?;
        decode_durations.push(decode_start.elapsed().as_secs_f64() * 1000.0);
        total_durations.push(total_start.elapsed().as_secs_f64() * 1000.0);
        last_geometry = Some(geometry);
    }
    let median = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    Ok((
        median(import_durations),
        median(decode_durations),
        median(total_durations),
        last_geometry.ok_or("decode produced no geometry")?,
    ))
}

fn ensure_geometry_counts(
    baseline: &PackedGeometry,
    decoded: &PackedGeometry,
    codec: &str,
    dataset: &str,
    profile: &str,
) -> Result<(), Box<dyn Error>> {
    if baseline.vertex_count() != decoded.vertex_count()
        || baseline.attributes().len() != decoded.attributes().len()
    {
        return Err(format!(
            "{dataset} {profile}: {codec} decoded {} points / {} attributes, expected {} / {}",
            decoded.vertex_count(),
            decoded.attributes().len(),
            baseline.vertex_count(),
            baseline.attributes().len()
        )
        .into());
    }
    Ok(())
}

fn packed_attribute_values(attribute: &PackedAttribute) -> Result<Vec<f64>, Box<dyn Error>> {
    let width = attribute.component_type().byte_width();
    let expected = attribute
        .count()
        .checked_mul(usize::from(attribute.components()))
        .and_then(|value| value.checked_mul(width))
        .ok_or("packed attribute byte length overflow")?;
    if attribute.bytes().len() != expected {
        return Err(format!(
            "{} has {} bytes, expected {expected}",
            attribute.semantic(),
            attribute.bytes().len()
        )
        .into());
    }
    let normalized = attribute.normalized();
    attribute
        .bytes()
        .chunks_exact(width)
        .map(|bytes| {
            let value = match attribute.component_type() {
                ComponentType::I8 => {
                    let value = i8::from_le_bytes([bytes[0]]) as f64;
                    if normalized {
                        (value / i8::MAX as f64).max(-1.0)
                    } else {
                        value
                    }
                }
                ComponentType::U8 => {
                    let value = f64::from(bytes[0]);
                    if normalized {
                        value / u8::MAX as f64
                    } else {
                        value
                    }
                }
                ComponentType::I16 => {
                    let value = i16::from_le_bytes(bytes.try_into()?) as f64;
                    if normalized {
                        (value / i16::MAX as f64).max(-1.0)
                    } else {
                        value
                    }
                }
                ComponentType::U16 => {
                    let value = f64::from(u16::from_le_bytes(bytes.try_into()?));
                    if normalized {
                        value / u16::MAX as f64
                    } else {
                        value
                    }
                }
                ComponentType::I32 => {
                    let value = i32::from_le_bytes(bytes.try_into()?) as f64;
                    if normalized {
                        (value / i32::MAX as f64).max(-1.0)
                    } else {
                        value
                    }
                }
                ComponentType::U32 => {
                    let value = f64::from(u32::from_le_bytes(bytes.try_into()?));
                    if normalized {
                        value / u32::MAX as f64
                    } else {
                        value
                    }
                }
                ComponentType::F32 => f64::from(f32::from_le_bytes(bytes.try_into()?)),
                ComponentType::F64 => f64::from_le_bytes(bytes.try_into()?),
                unsupported => {
                    return Err(format!(
                        "unsupported benchmark attribute type {unsupported:?} for {}",
                        attribute.semantic()
                    )
                    .into());
                }
            };
            if !value.is_finite() {
                return Err(format!(
                    "{} contains a non-finite decoded value",
                    attribute.semantic()
                )
                .into());
            }
            Ok(value)
        })
        .collect()
}

fn geometry_quality(
    baseline: &PackedGeometry,
    decoded: &PackedGeometry,
) -> Result<serde_json::Value, Box<dyn Error>> {
    ensure_geometry_counts(baseline, decoded, "output", "comparison", "geometry")?;
    let expected: BTreeMap<&str, &PackedAttribute> = baseline
        .attributes()
        .iter()
        .map(|attribute| (attribute.semantic(), attribute))
        .collect();
    let actual: BTreeMap<&str, &PackedAttribute> = decoded
        .attributes()
        .iter()
        .map(|attribute| (attribute.semantic(), attribute))
        .collect();
    if expected.keys().ne(actual.keys()) {
        return Err("decoded glTF attribute semantics differ from the unquantized base".into());
    }
    let mut attribute_errors = serde_json::Map::new();
    let mut all_maximum = 0.0f64;
    let mut position_sum_squared = 0.0f64;
    let mut position_count = 0usize;
    let mut rotation_sum_squared = 0.0f64;
    let mut rotation_count = 0usize;
    let mut scale_sum_squared = 0.0f64;
    let mut scale_count = 0usize;
    let mut opacity_sum_squared = 0.0f64;
    let mut opacity_count = 0usize;
    let mut sh_sum_squared = 0.0f64;
    let mut sh_count = 0usize;
    let mut sh_maximum = 0.0f64;

    for (&semantic, &expected_attribute) in &expected {
        let actual_attribute = actual[semantic];
        if expected_attribute.count() != actual_attribute.count()
            || expected_attribute.components() != actual_attribute.components()
        {
            return Err(format!("attribute layout changed for {semantic}").into());
        }
        let expected_values = packed_attribute_values(expected_attribute)?;
        let actual_values = packed_attribute_values(actual_attribute)?;
        let mut sum_squared = 0.0f64;
        let mut maximum = 0.0f64;
        for (left, right) in expected_values.iter().zip(&actual_values) {
            if !left.is_finite() || !right.is_finite() {
                return Err(format!("non-finite decoded or reference value for {semantic}").into());
            }
            let error = (left - right).abs();
            sum_squared += error * error;
            maximum = maximum.max(error);
            all_maximum = all_maximum.max(error);
            if semantic == "POSITION" {
                position_sum_squared += error * error;
            }
            if semantic == "KHR_gaussian_splatting:SCALE" {
                scale_sum_squared += error * error;
                scale_count += 1;
            }
            if semantic == "KHR_gaussian_splatting:OPACITY" {
                opacity_sum_squared += error * error;
                opacity_count += 1;
            }
            if semantic.starts_with("KHR_gaussian_splatting:SH_DEGREE_") {
                sh_sum_squared += error * error;
                sh_count += 1;
                sh_maximum = sh_maximum.max(error);
            }
        }
        if semantic == "POSITION" {
            position_count = expected_attribute.count();
        }
        if semantic == "KHR_gaussian_splatting:ROTATION" && expected_attribute.components() == 4 {
            for (left, right) in expected_values
                .chunks_exact(4)
                .zip(actual_values.chunks_exact(4))
            {
                let norm_left = left.iter().map(|value| value * value).sum::<f64>().sqrt();
                let norm_right = right.iter().map(|value| value * value).sum::<f64>().sqrt();
                if norm_left == 0.0 || norm_right == 0.0 {
                    return Err("zero-length rotation quaternion".into());
                }
                let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f64>()
                    / (norm_left * norm_right);
                let angle = 2.0 * dot.abs().clamp(0.0, 1.0).acos().to_degrees();
                rotation_sum_squared += angle * angle;
                rotation_count += 1;
            }
        }
        attribute_errors.insert(
            semantic.to_owned(),
            serde_json::json!({
                "rmse": (sum_squared / expected_values.len().max(1) as f64).sqrt(),
                "max_abs_error": maximum
            }),
        );
    }

    let position_rmse = (position_sum_squared / position_count.max(1) as f64).sqrt();
    let rotation_rmse = (rotation_sum_squared / rotation_count.max(1) as f64).sqrt();
    Ok(serde_json::json!({
        "attributes": attribute_errors,
        "max_abs_error": all_maximum,
        "position_rmse_euclidean": position_rmse,
        "rotation_quaternion_angular_rmse_deg": if rotation_count > 0 { Some(rotation_rmse) } else { None },
        "scale_accessor_rmse": if scale_count > 0 { Some((scale_sum_squared / scale_count as f64).sqrt()) } else { None },
        "opacity_accessor_rmse": if opacity_count > 0 { Some((opacity_sum_squared / opacity_count as f64).sqrt()) } else { None },
        "sh_coefficient_scalar_rmse": if sh_count > 0 { Some((sh_sum_squared / sh_count as f64).sqrt()) } else { None },
        "sh_coefficient_max_abs_error": if sh_count > 0 { Some(sh_maximum) } else { None },
        "sh_view_direction_color_rmse": sh_view_color_quality(&expected, &actual, baseline.vertex_count())?
    }))
}

fn sh_semantic_index(semantic: &str) -> Option<usize> {
    let rest = semantic.strip_prefix("KHR_gaussian_splatting:SH_DEGREE_")?;
    let (degree, coefficient) = rest.split_once("_COEF_")?;
    let degree: usize = degree.parse().ok()?;
    let coefficient: usize = coefficient.parse().ok()?;
    (degree <= 3 && coefficient < 2 * degree + 1).then_some(degree * degree + coefficient)
}

fn sample_attribute_values(values: &[f64], components: usize, stride: usize) -> Vec<f64> {
    let count = values.len() / components;
    (0..count)
        .step_by(stride)
        .flat_map(|point| {
            let start = point * components;
            values[start..start + components].iter().copied()
        })
        .collect()
}

fn sh_view_color_quality(
    expected: &BTreeMap<&str, &PackedAttribute>,
    actual: &BTreeMap<&str, &PackedAttribute>,
    points: usize,
) -> Result<serde_json::Value, Box<dyn Error>> {
    if points == 0 {
        return Err("cannot evaluate SH color for an empty splat set".into());
    }
    let stride = points.div_ceil(100_000);
    let sampled_points = points.div_ceil(stride);
    let mut coefficients: Vec<Option<(Vec<f64>, Vec<f64>)>> = (0..16).map(|_| None).collect();
    for (&semantic, expected_attribute) in expected {
        let Some(index) = sh_semantic_index(semantic) else {
            continue;
        };
        let actual_attribute = actual[semantic];
        if expected_attribute.components() != 3 || actual_attribute.components() != 3 {
            return Err(format!("SH attribute {semantic} must have three components").into());
        }
        let expected_values = packed_attribute_values(expected_attribute)?;
        let actual_values = packed_attribute_values(actual_attribute)?;
        coefficients[index] = Some((
            sample_attribute_values(&expected_values, 3, stride),
            sample_attribute_values(&actual_values, 3, stride),
        ));
    }
    let coefficients: Vec<(Vec<f64>, Vec<f64>)> = coefficients
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            value.ok_or_else(|| format!("Gaussian splat SH coefficient {index} is missing").into())
        })
        .collect::<Result<_, Box<dyn Error>>>()?;

    let directions: [(&str, [f32; 3]); 8] = [
        ("camera_front", [0.0, 0.0, 1.0]),
        ("camera_back", [0.0, 0.0, -1.0]),
        ("camera_right", [1.0, 0.0, 0.0]),
        ("camera_left", [-1.0, 0.0, 0.0]),
        ("camera_up", [0.0, 1.0, 0.0]),
        ("camera_down", [0.0, -1.0, 0.0]),
        (
            "camera_oblique_a",
            [0.577_350_26, 0.577_350_26, 0.577_350_26],
        ),
        (
            "camera_oblique_b",
            [-0.577_350_26, 0.577_350_26, -0.577_350_26],
        ),
    ];
    let mut direction_errors = serde_json::Map::new();
    let mut total_squared = 0.0f64;
    let mut total_values = 0usize;
    for (name, direction) in directions {
        let basis = sh_basis(direction);
        let mut squared = 0.0f64;
        for point in 0..sampled_points {
            for channel in 0..3 {
                let mut expected_color = 0.5f64;
                let mut actual_color = 0.5f64;
                for (coefficient, (expected_values, actual_values)) in
                    coefficients.iter().enumerate()
                {
                    let offset = point * 3 + channel;
                    expected_color += expected_values[offset] * f64::from(basis[coefficient]);
                    actual_color += actual_values[offset] * f64::from(basis[coefficient]);
                }
                let error = (expected_color.clamp(0.0, 1.0) - actual_color.clamp(0.0, 1.0)).abs();
                squared += error * error;
            }
        }
        total_squared += squared;
        total_values += sampled_points * 3;
        direction_errors.insert(
            name.to_owned(),
            serde_json::json!({
                "rmse": (squared / (sampled_points * 3).max(1) as f64).sqrt(),
                "sampled_splats": sampled_points
            }),
        );
    }
    Ok(serde_json::json!({
        "aggregate_rmse": (total_squared / total_values.max(1) as f64).sqrt(),
        "sampled_splats": sampled_points,
        "directions": direction_errors
    }))
}

fn import_file(path: &Path) -> Result<Import, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let options = ImportOptions {
        base_path: path.parent(),
        external_file_policy: ExternalFilePolicy::Allow,
        ..ImportOptions::default()
    };
    Ok(draco_gltf::import_slice_with_options(&bytes, &options)?)
}

fn measure_mesh_load(path: &Path) -> Result<(f64, usize, usize), Box<dyn Error>> {
    let (_, _, total_ms, geometry) = decode_glb_median(path, 3)?;
    let indices = geometry.indices().map_or(0, |value| value.count());
    Ok((total_ms, geometry.vertex_count(), indices))
}

fn integrate_mesh(
    input_glb: &Path,
    gltfpack: &Path,
    output_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(output_dir)?;
    let label = input_glb
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("mesh");
    let mut meshopt_variants = Vec::new();
    for (name, compression, fallback) in [
        ("c", "-c", false),
        ("cc", "-cc", false),
        ("cf", "-cf", true),
    ] {
        let path = output_dir.join(format!("{label}-meshopt-{name}.glb"));
        let start = Instant::now();
        let mut command = Command::new(gltfpack);
        command
            .args([
                "-i",
                input_glb.to_str().ok_or("GLB path is not UTF-8")?,
                "-o",
            ])
            .arg(&path)
            .args([compression, "-ce", "khr"]);
        let (output, gltfpack_peak_working_set_bytes) =
            run_command_with_peak_working_set(&mut command)?;
        let gltfpack_total_ms = start.elapsed().as_secs_f64() * 1000.0;
        if !output.status.success() {
            return Err(format!(
                "gltfpack {compression} failed for {}: {}",
                input_glb.display(),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        let bytes = fs::read(&path)?;
        let (package_bytes, gzip, brotli, sidecar_bytes) = asset_sizes(&path)?;
        let (total_ms, vertices, indices) = measure_mesh_load(&path)?;
        meshopt_variants.push(serde_json::json!({
            "name": name,
            "fallback": fallback,
            "path": path,
            "bytes": bytes.len(),
            "package_bytes": package_bytes,
            "sidecar_bytes": sidecar_bytes,
            "gzip_bytes": gzip,
            "brotli_bytes": brotli,
            "gltfpack_total_ms": gltfpack_total_ms,
            "gltfpack_peak_working_set_bytes": gltfpack_peak_working_set_bytes,
            "file_to_geometry_ms_median_3_runs": total_ms,
            "vertices": vertices,
            "indices": indices
        }));
    }

    let original = import_file(input_glb)?;
    let mut draco_variants = Vec::new();
    for (name, mode, fallback) in [
        ("draco_only", CompressionMode::DracoOnly, false),
        ("fallback", CompressionMode::Fallback, true),
    ] {
        let mut import = original.clone();
        let mut options = CompressionOptions::default();
        options.mode = mode;
        options.quantization = QuantizationBits::GLTF;
        let encode_start = Instant::now();
        let report = import.compress_primitive(MeshIndex(0), 0, options)?;
        let compress_ms = encode_start.elapsed().as_secs_f64() * 1000.0;
        let serialize_start = Instant::now();
        let bytes = import.to_bytes(OutputFormat::GlbV2)?;
        let serialize_ms = serialize_start.elapsed().as_secs_f64() * 1000.0;
        let path = output_dir.join(format!("{label}-{name}.glb"));
        fs::write(&path, &bytes)?;
        let (total_ms, vertices, indices) = measure_mesh_load(&path)?;
        let (package_bytes, gzip, brotli, sidecar_bytes) = asset_sizes(&path)?;
        draco_variants.push(serde_json::json!({
            "name": name,
            "fallback": fallback,
            "path": path,
            "bytes": bytes.len(),
            "package_bytes": package_bytes,
            "sidecar_bytes": sidecar_bytes,
            "gzip_bytes": gzip,
            "brotli_bytes": brotli,
            "draco_payload_bytes": report.encoded_bytes,
            "compress_ms": compress_ms,
            "serialize_ms": serialize_ms,
            "file_to_geometry_ms_median_3_runs": total_ms,
            "vertices": vertices,
            "indices": indices,
            "roundtrip_readable": vertices > 0
        }));
    }
    let process_peak_working_set_bytes = peak_working_set_bytes();
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "kind": "mesh_gltf_integration",
            "dataset": label,
            "process_peak_working_set_bytes": process_peak_working_set_bytes,
            "meshopt_variants": meshopt_variants,
            "draco_gltf_variants": draco_variants
        }))?
    );
    Ok(())
}

fn compressed_sizes(bytes: &[u8]) -> Result<(usize, usize), Box<dyn Error>> {
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gzip.write_all(bytes)?;
    let gzip = gzip.finish()?.len();
    let mut brotli = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
    brotli.write_all(bytes)?;
    let brotli = brotli.into_inner().len();
    Ok((gzip, brotli))
}

fn asset_sizes(glb: &Path) -> Result<(u64, u64, u64, u64), Box<dyn Error>> {
    let parent = glb.parent().ok_or("GLB path has no parent directory")?;
    let stem = glb
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("GLB file name is not UTF-8")?;
    let sidecar_prefix = format!("{stem}.");
    let mut total = 0u64;
    let mut gzip_total = 0u64;
    let mut brotli_total = 0u64;
    let mut sidecar_total = 0u64;
    for entry in fs::read_dir(parent)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if path != glb && !name.starts_with(&sidecar_prefix) {
            continue;
        }
        let bytes = fs::read(&path)?;
        let (gzip, brotli) = compressed_sizes(&bytes)?;
        total += bytes.len() as u64;
        gzip_total += gzip as u64;
        brotli_total += brotli as u64;
        if path != glb {
            sidecar_total += bytes.len() as u64;
        }
    }
    Ok((total, gzip_total, brotli_total, sidecar_total))
}

fn write_summary(results_path: &Path) -> Result<(), Box<dyn Error>> {
    let input = fs::read_to_string(results_path)?;
    let mut rows = Vec::new();
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        rows.push(serde_json::from_str::<serde_json::Value>(line)?);
    }

    let mut report = String::from(
        "# Draco and Meshoptimizer benchmark\n\nRaw per-case measurements are in `raw-results.jsonl`; this file is the compact index.\n\n",
    );
    report.push_str("## Codec curves: identical quantized point records\n\n");
    report.push_str("| Dataset | Points | Bits | Meshopt L2 B | Meshopt L3 B | Draco sequential B | Meshopt prep ms | Meshopt encode ms | Draco encode ms | Fidelity |\n| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");
    let mut curves: BTreeMap<(String, u64), Vec<&serde_json::Value>> = BTreeMap::new();
    for row in rows.iter().filter(|row| row["kind"] == "codec_curve") {
        curves
            .entry((
                row["dataset"].as_str().unwrap_or("?").to_owned(),
                row["quality_bits"].as_u64().unwrap_or(0),
            ))
            .or_default()
            .push(row);
    }
    for ((dataset, bits), cases) in curves {
        let level = |number: u64| {
            cases.iter().find(|row| {
                row["meshopt"]["mode"]
                    .as_str()
                    .is_some_and(|mode| mode.ends_with(&format!("level_{number}")))
            })
        };
        let l2 = level(2);
        let l3 = level(3);
        let representative = l2.or(l3).copied();
        if let Some(row) = representative {
            let fidelity = row["fidelity"]["position"]["rmse_euclidean"]
                .as_f64()
                .map(|value| format!("pos RMSE {value:.5}"))
                .unwrap_or_else(|| "see raw per-attribute metrics".into());
            report.push_str(&format!(
                "| {dataset} | {} | {bits} | {} | {} | {} | {} | {} | {} | {fidelity} |\n",
                row["points"].as_u64().unwrap_or(0),
                l2.map(|value| value["meshopt"]["encoded_bytes"].as_u64().unwrap_or(0))
                    .unwrap_or(0),
                l3.map(|value| value["meshopt"]["encoded_bytes"].as_u64().unwrap_or(0))
                    .unwrap_or(0),
                row["draco_core"]["encoded_bytes"].as_u64().unwrap_or(0),
                l2.map(|value| value["meshopt"]["prepare_ms"].as_f64().unwrap_or(0.0))
                    .unwrap_or(0.0),
                l2.map(|value| value["meshopt"]["encode_ms_median"].as_f64().unwrap_or(0.0))
                    .unwrap_or(0.0),
                row["draco_core"]["encode_ms_median_3_runs"]
                    .as_f64()
                    .unwrap_or(0.0),
            ));
        }
    }

    report.push_str("\n## Draco point-cloud coding choices\n\n| Dataset | Points | Mode | Bits | Bytes | Encode ms | Decode ms | Exact point set |\n| --- | ---: | --- | ---: | ---: | ---: | ---: | --- |\n");
    for row in rows.iter().filter(|row| row["kind"] == "draco_mode") {
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            row["dataset"].as_str().unwrap_or("?"),
            row["points"].as_u64().unwrap_or(0),
            row["mode"].as_str().unwrap_or("failed"),
            row["quality_bits"].as_u64().unwrap_or(0),
            row["encoded_bytes"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "—".into()),
            row["encode_ms_single_run"]
                .as_f64()
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "—".into()),
            row["decode_ms_median_3_runs"]
                .as_f64()
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "—".into()),
            row["roundtrip_point_set_exact"]
                .as_bool()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "failed".into()),
        ));
    }

    let matched_splat_rows: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|row| row["kind"] == "splat_gltf_matched")
        .collect();
    if !matched_splat_rows.is_empty() {
        report.push_str("\n## Gaussian splat GLB: size, error and loading time\n\n");
        report.push_str("Both codecs start from the same PLY. Compare size at similar measured error across budgets. Equal nominal bits do not imply equal precision. Directional RGB measures sampled per-splat error without rasterization.\n\n");
        report.push_str("GLB→geometry includes reading the local file, Meshopt buffer decoding and filters during import, and Draco decoding during primitive reads. The stage timings and per-attribute measurements remain in JSONL. External gzip and Brotli apply to size only.\n\n");
        report.push_str("| Dataset | Codec | Profile | GLB B | gzip B | Brotli B | GLB→geometry ms | Position RMSE | Rotation RMSE° | Scale RMSE | Opacity RMSE | SH RMSE | Directional RGB RMSE |\n| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
        let number = |value: &serde_json::Value, digits: usize| {
            value
                .as_f64()
                .map(|value| format!("{value:.digits$}"))
                .unwrap_or_else(|| "—".into())
        };
        for row in &matched_splat_rows {
            for (key, name) in [("meshopt", "Meshopt"), ("draco_gltf", "Draco")] {
                let codec = &row[key];
                let quality = &codec["quality_vs_unquantized_base"];
                report.push_str(&format!(
                    "| {} | {name} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                    row["dataset"].as_str().unwrap_or("?"),
                    row["profile"].as_str().unwrap_or("?"),
                    codec["glb_bytes"].as_u64().unwrap_or(0),
                    codec["gzip_bytes"].as_u64().unwrap_or(0),
                    codec["brotli_bytes"].as_u64().unwrap_or(0),
                    number(&codec["file_to_geometry_ms_median_3_runs"], 2),
                    number(&quality["position_rmse_euclidean"], 6),
                    number(&quality["rotation_quaternion_angular_rmse_deg"], 6),
                    number(&quality["scale_accessor_rmse"], 6),
                    number(&quality["opacity_accessor_rmse"], 6),
                    number(&quality["sh_coefficient_scalar_rmse"], 6),
                    number(
                        &quality["sh_view_direction_color_rmse"]["aggregate_rmse"],
                        6
                    ),
                ));
            }
        }
    }

    report.push_str("\n## Triangle mesh glTF integration\n\n| Variant | Codec | Package B | gzip B | Brotli B | Vertices | Indices |\n| --- | --- | ---: | ---: | ---: | ---: | ---: |\n");
    for row in rows
        .iter()
        .filter(|row| row["kind"] == "mesh_gltf_integration")
    {
        for variant in row["meshopt_variants"].as_array().into_iter().flatten() {
            report.push_str(&format!(
                "| gltfpack {} | Meshopt | {} | {} | {} | {} | {} |\n",
                variant["name"].as_str().unwrap_or("?"),
                variant["package_bytes"].as_u64().unwrap_or(0),
                variant["gzip_bytes"].as_u64().unwrap_or(0),
                variant["brotli_bytes"].as_u64().unwrap_or(0),
                variant["vertices"].as_u64().unwrap_or(0),
                variant["indices"].as_u64().unwrap_or(0),
            ));
        }
        for variant in row["draco_gltf_variants"].as_array().into_iter().flatten() {
            report.push_str(&format!(
                "| draco-gltf {} | Draco | {} | {} | {} | {} | {} |\n",
                variant["name"].as_str().unwrap_or("?"),
                variant["package_bytes"].as_u64().unwrap_or(0),
                variant["gzip_bytes"].as_u64().unwrap_or(0),
                variant["brotli_bytes"].as_u64().unwrap_or(0),
                variant["vertices"].as_u64().unwrap_or(0),
                variant["indices"].as_u64().unwrap_or(0),
            ));
        }
    }
    report.push_str("\nGaussian point-cloud bytes in KHR_draco_mesh_compression are experimental and outside the extension's standard TRIANGLES/TRIANGLE_STRIP mode table.\n");

    let summary_path = results_path.with_file_name("summary.md");
    fs::write(&summary_path, report)?;
    println!("wrote {}", summary_path.display());
    Ok(())
}
