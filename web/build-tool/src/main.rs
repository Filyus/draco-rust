use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use miniz_oxide::deflate::compress_to_vec;

const MODULES: &[&str] = &[
    "obj-reader-wasm",
    "obj-writer-wasm",
    "ply-reader-wasm",
    "ply-writer-wasm",
    "gltf-document-wasm",
    "gltf-compact-wasm",
    "gltf-canonicalize-wasm",
    "fbx-reader-wasm",
    "fbx-writer-wasm",
];

const WASM_OPT_ARGS: &[&str] = &[
    "-Oz",
    "--converge",
    "--low-memory-unused",
    "--enable-bulk-memory",
    "--enable-nontrapping-float-to-int",
    "--enable-sign-ext",
    "--enable-mutable-globals",
];
const GLTF_DOCUMENT_GZIP_BUDGET: usize = 160 * 1024;

#[derive(Clone, Debug)]
struct Config {
    debug: bool,
    no_optimize: bool,
    features: Vec<String>,
    serve: bool,
    port: u16,
    jobs: usize,
    verbose: bool,
    force: bool,
    web_dir: PathBuf,
    output_dir: PathBuf,
}

#[derive(Debug)]
struct BuildResult {
    module: String,
    output_name: String,
    success: bool,
    skipped: bool,
    elapsed: Duration,
    log: Vec<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut config = parse_args()?;
    config.web_dir = executable_web_dir()?;
    config.output_dir = config.web_dir.join("www").join("pkg");
    fs::create_dir_all(&config.output_dir)
        .map_err(|error| format!("failed to create {}: {error}", config.output_dir.display()))?;

    println!("Building Draco Web WASM Modules");
    println!("================================");
    println!();
    println!("Output directory: {}", config.output_dir.display());

    let default_jobs = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(MODULES.len());
    let jobs = if config.jobs == 0 {
        default_jobs
    } else {
        config.jobs.clamp(1, MODULES.len())
    };
    config.jobs = jobs;
    println!("Parallel jobs: {jobs}");

    let config = Arc::new(config);
    let queue = Arc::new(Mutex::new(MODULES.to_vec()));
    let (sender, receiver) = mpsc::channel();

    for _ in 0..jobs {
        let queue = Arc::clone(&queue);
        let sender = sender.clone();
        let config = Arc::clone(&config);

        thread::spawn(move || loop {
            let module = {
                let mut queue = queue.lock().expect("module queue poisoned");
                queue.pop()
            };

            let Some(module) = module else {
                break;
            };

            let result = build_module(&config, module);
            if sender.send(result).is_err() {
                break;
            }
        });
    }
    drop(sender);

    let mut failed = Vec::new();
    for result in receiver {
        write_build_result(&config, &result);
        if !result.success {
            failed.push(result.module);
        }
    }

    if failed.is_empty() && !config.debug && !config.no_optimize {
        let document_wasm = config.output_dir.join("gltf_document_bg.wasm");
        match check_gltf_document_size(&document_wasm) {
            Ok((raw_size, gzip_size)) => println!(
                "glTF document size: {raw_size} raw, {gzip_size} gzip ({:.1} KiB / {:.0} KiB budget)",
                gzip_size as f64 / 1024.0,
                GLTF_DOCUMENT_GZIP_BUDGET as f64 / 1024.0
            ),
            Err(error) => {
                eprintln!("Error: {error}");
                failed.push("gltf-document-size".to_string());
            }
        }
    }

    if !failed.is_empty() {
        return Err(format!("build failed for modules: {}", failed.join(", ")));
    }

    println!();
    println!("================================");
    println!("Build complete!");

    if config.serve {
        let www_dir = config.web_dir.join("www");
        let server_manifest = config.web_dir.join("dev-server").join("Cargo.toml");
        println!();
        println!("Starting web server...");
        println!("Serving from: {}", www_dir.display());
        println!("WASM gzip compression: enabled");

        let status = Command::new("cargo")
            .arg("run")
            .arg("--manifest-path")
            .arg(server_manifest)
            .arg("--")
            .arg(www_dir)
            .arg(config.port.to_string())
            .status()
            .map_err(|error| format!("failed to start dev server: {error}"))?;
        if !status.success() {
            return Err(format!("dev server exited with {status}"));
        }
    } else {
        println!();
        println!("To serve the web app, run:");
        println!("  ./build.sh --serve");
        println!("  # or");
        println!("  ./build.ps1 -Serve");
    }

    Ok(())
}

fn check_gltf_document_size(path: &Path) -> Result<(usize, usize), String> {
    let wasm = fs::read(path)
        .map_err(|error| format!("failed to read {} for size check: {error}", path.display()))?;
    // RFC 1952 framing is 10 bytes of header and 8 bytes of trailer around the
    // raw DEFLATE stream. Only the exact byte length matters for this budget.
    let gzip_size = 10usize
        .checked_add(compress_to_vec(&wasm, 9).len())
        .and_then(|size| size.checked_add(8))
        .ok_or("gzip size overflow")?;
    if gzip_size > GLTF_DOCUMENT_GZIP_BUDGET {
        return Err(format!(
            "{} is {gzip_size} bytes ({:.1} KiB) gzip, exceeding the {:.0} KiB glTF document budget",
            path.display(),
            gzip_size as f64 / 1024.0,
            GLTF_DOCUMENT_GZIP_BUDGET as f64 / 1024.0
        ));
    }
    Ok((wasm.len(), gzip_size))
}

fn parse_args() -> Result<Config, String> {
    let mut config = Config {
        debug: false,
        no_optimize: false,
        features: Vec::new(),
        serve: false,
        port: 8080,
        jobs: 0,
        verbose: false,
        force: false,
        web_dir: PathBuf::new(),
        output_dir: PathBuf::new(),
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--debug" => config.debug = true,
            "--no-optimize" => config.no_optimize = true,
            "--serve" => config.serve = true,
            "--verbose-build" => config.verbose = true,
            "--force" => config.force = true,
            "--port" => {
                let value = args.next().ok_or("--port requires a value")?;
                config.port = value
                    .parse()
                    .map_err(|_| format!("invalid --port value: {value}"))?;
            }
            "--jobs" => {
                let value = args.next().ok_or("--jobs requires a value")?;
                config.jobs = value
                    .parse()
                    .map_err(|_| format!("invalid --jobs value: {value}"))?;
            }
            "--features" => {
                let value = args.next().ok_or("--features requires a value")?;
                for feature in value.split(',').filter(|feature| !feature.is_empty()) {
                    config.features.push(feature.to_string());
                }
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    Ok(config)
}

fn print_help() {
    println!("Usage: draco-web-build [options]");
    println!();
    println!("Options:");
    println!("  --debug                  Build with wasm-pack --dev");
    println!("  --no-optimize            Skip manual wasm-opt");
    println!("  --features <list>        Comma-separated cargo features");
    println!("  --serve                  Start the local web server after building");
    println!("  --port <port>            Server port (default: 8080)");
    println!("  --jobs <n>               Parallel module builds");
    println!("  --verbose-build          Print wasm-pack and wasm-opt output");
    println!("  --force                  Rebuild even when stamps are up to date");
}

fn executable_web_dir() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("cannot resolve web dir from {}", manifest_dir.display()))
}

fn build_module(config: &Config, module: &str) -> BuildResult {
    let timer = Instant::now();
    let output_name = output_name(module);
    let mut log = Vec::new();
    let module_path = config.web_dir.join(module);

    let mut result = BuildResult {
        module: module.to_string(),
        output_name: output_name.clone(),
        success: false,
        skipped: false,
        elapsed: Duration::default(),
        log: Vec::new(),
    };

    let input_latest = match input_latest(&config.web_dir, module) {
        Ok(value) => value,
        Err(error) => {
            log.push(format!("Error: {error}"));
            result.elapsed = timer.elapsed();
            result.log = log;
            return result;
        }
    };

    if !config.force && module_up_to_date(config, module, &output_name, input_latest) {
        result.success = true;
        result.skipped = true;
        result.elapsed = timer.elapsed();
        return result;
    }

    if let Err(error) = remove_stale_files(&config.output_dir, &output_name) {
        log.push(format!("Error removing stale files: {error}"));
        result.elapsed = timer.elapsed();
        result.log = log;
        return result;
    }

    let module_output_dir =
        env::temp_dir().join(format!("draco-web-build-{module}-{}", unique_suffix()));
    if let Err(error) = fs::create_dir_all(&module_output_dir) {
        log.push(format!(
            "Error creating temp dir {}: {error}",
            module_output_dir.display()
        ));
        result.elapsed = timer.elapsed();
        result.log = log;
        return result;
    }

    log.push(format!("Building {module}..."));
    let wasm_pack_args = wasm_pack_args(config, &output_name, &module_output_dir, &mut log);
    log.push(format!(
        "Running: wasm-pack {}",
        wasm_pack_args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    ));

    let build_status = run_command("wasm-pack", &wasm_pack_args, &module_path, &mut log);
    if let Err(error) = build_status {
        log.push(format!("Error: {error}"));
        let _ = fs::remove_dir_all(&module_output_dir);
        result.elapsed = timer.elapsed();
        result.log = log;
        return result;
    }

    let wasm_file = module_output_dir.join(format!("{output_name}_bg.wasm"));
    if !config.debug && !config.no_optimize && wasm_file.exists() {
        match find_wasm_opt() {
            Some(wasm_opt) => {
                log.push("Optimizing with wasm-opt...".to_string());
                let mut args = WASM_OPT_ARGS.iter().map(OsString::from).collect::<Vec<_>>();
                args.insert(0, wasm_file.clone().into_os_string());
                args.push("-o".into());
                args.push(wasm_file.clone().into_os_string());

                if let Err(error) = run_command(&wasm_opt, &args, &config.web_dir, &mut log) {
                    log.push(format!("Error: {error}"));
                    let _ = fs::remove_dir_all(&module_output_dir);
                    result.elapsed = timer.elapsed();
                    result.log = log;
                    return result;
                }
            }
            None => log.push("wasm-opt not found; leaving wasm unoptimized".to_string()),
        }
    }

    if let Err(error) = finalize_module_output(&module_output_dir, &config.output_dir, &output_name)
    {
        log.push(format!("Error: {error}"));
        let _ = fs::remove_dir_all(&module_output_dir);
        result.elapsed = timer.elapsed();
        result.log = log;
        return result;
    }

    if let Err(error) = write_build_stamp(config, module, &output_name, input_latest) {
        log.push(format!("Error writing build stamp: {error}"));
        let _ = fs::remove_dir_all(&module_output_dir);
        result.elapsed = timer.elapsed();
        result.log = log;
        return result;
    }

    let _ = fs::remove_dir_all(&module_output_dir);
    result.success = true;
    result.elapsed = timer.elapsed();
    result.log = log;
    result
}

fn wasm_pack_args(
    config: &Config,
    output_name: &str,
    output_dir: &Path,
    log: &mut Vec<String>,
) -> Vec<OsString> {
    let mut args = vec![OsString::from("build")];
    if config.debug {
        args.push("--dev".into());
    } else {
        args.push("--release".into());
        args.push("--no-opt".into());
    }

    args.push("--target".into());
    args.push("web".into());
    args.push("--out-dir".into());
    args.push(output_dir.into());
    args.push("--out-name".into());
    args.push(output_name.into());

    let features = effective_features(config);
    if config.debug
        && features
            .iter()
            .any(|feature| feature == "console_error_panic_hook")
    {
        log.push("Debug build: enabling feature 'console_error_panic_hook'".to_string());
    }

    if !features.is_empty() {
        args.push("--".into());
        args.push("--features".into());
        args.push(features.join(",").into());
    }

    args
}

fn effective_features(config: &Config) -> Vec<String> {
    let mut features = config.features.iter().cloned().collect::<BTreeSet<_>>();
    if config.debug {
        features.insert("console_error_panic_hook".to_string());
    }
    features.into_iter().collect()
}

fn run_command<P: AsRef<Path>>(
    program: P,
    args: &[OsString],
    cwd: &Path,
    log: &mut Vec<String>,
) -> Result<(), String> {
    let output = Command::new(program.as_ref())
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env("CARGO_TERM_COLOR", "never")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("failed to run {}: {error}", program.as_ref().display()))?;

    push_output(log, &output.stdout);
    push_output(log, &output.stderr);

    if !output.status.success() {
        return Err(format!(
            "{} exited with {}",
            program.as_ref().display(),
            output.status
        ));
    }

    Ok(())
}

fn push_output(log: &mut Vec<String>, output: &[u8]) {
    let text = String::from_utf8_lossy(output);
    for line in text.lines() {
        let line = format_log_line(line);
        if !line.is_empty() {
            log.push(line);
        }
    }
}

fn format_log_line(line: &str) -> String {
    let ascii = line
        .chars()
        .filter(|ch| matches!(ch, '\t' | '\n' | '\r') || ch.is_ascii_graphic() || *ch == ' ')
        .collect::<String>();
    ascii.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn finalize_module_output(
    module_output_dir: &Path,
    output_dir: &Path,
    output_name: &str,
) -> Result<(), String> {
    for entry in fs::read_dir(module_output_dir)
        .map_err(|error| format!("failed to read {}: {error}", module_output_dir.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read output entry: {error}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(file_name) = path.file_name() else {
            continue;
        };
        let file_name_text = file_name.to_string_lossy();
        if file_name_text.starts_with(output_name)
            || file_name_text == "package.json"
            || file_name_text == "LICENSE"
        {
            fs::copy(&path, output_dir.join(file_name)).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    path.display(),
                    output_dir.display()
                )
            })?;
        }
    }

    Ok(())
}

fn output_name(module: &str) -> String {
    module
        .strip_suffix("-wasm")
        .unwrap_or(module)
        .replace('-', "_")
}

fn write_build_result(config: &Config, result: &BuildResult) {
    if result.skipped {
        println!(
            "Skip {} -> {}_bg.wasm (unchanged)",
            result.module, result.output_name
        );
    } else if result.success {
        println!(
            "Done {} -> {}_bg.wasm ({:.2}s)",
            result.module,
            result.output_name,
            result.elapsed.as_secs_f64()
        );
    } else {
        println!(
            "Failed {} ({:.2}s)",
            result.module,
            result.elapsed.as_secs_f64()
        );
    }

    if config.verbose || !result.success {
        for line in &result.log {
            println!("  {line}");
        }
    }
}

fn input_latest(web_dir: &Path, module: &str) -> Result<u128, String> {
    let repo_root = web_dir
        .parent()
        .ok_or_else(|| format!("cannot resolve repo root from {}", web_dir.display()))?;
    let module_path = web_dir.join(module);
    let input_paths = [
        module_path.join("Cargo.toml"),
        module_path.join("src"),
        web_dir.join("Cargo.toml"),
        web_dir.join("Cargo.lock"),
        web_dir.join("build-tool").join("Cargo.toml"),
        web_dir.join("build-tool").join("src"),
        repo_root
            .join("crates")
            .join("draco-core")
            .join("Cargo.toml"),
        repo_root.join("crates").join("draco-core").join("src"),
        repo_root.join("crates").join("draco-io").join("Cargo.toml"),
        repo_root.join("crates").join("draco-io").join("src"),
    ];

    let mut latest = 0;
    for path in input_paths {
        if !path.exists() {
            continue;
        }
        collect_latest_mtime(&path, &mut latest)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    }
    Ok(latest)
}

fn collect_latest_mtime(path: &Path, latest: &mut u128) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    if metadata.is_file() {
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        *latest = (*latest).max(modified.as_millis());
        return Ok(());
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        collect_latest_mtime(&entry.path(), latest)?;
    }
    Ok(())
}

fn module_up_to_date(config: &Config, module: &str, output_name: &str, input_latest: u128) -> bool {
    let wasm_file = config.output_dir.join(format!("{output_name}_bg.wasm"));
    let js_file = config.output_dir.join(format!("{output_name}.js"));
    let stamp_path = stamp_path(&config.output_dir, output_name);
    if !wasm_file.exists() || !js_file.exists() || !stamp_path.exists() {
        return false;
    }

    let Ok(stamp) = fs::read_to_string(stamp_path) else {
        return false;
    };

    stamp.contains(&format!("\"module\":\"{module}\""))
        && stamp.contains(&format!("\"config_key\":\"{}\"", config_key(config)))
        && stamp.contains(&format!("\"input_latest_millis\":{input_latest}"))
}

fn write_build_stamp(
    config: &Config,
    module: &str,
    output_name: &str,
    input_latest: u128,
) -> io::Result<()> {
    let stamp = format!(
        "{{\"module\":\"{}\",\"config_key\":\"{}\",\"input_latest_millis\":{},\"built_at_unix_millis\":{}}}",
        module,
        config_key(config),
        input_latest,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    fs::write(stamp_path(&config.output_dir, output_name), stamp)
}

fn config_key(config: &Config) -> String {
    format!(
        "debug={};no_optimize={};features={};wasm_opt_args={}",
        config.debug,
        config.no_optimize,
        effective_features(config).join(","),
        WASM_OPT_ARGS.join(",")
    )
}

fn stamp_path(output_dir: &Path, output_name: &str) -> PathBuf {
    output_dir.join(format!("{output_name}.build-stamp.json"))
}

fn remove_stale_files(output_dir: &Path, output_name: &str) -> io::Result<()> {
    if !output_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name() else {
            continue;
        };
        if file_name.to_string_lossy().starts_with(output_name) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn find_wasm_opt() -> Option<PathBuf> {
    find_on_path("wasm-opt").or_else(|| {
        let mut candidates = Vec::new();
        if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
            candidates.push(
                PathBuf::from(home)
                    .join(".cargo")
                    .join("bin")
                    .join(exe_name("wasm-opt")),
            );
        }
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            let wasm_pack_dir = PathBuf::from(local_app_data).join(".wasm-pack");
            if let Ok(entries) = fs::read_dir(wasm_pack_dir) {
                for entry in entries.flatten() {
                    candidates.push(entry.path().join("bin").join(exe_name("wasm-opt")));
                }
            }
        }
        candidates.into_iter().find(|path| path.exists())
    })
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    for dir in env::split_paths(&paths) {
        let candidate = dir.join(exe_name(program));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn unique_suffix() -> String {
    format!(
        "{}-{:?}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
