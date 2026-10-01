//! One raw rANS stream of geometric residuals with a few outliers across
//! `[0, 2^bits)`: the scale sets how much of the mass sits in symbols that own
//! whole buckets, the outliers' width sets the coder's precision. Prints the
//! best decode time per symbol and the stream's size.
//!
//! usage: rbench <scale> <bits> [count] [rounds]
use draco_core::symbol_encoding::{decode_symbols, encode_symbols, SymbolEncodingOptions};
use draco_core::{DecoderBuffer, EncoderBuffer};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scale: f64 = args[1].parse().unwrap();
    let bits: u32 = args[2].parse().unwrap();
    let count: usize = args.get(3).map_or(4_000_000, |v| v.parse().unwrap());
    let rounds: usize = args.get(4).map_or(7, |v| v.parse().unwrap());
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let top = (1u64 << bits) - 1;
    let symbols: Vec<u32> = (0..count)
        .map(|_| {
            let u = ((next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
            if next() % 200 == 0 {
                (next() % (top + 1)) as u32
            } else {
                ((-u.ln() * scale) as u64).min(top) as u32
            }
        })
        .collect();
    let options = SymbolEncodingOptions::default();
    let mut encoded = EncoderBuffer::new();
    encode_symbols(&symbols, 1, &options, &mut encoded).expect("encodes");
    let bytes = encoded.data().to_vec();
    let mut best = f64::MAX;
    let mut out = Vec::new();
    for _ in 0..rounds {
        let mut buffer = DecoderBuffer::new(&bytes);
        let started = Instant::now();
        decode_symbols(count, 1, &options, &mut buffer, &mut out).expect("decodes");
        best = best.min(started.elapsed().as_secs_f64());
    }
    assert!(out == symbols, "decoded what was encoded");
    println!(
        "scale {scale} bits {bits}: {:.2} ns/symbol, {} bytes",
        best * 1e9 / count as f64,
        bytes.len()
    );
}
