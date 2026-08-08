//! Measures how much of the *compiled* crate is error prose.
//!
//! The 2.0 release replaced a few hundred bare `bool` refusals with sentences,
//! and sentences grow without anyone deciding to grow them: each is
//! individually justified, and the sum is never reviewed. This measures the sum
//! against the machine code it ships beside, so the number gets looked at
//! rather than discovered.
//!
//! ## Why the binary and not the source
//!
//! Counting source characters answers a different question. A message the
//! optimiser drops still costs source; a message repeated in ten files is one
//! string in the artifact; and there is nothing to compare a character count
//! *to* — "4% of the source" says nothing about what the user downloads. The
//! binary answers the question directly: these bytes of text, that much
//! machine code, both in the same file.
//!
//! ## How
//!
//! 1. The message fragments are read out of the source, because that is where
//!    it is knowable that a string is a refusal rather than a key or a path.
//!    `format!` arguments are split at their placeholders, since that is how
//!    rustc stores them — `"read {n} of {m}"` is two fragments in the artifact,
//!    not one string.
//! 2. An example is built in release into its own target directory, so the
//!    nested build neither blocks on the outer one nor pollutes it. Cold it
//!    costs about fifteen seconds; after that it is cached. `debug_cube_v11`
//!    is the one chosen because it encodes *and* decodes: a decode-only
//!    example leaves every encoder message unlinked, and the ratio would then
//!    measure which example was picked as much as anything else. It is built
//!    `--all-features` so the number cannot move when the default set is next
//!    edited -- not because the extra features carry messages. They carry
//!    none, and they delete a dozen: see below.
//! 3. The artifact's `.text` is read from its own section table — PE on
//!    Windows, ELF elsewhere — and each fragment is searched for in the file.
//!
//! ## What the number is a share *of*
//!
//! Of the machine code in one linked artifact, which is narrower than "the
//! crate". About a quarter of the crate's message fragments are absent from it,
//! and the report names them because the reason matters:
//!
//! - **Not linked** (roughly 114 of 145 when this was written). The example
//!   encodes and decodes a mesh, so the KD-tree encoder, the point-cloud
//!   encoder and the normal encoder are never called and the linker never
//!   pulls them in. Their messages do not ship *in this program*, which is the
//!   honest answer for this program and the wrong answer for the crate.
//! - **Compiled out by `--all-features`.** A dozen messages live under
//!   `#[cfg(not(feature = …))]` and say so — "Point cloud decode support is
//!   disabled". Turning every feature on deletes them. There is no feature set
//!   that contains every message: the disabled-path text and the enabled-path
//!   text exclude each other by construction.
//! - **Never monomorphised.** `Metadata::set_i32_array` takes
//!   `name: impl Into<String>` and has no caller in the workspace, so no
//!   instance of it is ever generated and `-C link-dead-code` cannot retain
//!   what was never emitted. Absent for a reason that has nothing to do with
//!   reachability.
//! - **Proved unreachable.** The residue, and the part worth reading. Three
//!   things turned up in it, and only one was a mistake:
//!
//!   `"Invalid sub-metadata count"` repeated a check `decode_bounded_count`
//!   had already made on the same buffer state, so it could not fire. That
//!   one was deleted.
//!
//!   `"Bit stream size too large"` guards `usize::try_from(u64)`, which is
//!   infallible where usize is 64 bits and fallible on the wasm32 target this
//!   crate ships to the web. It is dead *here* and load-bearing there, which
//!   is the reason this list is read rather than acted on: "the compiler
//!   dropped it" is not "delete it".
//!
//!   `"rANS precision {n} bits has no encoder"` sits on a match arm whose
//!   input was clamped to 1..=18 a few lines earlier, and the `checked_mul`
//!   overflow messages sit where the bound is already established. Those are
//!   guards kept against a premise moving, and they cost nothing in the
//!   binary precisely because the premise currently holds.
//!
//!   Reading the list also found `encode_raw_symbols_typed`, dead since before
//!   this release and carrying a message of its own.
//!
//! `RUSTFLAGS="-C link-dead-code"` gives the whole-crate view -- 31 absent
//! instead of 145 -- at the cost of a denominator stuffed with code nobody
//! ships, so it is a diagnostic to run by hand rather than the measurement.
//!
//! ## What it cannot do
//!
//! A fragment is counted once however often it occurs, which is what the linker
//! does with equal strings but not what it does with overlapping ones. Short
//! fragments are dropped because they collide with unrelated bytes. A message
//! assembled at runtime from pieces too small to survive that filter is
//! invisible. The ratio is a trend, not an audit.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The call heads whose string arguments are a refusal's text.
const ERROR_CALL_HEADS: &[&str] = &["DracoError::", "error_status("];

/// Below this a fragment stops being evidence of itself: four-character runs
/// occur throughout compiled code by chance.
const MIN_FRAGMENT: usize = 8;

/// Measured at the 2.0.0 release; printed on every run, so moving the ceiling
/// is a decision made against a number.
const RATIO_AT_2_0_0: f64 = 0.036_31;

/// Prose may reach this share of `.text` before the build fails. Wide on
/// purpose: converting another path to `Status` should not need this file
/// edited, while doubling the crate's messages should not pass unnoticed.
const MAX_RATIO: f64 = 0.050;

// ---------------------------------------------------------------------------
// Reading the messages out of the source
// ---------------------------------------------------------------------------

/// Every string literal lexically inside a `DracoError::…(…)` or
/// `error_status(…)` call, `#[cfg(test)]` modules excluded.
///
/// A single pass, because none of this survives a regex: `//` occurs inside
/// string literals, `"` occurs inside comments, and raw strings hold both.
fn error_literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let error_spans = error_call_spans(&chars);
    let test_spans = cfg_test_spans(&chars);
    let mut found = Vec::new();

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];

        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let mut depth = 1usize;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }

        if c == 'r' && !is_ident_char(i.checked_sub(1).map(|p| chars[p])) {
            if let Some((body_start, body_end, end)) = raw_string_at(&chars, i) {
                if in_any(&error_spans, i) && !in_any(&test_spans, i) {
                    found.push(chars[body_start..body_end].iter().collect());
                }
                i = end;
                continue;
            }
        }

        if c == '"' {
            let start = i;
            i += 1;
            let body_start = i;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    break;
                }
                i += 1;
            }
            let body_end = i.min(chars.len());
            i = (i + 1).min(chars.len());
            if in_any(&error_spans, start) && !in_any(&test_spans, start) {
                found.push(chars[body_start..body_end].iter().collect());
            }
            continue;
        }

        if c == '\'' {
            if let Some(end) = char_literal_end(&chars, i) {
                i = end;
                continue;
            }
        }

        i += 1;
    }

    found
}

/// Splits a message the way rustc stores it: the parts around each `{…}`
/// placeholder are separate strings in the artifact.
///
/// `\n` and friends are unescaped first, and a `{{` escape is folded to the
/// single brace it prints as, so the fragment matches the bytes emitted.
fn fragments(literal: &str) -> Vec<String> {
    let unescaped = unescape(literal);

    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = unescaped.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                current.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                current.push('}');
            }
            '{' => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    out.push(current);
    // Whitespace-only pieces are not prose, and would match everywhere.
    out.retain(|f| f.len() >= MIN_FRAGMENT && !f.trim().is_empty());
    out
}

/// Resolves the escapes rustc resolves, in one left-to-right pass so that a
/// `\\` cannot be mistaken for the start of the next escape.
///
/// The case that matters here is the line continuation: a backslash at end of
/// line eats the newline *and the indentation after it*, and the newline is
/// CRLF in this checkout. Handling only `\` + LF left the backslash, the CR and
/// twenty spaces sitting inside the fragment, which then matched nothing in the
/// artifact and quietly went missing from the count.
fn unescape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut chars = literal.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some('"') => {
                chars.next();
                out.push('"');
            }
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            Some('\r') | Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            _ => out.push('\\'),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Reading `.text` out of the artifact
// ---------------------------------------------------------------------------

/// The size of the machine-code section, from the artifact's own section table.
fn text_section_size(binary: &[u8]) -> Option<usize> {
    if binary.starts_with(b"\x7fELF") {
        elf_text_size(binary)
    } else if binary.starts_with(b"MZ") {
        pe_text_size(binary)
    } else {
        None
    }
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

/// 64-bit little-endian ELF only, which is every platform this is built on.
fn elf_text_size(b: &[u8]) -> Option<usize> {
    if *b.get(4)? != 2 {
        return None; // not ELF64
    }
    let sh_off = u64_at(b, 0x28)? as usize;
    let sh_entsize = u16_at(b, 0x3A)? as usize;
    let sh_num = u16_at(b, 0x3C)? as usize;
    let sh_strndx = u16_at(b, 0x3E)? as usize;

    let strtab_hdr = sh_off + sh_strndx * sh_entsize;
    let strtab_off = u64_at(b, strtab_hdr + 0x18)? as usize;

    for i in 0..sh_num {
        let hdr = sh_off + i * sh_entsize;
        let name_off = u32_at(b, hdr)? as usize;
        let name = c_string_at(b, strtab_off + name_off)?;
        if name == ".text" {
            return Some(u64_at(b, hdr + 0x20)? as usize);
        }
    }
    None
}

fn pe_text_size(b: &[u8]) -> Option<usize> {
    let pe_off = u32_at(b, 0x3C)? as usize;
    if b.get(pe_off..pe_off + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe_off + 4;
    let num_sections = u16_at(b, coff + 2)? as usize;
    let opt_size = u16_at(b, coff + 16)? as usize;
    let table = coff + 20 + opt_size;

    for i in 0..num_sections {
        let hdr = table + i * 40;
        let raw = b.get(hdr..hdr + 8)?;
        let name = std::str::from_utf8(raw).ok()?.trim_end_matches('\0');
        if name == ".text" {
            // Virtual size, not raw size: raw is padded to file alignment.
            let virtual_size = u32_at(b, hdr + 8)? as usize;
            let raw_size = u32_at(b, hdr + 16)? as usize;
            return Some(if virtual_size == 0 {
                raw_size
            } else {
                virtual_size
            });
        }
    }
    None
}

fn c_string_at(b: &[u8], off: usize) -> Option<&str> {
    let rest = b.get(off..)?;
    let end = rest.iter().position(|&c| c == 0)?;
    std::str::from_utf8(&rest[..end]).ok()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Shared lexing helpers
// ---------------------------------------------------------------------------

fn is_ident_char(c: Option<char>) -> bool {
    matches!(c, Some(c) if c.is_alphanumeric() || c == '_')
}

fn raw_string_at(chars: &[char], i: usize) -> Option<(usize, usize, usize)> {
    let mut j = i + 1;
    let mut hashes = 0usize;
    while j < chars.len() && chars[j] == '#' {
        hashes += 1;
        j += 1;
    }
    if j >= chars.len() || chars[j] != '"' {
        return None;
    }
    let body_start = j + 1;
    let mut k = body_start;
    while k < chars.len() {
        if chars[k] == '"' {
            let mut closing = 0usize;
            while closing < hashes && k + 1 + closing < chars.len() && chars[k + 1 + closing] == '#'
            {
                closing += 1;
            }
            if closing == hashes {
                return Some((body_start, k, k + 1 + hashes));
            }
        }
        k += 1;
    }
    Some((body_start, chars.len(), chars.len()))
}

fn char_literal_end(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < chars.len() && chars[j] == '\\' {
        j += 2;
    } else if j < chars.len() {
        j += 1;
    }
    if j < chars.len() && chars[j] == '\'' {
        Some(j + 1)
    } else {
        None
    }
}

fn in_any(spans: &[(usize, usize)], index: usize) -> bool {
    spans
        .binary_search_by(|&(start, end)| {
            if index < start {
                std::cmp::Ordering::Greater
            } else if index >= end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn error_call_spans(chars: &[char]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for head in ERROR_CALL_HEADS {
        let pattern: Vec<char> = head.chars().collect();
        let mut i = 0;
        while i + pattern.len() <= chars.len() {
            if chars[i..i + pattern.len()] == pattern[..] {
                if let Some(span) = call_parens(chars, i + pattern.len()) {
                    spans.push(span);
                }
                i += pattern.len();
            } else {
                i += 1;
            }
        }
    }
    merge(spans)
}

fn call_parens(chars: &[char], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
        i += 1;
    }
    if i >= chars.len() || chars[i] != '(' {
        return None;
    }
    let open = i;
    let mut depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, i + 1));
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == '"' {
                        break;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn cfg_test_spans(chars: &[char]) -> Vec<(usize, usize)> {
    let needles: [Vec<char>; 2] = [
        "#[cfg(test)]".chars().collect(),
        "#[cfg(all(test".chars().collect(),
    ];
    let mut spans = Vec::new();
    for needle in &needles {
        let mut i = 0;
        while i + needle.len() <= chars.len() {
            if chars[i..i + needle.len()] == needle[..] {
                if let Some(end) = block_after(chars, i) {
                    spans.push((i, end));
                }
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
    merge(spans)
}

fn block_after(chars: &[char], from: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() && chars[i] != '{' {
        if chars[i] == ';' {
            return None;
        }
        i += 1;
    }
    let mut depth = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn merge(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 <= last.1 => last.1 = last.1.max(span.1),
            _ => merged.push(span),
        }
    }
    merged
}

// ---------------------------------------------------------------------------
// Driving the build
// ---------------------------------------------------------------------------

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(&crate_dir().join("src"), &mut files);
    files.sort();
    files
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Builds an example in release, in a target directory of its own so the nested
/// build does not wait on the lock the outer `cargo test` is holding.
fn build_artifact() -> PathBuf {
    let workspace = crate_dir().parent().expect("workspace dir").to_path_buf();
    let target = workspace.join("target").join("prose-ratio");
    let status = Command::new(env!("CARGO"))
        .current_dir(&workspace)
        .args([
            "build",
            "--release",
            "--all-features",
            "-p",
            "draco-core",
            "--example",
            "debug_cube_v11",
            "--target-dir",
        ])
        .arg(&target)
        .status()
        .expect("run cargo");
    assert!(status.success(), "nested release build failed");

    let dir = target.join("release").join("examples");
    let entries = fs::read_dir(&dir).expect("examples dir");
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let is_artifact = name.starts_with("debug_cube_v11")
            && path
                .extension()
                .is_none_or(|e| e == "exe");
        if is_artifact && path.is_file() {
            return path;
        }
    }
    panic!("no debug_cube_v11 artifact under {}", dir.display());
}

#[test]
fn error_prose_stays_a_small_part_of_the_compiled_code() {
    let files = source_files();
    assert!(
        files.len() > 50,
        "found only {} source files: the walk is wrong, not the crate",
        files.len()
    );

    let mut all: BTreeSet<String> = BTreeSet::new();
    for path in &files {
        let source = fs::read_to_string(path).expect("read source");
        for literal in error_literals(&source) {
            all.extend(fragments(&literal));
        }
    }
    assert!(
        all.len() > 100,
        "only {} message fragments found, so the scanner is broken rather than \
         the crate being terse",
        all.len()
    );

    let artifact = build_artifact();
    let binary = fs::read(&artifact).expect("read artifact");
    let text = text_section_size(&binary).unwrap_or_else(|| {
        panic!(
            "no .text section in {} ({} bytes)",
            artifact.display(),
            binary.len()
        )
    });

    let mut shipped = 0usize;
    let mut missing: Vec<&str> = Vec::new();
    for fragment in &all {
        if contains(&binary, fragment.as_bytes()) {
            shipped += fragment.len();
        } else {
            missing.push(fragment);
        }
    }
    // Named rather than counted: a fragment can be absent because the optimiser
    // dropped its branch, or because this file mangled it. Only reading them
    // tells the two apart, and the second is a bug here.
    let sample: Vec<String> = missing
        .iter()
        .take(5)
        .map(|f| format!("{f:?}"))
        .collect();

    let ratio = shipped as f64 / text as f64;
    let report = format!(
        "artifact {} ({} bytes)\n\
         .text {text} bytes, error prose {shipped} bytes -> {:.3}% of the machine code\n\
         {} distinct fragments, {} of them not in the artifact:\n  {}\n\
         (was {:.3}% at 2.0.0, ceiling {:.3}%)",
        artifact.file_name().unwrap_or_default().to_string_lossy(),
        binary.len(),
        ratio * 100.0,
        all.len(),
        missing.len(),
        sample.join("\n  "),
        RATIO_AT_2_0_0 * 100.0,
        MAX_RATIO * 100.0,
    );
    eprintln!("{report}");

    assert!(shipped > 0, "no message reached the artifact\n{report}");
    assert!(ratio <= MAX_RATIO, "error prose grew past the ceiling\n{report}");
}

/// The two readers are what is most likely to be wrong, so they are pinned on
/// input whose answer can be counted by hand.
#[cfg(test)]
mod readers {
    use super::{error_literals, fragments, text_section_size};

    #[test]
    fn a_message_inside_an_error_call_is_found() {
        let found = error_literals(r#"fn f() { Err(DracoError::general("buffer ran out")) }"#);
        assert_eq!(found, vec!["buffer ran out".to_string()]);
    }

    #[test]
    fn a_string_outside_an_error_call_is_not() {
        assert!(error_literals(r#"fn f() { let name = "buffer ran out"; }"#).is_empty());
    }

    #[test]
    fn a_format_argument_inside_an_error_call_is_found() {
        let found = error_literals(r#"fn f() { DracoError::buffer(format!("ran out at {i}")) }"#);
        assert_eq!(found, vec!["ran out at {i}".to_string()]);
    }

    #[test]
    fn a_double_slash_inside_a_string_does_not_open_a_comment() {
        let found = error_literals(r#"fn f() { DracoError::general("see http://x for why"); }"#);
        assert_eq!(found, vec!["see http://x for why".to_string()]);
    }

    #[test]
    fn a_quote_inside_a_comment_does_not_open_a_string() {
        assert!(error_literals("// a \" quote\nfn f() { let y = 1; }").is_empty());
    }

    #[test]
    fn a_test_module_is_skipped() {
        let found = error_literals(
            r#"fn f() {}
#[cfg(test)]
mod tests {
    fn g() { DracoError::general("a message only tests can reach"); }
}"#,
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_paren_inside_a_message_does_not_end_the_call_early() {
        let found = error_literals(r#"fn f() { DracoError::general("bits (2..=30) exceeded"); }"#);
        assert_eq!(found, vec!["bits (2..=30) exceeded".to_string()]);
    }

    #[test]
    fn a_placeholder_splits_a_message_the_way_rustc_stores_it() {
        assert_eq!(
            fragments("Symbol count {n} is not a multiple of {components} components"),
            vec![
                "Symbol count ".to_string(),
                " is not a multiple of ".to_string(),
                " components".to_string(),
            ]
        );
    }

    #[test]
    fn a_short_fragment_is_dropped_as_noise() {
        // " of " would match anywhere in the artifact.
        assert_eq!(fragments("{a} of {b}"), Vec::<String>::new());
    }

    #[test]
    fn an_escaped_brace_is_not_a_placeholder() {
        assert_eq!(
            fragments("interface {{}} is not a type here"),
            vec!["interface {} is not a type here".to_string()]
        );
    }

    #[test]
    fn a_line_continuation_closes_up_the_indentation() {
        assert_eq!(
            fragments("a message split \\\n                 across lines"),
            vec!["a message split across lines".to_string()]
        );
    }

    #[test]
    fn the_running_test_binary_has_a_text_section() {
        // Whatever this platform is, the reader must handle the file it is
        // running as -- otherwise the measurement below is silently skipped.
        let self_path = std::env::current_exe().expect("current exe");
        let bytes = std::fs::read(self_path).expect("read self");
        let size = text_section_size(&bytes).expect("no .text section found in the test binary");
        assert!(size > 4096, "implausible .text size {size}");
    }
}
