#[test]
fn test_octahedral_roundtrip() {
    use draco_core::normal_compression_utils::OctahedronToolBox;

    let mut toolbox = OctahedronToolBox::new();
    toolbox.set_quantization_bits(10);

    // Test various normals
    let test_normals: [[f32; 3]; 8] = [
        [1.0, 0.0, 0.0],         // +X
        [-1.0, 0.0, 0.0],        // -X
        [0.0, 1.0, 0.0],         // +Y
        [0.0, -1.0, 0.0],        // -Y
        [0.0, 0.0, 1.0],         // +Z
        [0.0, 0.0, -1.0],        // -Z
        [0.577, 0.577, 0.577],   // diagonal
        [-0.577, 0.577, -0.577], // left hemisphere
    ];

    for normal in &test_normals {
        let (s, t) = toolbox.float_vector_to_quantized_octahedral_coords(normal);
        let decoded = toolbox.quantized_octahedral_coords_to_unit_vector(s, t);

        let dot = normal[0] * decoded[0] + normal[1] * decoded[1] + normal[2] * decoded[2];
        println!(
            "Normal {:?} -> s={}, t={} -> {:?}, dot={}",
            normal, s, t, decoded, dot
        );

        // For 10-bit quantization, we expect some error but should be close
        assert!(dot > 0.95, "Dot product too low: {}", dot);
    }
}
