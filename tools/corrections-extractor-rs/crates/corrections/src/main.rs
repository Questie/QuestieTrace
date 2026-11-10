//! `corrections-extractor-rs`: extracts Questie corrections and translations from a local
//! trace-data checkout. See README.md.

use corrections::run::{run, Options};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "\
Usage: corrections-extractor-rs [--trace-data DIR] [--output DIR] [--sample PERCENT] [--threads N]

  --trace-data DIR   trace-data checkout (default: $TRACE_DATA_DIR)
  --output DIR       where the modules go (default: output, or output/sample-<PERCENT>)
  --sample PERCENT   only this percentage of submissions, the same ones every run
                     (default: $EXTRACT_SAMPLE)
  --threads N        decoding threads (default: one per core)";

struct Args {
    trace_data: PathBuf,
    output: PathBuf,
    sample_percent: Option<f64>,
    threads: Option<usize>,
}

fn parse_sample(value: &str) -> Result<f64, String> {
    let percent: f64 = value.parse().map_err(|_| format!("--sample must be a percentage (got \"{value}\")"))?;
    if !(percent > 0.0 && percent <= 100.0) {
        return Err(format!("--sample must be above 0 and at most 100 (got \"{value}\")"));
    }
    Ok(percent)
}

fn parse_args() -> Result<Args, String> {
    let mut trace_data = std::env::var_os("TRACE_DATA_DIR").map(PathBuf::from);
    let mut output = None;
    let mut sample_percent = match std::env::var("EXTRACT_SAMPLE") {
        Ok(value) if !value.is_empty() => Some(parse_sample(&value)?),
        _ => None,
    };
    let mut threads = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--trace-data" => trace_data = Some(PathBuf::from(value()?)),
            "--output" => output = Some(PathBuf::from(value()?)),
            "--sample" => sample_percent = Some(parse_sample(&value()?)?),
            "--threads" => {
                let raw = value()?;
                threads = Some(raw.parse().map_err(|_| format!("--threads must be a whole number (got \"{raw}\")"))?);
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("Unknown argument {other}\n\n{USAGE}")),
        }
    }
    let trace_data = trace_data.ok_or_else(|| format!("Set --trace-data or TRACE_DATA_DIR to a trace-data checkout.\n\n{USAGE}"))?;
    // A sample gets its own output, so it never replaces the full one.
    let output = output.unwrap_or_else(|| match sample_percent {
        Some(percent) => PathBuf::from("output").join(format!("sample-{percent}")),
        None => PathBuf::from("output"),
    });
    Ok(Args {
        trace_data,
        output,
        sample_percent,
        threads,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(threads) = args.threads {
        if let Err(e) = rayon::ThreadPoolBuilder::new().num_threads(threads).build_global() {
            eprintln!("Could not start {threads} threads: {e}");
            return ExitCode::FAILURE;
        }
    }
    let started = Instant::now();
    let options = Options {
        trace_data: args.trace_data,
        output: args.output.clone(),
        sample_percent: args.sample_percent,
    };
    match run(&options, &|line| eprintln!("{line}")) {
        Ok(meta) => {
            let seconds = started.elapsed().as_secs();
            let took = if seconds < 60 {
                format!("{seconds}s")
            } else {
                format!("{}m {}s", seconds / 60, seconds % 60)
            };
            println!(
                "Extracted {} session(s) from {} submission(s) into {} in {took}",
                meta.session_count,
                meta.submission_count,
                args.output.display()
            );
            for skipped in &meta.skipped_files {
                eprintln!("Skipped {}: {}", skipped.name, skipped.error);
            }
            if meta.duplicate_sessions_skipped > 0 {
                println!(
                    "Ignored {} older copies of sessions that later submissions exported again (the newest copy is used)",
                    meta.duplicate_sessions_skipped
                );
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
