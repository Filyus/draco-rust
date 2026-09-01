//! Locating the C++ Draco command-line tools the parity tests compare against.
//!
//! The tools are built outside this repository, so every parity test has to
//! find them at run time. Two things make that error-prone and are handled
//! here once rather than in each test:
//!
//! * the executables carry a `.exe` suffix on Windows and none elsewhere, so a
//!   literal file name pins the suite to one platform;
//! * a test that cannot find them and quietly returns reports success, which is
//!   indistinguishable from a comparison that ran and agreed. Set
//!   `DRACO_REQUIRE_CPP_TOOLS` to turn that silence into a failure — CI does,
//!   so a job that exists to run these comparisons cannot pass without them.
//!
//! `DRACO_CPP_BUILD_DIR` names the build; `DRACO_CPP_DECODER` and
//! `DRACO_CPP_ENCODER` override an individual tool. Without either, the
//! well-known build directories beside the repository are probed.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const DECODER: &str = "draco_decoder";
pub const ENCODER: &str = "draco_encoder";

pub const BUILD_HINT: &str = "\
C++ Draco tools are required for this comparison. Point DRACO_CPP_BUILD_DIR at a
build of them, or build them next to this repository:
  cmake -S . -B build && cmake --build build --config Release \
--target draco_decoder draco_encoder";

/// The file name a Draco tool has on this platform.
pub fn tool_file_name(tool: &str) -> String {
    if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    }
}

/// The per-tool environment override, if this tool has one set.
fn tool_env_var(tool: &str) -> &'static str {
    match tool {
        DECODER => "DRACO_CPP_DECODER",
        ENCODER => "DRACO_CPP_ENCODER",
        _ => "",
    }
}

/// Probe one build directory, covering both CMake layouts in the wild: a
/// single-configuration build puts the tools at its root, a multi-configuration
/// one puts them under the configuration name, and older Draco nested both
/// under `src/draco`.
fn tool_in_build_dir(build_dir: &Path, tool: &str) -> Option<PathBuf> {
    let file_name = tool_file_name(tool);
    let roots = [build_dir.to_path_buf(), build_dir.join("src").join("draco")];

    for root in roots {
        let direct = root.join(&file_name);
        if direct.exists() {
            return Some(direct);
        }
        for config in ["Release", "Debug"] {
            let configured = root.join(config).join(&file_name);
            if configured.exists() {
                return Some(configured);
            }
        }
    }

    None
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates directory")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

/// Locate a C++ Draco tool, or `None` if this machine has no build of it.
pub fn find_cpp_tool(tool: &str) -> Option<PathBuf> {
    let env_var = tool_env_var(tool);
    if !env_var.is_empty() {
        if let Ok(path) = std::env::var(env_var) {
            let path = PathBuf::from(path);
            assert!(
                path.exists(),
                "{env_var} points at a missing {tool}: {}\n{BUILD_HINT}",
                path.display()
            );
            return Some(path);
        }
    }

    if let Ok(build_dir) = std::env::var("DRACO_CPP_BUILD_DIR") {
        if let Some(path) = tool_in_build_dir(Path::new(&build_dir), tool) {
            return Some(path);
        }
    }

    let root = repo_root();
    ["build-original", "build"]
        .into_iter()
        .find_map(|dir| tool_in_build_dir(&root.join(dir), tool))
}

/// True when a missing tool must fail the run rather than skip it.
pub fn cpp_tools_required() -> bool {
    std::env::var_os("DRACO_REQUIRE_CPP_TOOLS").is_some()
}

/// Locate a tool a test cannot run without.
pub fn require_cpp_tool(tool: &str) -> PathBuf {
    find_cpp_tool(tool).unwrap_or_else(|| panic!("Could not find {tool}.\n{BUILD_HINT}"))
}

/// Locate a tool, reporting the skip when the machine has no build of it.
///
/// Returns `None` only where a skip is allowed; under `DRACO_REQUIRE_CPP_TOOLS`
/// the absence is a failure instead.
pub fn optional_cpp_tool(tool: &str) -> Option<PathBuf> {
    if let Some(path) = find_cpp_tool(tool) {
        return Some(path);
    }
    assert!(
        !cpp_tools_required(),
        "DRACO_REQUIRE_CPP_TOOLS is set and {tool} was not found.\n{BUILD_HINT}"
    );
    eprintln!("Skipping: {tool} not found. {BUILD_HINT}");
    None
}

/// Locate both tools, or report a skip if either is missing.
pub fn optional_cpp_codec() -> Option<(PathBuf, PathBuf)> {
    let encoder = optional_cpp_tool(ENCODER)?;
    let decoder = optional_cpp_tool(DECODER)?;
    Some((encoder, decoder))
}
