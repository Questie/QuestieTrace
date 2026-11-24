//! One run of the compactor against the archive bucket. Meant to be started by a timer.

use anyhow::{bail, Context, Result};
use jiff::{SignedDuration, Timestamp};
use std::fs::File;
use std::path::PathBuf;
use trace_compactor::s3::S3Bucket;
use trace_compactor::Config;
use trace_pack::pack::PackOptions;
use trace_pack::values::TpPolicy;

const USAGE: &str = "\
usage:
  trace-compactor run [--writer NAME] [--settle-hours N] [--pack-input-mib N]
                      [--build-input-mib N] [--threads N] [--level N] [--exact]
                      [--tp-schema N,N,...] [--lock FILE]

run     works on the intake's removal requests, then packs every loose submission the
        archive bucket stored more than the settle window ago and deletes what it packed. A
        run that was stopped is finished first. Only one run may work on a bucket at a time,
        whatever its writer name.
        --writer NAME       names the packs, packs/NAME/..., and this writer's record of the
                            pack it is storing. Default: live
        --settle-hours N    how long a loose submission waits. Default: 48
        --pack-input-mib N  the most loose JSON one pack is built from. Default: 2048
        --build-input-mib N the most loose JSON being built at one moment. Default: 64
        --threads N         how many submissions are built at once, within that.
                            Default: one per core
        --level N           zstd level. Default: 9
        --exact             no value encoding: every export stored as inflated
        --tp-schema ...     round tp to microseconds in sessions of these schemaVersions
        --lock FILE         held while running, so that two runs on this machine cannot
                            overlap. Default: trace-compactor.lock in the temporary
                            directory. A container has a temporary directory of its own:
                            there, whatever starts the containers has to keep runs apart.

Memory: a pack is built and uploaded from memory. A run needs about 22 times
--build-input-mib while it builds, plus up to two thirds of --pack-input-mib for the pack it
holds, and twice a pack's size to rewrite one for a removal. A submission larger than
--build-input-mib is built alone and needs about 22 times its own size, which nothing here
bounds.

The bucket is read from the environment: ARCHIVE_S3_ENDPOINT (host only), ARCHIVE_S3_REGION,
ARCHIVE_S3_BUCKET, ARCHIVE_S3_ACCESS_KEY_ID and ARCHIVE_S3_SECRET_ACCESS_KEY. The key must
be allowed to read, write and delete in that bucket.";

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("run") => run(Arguments::parse(&arguments[1..])?),
        _ => bail!(USAGE),
    }
}

struct Arguments {
    config: Config,
    threads: Option<usize>,
    lock: Option<PathBuf>,
}

impl Arguments {
    fn parse(arguments: &[String]) -> Result<Arguments> {
        let mut parsed = Arguments {
            config: Config {
                writer: "live".into(),
                settle: SignedDuration::from_hours(48),
                pack_input_bytes: 2048 << 20,
                build_input_bytes: 64 << 20,
                options: PackOptions::default(),
            },
            threads: None,
            lock: None,
        };
        let mut rest = arguments.iter();
        while let Some(flag) = rest.next() {
            let mut value = || rest.next().with_context(|| format!("{flag} needs a value\n{USAGE}"));
            match flag.as_str() {
                "--writer" => parsed.config.writer = value()?.clone(),
                "--settle-hours" => parsed.config.settle = SignedDuration::from_hours(value()?.parse::<u32>().context("--settle-hours")?.into()),
                "--pack-input-mib" => parsed.config.pack_input_bytes = value()?.parse::<u64>().context("--pack-input-mib")? << 20,
                "--build-input-mib" => parsed.config.build_input_bytes = value()?.parse::<u64>().context("--build-input-mib")? << 20,
                "--threads" => parsed.threads = Some(value()?.parse().context("--threads")?),
                "--level" => parsed.config.options.level = value()?.parse().context("--level")?,
                "--exact" => parsed.config.options.value_encoding = false,
                "--tp-schema" => {
                    let versions = value()?
                        .split(',')
                        .map(|version| version.trim().parse())
                        .collect::<Result<_, _>>()
                        .context("--tp-schema")?;
                    parsed.config.options.tp = TpPolicy::SchemaVersions(versions);
                }
                "--lock" => parsed.lock = Some(value()?.into()),
                _ => bail!("unknown option {flag}\n{USAGE}"),
            }
        }
        // The name becomes part of object keys.
        let writer = &parsed.config.writer;
        if writer.is_empty() || !writer.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
            bail!("--writer takes lowercase letters, digits and hyphens");
        }
        Ok(parsed)
    }
}

fn run(arguments: Arguments) -> Result<()> {
    let Arguments { config, threads, lock } = arguments;

    // Two runs at once would each finish, or discard, the other's intent, and a removal
    // rewrites packs of every writer.
    let lock_path = lock.unwrap_or_else(|| std::env::temp_dir().join("trace-compactor.lock"));
    let lock = File::create(&lock_path).with_context(|| format!("opening {}", lock_path.display()))?;
    if lock.try_lock().is_err() {
        bail!("another run holds {}", lock_path.display());
    }

    if let Some(threads) = threads {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().context("--threads")?;
    }
    let bucket = bucket_from_environment()?;

    let report = trace_compactor::run(&bucket, &config, Timestamp::now())?;
    let removals = &report.removals;
    for removed in &removals.removed {
        println!(
            "removed {}: {} loose, rewrote {} packs {}",
            removed.id,
            removed.loose_deleted,
            removed.packs_rewritten.len(),
            removed.packs_rewritten.join(" ")
        );
    }
    for id in &removals.answered {
        println!("answered the removal request for {id}");
    }
    for pack in &report.packed.packs {
        println!(
            "{}  {} submissions ({} kept whole)  {:.1} MiB from {:.1} MiB",
            pack.key,
            pack.submissions,
            pack.kept_whole,
            mib(pack.pack_bytes),
            mib(pack.loose_bytes)
        );
        for fallback in &pack.value_fallbacks {
            println!("  WARNING stored exact after failing its check: {fallback}");
        }
    }
    println!(
        "{} submissions removed, {} removal requests answered, {} packs written",
        removals.removed.len(),
        removals.answered.len(),
        report.packed.packs.len()
    );

    // What needs someone to look comes last, and makes the run a failed one, so that whatever
    // watches the timer shows it.
    if let Some(reason) = &removals.stopped {
        println!("REMOVALS STOPPED {reason}");
    }
    for unanswered in &removals.unanswered {
        println!("REMOVAL NOT ANSWERED {unanswered}");
    }
    for left in &report.packed.left_loose {
        println!("NOT PACKED {left}");
    }
    let open = removals.stopped.iter().len() + removals.unanswered.len() + report.packed.left_loose.len();
    if open > 0 {
        bail!("{open} things need someone to look: see the lines above");
    }
    Ok(())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn bucket_from_environment() -> Result<S3Bucket> {
    let variable = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .with_context(|| format!("{name} is not set\n{USAGE}"))
    };
    let endpoint = variable("ARCHIVE_S3_ENDPOINT")?;
    // A host only, as the intake takes it: the connection is always HTTPS.
    if endpoint.contains('/') {
        bail!("ARCHIVE_S3_ENDPOINT is a host name, without https:// or a path");
    }
    S3Bucket::new(
        &format!("https://{endpoint}"),
        &variable("ARCHIVE_S3_REGION")?,
        &variable("ARCHIVE_S3_BUCKET")?,
        &variable("ARCHIVE_S3_ACCESS_KEY_ID")?,
        &variable("ARCHIVE_S3_SECRET_ACCESS_KEY")?,
    )
}
