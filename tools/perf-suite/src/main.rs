//! Every comparison `PERFORMANCE.md` quotes, in one run.
//!
//! Before a release that claims anything about speed, the figures in
//! `PERFORMANCE.md` have to be re-taken, and taking them by hand means a
//! harness at a time, each with its own flags, against whichever C++ build
//! the shell happens to point at. This runs all of them the documented way and
//! writes one dated report:
//!
//! 1. Preflight: the C++ reference is pinned through `DRACO_CPP_BUILD_DIR` and
//!    `DRACO_CPP_SOURCE_DIR`, is a Release build, and carries no local patch.
//!    The tree is clean, so the figures belong to a commit.
//! 2. Every harness is built first, with `DRACO_REQUIRE_CPP_BRIDGE=1` so a
//!    missing bridge fails the build instead of skipping the comparisons.
//! 3. The harnesses run one at a time, each with
//!    `PERF_JSONL` naming the file it appends its rows to, and its printed
//!    output kept beside that file.
//! 4. `report.md` collects the rows into the tables `PERFORMANCE.md` carries.
//!
//! ```text
//! DRACO_CPP_BUILD_DIR=... DRACO_CPP_SOURCE_DIR=... [ZSTD_SOURCE_DIR=...] \
//!   cargo run --release --manifest-path tools/perf-suite/Cargo.toml -- [options]
//! ```
//!
//! Run with `--help` for the options.

mod json;
mod preflight;
mod report;

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;

const USAGE: &str = "\
perf-suite: every comparison PERFORMANCE.md quotes, in one run

usage: perf-suite [options]
       perf-suite --report <dir>

options:
  --out <dir>        where the logs, rows and report go
                     (default .scratch/perf-suite/<date>-<commit>)
  --only <names>     comma-separated step names or prefixes, e.g. model_matrix,ktx2
  --sweep-runs <n>   runs of the seeded sweep, reported as their median (default 3)
  --allow-dirty      measure a tree with uncommitted changes, and say so
  --dry-run          check the preconditions and print the plan, run nothing
  --list             print the steps
  --report <dir>     rewrite the report of an earlier run from its rows

environment:
  DRACO_CPP_BUILD_DIR, DRACO_CPP_SOURCE_DIR  the C++ Draco compared against (required)
  ZSTD_SOURCE_DIR    a facebook/zstd checkout; without it the zstd step is skipped
";

struct Options {
    out: Option<PathBuf>,
    only: Vec<String>,
    sweep_runs: usize,
    allow_dirty: bool,
    dry_run: bool,
    list: bool,
    report: Option<PathBuf>,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut options = Options {
            out: None,
            only: Vec::new(),
            sweep_runs: 3,
            allow_dirty: false,
            dry_run: false,
            list: false,
            report: None,
        };
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
            match arg.as_str() {
                "--out" => options.out = Some(value()?.into()),
                "--only" => {
                    options.only = value()?
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                "--sweep-runs" => {
                    options.sweep_runs = value()?
                        .parse()
                        .ok()
                        .filter(|&runs| runs > 0)
                        .ok_or_else(|| "--sweep-runs takes a positive count".to_owned())?;
                }
                "--allow-dirty" => options.allow_dirty = true,
                "--dry-run" => options.dry_run = true,
                "--list" => options.list = true,
                "--report" => options.report = Some(value()?.into()),
                "--help" | "-h" => return Err(String::new()),
                _ => return Err(format!("unknown argument {arg}")),
            }
        }
        Ok(options)
    }
}

/// One harness run: the cargo arguments, and what it needs to be meaningful.
struct Step {
    name: String,
    args: Vec<String>,
    /// Why the step cannot run here, if it cannot.
    unavailable: Option<String>,
}

const BRIDGE: [&str; 5] = [
    "--release",
    "--manifest-path",
    "crates/Cargo.toml",
    "-p",
    "draco-cpp-test-bridge",
];

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|&part| part.to_owned()).collect()
}

/// The builds that make every later `cargo run` and `cargo test` a no-op, so
/// no step's clock overlaps a compile and a broken build stops the run before
/// anything is measured.
fn builds(zstd: bool) -> Vec<Vec<String>> {
    let mut builds = vec![
        strings(&[&["build"][..], &BRIDGE[..], &["--examples", "--tests"][..]].concat()),
        strings(&[
            "build",
            "--release",
            "--manifest-path",
            "tools/basis-cpp-oracle/Cargo.toml",
            "--example",
            "speed",
        ]),
    ];
    if zstd {
        builds.push(strings(&[
            "build",
            "--release",
            "--manifest-path",
            "tools/zstd-bench/Cargo.toml",
        ]));
    }
    builds
}

/// Every step, in the order `PERFORMANCE.md` lists them.
fn steps(sweep_runs: usize, zstd: bool) -> Vec<Step> {
    let bridge = |verb: &str, rest: &[&str]| -> Vec<String> {
        strings(&[&[verb][..], &BRIDGE[..], rest].concat())
    };
    let mut steps = Vec::new();
    for run in 1..=sweep_runs {
        steps.push(Step {
            name: format!("seeded_sweep_{run}"),
            args: bridge(
                "test",
                &[
                    "--test",
                    "profile_sequential_pipeline",
                    "--",
                    "--exact",
                    "profile_seeded_mesh_sweep",
                    "--nocapture",
                ],
            ),
            unavailable: None,
        });
    }
    for speed in ["4", "10"] {
        steps.push(Step {
            name: format!("model_matrix_speed{speed}"),
            args: bridge(
                "run",
                &[
                    "--example",
                    "model_matrix",
                    "--",
                    "9",
                    "20",
                    speed,
                    "10",
                    "bunny=testdata/bunny_cpp_standard.drc",
                    "lamp=testdata/lamp_cpp_std.drc",
                    "car=testdata/car.drc",
                ],
            ),
            unavailable: None,
        });
    }
    for (name, test, filter) in [
        ("real_models", "bench_real_models", None),
        ("grid_decode", "bench_decode_cpp_vs_rust", None),
        (
            "grid_encode",
            "bench_encode_cpp_vs_rust",
            Some("bench_encode_cpp_vs_rust"),
        ),
    ] {
        let mut rest = vec!["--test", test, "--"];
        if let Some(filter) = filter {
            rest.extend(["--exact", filter]);
        }
        rest.push("--nocapture");
        steps.push(Step {
            name: name.to_owned(),
            args: bridge("test", &rest),
            unavailable: None,
        });
    }
    // One test at a time: its two tests would otherwise time side by side.
    steps.push(Step {
        name: "encode_decode_matrix".to_owned(),
        args: bridge(
            "test",
            &[
                "--test",
                "bench_encode_decode_matrix",
                "--",
                "--nocapture",
                "--test-threads=1",
            ],
        ),
        unavailable: None,
    });
    steps.push(Step {
        name: "ktx2".to_owned(),
        args: strings(&[
            "run",
            "--release",
            "--manifest-path",
            "tools/basis-cpp-oracle/Cargo.toml",
            "--example",
            "speed",
        ]),
        unavailable: None,
    });
    steps.push(Step {
        name: "zstd".to_owned(),
        args: strings(&[
            "run",
            "--release",
            "--manifest-path",
            "tools/zstd-bench/Cargo.toml",
        ]),
        unavailable: (!zstd)
            .then(|| "ZSTD_SOURCE_DIR is not set, so there is no C zstd to compare".to_owned()),
    });
    steps
}

/// Two levels above this crate, taken by name: `canonicalize` would hand back
/// a `\\?\` path on Windows, which some tools in a cargo build refuse.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("tools/perf-suite sits two levels down")
        .to_owned()
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// A cargo command in the repository root, with the bridge required. Rows go
/// to `rows` when given, and output to `log`.
fn run_cargo(root: &Path, args: &[String], rows: Option<&Path>, log: &Path) -> Result<(), String> {
    let file = File::create(log).map_err(|error| format!("{}: {error}", log.display()))?;
    let stderr = file
        .try_clone()
        .map_err(|error| format!("{}: {error}", log.display()))?;
    let mut command = Command::new(cargo());
    command
        .args(args)
        .current_dir(root)
        .env("DRACO_REQUIRE_CPP_BRIDGE", "1")
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(stderr);
    match rows {
        Some(rows) => command.env("PERF_JSONL", rows),
        None => command.env_remove("PERF_JSONL"),
    };
    let status = command
        .status()
        .map_err(|error| format!("cannot start cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo exited with {status}"))
    }
}

fn tail(log: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn count_rows(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

/// A directory nothing else wrote to: the default name, with a counter added
/// if an earlier run of the same day and commit holds it.
fn fresh_dir(root: &Path, date: &str, commit: &str) -> PathBuf {
    let base = root
        .join(".scratch/perf-suite")
        .join(format!("{date}-{}", &commit[..commit.len().min(8)]));
    let mut dir = base.clone();
    let mut counter = 2;
    while dir.exists() {
        dir = PathBuf::from(format!("{}-{counter}", base.display()));
        counter += 1;
    }
    dir
}

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            if message.is_empty() {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match run(options) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("perf-suite: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> Result<ExitCode, String> {
    let root = repo_root();
    if let Some(dir) = &options.report {
        let path = report::write(dir)?;
        println!("{}", path.display());
        return Ok(ExitCode::SUCCESS);
    }

    let zstd = std::env::var_os("ZSTD_SOURCE_DIR").is_some();
    let mut plan = steps(options.sweep_runs, zstd);
    if !options.only.is_empty() {
        plan.retain(|step| {
            options
                .only
                .iter()
                .any(|name| step.name.starts_with(name.as_str()))
        });
        if plan.is_empty() {
            return Err(format!("no step matches {}", options.only.join(",")));
        }
    }
    if options.list {
        for step in &plan {
            println!("{}", step.name);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let reference = preflight::reference()?;
    let checkout = preflight::checkout(&root)?;
    if !checkout.changes.is_empty() && !options.allow_dirty {
        let shown: Vec<&str> = checkout
            .changes
            .iter()
            .take(12)
            .map(String::as_str)
            .collect();
        return Err(format!(
            "the tree has {} uncommitted change(s), so the figures would belong to no commit. \
             Commit first, or pass --allow-dirty.\n  {}",
            checkout.changes.len(),
            shown.join("\n  ")
        ));
    }
    let date = preflight::today();
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| fresh_dir(&root, &date, &checkout.commit));

    let builds = builds(zstd);
    let display = |args: &[String]| format!("cargo {}", args.join(" "));
    println!(
        "C++ Draco {} from {}",
        reference.version,
        reference.library.display()
    );
    println!(
        "commit {}{}",
        checkout.commit,
        if checkout.changes.is_empty() {
            String::new()
        } else {
            format!(", {} uncommitted change(s)", checkout.changes.len())
        }
    );
    println!("output {}", out.display());
    if options.dry_run {
        println!("\nbuilds:");
        for args in &builds {
            println!("  {}", display(args));
        }
        println!("\nsteps:");
        for step in &plan {
            match &step.unavailable {
                Some(reason) => println!("  {}: skipped, {reason}", step.name),
                None => println!("  {}: {}", step.name, display(&step.args)),
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    std::fs::create_dir_all(&out).map_err(|error| format!("{}: {error}", out.display()))?;
    let threads = std::thread::available_parallelism().map_or(0, usize::from);
    let mut meta = vec![
        ("date", date),
        ("commit", checkout.commit.clone()),
        ("uncommitted", checkout.changes.len().to_string()),
        ("cpu", preflight::cpu()),
        ("threads", threads.to_string()),
        ("os", std::env::consts::OS.to_owned()),
        ("rustc", preflight::rustc()),
        ("cpp_version", reference.version.clone()),
        ("cpp_library", reference.library.display().to_string()),
        ("cpp_source", reference.source_dir.display().to_string()),
        ("cpp_build", reference.build_dir.display().to_string()),
    ];
    // Anything here changes the code being timed, so the report names it.
    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "ZSTD_SOURCE_DIR"] {
        if let Some(value) = std::env::var_os(name) {
            meta.push((name, value.to_string_lossy().into_owned()));
        }
    }
    let meta_path = out.join("meta.json");
    std::fs::write(&meta_path, json::object(&meta) + "\n")
        .map_err(|error| format!("{}: {error}", meta_path.display()))?;

    for (index, args) in builds.iter().enumerate() {
        let log = out.join(format!("build-{}.log", index + 1));
        println!("building: {}", display(args));
        if let Err(message) = run_cargo(&root, args, None, &log) {
            return Err(format!(
                "build failed, nothing measured: {message}\n{}\n(full log: {})",
                tail(&log, 30),
                log.display()
            ));
        }
    }

    let steps_path = out.join("steps.jsonl");
    let mut failed = 0;
    for (index, step) in plan.iter().enumerate() {
        let (status, rows, seconds, note) = if let Some(reason) = &step.unavailable {
            println!(
                "[{}/{}] {}: skipped, {reason}",
                index + 1,
                plan.len(),
                step.name
            );
            ("skipped", 0, 0.0, reason.clone())
        } else {
            let rows_path = out.join(format!("{}.jsonl", step.name));
            let log = out.join(format!("{}.log", step.name));
            let started = Instant::now();
            let result = run_cargo(&root, &step.args, Some(&rows_path), &log);
            let seconds = started.elapsed().as_secs_f64();
            let rows = count_rows(&rows_path);
            // A harness that printed no row compared nothing, whatever its
            // exit status says.
            let (status, note) = match result {
                Ok(()) if rows > 0 => ("ok", String::new()),
                Ok(()) => ("failed", "it wrote no rows".to_owned()),
                Err(message) => ("failed", message),
            };
            println!(
                "[{}/{}] {}: {status}, {rows} rows, {seconds:.0} s{}",
                index + 1,
                plan.len(),
                step.name,
                if note.is_empty() {
                    String::new()
                } else {
                    format!(", {note}")
                }
            );
            if status == "failed" {
                failed += 1;
                eprintln!("{}\n(full log: {})", tail(&log, 20), log.display());
            }
            (status, rows, seconds, note)
        };
        let line = json::object(&[
            ("step", step.name.clone()),
            ("status", status.to_owned()),
            ("rows", rows.to_string()),
            ("seconds", format!("{seconds:.1}")),
            ("note", note),
        ]);
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&steps_path)
            .and_then(|mut file| writeln!(file, "{line}"))
            .map_err(|error| format!("{}: {error}", steps_path.display()))?;
    }

    let report = report::write(&out)?;
    println!("\nreport: {}", report.display());
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        eprintln!("{failed} step(s) failed");
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_has_its_own_name() {
        let plan = steps(3, true);
        let mut names: Vec<&str> = plan.iter().map(|step| step.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), plan.len());
    }

    #[test]
    fn zstd_is_skipped_with_a_reason_without_its_source() {
        let plan = steps(1, false);
        let zstd = plan.iter().find(|step| step.name == "zstd").unwrap();
        assert!(zstd.unavailable.is_some());
        assert!(plan
            .iter()
            .filter(|step| step.name != "zstd")
            .all(|step| step.unavailable.is_none()));
    }

    #[test]
    fn options_read_back_what_was_given() {
        let args = [
            "--only",
            "model_matrix, ktx2",
            "--sweep-runs",
            "2",
            "--allow-dirty",
        ];
        let options = Options::parse(args.iter().map(|&arg| arg.to_owned())).unwrap();
        assert_eq!(options.only, ["model_matrix", "ktx2"]);
        assert_eq!(options.sweep_runs, 2);
        assert!(options.allow_dirty && !options.dry_run);
        assert!(Options::parse(["--sweep-runs", "0"].iter().map(|&a| a.to_owned())).is_err());
        assert!(Options::parse(["--sweep-runs"].iter().map(|&a| a.to_owned())).is_err());
        assert!(Options::parse(["--pause", "5"].iter().map(|&a| a.to_owned())).is_err());
    }
}
