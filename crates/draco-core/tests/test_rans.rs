use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::rans_bit_decoder::RAnsBitDecoder;
use draco_core::rans_bit_encoder::RAnsBitEncoder;
use draco_core::rans_symbol_decoder::RAnsSymbolDecoder;
use draco_core::rans_symbol_encoder::RAnsSymbolEncoder;

#[test]
fn test_rans_bit_coding() {
    let mut encoder = RAnsBitEncoder::new();
    let mut buffer = EncoderBuffer::new();

    encoder.start_encoding();
    encoder.encode_bit(true);
    encoder.encode_bit(false);
    encoder.encode_bit(true);
    encoder.end_encoding(&mut buffer);

    let data = buffer.data();
    println!("Encoded data: {:?}", data);

    let mut decoder_buffer = DecoderBuffer::new(data);
    let mut decoder = RAnsBitDecoder::new();
    assert!(decoder.start_decoding(&mut decoder_buffer));

    assert!(decoder.decode_next_bit());
    assert!(!decoder.decode_next_bit());
    assert!(decoder.decode_next_bit());

    decoder.end_decoding();
}

/// Test that num_symbols==1 correctly consumes the probability table byte.
/// This is a regression test for a bug where the decoder early-returned without
/// reading the probability byte, causing buffer misalignment.
#[test]
fn test_rans_symbol_single_symbol_consumes_probability_byte() {
    // Encode with a single symbol (all values are 0)
    let mut encoder: RAnsSymbolEncoder<12> = RAnsSymbolEncoder::new();
    let mut enc_buffer = EncoderBuffer::new();

    // Frequency table with just one symbol
    let frequencies = [10u64];
    assert!(encoder.create(&frequencies, 1, &mut enc_buffer));

    encoder.start_encoding(&mut enc_buffer);
    // Encode 5 symbols (all symbol 0)
    for _ in 0..5 {
        encoder.encode_symbol(0);
    }
    encoder.end_encoding(&mut enc_buffer);

    // Add a sentinel byte after the rANS data to verify buffer position
    enc_buffer.encode_u8(0xAB);

    let data = enc_buffer.data();

    // Decode and verify buffer position is correct
    let mut dec_buffer = DecoderBuffer::new(data);
    dec_buffer.set_version(2, 2); // v2.2

    let mut decoder = RAnsSymbolDecoder::new(12);
    assert!(decoder.create(&mut dec_buffer), "Failed to create decoder");
    assert!(
        decoder.start_decoding(&mut dec_buffer),
        "Failed to start decoding"
    );

    // Decode 5 symbols
    for _ in 0..5 {
        let sym = decoder.decode_symbol();
        assert_eq!(sym, 0, "Expected symbol 0");
    }

    // Verify we can read the sentinel byte (buffer position is correct)
    let sentinel = dec_buffer
        .decode_u8()
        .expect("Failed to read sentinel byte");
    assert_eq!(
        sentinel, 0xAB,
        "Sentinel byte mismatch - buffer position incorrect"
    );

    // Verify we consumed all data
    assert_eq!(
        dec_buffer.remaining_data().len(),
        0,
        "Buffer should be fully consumed"
    );
}

/// Test that num_symbols==0 doesn't try to read anything except the count.
#[test]
fn test_rans_symbol_zero_symbols() {
    let mut enc_buffer = EncoderBuffer::new();

    // Manually encode num_symbols=0 as varint (single byte 0)
    enc_buffer.encode_varint(0u64);
    // Add a size prefix (0 bytes of rANS data)
    enc_buffer.encode_varint(0u64);
    // Add sentinel
    enc_buffer.encode_u8(0xCD);

    let data = enc_buffer.data();
    let mut dec_buffer = DecoderBuffer::new(data);
    dec_buffer.set_version(2, 2);

    let mut decoder = RAnsSymbolDecoder::new(12);
    assert!(
        decoder.create(&mut dec_buffer),
        "Failed to create decoder for 0 symbols"
    );
    assert!(
        decoder.start_decoding(&mut dec_buffer),
        "Failed to start decoding for 0 symbols"
    );

    // Decoding should return 0 for any symbol request
    assert_eq!(decoder.decode_symbol(), 0);

    // Verify sentinel is readable
    let sentinel = dec_buffer.decode_u8().expect("Failed to read sentinel");
    assert_eq!(sentinel, 0xCD);
}

/// Test that malformed probability tables (sum > precision) are rejected without panic.
#[test]
fn test_rans_symbol_malformed_probability_table_rejected() {
    // Construct raw bytes for a malformed probability table:
    // For 12-bit precision (4096 total), we encode 2 symbols each with prob 3000 (sum = 6000 > 4096)
    // This should fail during LUT construction.
    //
    // Encoding format:
    // - varint for num_symbols = 2 (single byte: 0x02)
    // - For prob=3000, mode=1 (needs 1 extra byte since 3000 >= 64 and < 16384)
    //   byte0 = ((3000 & 0x3F) << 2) | 1 = ((56) << 2) | 1 = 225
    //   byte1 = (3000 >> 6) = 46
    //   Decoding verification: prob = (225 >> 2) = 56, then prob |= 46 << 6 = 56 | 2944 = 3000 ✓

    let malformed_data: Vec<u8> = vec![
        0x02, // num_symbols = 2 (varint)
        225,  // symbol 0: prob=3000, mode=1
        46,   // symbol 0: extra byte
        225,  // symbol 1: prob=3000, mode=1
        46,   // symbol 1: extra byte
    ];

    let mut dec_buffer = DecoderBuffer::new(&malformed_data);
    dec_buffer.set_version(2, 2);

    let mut decoder = RAnsSymbolDecoder::new(12);
    // This should fail because probabilities sum to 6000 > 4096
    assert!(
        !decoder.create(&mut dec_buffer),
        "Should reject probability table where sum exceeds precision"
    );
}

/// Test backward compatibility with pre-v2.0 size prefix encoding (u32 instead of varint).
#[test]
fn test_rans_symbol_pre_v2_backward_compat() {
    // Pre-v2.0 (C++ Draco) format:
    //   - num_symbols is a fixed u32 (little-endian)
    //   - rANS byte-count is a fixed u64 (little-endian)
    let mut enc_buffer = EncoderBuffer::new();
    enc_buffer.set_version(1, 9); // Pre-v2.0

    // num_symbols = 1 as u32 LE
    enc_buffer.encode_u32(1);
    // Probability for single symbol = 4096 (full precision for 12-bit)
    // 4096 >= 64 and < 16384, so mode=1
    // byte0 = ((4096 & 0x3F) << 2) | 1 = (0 << 2) | 1 = 1
    // byte1 = 4096 >> 6 = 64
    enc_buffer.encode_u8(1); // prob=4096 low 6 bits = 0, mode=1
    enc_buffer.encode_u8(64); // prob >> 6 = 64

    // Size prefix as u64 (pre-v2.0 uses fixed 8-byte size)
    enc_buffer.encode_u64(0u64); // 0 bytes of rANS data for single symbol

    // Sentinel
    enc_buffer.encode_u8(0xEF);

    let data = enc_buffer.data();
    let mut dec_buffer = DecoderBuffer::new(data);
    dec_buffer.set_version(1, 9); // Pre-v2.0

    let mut decoder = RAnsSymbolDecoder::new(12);
    assert!(
        decoder.create(&mut dec_buffer),
        "Failed to create decoder for pre-v2.0"
    );
    assert!(
        decoder.start_decoding(&mut dec_buffer),
        "Failed to start decoding for pre-v2.0"
    );

    // Single symbol always returns 0
    assert_eq!(decoder.decode_symbol(), 0);

    // Verify sentinel
    let sentinel = dec_buffer.decode_u8().expect("Failed to read sentinel");
    assert_eq!(
        sentinel, 0xEF,
        "Pre-v2.0 backward compat failed - buffer position wrong"
    );
}
