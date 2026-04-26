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
    assert!(encode_symbols(
        &symbols,
        num_components,
        &options,
        &mut enc_buf
    ));

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
    let mut out_symbols = vec![0u32; num_values];
    assert!(decode_symbols(
        num_values,
        num_components,
        &options,
        &mut dec_buf,
        &mut out_symbols
    ));

    assert_eq!(
        symbols, out_symbols,
        "Round-trip failed for small alphabet with zero freq"
    );
}
