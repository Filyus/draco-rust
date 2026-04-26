// Profile sequential encoding to identify bottlenecks

mod common;

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static OUTPUT_LOCK: Mutex<()> = Mutex::new(());

fn duration_to_us(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn avg_duration_us(duration: Duration, iterations: u32) -> f64 {
    duration_to_us(duration) / f64::from(iterations)
}

fn create_grid_mesh(grid_size: usize) -> (Mesh, Vec<f32>, Vec<u32>) {
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    // Create positions
    let mut positions = Vec::with_capacity(num_points * 3);
    for y in 0..grid_size {
        for x in 0..grid_size {
            let px = x as f32;
            let py = y as f32;
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;
            positions.push(px);
            positions.push(py);
            positions.push(pz);
        }
    }

    // Create faces
    let mut faces = Vec::with_capacity(num_faces * 3);
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = (y * grid_size + x) as u32;
            let p1 = (y * grid_size + x + 1) as u32;
            let p2 = ((y + 1) * grid_size + x) as u32;
            let p3 = ((y + 1) * grid_size + x + 1) as u32;

            faces.push(p0);
            faces.push(p1);
            faces.push(p2);

            faces.push(p1);
            faces.push(p3);
            faces.push(p2);
        }
    }

    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);
    mesh.set_num_faces(num_faces);

    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    for i in 0..num_points {
        let offset = i * 3 * 4;
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3].to_le_bytes(), Some(offset));
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3 + 1].to_le_bytes(), Some(offset + 4));
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3 + 2].to_le_bytes(), Some(offset + 8));
    }
    mesh.add_attribute(pos_attr);

    for i in 0..num_faces {
        mesh.set_face(
            FaceIndex(i as u32),
            [
                PointIndex(faces[i * 3]),
                PointIndex(faces[i * 3 + 1]),
                PointIndex(faces[i * 3 + 2]),
            ],
        );
    }

    (mesh, positions, faces)
}

#[test]
fn profile_sequential_pipeline() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Profiling Sequential Encoding (Speed 10) ===\n");

    for grid_size in [50, 100] {
        let (mesh, positions, faces) = create_grid_mesh(grid_size);
        let num_points = positions.len() / 3;
        let num_faces = faces.len() / 3;

        println!(
            "Grid {}x{}: {} points, {} faces",
            grid_size, grid_size, num_points, num_faces
        );

        // Warm up
        for _ in 0..3 {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", 10);
            options.set_global_int("decoding_speed", 10);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());
            let mut encoder_buffer = EncoderBuffer::new();
            let _ = encoder.encode(&options, &mut encoder_buffer);
        }

        // Time encoding
        let iterations = 20;
        let mut times = Vec::new();

        for _ in 0..iterations {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", 10);
            options.set_global_int("decoding_speed", 10);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());
            let mut encoder_buffer = EncoderBuffer::new();

            let start = Instant::now();
            let _ = encoder.encode(&options, &mut encoder_buffer);
            let elapsed = start.elapsed();
            times.push(duration_to_us(elapsed) / 1000.0);
        }

        let avg: f64 = times.iter().sum::<f64>() / times.len() as f64;
        let min: f64 = times.iter().cloned().fold(f64::MAX, f64::min);
        let max: f64 = times.iter().cloned().fold(f64::MIN, f64::max);

        println!(
            "  Rust avg: {:.2}ms  min: {:.2}ms  max: {:.2}ms",
            avg, min, max
        );

        // C++ comparison
        let cpp_time = unsafe {
            let mut output_size = 0usize;
            draco_cpp_test_bridge::draco_benchmark_encode_mesh(
                num_points as u32,
                positions.as_ptr(),
                num_faces as u32,
                faces.as_ptr(),
                10,
                10,
                10,
                iterations,
                &mut output_size as *mut usize,
            )
        };

        if cpp_time >= 0 {
            let cpp_ms = cpp_time as f64 / 1000.0;
            println!("  C++  avg: {:.2}ms", cpp_ms);
            println!("  Speedup (C++/Rust): {:.2}x", cpp_ms / avg);
        }

        println!();
    }
}

#[test]
fn profile_detailed_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Detailed Encoding Breakdown (Speed 10) ===\n");

    // Use 100x100 grid for meaningful measurements
    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    println!(
        "Grid {}x{}: {} points, {} faces\n",
        grid_size, grid_size, num_points, num_faces
    );

    // Warm up
    for _ in 0..3 {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let _ = encoder.encode(&options, &mut encoder_buffer);
    }

    let iterations = 50;

    // Profile individual components

    // 1. Mesh clone
    let start = Instant::now();
    for _ in 0..iterations {
        let _cloned = mesh.clone();
    }
    let mesh_clone_us = avg_duration_us(start.elapsed(), iterations);

    // 2. Options setup
    let start = Instant::now();
    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);
        std::hint::black_box(&options);
    }
    let options_us = avg_duration_us(start.elapsed(), iterations);

    // 3. Encoder creation + set_mesh
    let start = Instant::now();
    for _ in 0..iterations {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        std::hint::black_box(&encoder);
    }
    let encoder_setup_us = avg_duration_us(start.elapsed(), iterations);

    // 4. Full encode
    let start = Instant::now();
    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let _ = encoder.encode(&options, &mut encoder_buffer);
    }
    let total_us = avg_duration_us(start.elapsed(), iterations);

    // Estimate time spent in actual encoding (exclude clone/setup)
    let encoding_core_us = total_us - mesh_clone_us - options_us;

    println!("Component breakdown (avg over {} iterations):", iterations);
    println!(
        "  Mesh clone:          {:7.1} µs ({:5.1}%)",
        mesh_clone_us,
        mesh_clone_us / total_us * 100.0
    );
    println!(
        "  Options setup:       {:7.1} µs ({:5.1}%)",
        options_us,
        options_us / total_us * 100.0
    );
    println!(
        "  Encoder setup:       {:7.1} µs ({:5.1}%)",
        encoder_setup_us - mesh_clone_us,
        (encoder_setup_us - mesh_clone_us) / total_us * 100.0
    );
    println!(
        "  Encoding (core):     {:7.1} µs ({:5.1}%)",
        encoding_core_us,
        encoding_core_us / total_us * 100.0
    );
    println!("  ─────────────────────────────");
    println!("  TOTAL:               {:7.1} µs", total_us);
    println!();

    // C++ comparison
    let cpp_time = unsafe {
        let mut output_size = 0usize;
        draco_cpp_test_bridge::draco_benchmark_encode_mesh(
            num_points as u32,
            positions.as_ptr(),
            num_faces as u32,
            faces.as_ptr(),
            10,
            10,
            10,
            iterations,
            &mut output_size as *mut usize,
        )
    };

    if cpp_time >= 0 {
        let cpp_us = cpp_time as f64;
        println!("C++ avg:               {:7.1} µs", cpp_us);
        println!("C++/Rust speedup:      {:7.2}x", cpp_us / total_us);
        println!();
        println!("If Rust matched C++ at encoding core, total would be:");
        let hypothetical = mesh_clone_us + options_us + cpp_us;
        println!(
            "  {:7.1} µs (speedup: {:.2}x)",
            hypothetical,
            cpp_us / hypothetical
        );
    }
}

use draco_core::attribute_quantization_transform::AttributeQuantizationTransform;
use draco_core::attribute_transform::AttributeTransform;
use draco_core::symbol_encoding::{encode_symbols, SymbolEncodingOptions};

#[test]
fn profile_encoding_stages() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Profiling Individual Encoding Stages (Speed 10) ===\n");

    let grid_size = 100;
    let (mesh, _, _) = create_grid_mesh(grid_size);
    let num_points = mesh.num_points();
    let num_components = 3;

    println!(
        "Grid {}x{}: {} points, {} components\n",
        grid_size, grid_size, num_points, num_components
    );

    let iterations = 100;

    // Get the position attribute
    let pos_att = mesh.attribute(0);
    let point_ids: Vec<PointIndex> = (0..num_points).map(|i| PointIndex(i as u32)).collect();

    // Stage 1: Quantization transform computation
    let start = Instant::now();
    for _ in 0..iterations {
        let mut q_transform = AttributeQuantizationTransform::new();
        q_transform.compute_parameters(pos_att, 10);
        std::hint::black_box(&q_transform);
    }
    let quant_compute_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 2: Quantization transform application
    let mut q_transform = AttributeQuantizationTransform::new();
    q_transform.compute_parameters(pos_att, 10);

    let start = Instant::now();
    for _ in 0..iterations {
        let mut portable = PointAttribute::default();
        q_transform.transform_attribute(pos_att, &point_ids, &mut portable);
        std::hint::black_box(&portable);
    }
    let quant_apply_us = avg_duration_us(start.elapsed(), iterations);

    // Get quantized values for symbol encoding test
    let mut portable = PointAttribute::default();
    q_transform.transform_attribute(pos_att, &point_ids, &mut portable);

    // Stage 3: Value gathering from portable attribute
    let start = Instant::now();
    for _ in 0..iterations {
        let mut values = Vec::with_capacity(num_points * 3);
        let data = portable.buffer().data();
        let byte_stride = portable.byte_stride() as usize;
        for i in 0..num_points {
            let offset = i * byte_stride;
            let x = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let y = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let z = u32::from_le_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
            ]);
            values.push(x as i32);
            values.push(y as i32);
            values.push(z as i32);
        }
        std::hint::black_box(&values);
    }
    let gather_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 4: Delta prediction + wrap transform (simulating what happens)
    let mut values: Vec<i32> = Vec::with_capacity(num_points * 3);
    {
        let data = portable.buffer().data();
        let byte_stride = portable.byte_stride() as usize;
        for i in 0..num_points {
            let offset = i * byte_stride;
            let x = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let y = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let z = u32::from_le_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
            ]);
            values.push(x as i32);
            values.push(y as i32);
            values.push(z as i32);
        }
    }

    let start = Instant::now();
    for _ in 0..iterations {
        let num_values = values.len();
        let mut corrections = vec![0i32; num_values];

        // Compute min/max
        let mut min_val = values[0];
        let mut max_val = values[0];
        for &val in &values[1..] {
            if val < min_val {
                min_val = val;
            }
            if val > max_val {
                max_val = val;
            }
        }

        let dif = (max_val as i64) - (min_val as i64);
        let max_dif = (1 + dif) as i32;
        let max_correction = max_dif / 2;
        let min_correction = -max_correction - if (max_dif & 1) == 0 { 0 } else { -1 };
        let max_correction_adj = if (max_dif & 1) == 0 {
            max_correction - 1
        } else {
            max_correction
        };

        // Delta + wrap
        let mut i = num_values - 3;
        while i >= 3 {
            let orig_x = values[i];
            let orig_y = values[i + 1];
            let orig_z = values[i + 2];

            let pred_x = values[i - 3].clamp(min_val, max_val);
            let pred_y = values[i - 2].clamp(min_val, max_val);
            let pred_z = values[i - 1].clamp(min_val, max_val);

            let mut corr_x = orig_x.wrapping_sub(pred_x);
            let mut corr_y = orig_y.wrapping_sub(pred_y);
            let mut corr_z = orig_z.wrapping_sub(pred_z);

            if corr_x < min_correction {
                corr_x = corr_x.wrapping_add(max_dif);
            } else if corr_x > max_correction_adj {
                corr_x = corr_x.wrapping_sub(max_dif);
            }
            if corr_y < min_correction {
                corr_y = corr_y.wrapping_add(max_dif);
            } else if corr_y > max_correction_adj {
                corr_y = corr_y.wrapping_sub(max_dif);
            }
            if corr_z < min_correction {
                corr_z = corr_z.wrapping_add(max_dif);
            } else if corr_z > max_correction_adj {
                corr_z = corr_z.wrapping_sub(max_dif);
            }

            corrections[i] = corr_x;
            corrections[i + 1] = corr_y;
            corrections[i + 2] = corr_z;

            i -= 3;
        }
        corrections[0] = values[0];
        corrections[1] = values[1];
        corrections[2] = values[2];

        std::hint::black_box(&corrections);
    }
    let delta_wrap_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 5: Zigzag encoding (convert signed to unsigned)
    let start = Instant::now();
    for _ in 0..iterations {
        let mut symbols = Vec::with_capacity(values.len());
        for &val in &values {
            let zigzag = if val < 0 {
                ((-val as u32) << 1) - 1
            } else {
                (val as u32) << 1
            };
            symbols.push(zigzag);
        }
        std::hint::black_box(&symbols);
    }
    let zigzag_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 6: Symbol encoding (the entropy coding stage)
    // Prepare symbols (zigzag encoded)
    let symbols: Vec<u32> = values
        .iter()
        .map(|&val| {
            if val < 0 {
                ((-val as u32) << 1) - 1
            } else {
                (val as u32) << 1
            }
        })
        .collect();

    let start = Instant::now();
    for _ in 0..iterations {
        let mut buffer = EncoderBuffer::new();
        let options = SymbolEncodingOptions {
            compression_level: 7,
        };
        encode_symbols(&symbols, 3, &options, &mut buffer);
        std::hint::black_box(&buffer);
    }
    let symbol_encode_us = avg_duration_us(start.elapsed(), iterations);

    // Print results
    let total_staged = quant_compute_us
        + quant_apply_us
        + gather_us
        + delta_wrap_us
        + zigzag_us
        + symbol_encode_us;

    println!("Stage breakdown (avg over {} iterations):", iterations);
    println!(
        "  1. Quantization compute:  {:7.1} µs ({:5.1}%)",
        quant_compute_us,
        quant_compute_us / total_staged * 100.0
    );
    println!(
        "  2. Quantization apply:    {:7.1} µs ({:5.1}%)",
        quant_apply_us,
        quant_apply_us / total_staged * 100.0
    );
    println!(
        "  3. Value gathering:       {:7.1} µs ({:5.1}%)",
        gather_us,
        gather_us / total_staged * 100.0
    );
    println!(
        "  4. Delta + wrap:          {:7.1} µs ({:5.1}%)",
        delta_wrap_us,
        delta_wrap_us / total_staged * 100.0
    );
    println!(
        "  5. Zigzag encoding:       {:7.1} µs ({:5.1}%)",
        zigzag_us,
        zigzag_us / total_staged * 100.0
    );
    println!(
        "  6. Symbol encoding:       {:7.1} µs ({:5.1}%)",
        symbol_encode_us,
        symbol_encode_us / total_staged * 100.0
    );
    println!("  ─────────────────────────────────────");
    println!("  Staged total:             {:7.1} µs", total_staged);
    println!();
    println!("Note: Full encode includes header, connectivity, attribute metadata,");
    println!("      and other bookkeeping not measured in individual stages.");
}

#[test]
fn profile_symbol_encoding_details() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Symbol Encoding Breakdown ===\n");

    // Create test data similar to what we have in a 100x100 mesh encode
    let num_points = 10201;
    let num_components = 3;
    let num_values = num_points * num_components;

    // Simulate quantized position values (11 bits for 100x100 @ speed 10)
    let max_value = 2047u32; // 11 bits
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let symbols: Vec<u32> = (0..num_values)
        .map(|i| {
            // Create somewhat realistic distribution - zigzag of deltas
            let mut h = DefaultHasher::new();
            i.hash(&mut h);
            (h.finish() as u32) % (max_value + 1)
        })
        .collect();

    let iterations = 100;

    // Stage A: Compute bit lengths
    let start = Instant::now();
    for _ in 0..iterations {
        let mut bit_lengths = Vec::with_capacity(symbols.len() / num_components);
        for chunk in symbols.chunks(num_components) {
            let mut max_component_value = chunk[0];
            for &val in &chunk[1..] {
                if val > max_component_value {
                    max_component_value = val;
                }
            }
            let bit_length = if max_component_value > 0 {
                32 - max_component_value.leading_zeros()
            } else {
                1
            };
            bit_lengths.push(bit_length);
        }
        std::hint::black_box(&bit_lengths);
    }
    let bit_lengths_us = avg_duration_us(start.elapsed(), iterations);

    // Prepare bit_lengths for reuse
    let _bit_lengths: Vec<u32> = symbols
        .chunks(num_components)
        .map(|chunk| {
            let max_comp = *chunk.iter().max().unwrap();
            if max_comp > 0 {
                32 - max_comp.leading_zeros()
            } else {
                1
            }
        })
        .collect();

    // Stage B: Compute frequencies for raw scheme
    let start = Instant::now();
    for _ in 0..iterations {
        let mut frequencies = vec![0u64; (max_value + 1) as usize];
        for &s in &symbols {
            frequencies[s as usize] += 1;
        }
        let mut num_unique: u32 = 0;
        for &f in &frequencies {
            if f > 0 {
                num_unique += 1;
            }
        }
        std::hint::black_box((frequencies, num_unique));
    }
    let freq_count_us = avg_duration_us(start.elapsed(), iterations);

    // Prepare frequencies
    let mut frequencies = vec![0u64; (max_value + 1) as usize];
    for &s in &symbols {
        frequencies[s as usize] += 1;
    }

    // Stage C: rANS table creation (probability normalization)
    let start = Instant::now();
    for _ in 0..iterations {
        // Simulate what RAnsSymbolEncoder::create does
        let rans_precision: u32 = 1 << 15; // typical precision
        let total_freq: u64 = symbols.len() as u64;
        let total_freq_d = total_freq as f64;
        let rans_precision_d = rans_precision as f64;

        let mut probs: Vec<u32> = Vec::with_capacity(frequencies.len());
        let mut total_rans_prob = 0u32;
        for &freq in &frequencies {
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            probs.push(rans_prob);
            total_rans_prob += rans_prob;
        }
        std::hint::black_box((probs, total_rans_prob));
    }
    let table_create_us = avg_duration_us(start.elapsed(), iterations);

    // Stage D: Full rANS encoding loop (the hot path)
    // Build actual probability table
    use draco_core::rans_symbol_coding::RAnsSymbol;

    let rans_precision: u32 = 1 << 15;
    let total_freq_d = symbols.len() as f64;
    let rans_precision_d = rans_precision as f64;

    let mut prob_table: Vec<RAnsSymbol> = frequencies
        .iter()
        .map(|&freq| {
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            RAnsSymbol {
                prob: rans_prob,
                cum_prob: 0,
            }
        })
        .collect();

    // Normalize and compute cumulative
    let mut total_prob = 0u32;
    for sym in &mut prob_table {
        sym.cum_prob = total_prob;
        total_prob += sym.prob;
    }

    let l_rans_base = rans_precision * 4;

    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf: Vec<u8> = Vec::with_capacity(symbols.len() * 2);

        for &symbol in symbols.iter().rev() {
            let sym = prob_table[symbol as usize];
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                buf.push((state & 0xFF) as u8);
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }

        std::hint::black_box((buf, state));
    }
    let rans_loop_us = avg_duration_us(start.elapsed(), iterations);

    // Now profile the actual encode_symbols call for comparison
    let start = Instant::now();
    for _ in 0..iterations {
        let mut buffer = EncoderBuffer::new();
        let options = SymbolEncodingOptions {
            compression_level: 7,
        };
        encode_symbols(&symbols, num_components, &options, &mut buffer);
        std::hint::black_box(&buffer);
    }
    let full_encode_us = avg_duration_us(start.elapsed(), iterations);

    let total_measured = bit_lengths_us + freq_count_us + table_create_us + rans_loop_us;

    println!(
        "Symbol encoding breakdown (avg over {} iterations, {} symbols):",
        iterations,
        symbols.len()
    );
    println!(
        "  A. Compute bit lengths:   {:7.1} µs ({:5.1}%)",
        bit_lengths_us,
        bit_lengths_us / total_measured * 100.0
    );
    println!(
        "  B. Compute frequencies:   {:7.1} µs ({:5.1}%)",
        freq_count_us,
        freq_count_us / total_measured * 100.0
    );
    println!(
        "  C. rANS table creation:   {:7.1} µs ({:5.1}%)",
        table_create_us,
        table_create_us / total_measured * 100.0
    );
    println!(
        "  D. rANS encoding loop:    {:7.1} µs ({:5.1}%)",
        rans_loop_us,
        rans_loop_us / total_measured * 100.0
    );
    println!("  ─────────────────────────────────────");
    println!("  Isolated total:           {:7.1} µs", total_measured);
    println!("  Full encode_symbols():    {:7.1} µs", full_encode_us);
    println!();
    println!(
        "Overhead in encode_symbols: {:.1} µs ({:.1}%)",
        full_encode_us - total_measured,
        (full_encode_us - total_measured) / full_encode_us * 100.0
    );
}

#[test]
fn profile_rans_loop_micro() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== rANS Loop Micro-benchmark ===\n");

    // Compare different approaches to the rANS encoding loop
    let num_symbols = 30603;
    let max_value = 2047u32;

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let symbols: Vec<u32> = (0..num_symbols)
        .map(|i| {
            let mut h = DefaultHasher::new();
            i.hash(&mut h);
            (h.finish() as u32) % (max_value + 1)
        })
        .collect();

    // Build frequency table
    let mut frequencies = vec![0u64; (max_value + 1) as usize];
    for &s in &symbols {
        frequencies[s as usize] += 1;
    }

    // Build probability table (simplified)
    use draco_core::rans_symbol_coding::RAnsSymbol;
    let rans_precision: u32 = 1 << 15;
    let total_freq_d = symbols.len() as f64;
    let rans_precision_d = rans_precision as f64;

    let mut prob_table: Vec<RAnsSymbol> = frequencies
        .iter()
        .map(|&freq| {
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            RAnsSymbol {
                prob: rans_prob,
                cum_prob: 0,
            }
        })
        .collect();

    let mut total_prob = 0u32;
    for sym in &mut prob_table {
        sym.cum_prob = total_prob;
        total_prob += sym.prob;
    }

    let l_rans_base = rans_precision * 4;
    let iterations = 100;

    // Approach 1: Current Rust implementation (Vec::push)
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf: Vec<u8> = Vec::with_capacity(num_symbols * 2);

        for &symbol in symbols.iter().rev() {
            let sym = prob_table[symbol as usize];
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                buf.push((state & 0xFF) as u8);
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state));
    }
    let vec_push_us = avg_duration_us(start.elapsed(), iterations);

    // Approach 2: Pre-allocated buffer with index
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf = vec![0u8; num_symbols * 2];
        let mut buf_offset = 0usize;

        for &symbol in symbols.iter().rev() {
            let sym = prob_table[symbol as usize];
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                buf[buf_offset] = (state & 0xFF) as u8;
                buf_offset += 1;
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state, buf_offset));
    }
    let prealloc_idx_us = avg_duration_us(start.elapsed(), iterations);

    // Approach 3: Unchecked index access
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf = vec![0u8; num_symbols * 2];
        let mut buf_offset = 0usize;

        for &symbol in symbols.iter().rev() {
            let sym = unsafe { *prob_table.get_unchecked(symbol as usize) };
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                unsafe {
                    *buf.get_unchecked_mut(buf_offset) = (state & 0xFF) as u8;
                }
                buf_offset += 1;
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state, buf_offset));
    }
    let unchecked_us = avg_duration_us(start.elapsed(), iterations);

    // Approach 4: Compute renorm_bound outside with /4 factor
    // l_rans_base / rans_precision = 4, so renorm_bound = 4 * 256 * p = 1024 * p
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf = vec![0u8; num_symbols * 2];
        let mut buf_offset = 0usize;

        for &symbol in symbols.iter().rev() {
            let sym = unsafe { *prob_table.get_unchecked(symbol as usize) };
            let p = sym.prob;
            let renorm_bound = 1024 * p; // Simplified

            while state >= renorm_bound {
                unsafe {
                    *buf.get_unchecked_mut(buf_offset) = (state & 0xFF) as u8;
                }
                buf_offset += 1;
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state, buf_offset));
    }
    let simplified_bound_us = avg_duration_us(start.elapsed(), iterations);

    println!(
        "rANS loop approaches ({} symbols, {} iterations):",
        num_symbols, iterations
    );
    println!("  1. Vec::push:             {:7.1} µs", vec_push_us);
    println!("  2. Pre-alloc + index:     {:7.1} µs", prealloc_idx_us);
    println!("  3. Unchecked access:      {:7.1} µs", unchecked_us);
    println!("  4. Simplified bound:      {:7.1} µs", simplified_bound_us);
    println!();
    println!(
        "Speedup from Vec::push to unchecked: {:.2}x",
        vec_push_us / unchecked_us
    );
}

#[test]
fn profile_full_encode_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Full Encode Breakdown ===\n");

    // Profile the actual full encoding pipeline including connectivity
    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    let iterations = 50;

    // Stage 1: Mesh clone
    let start = Instant::now();
    for _ in 0..iterations {
        let cloned = mesh.clone();
        std::hint::black_box(cloned);
    }
    let mesh_clone_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 2: CornerTable init (connectivity processing)
    use draco_core::corner_table::CornerTable;
    use draco_core::geometry_indices::VertexIndex;

    // Create face data in the format CornerTable expects
    let face_data: Vec<[VertexIndex; 3]> = (0..num_faces)
        .map(|i| {
            let f = mesh.face(FaceIndex(i as u32));
            [
                VertexIndex(f[0].0),
                VertexIndex(f[1].0),
                VertexIndex(f[2].0),
            ]
        })
        .collect();

    let start = Instant::now();
    for _ in 0..iterations {
        let mut ct = CornerTable::new(num_faces);
        ct.init(&face_data);
        std::hint::black_box(&ct);
    }
    let corner_table_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 3: Full Rust encoding
    let start = Instant::now();
    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let _ = encoder.encode(&options, &mut encoder_buffer);
        std::hint::black_box(encoder_buffer);
    }
    let full_encode_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 4: C++ encode for comparison
    let cpp_avg = unsafe {
        let mut output_size = 0usize;
        draco_cpp_test_bridge::draco_benchmark_encode_mesh(
            num_points as u32,
            positions.as_ptr(),
            num_faces as u32,
            faces.as_ptr(),
            10,
            10,
            10,
            iterations as u32,
            &mut output_size as *mut usize,
        ) as f64
    };

    println!(
        "Full encode breakdown ({}x{} mesh, {} iterations):",
        grid_size, grid_size, iterations
    );
    println!("  1. Mesh clone:            {:7.1} µs", mesh_clone_us);
    println!("  2. CornerTable init:      {:7.1} µs", corner_table_us);
    println!("  3. Full Rust encode:      {:7.1} µs", full_encode_us);
    println!("  4. Full C++ encode:       {:7.1} µs", cpp_avg);
    println!();
    println!("C++/Rust speedup: {:.2}x", cpp_avg / full_encode_us);
    println!();
    println!(
        "CornerTable as % of full: {:.1}%",
        corner_table_us / full_encode_us * 100.0
    );
    println!(
        "Mesh clone as % of full: {:.1}%",
        mesh_clone_us / full_encode_us * 100.0
    );

    // Now let's break down CornerTable init
    println!("\nCornerTable init sub-stages:");

    // Stage A: Just corner_to_vertex mapping
    let start = Instant::now();
    for _ in 0..iterations {
        let mut corner_to_vertex =
            vec![draco_core::geometry_indices::INVALID_VERTEX_INDEX; num_faces * 3];
        for (fi, face) in face_data.iter().enumerate() {
            for i in 0..3 {
                corner_to_vertex[fi * 3 + i] = face[i];
            }
        }
        std::hint::black_box(&corner_to_vertex);
    }
    let init_map_us = avg_duration_us(start.elapsed(), iterations);
    println!("  A. Init corner_to_vertex: {:7.1} µs", init_map_us);
    println!(
        "  B. Rest of init:          {:7.1} µs ({:.1}%)",
        corner_table_us - init_map_us,
        (corner_table_us - init_map_us) / corner_table_us * 100.0
    );
}

#[test]
fn profile_mesh_clone_overhead() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Profiling Mesh Clone Overhead ===\n");

    let (mesh, _, _) = create_grid_mesh(200);

    // Time mesh clone
    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        let _cloned = mesh.clone();
    }
    let elapsed = start.elapsed();
    let avg_clone = avg_duration_us(elapsed, iterations) / 1000.0;

    println!("Mesh clone (200x200): {:.3}ms", avg_clone);
}

#[test]
fn profile_point_ids_creation() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Profiling point_ids creation ===\n");

    for num_points in [2500, 10000, 40000] {
        let iterations = 1000;
        let start = Instant::now();
        for _ in 0..iterations {
            let point_ids: Vec<PointIndex> =
                (0..num_points).map(|i| PointIndex(i as u32)).collect();
            std::hint::black_box(&point_ids);
        }
        let elapsed = start.elapsed();
        let avg = avg_duration_us(elapsed, iterations) / 1000.0;

        println!("point_ids creation ({} points): {:.4}ms", num_points, avg);
    }
}

#[test]
fn profile_rust_vs_cpp_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    println!("\n=== Detailed C++ vs Rust Profile Breakdown ===\n");

    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;
    let iterations = 50;

    println!(
        "Grid {}x{}: {} points, {} faces ({} iterations)\n",
        grid_size, grid_size, num_points, num_faces, iterations
    );

    for speed in [0, 5, 10] {
        println!("=== Speed {} ===\n", speed);

        // C++ Profile
        let cpp_profile = draco_cpp_test_bridge::profile_cpp_encode(
            &positions, &faces, speed, speed, 10, iterations,
        )
        .expect("C++ profile failed");

        println!("C++ Breakdown:");
        println!(
            "  Mesh setup:       {:7.1} µs ({:.1}%)",
            cpp_profile.mesh_setup_us as f64,
            cpp_profile.mesh_setup_us as f64 / cpp_profile.total_time_us as f64 * 100.0
        );
        println!(
            "  Encoder setup:    {:7.1} µs ({:.1}%)",
            cpp_profile.encoder_setup_us as f64,
            cpp_profile.encoder_setup_us as f64 / cpp_profile.total_time_us as f64 * 100.0
        );
        println!(
            "  Actual encode:    {:7.1} µs ({:.1}%)",
            cpp_profile.encode_time_us as f64,
            cpp_profile.encode_time_us as f64 / cpp_profile.total_time_us as f64 * 100.0
        );
        println!(
            "  TOTAL:            {:7.1} µs\n",
            cpp_profile.total_time_us as f64
        );

        // Rust Profile
        let mut rust_mesh_setup_us = 0.0;
        let mut rust_encoder_setup_us = 0.0;
        let mut rust_encode_us = 0.0;
        let mut rust_total_us = 0.0;
        let mut rust_output_size = 0;

        for _ in 0..iterations {
            let total_start = Instant::now();

            // Mesh setup (clone since we have pre-built mesh)
            let mesh_start = Instant::now();
            let mesh_copy = mesh.clone();
            let mesh_elapsed = mesh_start.elapsed();

            // Encoder setup
            let encoder_start = Instant::now();
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh_copy);
            let encoder_elapsed = encoder_start.elapsed();

            // Actual encoding
            let encode_start = Instant::now();
            let mut encoder_buffer = EncoderBuffer::new();
            encoder
                .encode(&options, &mut encoder_buffer)
                .expect("Rust encode failed");
            let encode_elapsed = encode_start.elapsed();

            let total_elapsed = total_start.elapsed();

            rust_mesh_setup_us += duration_to_us(mesh_elapsed);
            rust_encoder_setup_us += duration_to_us(encoder_elapsed);
            rust_encode_us += duration_to_us(encode_elapsed);
            rust_total_us += duration_to_us(total_elapsed);
            rust_output_size = encoder_buffer.data().len();
        }

        let rust_mesh_setup = rust_mesh_setup_us / f64::from(iterations);
        let rust_encoder_setup = rust_encoder_setup_us / f64::from(iterations);
        let rust_encode = rust_encode_us / f64::from(iterations);
        let rust_total = rust_total_us / f64::from(iterations);

        println!("Rust Breakdown:");
        println!(
            "  Mesh clone:       {:7.1} µs ({:.1}%)",
            rust_mesh_setup,
            rust_mesh_setup / rust_total * 100.0
        );
        println!(
            "  Encoder setup:    {:7.1} µs ({:.1}%)",
            rust_encoder_setup,
            rust_encoder_setup / rust_total * 100.0
        );
        println!(
            "  Actual encode:    {:7.1} µs ({:.1}%)",
            rust_encode,
            rust_encode / rust_total * 100.0
        );
        println!("  TOTAL:            {:7.1} µs\n", rust_total);

        // Comparison
        println!("Comparison (encode only):");
        println!(
            "  C++ encode:   {:7.1} µs",
            cpp_profile.encode_time_us as f64
        );
        println!("  Rust encode:  {:7.1} µs", rust_encode);
        println!(
            "  Speedup:      {:.2}x {}",
            cpp_profile.encode_time_us as f64 / rust_encode,
            if rust_encode < cpp_profile.encode_time_us as f64 {
                "(Rust faster)"
            } else {
                "(C++ faster)"
            }
        );

        println!("\nComparison (total with mesh setup):");
        println!(
            "  C++ total:    {:7.1} µs",
            cpp_profile.total_time_us as f64
        );
        println!("  Rust total:   {:7.1} µs", rust_total);
        println!(
            "  Speedup:      {:.2}x {}",
            cpp_profile.total_time_us as f64 / rust_total,
            if rust_total < cpp_profile.total_time_us as f64 {
                "(Rust faster)"
            } else {
                "(C++ faster)"
            }
        );

        println!(
            "\nOutput sizes: C++={} Rust={} {}\n",
            cpp_profile.output_size,
            rust_output_size,
            if rust_output_size == cpp_profile.output_size {
                "✓"
            } else {
                "✗ MISMATCH"
            }
        );
        println!("{}\n", "-".repeat(50));
    }
}

#[test]
fn profile_decode_rust_vs_cpp() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;

    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    println!("\n=== Decode Performance: C++ vs Rust ===\n");

    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;
    let iterations = 50;

    println!(
        "Grid {}x{}: {} points, {} faces ({} iterations)\n",
        grid_size, grid_size, num_points, num_faces, iterations
    );

    for speed in [0, 5, 10] {
        println!("=== Speed {} ===\n", speed);

        // First encode to get data to decode
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut encoder_buffer)
            .expect("Encode failed");

        let encoded_data = encoder_buffer.data().to_vec();
        println!("Encoded size: {} bytes\n", encoded_data.len());

        // C++ Decode
        let cpp_result = draco_cpp_test_bridge::profile_cpp_decode(&encoded_data, iterations)
            .expect("C++ decode failed");

        println!("C++ Decode:");
        println!("  Time:       {:7.1} µs", cpp_result.decode_time_us as f64);
        println!("  Points:     {}", cpp_result.num_points);
        println!("  Faces:      {}\n", cpp_result.num_faces);

        // Rust Decode
        let mut rust_decode_us = 0.0;
        let mut rust_num_points = 0;
        let mut rust_num_faces = 0;
        let mut rust_decode_success = true;

        for iter in 0..iterations {
            let mut decoder_buffer = DecoderBuffer::new(&encoded_data);

            let mut out_mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();

            let start = Instant::now();
            match decoder.decode(&mut decoder_buffer, &mut out_mesh) {
                Ok(_) => {
                    rust_decode_us += duration_to_us(start.elapsed());
                    rust_num_points = out_mesh.num_points();
                    rust_num_faces = out_mesh.num_faces();
                }
                Err(e) => {
                    if iter == 0 {
                        println!("Rust Decode: SKIPPED ({})\n", e);
                        rust_decode_success = false;
                        break;
                    }
                }
            }
        }

        if !rust_decode_success {
            println!("{}\n", "-".repeat(50));
            continue;
        }

        let rust_avg = rust_decode_us / f64::from(iterations);

        println!("Rust Decode:");
        println!("  Time:       {:7.1} µs", rust_avg);
        println!("  Points:     {}", rust_num_points);
        println!("  Faces:      {}\n", rust_num_faces);

        // Comparison
        let speedup = cpp_result.decode_time_us as f64 / rust_avg;
        println!("Comparison:");
        println!("  C++:        {:7.1} µs", cpp_result.decode_time_us as f64);
        println!("  Rust:       {:7.1} µs", rust_avg);
        println!(
            "  Speedup:    {:.2}x {}",
            speedup,
            if speedup > 1.0 {
                "(Rust faster)"
            } else {
                "(C++ faster)"
            }
        );

        let points_match = rust_num_points == cpp_result.num_points as usize;
        let faces_match = rust_num_faces == cpp_result.num_faces as usize;
        println!(
            "  Points:     {} vs {} {}",
            cpp_result.num_points,
            rust_num_points,
            if points_match { "✓" } else { "✗" }
        );
        println!(
            "  Faces:      {} vs {} {}",
            cpp_result.num_faces,
            rust_num_faces,
            if faces_match { "✓" } else { "✗" }
        );

        println!("\n{}\n", "-".repeat(50));
    }
}

#[test]
fn profile_decode_sequential_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;

    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    println!("\n=== Sequential Decode Breakdown (Speed 10) ===\n");

    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;
    let iterations = 50;

    println!(
        "Grid {}x{}: {} points, {} faces ({} iterations)\n",
        grid_size, grid_size, num_points, num_faces, iterations
    );

    // Encode at speed 10
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", 10);
    options.set_global_int("decoding_speed", 10);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    let encoded_data = encoder_buffer.data().to_vec();
    println!("Encoded size: {} bytes\n", encoded_data.len());

    // C++ Decode
    let cpp_result = draco_cpp_test_bridge::profile_cpp_decode(&encoded_data, iterations)
        .expect("C++ decode failed");

    println!("C++ Decode:   {:7.1} µs", cpp_result.decode_time_us as f64);

    // Profile Rust decode stages
    let mut total_buffer_init = 0u128;
    let mut total_decode = 0u128;

    for _ in 0..iterations {
        // Buffer creation
        let start = Instant::now();
        let mut decoder_buffer = DecoderBuffer::new(&encoded_data);
        total_buffer_init += start.elapsed().as_nanos();

        let mut out_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        // Decode
        let start = Instant::now();
        decoder
            .decode(&mut decoder_buffer, &mut out_mesh)
            .expect("Decode failed");
        total_decode += start.elapsed().as_nanos();
    }

    let buf_init_us = total_buffer_init as f64 / iterations as f64 / 1000.0;
    let decode_us = total_decode as f64 / iterations as f64 / 1000.0;

    println!("\nRust Breakdown:");
    println!("  Buffer init:    {:7.2} µs", buf_init_us);
    println!("  Decode:       {:7.1} µs", decode_us);
    println!("  TOTAL:        {:7.1} µs", buf_init_us + decode_us);

    let speedup = cpp_result.decode_time_us as f64 / decode_us;
    println!(
        "\nSpeedup: {:.2}x {}",
        speedup,
        if speedup > 1.0 {
            "(Rust faster)"
        } else {
            "(C++ faster)"
        }
    );
}
