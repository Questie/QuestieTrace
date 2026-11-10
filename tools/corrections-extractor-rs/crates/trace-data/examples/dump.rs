//! Prints each submission given on the command line as one JSON line, exactly as
//! `JSON.stringify` prints the TypeScript decoder's result, so the two decoders can be diffed:
//! `{"traces":[...],"dropped":[...],"error":"..."}`.

use std::fmt::Write;
use trace_data::submission::Submission;
use trace_data::{format_number, Inflater, Value};

fn json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_value(out: &mut String, value: &Value) {
    match value.resolved() {
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) if n.is_finite() => out.push_str(&format_number(*n)),
        Value::Num(_) => out.push_str("null"),
        Value::Str(s) => json_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_value(out, item);
            }
            out.push(']');
        }
        Value::Object(object) => {
            out.push('{');
            for (i, (key, item)) in object.entries().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_string(out, &key.to_string());
                out.push(':');
                json_value(out, item);
            }
            out.push('}');
        }
        Value::Lazy(_) => unreachable!(),
    }
}

/// The TS side stringifies the whole trace; this reaches the same tree through `root`.
fn main() {
    let mut inflater = Inflater::new();
    for path in std::env::args().skip(1) {
        let submission = Submission::read(path.as_ref(), &mut inflater);
        let decoded = submission.decode();
        let mut out = String::from("{\"traces\":[");
        for (i, trace) in decoded.traces.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json_value(&mut out, trace.root());
        }
        out.push_str("],\"dropped\":[");
        for (i, message) in decoded.dropped_exports.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json_string(&mut out, message);
        }
        out.push_str("],\"error\":");
        match &decoded.error {
            Some(error) => json_string(&mut out, error),
            None => out.push_str("null"),
        }
        out.push('}');
        println!("{out}");
    }
}
