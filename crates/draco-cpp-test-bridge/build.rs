use std::env;
use std::path::PathBuf;

fn main() {
    // Skip building on docs.rs
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    // Inform cargo's cfg checker about our conditional `cpp_test_bridge_disabled` cfg so
    // the `check-cfg` lint does not warn about it being unexpected.
    println!("cargo:rustc-check-cfg=cfg(cpp_test_bridge_disabled)");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();

    // Paths to the original Draco checkout and pre-built C++ libraries. These
    // can live outside the Rust workspace for local interop tests.
    let draco_src = env::var_os("DRACO_CPP_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("src"));
    let draco_build = env::var_os("DRACO_CPP_BUILD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root.join("build-original"));
    println!("cargo:rerun-if-env-changed=DRACO_CPP_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=DRACO_CPP_BUILD_DIR");

    // Two CMake layouts are in the wild. Draco used to place its libraries
    // under `src/draco/`; current upstream puts them at the build root. Probe
    // for the one that exists rather than requiring a particular Draco vintage,
    // so a checkout can be pointed at either.
    let (lib_path, component_root) = {
        let nested = draco_build.join("src/draco/Release");
        if nested.exists() {
            (nested, draco_build.join("src/draco"))
        } else {
            (draco_build.join("Release"), draco_build.clone())
        }
    };
    if !lib_path.exists() {
        println!(
            "cargo:warning=C++ Draco library not found at {:?}. C++ test bridge will be disabled.",
            lib_path
        );
        println!("cargo:rustc-cfg=cpp_test_bridge_disabled");
        return;
    }

    // Compile the private C++ test bridge.
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("cpp/test_bridge.cpp")
        .include(&draco_src)
        // The build dir has draco_features.h at build-original/draco/draco_features.h
        // but headers include it as "draco/draco_features.h", so we include the parent
        .include(&draco_build)
        .flag_if_supported("/std:c++17")
        .flag_if_supported("-std=c++17")
        .opt_level(3);

    // Windows-specific settings
    if cfg!(target_os = "windows") {
        build.flag("/EHsc");
    }

    // Opt-in allocation counting for the C++ side. Compiled in only when the
    // env var is set at build time: the override costs an atomic per
    // new/delete, so timing binaries must be built without it.
    println!("cargo:rerun-if-env-changed=DRACO_BRIDGE_COUNT_ALLOCS");
    if env::var_os("DRACO_BRIDGE_COUNT_ALLOCS").is_some() {
        build.define("BRIDGE_COUNT_ALLOCS", None);
    }

    build.compile("cpp_test_bridge_wrapper");
    // Ensure the compiled wrapper is linked into all test binaries
    println!("cargo:rustc-link-lib=static=cpp_test_bridge_wrapper");

    // Link to the pre-built Draco library
    println!("cargo:rustc-link-search=native={}", lib_path.display());
    println!("cargo:rustc-link-lib=static=draco");

    // Link all the component libraries (Draco builds as multiple static libs)
    let component_libs = [
        "draco_animation",
        "draco_animation_dec",
        "draco_animation_enc",
        "draco_attributes",
        "draco_compression_attributes_dec",
        "draco_compression_attributes_enc",
        "draco_compression_attributes_pred_schemes_enc",
        "draco_compression_bit_coders",
        "draco_compression_decode",
        "draco_compression_encode",
        "draco_compression_entropy",
        "draco_compression_mesh_dec",
        "draco_compression_mesh_enc",
        "draco_compression_point_cloud_dec",
        "draco_compression_point_cloud_enc",
        "draco_core",
        "draco_mesh",
        "draco_metadata",
        "draco_metadata_dec",
        "draco_metadata_enc",
        "draco_points_dec",
        "draco_points_enc",
        "draco_point_cloud",
        // Named `draco_src_io` in older layouts, `draco_io` in current
        // upstream; `draco_compression_options` only exists in the latter.
        // Missing entries are skipped below, so listing both is harmless.
        "draco_src_io",
        "draco_io",
        "draco_compression_options",
    ];

    for lib_name in component_libs {
        let lib_dir = component_root.join(format!("{}.dir/Release", lib_name));
        if lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", lib_dir.display());
            println!("cargo:rustc-link-lib=static={}", lib_name);
        }
    }

    // Link C++ standard library
    if cfg!(target_os = "windows") {
        // MSVC links automatically
    } else if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }

    println!("cargo:rerun-if-changed=cpp/test_bridge.cpp");
}
