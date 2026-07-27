//! Decode every mip level of a KTX2 file and write them as raw RGBA.
//!
//! A debugging aid for comparing against another transcoder byte for byte:
//! `dump_rgba <input.ktx2> <output-prefix>` writes `<prefix>.<level>.rgba`.

use std::path::PathBuf;

use draco_texture::ktx2::Ktx2;
use draco_texture::transcode::Transcoder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("usage: dump_rgba <input.ktx2> <prefix>")?,
    );
    let prefix = args
        .next()
        .ok_or("usage: dump_rgba <input.ktx2> <prefix>")?;

    let bytes = std::fs::read(&input)?;
    let file = Ktx2::parse(&bytes)?;
    let transcoder = Transcoder::new(&file)?;

    for level in 0..file.level_count() {
        let image = transcoder.decode_rgba(&file, level, 0, 0)?;
        let path = format!("{prefix}.{level}.rgba");
        std::fs::write(&path, &image.rgba)?;
        println!(
            "{path} {}x{} {} bytes",
            image.width,
            image.height,
            image.rgba.len()
        );
    }
    Ok(())
}
