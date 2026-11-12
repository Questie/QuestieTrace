//! Compares two pipeline files by value, not by bytes: the gate between this pipeline and the
//! TypeScript one.
//!
//! cargo run --release -p relations --example compare -- EXPECTED ACTUAL [--tolerance 1e-9] [--max 20]
//!
//! - `.json`: one document. `.jsonl`: one record per line, matched by their `key` field when they
//!   have one (else by line), read one record at a time, so even the full archive's 5 GB
//!   episodes.jsonl compares in little memory. Anything else is compared as text, line by line.
//! - Object key order never matters; array order does (outputs are sorted).
//! - Numbers are equal when they differ by at most `tolerance`, relative to the larger one (or
//!   absolute below 1). `1` and `1.0` are equal.
//! - `generatedAt` keys are ignored, wherever they are; in text files, ISO 8601 timestamps are.
//!
//! Prints every difference (up to `--max`) with its path, and exits 1 when there are any.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::process::ExitCode;

struct Compare {
    tolerance: f64,
    max: usize,
    differences: usize,
}

impl Compare {
    fn report(&mut self, path: &str, message: String) {
        self.differences += 1;
        if self.differences <= self.max {
            println!("{path}: {message}");
        }
    }

    fn numbers_equal(&self, a: f64, b: f64) -> bool {
        a == b || (a - b).abs() <= self.tolerance * a.abs().max(b.abs()).max(1.0)
    }

    fn value(&mut self, path: &str, expected: &Value, actual: &Value) {
        match (expected, actual) {
            (Value::Number(a), Value::Number(b)) => {
                let (a, b) = (a.as_f64().unwrap_or(f64::NAN), b.as_f64().unwrap_or(f64::NAN));
                if !self.numbers_equal(a, b) {
                    self.report(path, format!("{a} != {b}"));
                }
            }
            (Value::Object(a), Value::Object(b)) => {
                for (key, value) in a {
                    if key == "generatedAt" {
                        continue;
                    }
                    match b.get(key) {
                        Some(other) => self.value(&format!("{path}.{key}"), value, other),
                        None => self.report(path, format!("missing key {key:?} (expected {})", short(value))),
                    }
                }
                for (key, value) in b {
                    if key != "generatedAt" && !a.contains_key(key) {
                        self.report(path, format!("extra key {key:?} = {}", short(value)));
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    self.report(path, format!("length {} != {}", a.len(), b.len()));
                }
                for (i, (x, y)) in a.iter().zip(b).enumerate() {
                    self.value(&format!("{path}[{i}]"), x, y);
                }
            }
            (a, b) if a == b => {}
            (a, b) => self.report(path, format!("{} != {}", short(a), short(b))),
        }
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 120 {
        format!("{}…", text.chars().take(120).collect::<String>())
    } else {
        text
    }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("reading {path}: {error}"))
}

/// Byte offset and key of every record of a JSONL file, without keeping the records.
fn jsonl_index(path: &str) -> Vec<(u64, Option<String>)> {
    #[derive(serde::Deserialize)]
    struct Keyed {
        key: Option<String>,
    }
    let mut reader = BufReader::new(File::open(path).unwrap_or_else(|error| panic!("reading {path}: {error}")));
    let (mut index, mut offset, mut line) = (Vec::new(), 0u64, String::new());
    loop {
        line.clear();
        let read = reader.read_line(&mut line).unwrap_or_else(|error| panic!("reading {path}: {error}"));
        if read == 0 {
            return index;
        }
        if !line.trim().is_empty() {
            let keyed: Keyed = serde_json::from_str(&line).unwrap_or_else(|error| panic!("{path} at byte {offset}: {error}"));
            index.push((offset, keyed.key));
        }
        offset += read as u64;
    }
}

/// The record at `offset`.
fn jsonl_record(file: &mut BufReader<File>, offset: u64) -> Value {
    file.seek(SeekFrom::Start(offset)).expect("seekable file");
    let mut line = String::new();
    file.read_line(&mut line).expect("a readable line");
    serde_json::from_str(&line).expect("a record read once already")
}

/// Compares two JSONL files record by record, matched by `key` when every record has one.
fn compare_jsonl(compare: &mut Compare, expected: &str, actual: &str) {
    let (a, b) = (jsonl_index(expected), jsonl_index(actual));
    let open = |path: &str| BufReader::new(File::open(path).expect("opened once already"));
    let (mut a_file, mut b_file) = (open(expected), open(actual));
    let keyed =
        |index: &[(u64, Option<String>)]| -> Option<BTreeMap<String, u64>> { index.iter().map(|(offset, key)| Some((key.clone()?, *offset))).collect() };
    match (keyed(&a), keyed(&b)) {
        (Some(a_keys), Some(b_keys)) => {
            println!("{} vs {} records, matched by key", a_keys.len(), b_keys.len());
            for (key, offset) in &a_keys {
                match b_keys.get(key) {
                    Some(other) => compare.value(&format!("[{key}]"), &jsonl_record(&mut a_file, *offset), &jsonl_record(&mut b_file, *other)),
                    None => compare.report(&format!("[{key}]"), "missing record".into()),
                }
            }
            for key in b_keys.keys().filter(|key| !a_keys.contains_key(*key)) {
                compare.report(&format!("[{key}]"), "extra record".into());
            }
        }
        _ => {
            if a.len() != b.len() {
                compare.report("", format!("{} != {} records", a.len(), b.len()));
            }
            for (line, ((a_offset, _), (b_offset, _))) in a.iter().zip(&b).enumerate() {
                compare.value(
                    &format!("[line {}]", line + 1),
                    &jsonl_record(&mut a_file, *a_offset),
                    &jsonl_record(&mut b_file, *b_offset),
                );
            }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(|v| v.parse::<f64>().expect("a number"))
    };
    let [expected, actual] = [&args[0], &args[1]];
    let mut compare = Compare {
        tolerance: option("--tolerance").unwrap_or(1e-9),
        max: option("--max").map_or(20, |n| n as usize),
        differences: 0,
    };
    if expected.ends_with(".jsonl") {
        compare_jsonl(&mut compare, expected, actual);
    } else if expected.ends_with(".json") {
        let parse = |path: &str| -> Value { serde_json::from_str(&read(path)).unwrap_or_else(|error| panic!("{path}: {error}")) };
        compare.value("", &parse(expected), &parse(actual));
    } else {
        // Generation times are the only expected difference in text outputs.
        let timestamp = regex::Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z").expect("a valid pattern");
        let mask = |text: String| timestamp.replace_all(&text, "<timestamp>").into_owned();
        let (a, b) = (mask(read(expected)), mask(read(actual)));
        let (a, b): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
        if a.len() != b.len() {
            compare.report("lines", format!("{} != {}", a.len(), b.len()));
        }
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            if x != y {
                compare.report(&format!("line {}", i + 1), format!("\n  - {x}\n  + {y}"));
            }
        }
    }
    if compare.differences == 0 {
        println!("same values: {expected} == {actual}");
        ExitCode::SUCCESS
    } else {
        println!("{} difference(s)", compare.differences);
        ExitCode::FAILURE
    }
}
