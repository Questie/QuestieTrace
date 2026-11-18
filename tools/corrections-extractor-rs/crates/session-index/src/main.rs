//! `session-index`: lists every recorded session of a local trace-data checkout once, and which
//! submissions hold nothing that is not kept from elsewhere. Reads the checkout only. See
//! README.md.

use session_index::index::Index;
use session_index::output::write_index;
use session_index::scan::scan_checkout;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "\
Usage: session-index [--trace-data DIR] [--output DIR]

  --trace-data DIR   trace-data checkout to read (default: $TRACE_DATA_DIR)
  --output DIR       where the index goes (default: session-index-output)

Decoding uses every core; set RAYON_NUM_THREADS to use fewer.";

struct Args {
    trace_data: PathBuf,
    output: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut trace_data = std::env::var_os("TRACE_DATA_DIR").map(PathBuf::from);
    let mut output = PathBuf::from("session-index-output");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--trace-data" => trace_data = Some(PathBuf::from(value()?)),
            "--output" => output = PathBuf::from(value()?),
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("Unknown argument {other}\n\n{USAGE}")),
        }
    }
    let trace_data = trace_data.ok_or_else(|| format!("Set --trace-data or TRACE_DATA_DIR to a trace-data checkout.\n\n{USAGE}"))?;
    Ok(Args { trace_data, output })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let started = Instant::now();
    let scanned = match scan_checkout(&args.trace_data, &|line| eprintln!("{line}")) {
        Ok(scanned) => scanned,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let index = Index::build(scanned);
    let summary = match write_index(&index, &args.output) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("Could not write the index to {}: {error}", args.output.display());
            return ExitCode::FAILURE;
        }
    };
    let seconds = started.elapsed().as_secs();
    println!("{}", serde_json::to_string_pretty(&summary).expect("a summary always serializes"));
    eprintln!("Wrote the index to {} in {}m {}s", args.output.display(), seconds / 60, seconds % 60);
    ExitCode::SUCCESS
}
