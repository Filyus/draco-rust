//! `--npm`: the `@draco-rust/*` packages.
//!
//! A package is split by what a consumer needs rather than by file format, and
//! each of its entries is a module of its own built with its own features, so a
//! bundler takes only the wasm an application imports. The packages' manifests
//! and READMEs are tracked in `web/npm/<package>/`; this builds the entries
//! next to copies of them in `web/npm/dist/<package>/`, ready for `npm pack` or
//! `npm publish`.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use super::{
    find_wasm_opt, measure_wasm_size, run_command, run_command_with_env, unique_suffix,
    wasm_opt_version, Config, WASM_OPT_ARGS, WASM_OPT_VERSION,
};

struct Entry {
    /// The export path, `.` for the package root.
    path: &'static str,
    features: &'static [&'static str],
}

struct Package {
    name: &'static str,
    module: &'static str,
    entries: &'static [Entry],
}

const PACKAGES: &[Package] = &[
    Package {
        name: "decoder",
        module: "drc-wasm",
        entries: &[
            Entry {
                path: ".",
                features: &["read", "point-cloud-decode"],
            },
            Entry {
                path: "./mesh",
                features: &["read"],
            },
            Entry {
                path: "./point-cloud",
                features: &["read", "point-cloud-only"],
            },
            Entry {
                path: "./legacy",
                features: &["read", "point-cloud-decode", "legacy-bitstream-decode"],
            },
        ],
    },
    Package {
        name: "encoder",
        module: "drc-wasm",
        entries: &[Entry {
            path: ".",
            features: &["write"],
        }],
    },
    Package {
        name: "gltf",
        module: "gltf-wasm",
        entries: &[
            Entry {
                path: ".",
                features: &["read", "draco-decode", "accessors", "raw-resources"],
            },
            Entry {
                path: "./validate",
                features: &[
                    "read",
                    "draco-decode",
                    "accessors",
                    "raw-resources",
                    "strict-validation",
                ],
            },
            Entry {
                path: "./writer",
                features: &["draco-encode", "accessors", "raw-resources"],
            },
        ],
    },
    Package {
        name: "obj",
        module: "obj-wasm",
        entries: &[Entry {
            path: ".",
            features: &["read", "write"],
        }],
    },
    Package {
        name: "ply",
        module: "ply-wasm",
        entries: &[Entry {
            path: ".",
            features: &["read", "write"],
        }],
    },
    Package {
        name: "stl",
        module: "stl-wasm",
        entries: &[Entry {
            path: ".",
            features: &["read", "write"],
        }],
    },
    Package {
        name: "fbx",
        module: "fbx-wasm",
        entries: &[Entry {
            path: ".",
            features: &["read", "write"],
        }],
    },
];

/// The files wasm-bindgen writes for an entry built with `--out-name index`.
const ENTRY_FILES: &[&str] = &[
    "index.js",
    "index.d.ts",
    "index_bg.wasm",
    "index_bg.wasm.d.ts",
];

pub(crate) fn build_packages(config: &Config, only: &[String]) -> Result<(), String> {
    let wasm_opt = find_wasm_opt().ok_or("--npm needs wasm-opt; point WASM_OPT at one")?;
    let version = wasm_opt_version(&wasm_opt);
    if version.as_deref() != Some(WASM_OPT_VERSION) {
        return Err(format!(
            "--npm builds what is published, so it needs wasm-opt {WASM_OPT_VERSION} and \
             found {}; point WASM_OPT at a Binaryen {WASM_OPT_VERSION} wasm-opt",
            version.as_deref().unwrap_or("none")
        ));
    }
    let packages: Vec<&Package> = PACKAGES
        .iter()
        .filter(|package| only.is_empty() || only.iter().any(|name| name == package.name))
        .collect();
    if packages.is_empty() {
        let known: Vec<_> = PACKAGES.iter().map(|package| package.name).collect();
        return Err(format!("no such package; known: {}", known.join(", ")));
    }

    let npm_dir = config.web_dir.join("npm");
    let repository = config
        .web_dir
        .parent()
        .ok_or("web/ has no parent directory")?;
    let license = repository.join("LICENSE");
    let version = read_version(&npm_dir.join("VERSION"))?;
    let provenance = Provenance::of(repository)?;
    println!(
        "Version {version}, from draco-rust {} ({})",
        provenance.commit,
        provenance.crates_text()
    );
    println!(
        "Building npm packages into {}",
        npm_dir.join("dist").display()
    );
    for package in packages {
        let template = npm_dir.join(package.name);
        let manifest = fs::read_to_string(template.join("package.json"))
            .map_err(|error| format!("{}: {error}", template.join("package.json").display()))?;
        for entry in package.entries {
            if !manifest.contains(&format!("\"{}\":", entry.path)) {
                return Err(format!(
                    "@draco-rust/{}: package.json does not export {}",
                    package.name, entry.path
                ));
            }
        }

        let dist = npm_dir.join("dist").join(package.name);
        if dist.exists() {
            fs::remove_dir_all(&dist)
                .map_err(|error| format!("failed to clear {}: {error}", dist.display()))?;
        }
        fs::create_dir_all(&dist)
            .map_err(|error| format!("failed to create {}: {error}", dist.display()))?;
        write(
            &dist.join("package.json"),
            &stamp_manifest(&manifest, &version, &provenance)
                .map_err(|error| format!("@draco-rust/{}: {error}", package.name))?,
        )?;
        let readme = fs::read_to_string(template.join("README.md"))
            .map_err(|error| format!("{}: {error}", template.join("README.md").display()))?;
        write(
            &dist.join("README.md"),
            &format!(
                "{}\n## This build\n\n`@draco-rust/{}@{version}` was built from draco-rust \
                 commit `{}`, with {}.\n",
                readme.trim_end(),
                package.name,
                provenance.commit,
                provenance.crates_text()
            ),
        )?;
        copy(&license, &dist.join("LICENSE"))?;

        for entry in package.entries {
            let target = match entry.path.strip_prefix("./") {
                Some(sub) => dist.join(sub),
                None => dist.clone(),
            };
            build_entry(config, package, entry, &wasm_opt, &target)?;
            let (raw, gzip) = measure_wasm_size(&target.join("index_bg.wasm"))?;
            println!(
                "  @draco-rust/{:<8} {:<14} {raw:>8} raw {gzip:>8} gzip ({:.1} KiB)",
                package.name,
                entry.path,
                gzip as f64 / 1024.0
            );
        }
    }
    Ok(())
}

fn build_entry(
    config: &Config,
    package: &Package,
    entry: &Entry,
    wasm_opt: &Path,
    target: &Path,
) -> Result<(), String> {
    let scratch = env::temp_dir().join(format!("draco-npm-{}-{}", package.name, unique_suffix()));
    fs::create_dir_all(&scratch)
        .map_err(|error| format!("failed to create {}: {error}", scratch.display()))?;
    let result = (|| {
        let mut log = Vec::new();
        let args: Vec<OsString> = [
            "build",
            "--release",
            "--no-opt",
            "--target",
            "web",
            "--out-name",
            "index",
            "--out-dir",
        ]
        .iter()
        .map(OsString::from)
        .chain([scratch.clone().into_os_string()])
        .chain(
            ["--", "--no-default-features", "--features"]
                .iter()
                .map(OsString::from),
        )
        .chain([OsString::from(entry.features.join(","))])
        .collect();
        run_command_with_env(
            "wasm-pack",
            &args,
            &config.web_dir.join(package.module),
            &[("CARGO_ENCODED_RUSTFLAGS", &config.rustflags)],
            &mut log,
        )
        .map_err(|error| format!("{error}\n{}", log.join("\n")))?;

        let wasm = scratch.join("index_bg.wasm");
        let mut opt_args: Vec<OsString> = vec![wasm.clone().into_os_string()];
        opt_args.extend(WASM_OPT_ARGS.iter().map(OsString::from));
        opt_args.push("-o".into());
        opt_args.push(wasm.into_os_string());
        run_command(wasm_opt, &opt_args, &config.web_dir, &mut log)
            .map_err(|error| format!("{error}\n{}", log.join("\n")))?;

        fs::create_dir_all(target)
            .map_err(|error| format!("failed to create {}: {error}", target.display()))?;
        for file in ENTRY_FILES {
            copy(&scratch.join(file), &target.join(file))?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&scratch);
    result.map_err(|error: String| {
        format!(
            "@draco-rust/{} {} ({}): {error}",
            package.name,
            entry.path,
            entry.features.join(",")
        )
    })
}

/// Every `@draco-rust/*` package carries this one version: they are built from
/// one tree and released together, so the number says which ones were tested
/// with each other.
fn read_version(path: &Path) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let version = text.trim();
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<_> = core.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "{}: {version:?} is not a semver version",
            path.display()
        ));
    }
    Ok(version.to_string())
}

/// What a package was built from: the commit, marked when the tree had
/// uncommitted changes to tracked files, and the version of each crate.
struct Provenance {
    commit: String,
    crates: Vec<(&'static str, String)>,
}

impl Provenance {
    fn of(repository: &Path) -> Result<Self, String> {
        let git = |args: &[&str]| -> Result<String, String> {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(repository)
                .args(args)
                .output()
                .map_err(|error| format!("failed to run git: {error}"))?;
            if !output.status.success() {
                return Err(format!("git {} failed", args.join(" ")));
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        };
        let mut commit = git(&["rev-parse", "--short=8", "HEAD"])?;
        if !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
            commit.push_str("-dirty");
        }
        let mut crates = Vec::new();
        for name in ["draco-core", "draco-io", "draco-gltf"] {
            let manifest = repository.join("crates").join(name).join("Cargo.toml");
            let text = fs::read_to_string(&manifest)
                .map_err(|error| format!("{}: {error}", manifest.display()))?;
            let version = text
                .lines()
                .find_map(|line| line.strip_prefix("version = \""))
                .and_then(|rest| rest.strip_suffix('"'))
                .ok_or_else(|| format!("{}: no version line", manifest.display()))?;
            crates.push((name, version.to_string()));
        }
        Ok(Self { commit, crates })
    }

    fn crates_text(&self) -> String {
        self.crates
            .iter()
            .map(|(name, version)| format!("{name} {version}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Fills the two placeholders a tracked manifest carries. The placeholder
/// version is not valid semver, so a template published by mistake is refused
/// by npm rather than released.
fn stamp_manifest(
    manifest: &str,
    version: &str,
    provenance: &Provenance,
) -> Result<String, String> {
    const VERSION: &str = r#""version": "set from web/npm/VERSION by build-tool --npm""#;
    const BUILT: &str = r#""draco-rust": "set by build-tool --npm""#;
    for placeholder in [VERSION, BUILT] {
        if manifest.matches(placeholder).count() != 1 {
            return Err(format!(
                "package.json must carry {placeholder} exactly once"
            ));
        }
    }
    let mut built = format!(
        "\"draco-rust\": {{\n    \"commit\": \"{}\"",
        provenance.commit
    );
    for (name, crate_version) in &provenance.crates {
        built.push_str(&format!(",\n    \"{name}\": \"{crate_version}\""));
    }
    built.push_str("\n  }");
    Ok(manifest
        .replace(VERSION, &format!("\"version\": \"{version}\""))
        .replace(BUILT, &built))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| format!("failed to write {}: {error}", path.display()))
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map(|_| ()).map_err(|error| {
        format!(
            "failed to copy {} to {}: {error}",
            from.display(),
            to.display()
        )
    })
}
