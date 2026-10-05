//! Machine-readable copies of the benchmark harnesses' result rows.
//!
//! Every harness whose figures `PERFORMANCE.md` quotes prints its rows for a
//! reader. With `PERF_JSONL` set to a file path it also appends each row to
//! that file as one JSON object a line, which is what `tools/perf-suite` reads
//! to write its report. Nothing is measured differently, and the printed
//! tables stay the same.

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Mutex;

/// One value of a recorded row.
pub enum Value {
    Text(String),
    Number(f64),
    Integer(i64),
    Flag(bool),
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Text(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Text(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Number(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Integer(value)
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::Integer(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Value::Integer(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Value::Integer(i64::from(value))
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value::Integer(i64::from(value))
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Flag(value)
    }
}

/// Tests in one binary run side by side, and a row has to land in one piece.
static WRITER: Mutex<()> = Mutex::new(());

/// Whether rows are being recorded, so a harness can skip building them.
pub fn enabled() -> bool {
    std::env::var_os("PERF_JSONL").is_some()
}

/// Appends one row of `table` to the file `PERF_JSONL` names, if it names one.
///
/// A row that cannot be written is reported on stderr and otherwise ignored:
/// the printed table is still there, and a benchmark should not fail because
/// its copy could not be kept.
pub fn record(table: &str, fields: Vec<(&str, Value)>) {
    let Some(path) = std::env::var_os("PERF_JSONL") else {
        return;
    };
    let mut line = String::from("{\"table\":");
    push_string(&mut line, table);
    for (key, value) in fields {
        line.push(',');
        push_string(&mut line, key);
        line.push(':');
        match value {
            Value::Text(text) => push_string(&mut line, &text),
            Value::Number(number) if number.is_finite() => {
                let _ = write!(line, "{number}");
            }
            Value::Number(_) => line.push_str("null"),
            Value::Integer(integer) => {
                let _ = write!(line, "{integer}");
            }
            Value::Flag(flag) => line.push_str(if flag { "true" } else { "false" }),
        }
    }
    line.push_str("}\n");

    let _guard = WRITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(error) = written {
        eprintln!("perf_record: could not append to {path:?}: {error}");
    }
}

fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_escaped_as_json_requires() {
        let mut out = String::new();
        push_string(&mut out, "a\"b\\c\nd\u{1}µs");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001µs\"");
    }
}
