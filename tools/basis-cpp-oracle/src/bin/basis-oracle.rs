//! The vendored reference transcoder, talking to the node gates over stdio.
//!
//! This is K17, the in-tree oracle: the same vendored C++ the parity tests
//! link is reached from Node without a three.js checkout or an emscripten
//! toolchain, which is what lets the KTX2 gates run on any machine with cargo
//! instead of skipping.
//!
//! Two ways to run it. One-shot answers a single question and is what a
//! person reaches for by hand; `serve` — the default, and what the gates
//! spawn — reads requests from stdin until EOF, because the differential gate
//! asks thousands of questions and a process spawn per answer would cost more
//! than the transcodes.
//!
//! Requests, one per question:
//!
//! ```text
//! T <level> <target> <nbytes>\n<nbytes of raw KTX2>   transcode those bytes
//! L <path>\n                                          level count of a file
//! ```
//!
//! Answers are framed so the reader never parses text: one status byte
//! (`1` answered, `0` refused), then a u32 little-endian payload length, then
//! the payload. A transcode's payload is the block bytes; `L` answers with a
//! four-byte little-endian count. A refusal is a `0` frame, not a crash: the
//! malformed files the gates feed it on purpose are exactly what refuses.
//!
//! A `T` request's payload is always read before the answer is decided, so a
//! refusal leaves the stream aligned for the next question. The one thing that
//! cannot be realigned is a `T` header that does not parse — nothing then says
//! how many bytes follow — so the session refuses that request and ends rather
//! than answering later questions about the wrong bytes.

use std::io::{BufRead, Read, Write};
use std::process::ExitCode;

use basis_cpp_oracle::{level_count, transcode, without_zstd, Target};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // One-shot, for a person at a shell:
        //   basis-oracle transcode <path> <level> <target>
        //   basis-oracle levels <path>
        Some("transcode") if args.len() == 4 => {
            let level: Option<u32> = args[2].parse().ok();
            let target: Option<i32> = args[3].parse().ok();
            let answer = match (level, target) {
                (Some(level), Some(raw)) => {
                    target_from_i32(raw).and_then(|target| answer_transcode(&args[1], level, target))
                }
                _ => None,
            };
            match answer {
                Some(bytes) => {
                    let _ = std::io::stdout().write_all(&bytes);
                    ExitCode::SUCCESS
                }
                None => ExitCode::FAILURE,
            }
        }
        Some("levels") if args.len() == 2 => match answer_levels(&args[1]) {
            Some(count) => {
                let _ = std::io::stdout().write_all(&count.to_le_bytes());
                ExitCode::SUCCESS
            }
            None => ExitCode::FAILURE,
        },
        Some(other) => {
            eprintln!("unknown mode {other:?}; usage: basis-oracle [serve | transcode <path> <level> <target> | levels <path>]");
            ExitCode::FAILURE
        }
        _ => serve(),
    }
}

fn serve() -> ExitCode {
    let mut stdin = std::io::stdin().lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    loop {
        // One line of question, then whatever raw bytes that question says
        // follow. `read_line` rather than `lines` because the byte payload
        // after the newline is not UTF-8 and must not be buffered as text.
        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        let answer = match fields.as_slice() {
            ["T", level, target, nbytes] => match (level.parse::<u32>(), target.parse::<i32>(), nbytes.parse::<usize>()) {
                (Ok(level), Ok(raw), Ok(nbytes)) => {
                    // The payload is consumed whatever the answer turns out to
                    // be, including for a target this build has no name for:
                    // leaving those bytes in the pipe would make the next
                    // request start mid-file, and every later answer would be
                    // about something nobody asked.
                    match read_payload(&mut stdin, nbytes) {
                        Some(original) => target_from_i32(raw)
                            .and_then(|target| answer_transcode_bytes(&original, level, target))
                            .map(|bytes| frame(1, &bytes))
                            .unwrap_or_else(refused),
                        None => refused(),
                    }
                }
                // A header that does not parse says nothing about how many
                // bytes follow, so the session cannot be realigned: the
                // refusal is the last honest answer this process can give.
                _ => {
                    let _ = out.write_all(&refused());
                    let _ = out.flush();
                    break;
                }
            },
            // Split once rather than on every space: the path is the rest of
            // the line, and a checkout directory may well hold a space.
            ["L", ..] => {
                let path = line[1..].trim();
                answer_levels(path)
                    .map(|count| frame(1, &count.to_le_bytes()))
                    .unwrap_or_else(refused)
            }
            _ => refused(),
        };
        // The reader blocks on these bytes, so a half-written frame would
        // hang it rather than fail it: flush before the next request.
        if out.write_all(&answer).is_err() || out.flush().is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}

/// Resolve the target enum, so a bad number is a refusal rather than a panic.
fn target_from_i32(raw: i32) -> Option<Target> {
    Some(match raw {
        0 => Target::Etc1Rgb,
        1 => Target::Etc2Rgba,
        2 => Target::Bc1Rgb,
        3 => Target::Bc3Rgba,
        4 => Target::Bc4R,
        5 => Target::Bc5Rg,
        6 => Target::Bc7Rgba,
        10 => Target::Astc4x4Rgba,
        13 => Target::Rgba32,
        20 => Target::Etc2EacR11,
        21 => Target::Etc2EacRg11,
        _ => return None,
    })
}

/// One framed answer: status byte, u32 little-endian length, payload.
fn frame(status: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(status);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn refused() -> Vec<u8> {
    frame(0, &[])
}

/// Read the `nbytes` of KTX2 a request announced.
///
/// Read before anything else is decided, so a refused request still consumes
/// its bytes and the next question starts on a clean boundary. The buffer is
/// grown from what arrives rather than from the announced count: the count is
/// the sender's word, and a mistyped one should cost a refusal, not the
/// process.
fn read_payload(stdin: &mut impl Read, nbytes: usize) -> Option<Vec<u8>> {
    let mut original = Vec::new();
    let read = stdin.take(nbytes as u64).read_to_end(&mut original).ok()?;
    (read == nbytes).then_some(original)
}

/// Transcode one level of the request's bytes, or refuse.
///
/// Zstd is undone here — the vendored build has none, and the bytes the gates
/// hand over are exactly what a file or a mutant carries.
fn answer_transcode_bytes(original: &[u8], level: u32, target: Target) -> Option<Vec<u8>> {
    let plain = without_zstd(original)?;
    transcode(&plain, level, target)
}

/// One-shot transcode straight from a file.
fn answer_transcode(path: &str, level: u32, target: Target) -> Option<Vec<u8>> {
    let original = std::fs::read(path).ok()?;
    let plain = without_zstd(&original)?;
    transcode(&plain, level, target)
}

fn answer_levels(path: &str) -> Option<u32> {
    let original = std::fs::read(path).ok()?;
    Some(level_count(&original))
}
