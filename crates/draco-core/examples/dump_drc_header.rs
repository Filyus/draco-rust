use std::fs::File;
use std::io::Read;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("Usage: dump_drc_header <path>");
    let mut f = File::open(&path).expect("Failed to open file");
    let mut buf = [0u8; 32];
    let n = f.read(&mut buf).expect("Failed to read");
    println!("Read {} bytes", n);
    println!(
        "Header hex: {}",
        buf[..n]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ")
    );
    // Check for DRACO magic
    if n >= 5 && &buf[..5] == b"DRACO" {
        println!("DRACO magic found");
        println!("Major: {} Minor: {}", buf[5], buf[6]);
    } else {
        println!("DRACO magic not found in first 5 bytes");
    }
}
