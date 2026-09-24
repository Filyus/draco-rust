//! Compile C zstd's decompressor from `ZSTD_SOURCE_DIR`, when it is set.
//!
//! The source is not vendored: it is only the reference to time against, and a
//! checkout of `facebook/zstd` is a path of the machine's, so it comes from the
//! environment. Without it the harness still runs and times `draco-texture`
//! alone.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(c_zstd)");
    println!("cargo::rerun-if-env-changed=ZSTD_SOURCE_DIR");
    let Some(source) = std::env::var_os("ZSTD_SOURCE_DIR") else {
        println!(
            "cargo::warning=ZSTD_SOURCE_DIR is not set, so there is no C zstd to compare against"
        );
        return;
    };
    let lib = PathBuf::from(source).join("lib");
    let mut build = cc::Build::new();
    for file in [
        "common/debug.c",
        "common/entropy_common.c",
        "common/error_private.c",
        "common/fse_decompress.c",
        "common/pool.c",
        "common/threading.c",
        "common/xxhash.c",
        "common/zstd_common.c",
        "decompress/huf_decompress.c",
        "decompress/zstd_ddict.c",
        "decompress/zstd_decompress.c",
        "decompress/zstd_decompress_block.c",
    ] {
        build.file(lib.join(file));
    }
    build
        .include(&lib)
        .include(lib.join("common"))
        // MSVC cannot assemble the AT&T-syntax Huffman loops, and Windows
        // builds of libzstd go without them; leaving them out everywhere keeps
        // one configuration on every machine.
        .define("ZSTD_DISABLE_ASM", "1")
        .opt_level(3)
        .compile("zstd_decompress");
    println!("cargo::rustc-cfg=c_zstd");
}
