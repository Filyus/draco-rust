#[test]
fn test_normal_octahedron_transform_encoding() {
    use draco_core::prediction_scheme::PredictionSchemeEncodingTransform;
    use draco_core::prediction_scheme_normal_octahedron_canonicalized_encoding_transform::PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform;

    // 10-bit quantization: max_quantized_value = 1023
    let transform = PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform::new(1023);

    // Test the octahedral coords for the 3 normals:
    // Normal (1,0,0) -> (511, 511) - center of diamond
    // Normal (-1,0,0) -> (1022, 1022) - corner
    // Normal (0,1,0) -> (1022, 511) - edge

    // Simulate what Delta encoder does:
    // Process backwards, each element is predicted by the previous

    // For point 2: orig=(1022, 511), pred=(1022, 1022)
    let orig2 = [1022, 511];
    let pred2 = [1022, 1022];
    let mut corr2 = [0i32; 2];
    transform.compute_correction(&orig2, &pred2, &mut corr2);
    println!(
        "Point 2: orig={:?}, pred={:?}, corr={:?}",
        orig2, pred2, corr2
    );

    // For point 1: orig=(1022, 1022), pred=(511, 511)
    let orig1 = [1022, 1022];
    let pred1 = [511, 511];
    let mut corr1 = [0i32; 2];
    transform.compute_correction(&orig1, &pred1, &mut corr1);
    println!(
        "Point 1: orig={:?}, pred={:?}, corr={:?}",
        orig1, pred1, corr1
    );

    // For point 0: orig=(511, 511), pred=(0, 0)
    let orig0 = [511, 511];
    let pred0 = [0, 0];
    let mut corr0 = [0i32; 2];
    transform.compute_correction(&orig0, &pred0, &mut corr0);
    println!(
        "Point 0: orig={:?}, pred={:?}, corr={:?}",
        orig0, pred0, corr0
    );
}

#[test]
fn test_normal_octahedron_transform_decoding() {
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::prediction_scheme::PredictionSchemeDecodingTransform;
    use draco_core::prediction_scheme_normal_octahedron_canonicalized_decoding_transform::PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform;

    // Simulate decoding what we encoded:
    // Encoded corrections (from above):
    // Point 0: corr=(511, 511)
    // Point 1: corr=(511, 511)
    // Point 2: corr=(511, 0)

    let mut transform = PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new();

    // Decode transform data (max_quantized_value=1023, center_value=511)
    let mut buffer = DecoderBuffer::new(&[
        0xFF, 0x03, 0x00, 0x00, // 1023 as i32
        0xFF, 0x01, 0x00, 0x00, // 511 as i32
    ]);
    assert!(transform.decode_transform_data(&mut buffer));

    // Decode point 0: pred=(0,0), corr=(511, 511)
    let pred0 = [0, 0];
    let corr0 = [511, 511];
    let mut out0 = [0i32; 2];
    transform.compute_original_value(&pred0, &corr0, &mut out0);
    println!(
        "Decoded Point 0: pred={:?}, corr={:?}, out={:?}",
        pred0, corr0, out0
    );

    // Decode point 1: pred=(decoded point 0), corr=(511, 511)
    let pred1 = out0;
    let corr1 = [511, 511];
    let mut out1 = [0i32; 2];
    transform.compute_original_value(&pred1, &corr1, &mut out1);
    println!(
        "Decoded Point 1: pred={:?}, corr={:?}, out={:?}",
        pred1, corr1, out1
    );

    // Decode point 2: pred=(decoded point 1), corr=(511, 0)
    let pred2 = out1;
    let corr2 = [511, 0];
    let mut out2 = [0i32; 2];
    transform.compute_original_value(&pred2, &corr2, &mut out2);
    println!(
        "Decoded Point 2: pred={:?}, corr={:?}, out={:?}",
        pred2, corr2, out2
    );

    // Expected:
    // Point 0: (511, 511) -> normal (1, 0, 0)
    // Point 1: (1022, 1022) -> normal (-1, 0, 0)
    // Point 2: (1022, 511) -> normal (0, 1, 0)
    assert_eq!(out0, [511, 511], "Point 0 should be (511, 511)");
    assert_eq!(out1, [1022, 1022], "Point 1 should be (1022, 1022)");
    assert_eq!(out2, [1022, 511], "Point 2 should be (1022, 511)");
}
