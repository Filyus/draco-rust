//! What has to hold before a figure is worth taking, and what a figure is
//! taken against.
//!
//! The C++ side links whichever build `DRACO_CPP_BUILD_DIR` names, and one
//! checkout on the maintainer's machine carries a `getenv("DRACO_VERBOSE")`
//! patch on both the encode and the decode path that makes C++ several times
//! slower. Nothing about that build's flags or size gives it away, so the check
//! here looks for the patch itself: in the library that gets linked, and in the
//! headers the bridge compiles against, since one patched file is a template
//! header and lands in the bridge rather than the library.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The C++ Draco a run compares against.
pub struct Reference {
    pub build_dir: PathBuf,
    pub source_dir: PathBuf,
    pub library: PathBuf,
    pub version: String,
}

const PATCH_MARKER: &str = "DRACO_VERBOSE";

pub fn reference() -> Result<Reference, String> {
    let pinned = |name: &str| {
        std::env::var_os(name).map(PathBuf::from).ok_or_else(|| {
            format!(
                "{name} is not set. The suite compares against an explicitly pinned C++ Draco \
                 and never the bridge's default; see PERFORMANCE.md, \"Pin the reference build\"."
            )
        })
    };
    let build_dir = pinned("DRACO_CPP_BUILD_DIR")?;
    let checkout = pinned("DRACO_CPP_SOURCE_DIR")?;
    // The same two spellings the bridge's build script accepts: the directory
    // holding `draco/`, or the checkout above it.
    let source_dir = if checkout.join("draco").is_dir() {
        checkout
    } else {
        checkout.join("src")
    };
    if !source_dir.join("draco").is_dir() {
        return Err(format!(
            "DRACO_CPP_SOURCE_DIR has no draco/ headers under {}",
            source_dir.display()
        ));
    }

    let library = library(&build_dir)?;
    let bytes = std::fs::read(&library)
        .map_err(|error| format!("cannot read {}: {error}", library.display()))?;
    if bytes
        .windows(PATCH_MARKER.len())
        .any(|window| window == PATCH_MARKER.as_bytes())
    {
        return Err(format!(
            "{} contains \"{PATCH_MARKER}\": it is the locally patched build, not stock Draco",
            library.display()
        ));
    }
    let patched = files_mentioning(&source_dir.join("draco"), PATCH_MARKER);
    if !patched.is_empty() {
        let list: Vec<String> = patched.iter().map(|p| p.display().to_string()).collect();
        return Err(format!(
            "the C++ source carries the \"{PATCH_MARKER}\" patch:\n  {}",
            list.join("\n  ")
        ));
    }

    let version = version(&source_dir).unwrap_or_else(|| "unknown".to_owned());
    Ok(Reference {
        build_dir,
        source_dir,
        library,
        version,
    })
}

/// The library the bridge links, probed in the bridge's own order. A Debug
/// library is refused: nothing timed against it says anything.
fn library(build_dir: &Path) -> Result<PathBuf, String> {
    for root in [build_dir.join("src/draco"), build_dir.to_owned()] {
        for (dir, debug) in [
            (root.join("Release"), false),
            (root.join("Debug"), true),
            (root.clone(), false),
        ] {
            for name in ["draco.lib", "libdraco.a"] {
                let path = dir.join(name);
                if path.is_file() {
                    return if debug {
                        Err(format!("{} is a Debug build", path.display()))
                    } else {
                        Ok(path)
                    };
                }
            }
        }
    }
    Err(format!(
        "no draco.lib or libdraco.a under {}",
        build_dir.display()
    ))
}

fn files_mentioning(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "h" || ext == "cc" || ext == "inl")
                && std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle))
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn version(source_dir: &Path) -> Option<String> {
    let header = std::fs::read_to_string(source_dir.join("draco/core/draco_version.h")).ok()?;
    let start = header.find("kDracoVersion[] = \"")? + "kDracoVersion[] = \"".len();
    let len = header[start..].find('"')?;
    Some(header[start..start + len].to_owned())
}

/// The repository's state. A dirty tree is refused unless allowed, because a
/// figure is quoted against a commit and has to be reproducible from it.
pub struct Checkout {
    pub commit: String,
    pub changes: Vec<String>,
}

pub fn checkout(root: &Path) -> Result<Checkout, String> {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .map_err(|error| format!("cannot run git: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let commit = git(&["rev-parse", "HEAD"])?.trim().to_owned();
    let changes = git(&["status", "--porcelain"])?
        .lines()
        .map(str::to_owned)
        .collect();
    Ok(Checkout { commit, changes })
}

/// What a command prints, or `None` if it cannot run or fails.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn rustc() -> String {
    output("rustc", &["-V"])
        .map(|text| text.trim().to_owned())
        .unwrap_or_else(|| "unknown rustc".to_owned())
}

pub fn cpu() -> String {
    let name = if cfg!(windows) {
        output(
            "reg",
            &[
                "query",
                r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
                "/v",
                "ProcessorNameString",
            ],
        )
        .and_then(|text| {
            let line = text.lines().find(|line| line.contains("REG_SZ"))?;
            Some(line.split("REG_SZ").nth(1)?.trim().to_owned())
        })
    } else if cfg!(target_os = "macos") {
        output("sysctl", &["-n", "machdep.cpu.brand_string"]).map(|text| text.trim().to_owned())
    } else {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|text| {
                let line = text.lines().find(|line| line.starts_with("model name"))?;
                Some(line.split(':').nth(1)?.trim().to_owned())
            })
    };
    name.filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown CPU".to_owned())
}

/// Today's UTC date, `YYYY-MM-DD`.
pub fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's days-to-civil conversion, proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_become_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_731), (2026, 10, 5));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
