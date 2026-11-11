//! Compares two pipeline files by value, not by bytes: the gate between this pipeline and the
//! TypeScript one.
//!
//! cargo run --release -p relations --example compare -- EXPECTED ACTUAL [--tolerance 1e-9] [--max 20]
//!
//! - `.json`: one document. `.jsonl`: one record per line, matched by their `key` field when they
//!   have one (else by line). Anything else is compared as text, line by line.
//! - Object key order never matters; array order does (outputs are sorted).
//! - Numbers are equal when they differ by at most `tolerance`, relative to the larger one (or
//!   absolute below 1). `1` and `1.0` are equal.
//! - `generatedAt` keys are ignored, wherever they are.
//!
//! Prints every difference (up to `--max`) with its path, and exits 1 when there are any.

use serde_json::Value;
use std::collections::BTreeMap;
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

fn jsonl(path: &str) -> Vec<Value> {
    read(path)
        .lines()
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(i, line)| serde_json::from_str(line).unwrap_or_else(|error| panic!("{path} line {}: {error}", i + 1)))
        .collect()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|v| v.parse::<f64>().expect("a number"));
    let [expected, actual] = [&args[0], &args[1]];
    let mut compare = Compare {
        tolerance: option("--tolerance").unwrap_or(1e-9),
        max: option("--max").map_or(20, |n| n as usize),
        differences: 0,
    };
    if expected.ends_with(".jsonl") {
        let (a, b) = (jsonl(expected), jsonl(actual));
        let keyed = |records: &[Value]| -> Option<BTreeMap<String, Value>> {
            records.iter().map(|r| Some((r.get("key")?.as_str()?.to_owned(), r.clone()))).collect()
        };
        match (keyed(&a), keyed(&b)) {
            (Some(a), Some(b)) => {
                println!("{} vs {} records, matched by key", a.len(), b.len());
                for (key, record) in &a {
                    match b.get(key) {
                        Some(other) => compare.value(&format!("[{key}]"), record, other),
                        None => compare.report(&format!("[{key}]"), "missing record".into()),
                    }
                }
                for key in b.keys().filter(|key| !a.contains_key(*key)) {
                    compare.report(&format!("[{key}]"), "extra record".into());
                }
            }
            _ => compare.value("", &Value::Array(a), &Value::Array(b)),
        }
    } else if expected.ends_with(".json") {
        let parse = |path: &str| -> Value { serde_json::from_str(&read(path)).unwrap_or_else(|error| panic!("{path}: {error}")) };
        compare.value("", &parse(expected), &parse(actual));
    } else {
        let (a, b) = (read(expected), read(actual));
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
