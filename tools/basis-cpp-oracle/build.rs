//! Compile the vendored reference transcoder.
//!
//! The configuration is deliberate and is the point of this crate: it is the
//! one an emscripten build gets, which is what `draco-texture` implements and
//! what a browser runs. `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY` at 0 is the
//! whole reason the ETC1S-to-ASTC comparison against the `basisu` crate has to
//! be skipped; here it can be set either way and the answer checked.
//!
//! Formats nothing here targets are switched off rather than vendored: ATC,
//! PVRTC, FXT1 and the higher-quality ASTC tables together are about 1.2 MB of
//! `.inc` that would be compiled and discarded.

fn main() {
    println!("cargo:rerun-if-changed=csrc");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("csrc/oracle.cpp")
        .file("csrc/basisu_transcoder.cpp")
        .include("csrc")
        // Read KTX2, not .basis. Zstd is undone on the Rust side before the
        // bytes get here, so no zstd library is needed or vendored.
        .define("BASISD_SUPPORT_KTX2", "1")
        .define("BASISD_SUPPORT_KTX2_ZSTD", "0")
        // The targets this crate is an oracle for.
        .define("BASISD_SUPPORT_DXT1", "1")
        .define("BASISD_SUPPORT_DXT5A", "1")
        .define("BASISD_SUPPORT_BC7_MODE5", "1")
        .define("BASISD_SUPPORT_ETC2_EAC_A8", "1")
        .define("BASISD_SUPPORT_ETC2_EAC_RG11", "1")
        .define("BASISD_SUPPORT_ASTC", "1")
        .define("BASISD_SUPPORT_UASTC", "1")
        // The emscripten profile, which is what a browser runs and what this
        // repository's transcoder implements.
        .define("BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY", "0")
        // Formats nothing here reaches, so their tables are not vendored.
        .define("BASISD_SUPPORT_ATC", "0")
        .define("BASISD_SUPPORT_PVRTC1", "0")
        .define("BASISD_SUPPORT_PVRTC2", "0")
        .define("BASISD_SUPPORT_FXT1", "0")
        .warnings(false);

    build.compile("basis_oracle");
}
