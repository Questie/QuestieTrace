//! Builds packs from a local trace-data checkout and checks them against it.
//!
//! This is the tool for developing the pack format and for the backfill's dry runs. It reads
//! `<trace-data>/submissions/<yyyy-mm>/*.json` and writes one pack per hour received.

use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use trace_data::source::{list_submissions, SubmissionFile};
use trace_pack::pack::{build_frame, check_against_file, FrameReport, PackOptions, PackReader, PackWriter, Scratch};
use trace_pack::record::Encoding;
use trace_pack::values::{EncodeStats, TpPolicy};

const USAGE: &str = "\
usage:
  trace-pack build  --trace-data DIR --out DIR [--only PREFIX] [--limit N] [--level N]
                    [--exact] [--tp-schema N,N,...]
  trace-pack verify --trace-data DIR --packs DIR [--only PREFIX] [--tp-schema N,N,...]
  trace-pack list   PACK

build   writes DIR/packs/backfill/YYYY/MM/DD/HH-1.pack, one per hour received. It never
        replaces a pack that exists.
        --only PREFIX    only submissions whose key starts with it, such as 2026-11/2026-11-08_18
        --limit N        at most N submissions, the newest first
        --exact          no value encoding: every export stored as inflated
        --tp-schema ...  round tp to microseconds in sessions of these schemaVersions
        With --only or --limit a pack can hold part of its hour: for trying things out only.
verify  compares every submission in the packs under DIR with its file, and requires every
        submission of the checkout (or of --only) to be in exactly one pack.
        --tp-schema ...  the versions the packs were built with; tp rounded elsewhere fails
list    prints a pack's index.";

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("build") => build(&Arguments::parse(&arguments[1..])?),
        Some("verify") => verify(&Arguments::parse(&arguments[1..])?),
        Some("list") => list(arguments.get(1).context(USAGE)?.as_ref()),
        _ => bail!(USAGE),
    }
}

#[derive(Default)]
struct Arguments {
    trace_data: Option<PathBuf>,
    out: Option<PathBuf>,
    packs: Option<PathBuf>,
    only: Option<String>,
    limit: Option<usize>,
    options: PackOptions,
}

impl Arguments {
    fn parse(arguments: &[String]) -> Result<Arguments> {
        let mut parsed = Arguments::default();
        let mut rest = arguments.iter();
        while let Some(flag) = rest.next() {
            let mut value = || rest.next().with_context(|| format!("{flag} needs a value\n{USAGE}"));
            match flag.as_str() {
                "--trace-data" => parsed.trace_data = Some(value()?.into()),
                "--out" => parsed.out = Some(value()?.into()),
                "--packs" => parsed.packs = Some(value()?.into()),
                "--only" => parsed.only = Some(value()?.clone()),
                "--limit" => parsed.limit = Some(value()?.parse().context("--limit")?),
                "--level" => parsed.options.level = value()?.parse().context("--level")?,
                "--exact" => parsed.options.value_encoding = false,
                "--tp-schema" => {
                    let versions = value()?
                        .split(',')
                        .map(|version| version.trim().parse())
                        .collect::<Result<_, _>>()
                        .context("--tp-schema")?;
                    parsed.options.tp = TpPolicy::SchemaVersions(versions);
                }
                _ => bail!("unknown option {flag}\n{USAGE}"),
            }
        }
        Ok(parsed)
    }

    /// The checkout's submissions that `--only` selects, newest first.
    fn selected_files(&self) -> Result<Vec<SubmissionFile>> {
        let trace_data = self.trace_data.as_deref().context(USAGE)?;
        let mut files = list_submissions(trace_data).with_context(|| format!("listing {}", trace_data.display()))?;
        if let Some(prefix) = &self.only {
            files.retain(|file| file.key.starts_with(prefix.as_str()));
        }
        Ok(files)
    }
}

// ---- build ----

/// The hour a submission was received, from its key `<yyyy-mm>/<yyyy-mm-dd>_<hhmmss>Z_<id>.json`:
/// `(yyyy, mm, dd, hh)`. None for a key of another shape.
fn received_hour(key: &str) -> Option<(&str, &str, &str, &str)> {
    let name = key.rsplit('/').next()?;
    let stamp = name.get(..13)?.as_bytes();
    let shaped = stamp.iter().enumerate().all(|(index, byte)| match index {
        4 | 7 => *byte == b'-',
        10 => *byte == b'_',
        _ => byte.is_ascii_digit(),
    });
    shaped.then(|| (&name[..4], &name[5..7], &name[8..10], &name[11..13]))
}

#[derive(Default)]
struct Totals {
    submissions: u64,
    kept_whole: u64,
    exports: [u64; 3],
    file_bytes: u64,
    cbor_bytes: u64,
    pack_bytes: u64,
    fallbacks: Vec<String>,
    values: EncodeStats,
}

impl Totals {
    fn add(&mut self, report: &FrameReport, encodings: &[Encoding]) {
        self.submissions += 1;
        self.file_bytes += report.file_bytes;
        self.cbor_bytes += report.cbor_bytes;
        self.fallbacks.extend(report.value_fallbacks.iter().cloned());
        self.values.add(&report.values);
        for encoding in encodings {
            self.exports[*encoding as usize] += 1;
        }
        self.kept_whole += encodings.contains(&Encoding::Raw) as u64;
    }
}

fn build(arguments: &Arguments) -> Result<()> {
    let out = arguments.out.as_deref().context(USAGE)?;
    let started = Instant::now();

    // Which submissions, grouped by the hour they were received.
    let mut files = arguments.selected_files()?;
    files.truncate(arguments.limit.unwrap_or(usize::MAX));
    let mut hours: BTreeMap<(&str, &str, &str, &str), Vec<&SubmissionFile>> = BTreeMap::new();
    for file in &files {
        let hour = received_hour(&file.key).with_context(|| format!("{}: the key does not start with the time received", file.key))?;
        hours.entry(hour).or_default().push(file);
    }

    let mut totals = Totals::default();
    for ((year, month, day, hour), mut files) in hours {
        let directory = out.join("packs/backfill").join(year).join(month).join(day);
        let path = directory.join(format!("{hour}-1.pack"));
        if path.exists() {
            bail!("{} exists already; a pack is never replaced", path.display());
        }
        files.sort_by(|a, b| a.key.cmp(&b.key));

        // Frames are built on every core; the pack takes them in key order.
        let frames = files
            .par_iter()
            .map_init(Scratch::new, |scratch, file| {
                let bytes = std::fs::read(&file.path).map_err(|error| format!("{}: {error}", file.key))?;
                build_frame(&file.key, &bytes, &arguments.options, scratch)
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(anyhow::Error::msg)?;

        let mut writer = PackWriter::new();
        for frame in frames {
            let encodings: Vec<Encoding> = frame.entry.exports.iter().map(|(encoding, _)| *encoding).collect();
            totals.add(&frame.report, &encodings);
            writer.add(frame);
        }
        let pack = writer.finish(arguments.options.level).map_err(anyhow::Error::msg)?;
        totals.pack_bytes += pack.len() as u64;
        write_new(&directory, &path, &pack)?;
        println!("{}  {} submissions  {:.1} MiB", path.display(), files.len(), mib(pack.len() as u64));
    }
    print_totals(&totals, started);
    Ok(())
}

/// Writes a pack that must not exist yet. It is written and flushed under another name first
/// and then linked into place, so a pack under its own name is always complete, and an
/// existing one is never replaced: linking fails if the name is taken.
fn write_new(directory: &Path, path: &Path, pack: &[u8]) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let unfinished = path.with_extension("pack.part");
    let mut file = File::create(&unfinished)?;
    file.write_all(pack)?;
    file.sync_all()?;
    std::fs::hard_link(&unfinished, path).with_context(|| format!("{} exists already; a pack is never replaced", path.display()))?;
    std::fs::remove_file(&unfinished)?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        100.0 * part as f64 / whole as f64
    }
}

fn print_totals(totals: &Totals, started: Instant) {
    let values = &totals.values;
    println!();
    println!("submissions        {}  ({} kept whole)", totals.submissions, totals.kept_whole);
    println!(
        "exports            {} value-encoded, {} exact",
        totals.exports[Encoding::Value as usize],
        totals.exports[Encoding::Exact as usize]
    );
    println!("JSON               {:.1} MiB", mib(totals.file_bytes));
    println!("CBOR               {:.1} MiB", mib(totals.cbor_bytes));
    println!(
        "packs              {:.1} MiB  ({:.1}% of JSON)",
        mib(totals.pack_bytes),
        percent(totals.pack_bytes, totals.file_bytes)
    );
    println!("in value-encoded exports:");
    for (name, field) in [("tp", 0), ("t", 1), ("x", 2), ("y", 3)] {
        let (converted, floats) = (values.converted[field], values.floats[field]);
        println!(
            "  {name:<2} floats        {converted} converted of {floats} ({:.2}%)",
            percent(converted, floats)
        );
    }
    let [policy, shape, range, order] = values.tp_sessions_kept;
    println!("  sessions         {}", values.sessions);
    println!(
        "  tp per session   {} rounded; kept as floats: {policy} by policy, {shape} by shape, {range} out of range, {order} by order",
        values.tp_sessions_converted
    );
    println!("value fallbacks    {}", totals.fallbacks.len());
    for fallback in totals.fallbacks.iter().take(20) {
        println!("  {fallback}");
    }
    println!("elapsed            {:.1} s", started.elapsed().as_secs_f64());
}

// ---- verify ----

fn pack_files(directory: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            pack_files(&path, found)?;
        } else if path.extension().is_some_and(|extension| extension == "pack") {
            found.push(path);
        }
    }
    Ok(())
}

fn verify(arguments: &Arguments) -> Result<()> {
    let trace_data = arguments.trace_data.as_deref().context(USAGE)?;
    let policy = &arguments.options.tp;
    let mut packs = Vec::new();
    pack_files(arguments.packs.as_deref().context(USAGE)?, &mut packs)?;
    packs.sort();
    if packs.is_empty() {
        bail!("no packs to verify");
    }

    // Every submission in every pack, against its file.
    let mut failures = Vec::new();
    let mut packed: HashMap<String, u32> = HashMap::new();
    for path in &packs {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let reader = PackReader::open(&bytes).map_err(|message| anyhow::anyhow!("{}: {message}", path.display()))?;
        let results: Vec<Result<(), String>> = (0..reader.entries().len())
            .into_par_iter()
            .map_init(Scratch::new, |scratch, index| {
                let entry = &reader.entries()[index];
                let file = std::fs::read(trace_data.join("submissions").join(&entry.name)).map_err(|error| format!("{}: {error}", entry.name))?;
                let record = reader.record(index)?;
                check_against_file(&record, entry, &file, policy, scratch).map_err(|message| format!("{}: {message}", entry.name))
            })
            .collect();
        failures.extend(results.into_iter().filter_map(Result::err));
        for entry in reader.entries() {
            *packed.entry(entry.name.clone()).or_default() += 1;
        }
    }

    // Completeness: each selected submission in exactly one pack, and nothing else in them.
    let expected = arguments.selected_files()?;
    for file in &expected {
        match packed.remove(&file.key) {
            Some(1) => {}
            Some(count) => failures.push(format!("{}: in {count} packs", file.key)),
            None => failures.push(format!("{}: in no pack", file.key)),
        }
    }
    failures.extend(packed.keys().map(|name| format!("{name}: in a pack, but not among the submissions to verify")));
    failures.sort();

    println!("{} packs, {} submissions expected, {} problems", packs.len(), expected.len(), failures.len());
    for failure in failures.iter().take(50) {
        println!("  {failure}");
    }
    if !failures.is_empty() {
        bail!("the packs do not match the checkout");
    }
    Ok(())
}

// ---- list ----

fn list(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let reader = PackReader::open(&bytes).map_err(anyhow::Error::msg)?;
    for entry in reader.entries() {
        let encodings: Vec<&str> = entry
            .exports
            .iter()
            .map(|(encoding, _)| match encoding {
                Encoding::Raw => "raw",
                Encoding::Exact => "exact",
                Encoding::Value => "value",
            })
            .collect();
        println!(
            "{}\t{}\t{}\t{}\t{}",
            entry.name,
            entry.offset,
            entry.length,
            entry.received_at,
            encodings.join(",")
        );
    }
    Ok(())
}
