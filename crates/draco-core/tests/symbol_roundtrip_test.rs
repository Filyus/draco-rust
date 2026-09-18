use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::symbol_encoding::SymbolEncodingOptions;
use draco_core::symbol_encoding::{decode_symbols, encode_symbols};

#[test]
fn test_rans_raw_symbol_roundtrip_small_alphabets_with_zeros() {
    // Small alphabet with a silent zero frequency in the middle: {0:2, 1:0, 2:1}
    let symbols: Vec<u32> = vec![0, 0, 2];
    let num_values = symbols.len();
    let num_components = 1usize;
    let options = SymbolEncodingOptions::default();

    // Encode
    let mut enc_buf = EncoderBuffer::new();
    encode_symbols(&symbols, num_components, &options, &mut enc_buf).unwrap();

    // Debug: show encoded bytes
    eprintln!(
        "Encoded bytes (len={}): {:?}",
        enc_buf.size(),
        enc_buf.data()
    );
    eprintln!(
        "Encoded hex: {}",
        enc_buf
            .data()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ")
    );

    // Decode
    let mut dec_buf = DecoderBuffer::new(enc_buf.data());
    let mut out_symbols = Vec::new();
    decode_symbols(
        num_values,
        num_components,
        &options,
        &mut dec_buf,
        &mut out_symbols,
    )
    .unwrap();

    assert_eq!(
        symbols, out_symbols,
        "Round-trip failed for small alphabet with zero freq"
    );
}

/// A plan is a promise: what the coder would have worked out for itself.
///
/// `encode_symbols_with_plan` exists so a caller that already ranked these
/// symbols does not pay for that twice, and it is only safe while it writes
/// exactly what `encode_symbols` writes. The two decide between tagged and raw
/// separately, and the alphabets below straddle both the estimate's verdict
/// and the bit-length cap above which raw is not weighed at all.
#[test]
fn a_plan_encodes_what_encode_symbols_encodes() {
    use draco_core::symbol_encoding::{encode_symbols_with_plan, plan_symbols};

    let cases: Vec<(&str, usize, Vec<u32>)> = vec![
        ("one symbol", 1, vec![7]),
        (
            "a flat byte alphabet",
            1,
            (0..1000).map(|i| i % 256).collect(),
        ),
        ("one value repeated", 3, vec![5; 999]),
        (
            "skewed, so raw wins",
            1,
            (0..4000)
                .map(|i| if i % 50 == 0 { i % 300 } else { 0 })
                .collect(),
        ),
        (
            "wider than the raw cap, so tagged is forced",
            3,
            (0..3000u32)
                .map(|i| i.wrapping_mul(2654435761) % (1 << 24))
                .collect(),
        ),
        (
            // Cheap to code raw and too wide to be allowed to: the one shape
            // that separates the cap from the estimate.
            "raw would win, but the alphabet is too wide for it",
            1,
            (0..200_000u32)
                .map(|i| if i % 2 == 0 { 0 } else { 500_000 })
                .collect(),
        ),
        ("nothing at all", 2, Vec::new()),
    ];

    for (name, num_components, symbols) in cases {
        let options = SymbolEncodingOptions::default();

        let mut plain = EncoderBuffer::new();
        encode_symbols(&symbols, num_components, &options, &mut plain).unwrap();

        let plan = plan_symbols(&symbols, num_components);
        let mut planned = EncoderBuffer::new();
        encode_symbols_with_plan(&symbols, num_components, &options, &plan, &mut planned).unwrap();

        assert_eq!(
            plain.data(),
            planned.data(),
            "{name}: the plan wrote different bytes than the coder would have"
        );
    }
}
