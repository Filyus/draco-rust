//! The rows of one run, as the tables `PERFORMANCE.md` carries.
//!
//! Everything here is read back from the run's directory, so a report can be
//! rewritten from an earlier run's rows with `--report <dir>`.

use crate::json::{self, Fields, Row};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub fn write(dir: &Path) -> Result<PathBuf, String> {
    let run = Run::load(dir)?;
    let text = render(&run);
    let path = dir.join("report.md");
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

struct Run {
    meta: Row,
    steps: Vec<Row>,
    /// Every row of every step, with the step it came from.
    rows: Vec<(String, Row)>,
}

fn read_rows(path: &Path) -> Result<Vec<Row>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            json::parse_line(line)
                .map_err(|error| format!("{}:{}: {error}", path.display(), index + 1))
        })
        .collect()
}

impl Run {
    fn load(dir: &Path) -> Result<Run, String> {
        let meta = read_rows(&dir.join("meta.json"))?
            .into_iter()
            .next()
            .ok_or_else(|| format!("{} holds no meta.json", dir.display()))?;
        let steps = read_rows(&dir.join("steps.jsonl"))?;
        let mut rows = Vec::new();
        for step in &steps {
            let name = step.text("step");
            for row in read_rows(&dir.join(format!("{name}.jsonl")))? {
                rows.push((name.clone(), row));
            }
        }
        Ok(Run { meta, steps, rows })
    }

    fn table(&self, name: &str) -> Vec<&Row> {
        self.rows
            .iter()
            .filter(|(_, row)| row.text("table") == name)
            .map(|(_, row)| row)
            .collect()
    }
}

/// The distinct values of `key` over `rows`, in the order they first appear.
fn distinct<T: PartialEq>(rows: &[&Row], key: impl Fn(&Row) -> T) -> Vec<T> {
    let mut seen = Vec::new();
    for row in rows {
        let value = key(row);
        if !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen
}

/// Speeds are written as numbers; these keep them whole and sortable.
fn speed(row: &Row) -> i64 {
    row.number("speed") as i64
}

fn speeds(rows: &[&Row]) -> Vec<i64> {
    let mut speeds = distinct(rows, speed);
    speeds.sort_unstable();
    speeds
}

fn median(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut values: Vec<f64> = values.into_iter().filter(|v| v.is_finite()).collect();
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

fn grouped(digits: &str) -> String {
    let mut out = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A figure with thousands grouped and `decimals` places, `n/a` if missing.
fn figure(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "n/a".to_owned();
    }
    let text = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let sign = if value < 0.0 && text.chars().any(|c| matches!(c, '1'..='9')) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{}", grouped(whole))
    } else {
        format!("{sign}{}.{fraction}", grouped(whole))
    }
}

fn us(value: f64) -> String {
    figure(value, 1)
}

fn ratio(numerator: f64, denominator: f64) -> String {
    let value = numerator / denominator;
    if value.is_finite() {
        format!("`{value:.2}x`")
    } else {
        "n/a".to_owned()
    }
}

fn code(text: impl AsRef<str>) -> String {
    format!("`{}`", text.as_ref())
}

/// A Markdown table. Columns from `numeric_from` on are right-aligned.
fn markdown(out: &mut String, header: &[&str], rows: &[Vec<String>], numeric_from: usize) {
    let _ = writeln!(out, "| {} |", header.join(" | "));
    let rule: Vec<&str> = (0..header.len())
        .map(|column| if column < numeric_from { "---" } else { "---:" })
        .collect();
    let _ = writeln!(out, "| {} |", rule.join(" | "));
    for row in rows {
        let _ = writeln!(out, "| {} |", row.join(" | "));
    }
    out.push('\n');
}

fn render(run: &Run) -> String {
    let mut out = String::new();
    let mut checks = Vec::new();
    let meta = &run.meta;
    let commit = meta.text("commit");
    let short = &commit[..commit.len().min(8)];
    let uncommitted: usize = meta.text("uncommitted").parse().unwrap_or(0);

    let _ = writeln!(out, "# Performance Suite, {}\n", meta.text("date"));
    let _ = writeln!(
        out,
        "Measured {} at `{short}`{}. {}, {} threads, {}, {}.",
        meta.text("date"),
        if uncommitted > 0 {
            format!(", **with {uncommitted} uncommitted change(s)**")
        } else {
            String::new()
        },
        meta.text("cpu"),
        meta.text("threads"),
        meta.text("os"),
        meta.text("rustc"),
    );
    let _ = writeln!(
        out,
        "C++ Draco {} from `{}`, checked free of the `DRACO_VERBOSE` patch.",
        meta.text("cpp_version"),
        meta.text("cpp_library"),
    );
    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        if meta.contains_key(name) {
            let _ = writeln!(out, "`{name}` was `{}`.", meta.text(name));
        }
    }
    out.push_str(
        "\nThe steps ran one at a time. A ratio is C++ time over Rust time, so \
         above `1x` favors Rust, except in the two texture sections, which \
         compare our time against the reference's.\n\n",
    );

    let step_rows: Vec<Vec<String>> = run
        .steps
        .iter()
        .map(|step| {
            vec![
                step.text("step"),
                step.text("status"),
                step.text("rows"),
                format!("{} s", step.text("seconds")),
                step.text("note"),
            ]
        })
        .collect();
    markdown(
        &mut out,
        &["step", "status", "rows", "time", "note"],
        &step_rows,
        2,
    );
    for step in &run.steps {
        if step.text("status") == "failed" {
            checks.push(format!(
                "step `{}` failed: {}",
                step.text("step"),
                step.text("note")
            ));
        }
    }

    let mut body = String::new();
    seeded_sweep(run, &mut body, &mut checks);
    model_matrix(run, &mut body, &mut checks);
    real_models(run, &mut body);
    grid_decode(run, &mut body);
    grid_encode(run, &mut body, &mut checks);
    encode_decode_matrix(run, &mut body, &mut checks);
    ktx2(run, &mut body);
    zstd(run, &mut body);

    out.push_str("## Checks\n\n");
    if checks.is_empty() {
        out.push_str(
            "Every step finished, and every cell that compares outputs found them equal.\n\n",
        );
    } else {
        for check in &checks {
            let _ = writeln!(out, "- {check}");
        }
        out.push('\n');
    }
    out.push_str(&body);
    out
}

fn no_rows(out: &mut String) {
    out.push_str("No rows in this run.\n\n");
}

fn seeded_sweep(run: &Run, out: &mut String, checks: &mut Vec<String>) {
    out.push_str("## Seeded Synthetic Sweep\n\n");
    let rows: Vec<(&String, &Row)> = run
        .rows
        .iter()
        .filter(|(_, row)| row.text("table") == "seeded_sweep")
        .map(|(step, row)| (step, row))
        .collect();
    if rows.is_empty() {
        return no_rows(out);
    }
    let mut runs: Vec<&String> = Vec::new();
    for (step, _) in &rows {
        if !runs.contains(step) {
            runs.push(step);
        }
    }
    let all: Vec<&Row> = rows.iter().map(|(_, row)| *row).collect();
    let samples = distinct(&all, |row| row.text("seed")).len();
    let _ = writeln!(
        out,
        "Position-only, {samples} seeded meshes over {} families, `us` per 1,000 \
         faces: the median over the meshes of a run, then the median over {} \
         run(s).\n",
        distinct(&all, |row| row.text("family")).len(),
        runs.len(),
    );

    let per_k = |row: &Row, side: &str| row.number(side) / row.number("faces") * 1000.0;
    let cell = |operation: &str, speed_value: i64| -> (f64, f64) {
        let mut cpp = Vec::new();
        let mut rust = Vec::new();
        for step in &runs {
            let of_run: Vec<&Row> = rows
                .iter()
                .filter(|(name, row)| {
                    name == step && row.text("operation") == operation && speed(row) == speed_value
                })
                .map(|(_, row)| *row)
                .collect();
            cpp.push(median(of_run.iter().map(|row| per_k(row, "cpp_us"))));
            rust.push(median(of_run.iter().map(|row| per_k(row, "rust_us"))));
        }
        (median(cpp), median(rust))
    };
    let table: Vec<Vec<String>> = speeds(&all)
        .into_iter()
        .map(|speed_value| {
            let (encode_cpp, encode_rust) = cell("encode", speed_value);
            let (decode_cpp, decode_rust) = cell("decode", speed_value);
            vec![
                speed_value.to_string(),
                format!("{} / {}", code(us(encode_cpp)), code(us(encode_rust))),
                ratio(encode_cpp, encode_rust),
                format!("{} / {}", code(us(decode_cpp)), code(us(decode_rust))),
                ratio(decode_cpp, decode_rust),
            ]
        })
        .collect();
    markdown(
        out,
        &[
            "Speed",
            "Encode C++ / Rust",
            "Encode",
            "Decode C++ / Rust",
            "Decode",
        ],
        &table,
        0,
    );

    for (step, row) in &rows {
        for (flag, what) in [
            ("bytes_match", "encoded sizes differ"),
            ("shapes_match", "decoded counts differ"),
        ] {
            if row.flag(flag) == Some(false) {
                checks.push(format!(
                    "{step}: {} {} speed {} {}: {what}",
                    row.text("family"),
                    row.text("seed"),
                    speed(row),
                    row.text("operation"),
                ));
            }
        }
    }
}

fn model_matrix(run: &Run, out: &mut String, checks: &mut Vec<String>) {
    out.push_str("## Named Models, Both Operations, One Process\n\n");
    let rows = run.table("model_matrix");
    if rows.is_empty() {
        return no_rows(out);
    }
    let _ = writeln!(
        out,
        "`model_matrix`, microseconds: the median of its rounds with the range \
         across them, {}-bit positions.\n",
        rows[0].text("qp"),
    );
    let spread = |row: &Row, key: &str| {
        code(format!(
            "{} [{}..{}]",
            us(row.number(&format!("{key}_us"))),
            us(row.number(&format!("{key}_min"))),
            us(row.number(&format!("{key}_max"))),
        ))
    };
    for speed_value in distinct(&rows, speed) {
        let _ = writeln!(out, "Speed {speed_value}:\n");
        let table: Vec<Vec<String>> = rows
            .iter()
            .filter(|row| speed(row) == speed_value)
            .map(|row| {
                vec![
                    row.text("model"),
                    figure(row.number("faces"), 0),
                    spread(row, "cpp_enc"),
                    spread(row, "rust_enc"),
                    ratio(row.number("cpp_enc_us"), row.number("rust_enc_us")),
                    spread(row, "cpp_dec"),
                    spread(row, "rust_dec"),
                    ratio(row.number("cpp_dec_us"), row.number("rust_dec_us")),
                ]
            })
            .collect();
        markdown(
            out,
            &[
                "model",
                "faces",
                "C++ encode",
                "Rust encode",
                "",
                "C++ decode",
                "Rust decode",
                "",
            ],
            &table,
            1,
        );
        for row in rows.iter().filter(|row| speed(row) == speed_value) {
            if row.number("cpp_bytes") != row.number("rust_bytes") {
                checks.push(format!(
                    "model_matrix speed {speed_value}, {}: C++ wrote {} bytes and Rust {}",
                    row.text("model"),
                    figure(row.number("cpp_bytes"), 0),
                    figure(row.number("rust_bytes"), 0),
                ));
            }
        }
    }
}

/// One row per `label`, one ratio column per speed.
fn ratios_by_speed(
    rows: &[&Row],
    label: impl Fn(&Row) -> String,
    cpp: &str,
    rust: &str,
) -> (Vec<i64>, Vec<Vec<String>>) {
    let speed_list = speeds(rows);
    let table = distinct(rows, &label)
        .into_iter()
        .map(|name| {
            let mine: Vec<&&Row> = rows.iter().filter(|row| label(row) == name).collect();
            let mut line = vec![name, figure(mine[0].number("faces"), 0)];
            for &speed_value in &speed_list {
                line.push(
                    mine.iter()
                        .find(|row| speed(row) == speed_value)
                        .map_or("".to_owned(), |row| {
                            ratio(row.number(cpp), row.number(rust))
                        }),
                );
            }
            line
        })
        .collect();
    (speed_list, table)
}

fn speed_header<'a>(
    first: &[&'a str],
    speeds: &[i64],
    storage: &'a mut Vec<String>,
) -> Vec<&'a str> {
    *storage = speeds.iter().map(|speed| format!("@{speed}")).collect();
    let mut header = first.to_vec();
    header.extend(storage.iter().map(String::as_str));
    header
}

fn real_models(run: &Run, out: &mut String) {
    out.push_str("## Real Models, Compress Then Decompress, Every Speed\n\n");
    let rows = run.table("real_models");
    if rows.is_empty() {
        return no_rows(out);
    }
    for (title, cpp, rust) in [
        ("Encode", "enc_cpp_us", "enc_rust_us"),
        ("Decode", "dec_cpp_us", "dec_rust_us"),
    ] {
        let _ = writeln!(out, "{title}, C++/Rust:\n");
        let (speed_list, table) = ratios_by_speed(&rows, |row| row.text("model"), cpp, rust);
        let mut storage = Vec::new();
        let header = speed_header(&["model", "faces"], &speed_list, &mut storage);
        markdown(out, &header, &table, 1);
    }
}

fn grid_label(row: &Row) -> String {
    let grid = row.number("grid") as i64;
    format!("{grid}x{grid}")
}

fn grid_decode(run: &Run, out: &mut String) {
    out.push_str("## Decode Through The C++ Bridge\n\n");
    let rows = run.table("grid_decode");
    if rows.is_empty() {
        return no_rows(out);
    }
    out.push_str("Synthetic grids, median per-iteration over batches, C++/Rust:\n\n");
    let (speed_list, mut table) = ratios_by_speed(&rows, grid_label, "cpp_us", "rust_us");
    let totals = run.table("grid_decode_total");
    for line in &mut table {
        let total = totals.iter().find(|row| grid_label(row) == line[0]);
        line.push(total.map_or("".to_owned(), |row| {
            ratio(row.number("cpp_us"), row.number("rust_us"))
        }));
    }
    let mut storage = Vec::new();
    let mut header = speed_header(&["grid", "faces"], &speed_list, &mut storage);
    header.push("overall");
    markdown(out, &header, &table, 1);
}

fn grid_encode(run: &Run, out: &mut String, checks: &mut Vec<String>) {
    out.push_str("## Encode Through The C++ Bridge\n\n");
    let rows = run.table("grid_encode");
    if rows.is_empty() {
        return no_rows(out);
    }
    out.push_str("Synthetic grids, averaged over the harness's iterations, C++/Rust:\n\n");
    let (speed_list, table) = ratios_by_speed(&rows, grid_label, "cpp_us", "rust_us");
    let mut storage = Vec::new();
    let header = speed_header(&["grid", "faces"], &speed_list, &mut storage);
    markdown(out, &header, &table, 1);
    for row in &rows {
        if row.number("cpp_bytes") != row.number("rust_bytes") {
            checks.push(format!(
                "grid_encode {} speed {}: C++ wrote {} bytes and Rust {}",
                grid_label(row),
                speed(row),
                figure(row.number("cpp_bytes"), 0),
                figure(row.number("rust_bytes"), 0),
            ));
        }
    }
}

fn encode_decode_matrix(run: &Run, out: &mut String, checks: &mut Vec<String>) {
    out.push_str("## Encode/Decode Matrix\n\n");
    let rows = run.table("encode_decode_matrix");
    if rows.is_empty() {
        return no_rows(out);
    }
    out.push_str("Both tests of the harness, C++/Rust:\n\n");
    let speed_list = speeds(&rows);
    let mut table = Vec::new();
    for mesh in distinct(&rows, |row| row.text("mesh")) {
        for operation in ["encode", "decode"] {
            let mine: Vec<&&Row> = rows
                .iter()
                .filter(|row| row.text("mesh") == mesh && row.text("operation") == operation)
                .collect();
            if mine.is_empty() {
                continue;
            }
            let mut line = vec![mesh.clone(), operation.to_owned()];
            for &speed_value in &speed_list {
                line.push(
                    mine.iter()
                        .find(|row| speed(row) == speed_value)
                        .map_or("".to_owned(), |row| {
                            ratio(row.number("cpp_us"), row.number("rust_us"))
                        }),
                );
            }
            table.push(line);
        }
    }
    let mut storage = Vec::new();
    let header = speed_header(&["mesh", "operation"], &speed_list, &mut storage);
    markdown(out, &header, &table, 2);
    for row in &rows {
        for (flag, what) in [
            ("bytes_match", "encoded sizes differ"),
            ("shapes_match", "decoded counts differ"),
        ] {
            if row.flag(flag) == Some(false) {
                checks.push(format!(
                    "encode_decode_matrix {} {} speed {}: {what}",
                    row.text("mesh"),
                    row.text("operation"),
                    speed(row),
                ));
            }
        }
    }
}

fn ktx2(run: &Run, out: &mut String) {
    out.push_str("## KTX2 Transcode Against The Reference\n\n");
    let rows = run.table("ktx2_transcode");
    if rows.is_empty() {
        return no_rows(out);
    }
    out.push_str(
        "Best of seven rounds, microseconds per round, our time over the \
         reference's: below `1x` is faster than the reference.\n\n",
    );
    let (mut all_ours, mut all_reference) = (0.0, 0.0);
    for codec in distinct(&rows, |row| row.text("codec")) {
        let mine: Vec<&&Row> = rows
            .iter()
            .filter(|row| row.text("codec") == codec)
            .collect();
        let mut table: Vec<Vec<String>> = mine
            .iter()
            .map(|row| {
                vec![
                    row.text("target"),
                    code(us(row.number("ours_us"))),
                    code(us(row.number("reference_us"))),
                    ratio(row.number("ours_us"), row.number("reference_us")),
                ]
            })
            .collect();
        let ours: f64 = mine.iter().map(|row| row.number("ours_us")).sum();
        let reference: f64 = mine.iter().map(|row| row.number("reference_us")).sum();
        all_ours += ours;
        all_reference += reference;
        table.push(vec![
            "all targets".to_owned(),
            code(us(ours)),
            code(us(reference)),
            ratio(ours, reference),
        ]);
        markdown(out, &[&codec, "ours", "reference", "ratio"], &table, 1);
    }
    let _ = writeln!(
        out,
        "Both codecs: {} against {}, {}.\n",
        code(us(all_ours)),
        code(us(all_reference)),
        ratio(all_ours, all_reference),
    );
}

fn zstd(run: &Run, out: &mut String) {
    out.push_str("## Zstd Decompression Against C zstd\n\n");
    let rows = run.table("zstd");
    if rows.is_empty() {
        return no_rows(out);
    }
    out.push_str("Best of seven rounds, microseconds, our time over C zstd's.\n\n");
    let mut table: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.text("fixture"),
                figure(row.number("bytes"), 0),
                code(us(row.number("ours_us"))),
                code(us(row.number("c_us"))),
                ratio(row.number("ours_us"), row.number("c_us")),
            ]
        })
        .collect();
    let sum = |key: &str| rows.iter().map(|row| row.number(key)).sum::<f64>();
    table.push(vec![
        "all".to_owned(),
        figure(sum("bytes"), 0),
        code(us(sum("ours_us"))),
        code(us(sum("c_us"))),
        ratio(sum("ours_us"), sum("c_us")),
    ]);
    markdown(
        out,
        &["fixture", "bytes out", "ours", "C zstd", "ratio"],
        &table,
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_group_thousands_and_keep_their_sign() {
        assert_eq!(figure(20_258.04, 1), "20,258.0");
        assert_eq!(figure(1_234_567.0, 0), "1,234,567");
        assert_eq!(figure(999.95, 1), "1,000.0");
        assert_eq!(figure(-1_500.0, 0), "-1,500");
        assert_eq!(figure(-0.01, 1), "0.0");
        assert_eq!(figure(f64::NAN, 1), "n/a");
    }

    #[test]
    fn medians_skip_what_is_missing() {
        assert_eq!(median([3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median([4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median([f64::NAN, 5.0]), 5.0);
        assert!(median([]).is_nan());
    }

    /// A run directory written the way the suite writes one, reported.
    #[test]
    fn a_run_becomes_a_report() {
        let dir = std::env::temp_dir().join(format!("perf-suite-report-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let meta = json::object(&[
            ("date", "2026-10-05".to_owned()),
            ("commit", "0123456789abcdef".to_owned()),
            ("uncommitted", "0".to_owned()),
        ]);
        std::fs::write(dir.join("meta.json"), meta + "\n").unwrap();
        let steps = [
            r#"{"step":"seeded_sweep_1","status":"ok","rows":"2","seconds":"1.0","note":""}"#,
            r#"{"step":"seeded_sweep_2","status":"ok","rows":"2","seconds":"1.0","note":""}"#,
            r#"{"step":"model_matrix_speed4","status":"ok","rows":"1","seconds":"1.0","note":""}"#,
            r#"{"step":"grid_encode","status":"failed","rows":"0","seconds":"1.0","note":"cargo exited with 101"}"#,
        ];
        std::fs::write(dir.join("steps.jsonl"), steps.join("\n") + "\n").unwrap();
        let sweep = |cpp: f64, rust: f64, matched: bool| {
            format!(
                r#"{{"table":"seeded_sweep","family":"grid","seed":"0x1","faces":2000,"operation":"encode","speed":5,"cpp_us":{cpp},"rust_us":{rust},"bytes_match":{matched}}}"#
            )
        };
        std::fs::write(
            dir.join("seeded_sweep_1.jsonl"),
            [sweep(800.0, 400.0, true), sweep(1000.0, 500.0, true)].join("\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("seeded_sweep_2.jsonl"),
            [sweep(600.0, 300.0, true), sweep(600.0, 300.0, false)].join("\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("model_matrix_speed4.jsonl"),
            r#"{"table":"model_matrix","model":"bunny","faces":69451,"speed":4,"qp":10,"cpp_enc_us":2000,"cpp_enc_min":1900,"cpp_enc_max":2100,"rust_enc_us":1000,"rust_enc_min":900,"rust_enc_max":1100,"cpp_dec_us":600,"cpp_dec_min":500,"cpp_dec_max":700,"rust_dec_us":400,"rust_dec_min":300,"rust_dec_max":500,"cpp_bytes":100,"rust_bytes":101}"#,
        )
        .unwrap();

        let report = std::fs::read_to_string(write(&dir).unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        // Run medians 450 and 300 us/1k faces C++, 225 and 150 Rust: the
        // median of the two runs is their mean.
        assert!(
            report.contains("| 5 | `375.0` / `187.5` | `2.00x` |"),
            "{report}"
        );
        assert!(report.contains("| bunny | 69,451 | `2,000.0 [1,900.0..2,100.0]`"));
        assert!(report
            .contains("`2.00x` | `600.0 [500.0..700.0]` | `400.0 [300.0..500.0]` | `1.50x` |"));
        assert!(report.contains("step `grid_encode` failed: cargo exited with 101"));
        assert!(report.contains("seeded_sweep_2: grid 0x1 speed 5 encode: encoded sizes differ"));
        assert!(report.contains("C++ wrote 100 bytes and Rust 101"));
        assert!(report.contains("## Zstd Decompression Against C zstd\n\nNo rows in this run."));
    }
}
