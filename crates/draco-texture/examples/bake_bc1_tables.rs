//! Regenerate the baked ETC1S-to-BC1 endpoint tables.
//!
//! `bake_bc1_tables <output-directory>` writes `etc1s_to_bc1_5.bin` and
//! `etc1s_to_bc1_6.bin`. The committed copies live in
//! `crates/draco-texture/src/tables`, and a unit test checks they still match
//! what this produces — so this exists to make the blobs reproducible, not
//! because anything runs it routinely.

use std::path::PathBuf;

use draco_texture::etc1s_to_bc1::bake_bc1_tables;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: bake_bc1_tables <output-directory>")?,
    );
    for bits in [5u32, 6] {
        let table = bake_bc1_tables(bits);
        let path = directory.join(format!("etc1s_to_bc1_{bits}.bin"));
        std::fs::write(&path, &table)?;
        println!("{} {} bytes", path.display(), table.len());
    }
    Ok(())
}
