//! Times the decoding stages on the submissions given on the command line, on one thread:
//! print decoding + inflating, the lazy decode the extractor uses, resolving every lazy value
//! afterwards, and a full eager decode for comparison.
//!
//! cargo run --release -p trace-data --example decode_bench -- <submission.json>...

use std::time::{Duration, Instant};
use trace_data::value::{decode_trace, decode_value, Arena, Value};
use trace_data::Inflater;

fn touch(value: &Value) -> usize {
    match value.resolved() {
        Value::Array(items) => items.iter().map(touch).sum::<usize>() + 1,
        Value::Object(object) => object.entries().map(|(_, v)| touch(v)).sum::<usize>() + 1,
        _ => 1,
    }
}

fn main() {
    let mut inflater = Inflater::new();
    let mut arena = Arena::new();
    let (mut inflate, mut lazy, mut resolve, mut eager) = (Duration::ZERO, Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let (mut bytes, mut nodes) = (0usize, 0usize);
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).expect("readable");
        let raw: serde_json::Value = serde_json::from_str(&text).expect("json");
        let Some(export_string) = raw["export_string"].as_str() else { continue };
        let started = Instant::now();
        let mut payloads = Vec::new();
        for export in trace_data::export::find_exports(export_string).unwrap_or_default() {
            if let trace_data::export::Export::Payload(p) = export {
                if let Ok(compressed) = trace_data::export::decode_for_print(p) {
                    if let Ok(inflated) = inflater.inflate(&compressed) {
                        payloads.push(inflated.bytes().to_vec());
                    }
                }
            }
        }
        inflate += started.elapsed();
        for cbor in &payloads {
            arena.reset();
            bytes += cbor.len();
            let started = Instant::now();
            let Ok(Some(root)) = decode_trace(cbor, &arena) else { continue };
            lazy += started.elapsed();
            let started = Instant::now();
            nodes += touch(&root);
            resolve += started.elapsed();
            arena.reset();
            let started = Instant::now();
            std::hint::black_box(decode_value(cbor, &arena).ok());
            eager += started.elapsed();
        }
    }
    let mb = bytes as f64 / 1e6;
    println!("{mb:.0} MB CBOR, {nodes} values");
    for (name, d) in [
        ("print-decode + inflate", inflate),
        ("lazy decode", lazy),
        ("resolve all lazies", resolve),
        ("eager full decode", eager),
    ] {
        println!("{name:<24} {:>6.2}s  {:>6.0} MB/s", d.as_secs_f64(), mb / d.as_secs_f64());
    }
}
