#[test]
fn test_octahedral_encoding_detailed() {
    use draco_core::normal_compression_utils::OctahedronToolBox;

    let mut toolbox = OctahedronToolBox::new();
    toolbox.set_quantization_bits(10);

    // The test normals - same as the test
    let normals: [[f32; 3]; 3] = [
        [1.0, 0.0, 0.0],  // +X
        [-1.0, 0.0, 0.0], // -X
        [0.0, 1.0, 0.0],  // +Y
    ];

    println!("Quantization: 10 bits");
    println!("max_value = {}", toolbox.max_value());
    println!("max_quantized_value = {}", toolbox.max_quantized_value());
    println!("center_value = {}", toolbox.center_value());
    println!(
        "dequantization_scale = {}",
        2.0 / toolbox.max_value() as f32
    );

    for normal in &normals {
        let (s, t) = toolbox.float_vector_to_quantized_octahedral_coords(normal);
        let decoded = toolbox.quantized_octahedral_coords_to_unit_vector(s, t);

        println!();
        println!("Input normal: {:?}", normal);
        println!("Encoded (s,t): ({}, {})", s, t);
        println!("Decoded normal: {:?}", decoded);

        let dot = normal[0] * decoded[0] + normal[1] * decoded[1] + normal[2] * decoded[2];
        println!("Dot product: {}", dot);
    }
}
