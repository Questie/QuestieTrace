//! One run of the compactor against the archive bucket, meant to be started by a timer, or
//! one run of the backfill into a local directory.

use anyhow::{bail, Context, Result};
use jiff::{SignedDuration, Timestamp};
use std::fs::File;
use std::path::PathBuf;
use trace_compactor::backfill;
use trace_compactor::rate_limit::RateLimit;
use trace_compactor::s3::{ReadOnlyS3, S3Bucket};
use trace_compactor::Config;
use trace_pack::pack::PackOptions;
use trace_pack::values::TpPolicy;

const USAGE: &str = "\
usage:
  trace-compactor run [--writer NAME] [--settle-hours N] [--pack-input-mib N]
                      [--build-input-mib N] [--threads N] [--level N] [--exact]
                      [--tp-schema N,N,...] [--lock FILE]
  trace-compactor backfill --out DIR [--also-packed DIR]... [--trace-data DIR]
                      [--only PREFIX] [--download-mb-per-second N] [--settle-hours N]
                      [--pack-input-mib N] [--build-input-mib N] [--threads N] [--level N]
                      [--exact] [--tp-schema N,N,...]

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
be allowed to read, write and delete in that bucket.

backfill  packs the old archive, one JSON object per submission, into a local directory:
        DIR/packs/backfill/YYYY/MM/DD/HH-<md5>.pack, by the hour received. It packs what is
        in no pack under DIR yet, so it can be stopped and run again, and it writes nothing
        to where it reads from.
        --out DIR           where the packs go
        --also-packed DIR   what is in a pack under DIR/packs is not packed again either. May
                            be given more than once. Only read
        --trace-data DIR   a trace-data checkout, DIR/submissions/<yyyy-mm>/*.json. Without
                            the old bucket it is all that is packed. With it, a file here
                            saves the download when it is the object the bucket lists.
        --only PREFIX       only submissions whose name starts with it, such as
                            2026-11/2026-11-08
        --download-mb-per-second N
                            the most all threads together download, in megabytes (millions
                            of bytes) a second. Default: no limit
        --settle-hours N    an hour is packed once it ended this long ago. Default: 48
        --pack-input-mib N  the most JSON one pack is built from; an hour with more gets
                            several packs. A removal rewrites a pack in memory on the
                            server, so this also bounds what that needs. Default: 2048
        The other options are those of run, with the same memory needs.

        The old bucket is read when SOURCE_S3_ENDPOINT (host only), SOURCE_S3_REGION,
        SOURCE_S3_BUCKET, SOURCE_S3_ACCESS_KEY_ID and SOURCE_S3_SECRET_ACCESS_KEY are set.
        A key that may only read is enough, and is what it should be given.";

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("run") => run(Arguments::parse(&arguments[1..])?),
        Some("backfill") => backfill(BackfillArguments::parse(&arguments[1..])?),
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

// ---- backfill ----

struct BackfillArguments {
    config: backfill::Config,
    trace_data: Option<PathBuf>,
    threads: Option<usize>,
    download_limit: Option<RateLimit>,
}

impl BackfillArguments {
    fn parse(arguments: &[String]) -> Result<BackfillArguments> {
        let mut out = None;
        let mut parsed = BackfillArguments {
            config: backfill::Config {
                out: PathBuf::new(),
                also_packed: Vec::new(),
                settle: SignedDuration::from_hours(48),
                pack_input_bytes: 2048 << 20,
                build_input_bytes: 64 << 20,
                only: None,
                options: PackOptions::default(),
            },
            trace_data: None,
            threads: None,
            download_limit: None,
        };
        let mut rest = arguments.iter();
        while let Some(flag) = rest.next() {
            let mut value = || rest.next().with_context(|| format!("{flag} needs a value\n{USAGE}"));
            match flag.as_str() {
                "--out" => out = Some(PathBuf::from(value()?)),
                "--also-packed" => parsed.config.also_packed.push(value()?.into()),
                "--download-mb-per-second" => {
                    let megabytes: u64 = value()?.parse().context("--download-mb-per-second")?;
                    parsed.download_limit = Some(RateLimit::new(megabytes * 1_000_000));
                }
                "--trace-data" => parsed.trace_data = Some(value()?.into()),
                "--only" => parsed.config.only = Some(value()?.clone()),
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
                _ => bail!("unknown option {flag}\n{USAGE}"),
            }
        }
        parsed.config.out = out.with_context(|| format!("--out is needed\n{USAGE}"))?;
        Ok(parsed)
    }
}

fn backfill(arguments: BackfillArguments) -> Result<()> {
    let BackfillArguments {
        config,
        trace_data,
        threads,
        download_limit,
    } = arguments;

    // Two runs into one directory would each pack what the other is packing.
    std::fs::create_dir_all(&config.out).with_context(|| format!("creating {}", config.out.display()))?;
    let lock_path = config.out.join("backfill.lock");
    let lock = File::create(&lock_path).with_context(|| format!("opening {}", lock_path.display()))?;
    if lock.try_lock().is_err() {
        bail!("another run holds {}", lock_path.display());
    }
    if let Some(threads) = threads {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().context("--threads")?;
    }

    let bucket = source_bucket_from_environment(download_limit)?;
    let source = backfill::Source {
        bucket: bucket.as_ref().map(|bucket| bucket as &dyn backfill::ReadBucket),
        local: trace_data,
    };
    let print_pack = |pack: &trace_compactor::packing::PackReport| {
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
    };
    let report = backfill::run(&source, &config, Timestamp::now(), &print_pack)?;

    let (submissions, json, packs) = report.packs.iter().fold((0, 0, 0), |(submissions, json, packs), pack| {
        (submissions + pack.submissions, json + pack.loose_bytes, packs + pack.pack_bytes)
    });
    println!(
        "{} packs written: {submissions} submissions, {:.1} MiB from {:.1} MiB",
        report.packs.len(),
        mib(packs),
        mib(json)
    );
    println!(
        "{} submissions were packed already, {} are of hours that have not settled",
        report.already_packed, report.not_settled
    );
    if !report.gone_from_source.is_empty() {
        println!("{} submissions are in a pack and no longer in the bucket:", report.gone_from_source.len());
        report.gone_from_source.iter().take(20).for_each(|name| println!("  {name}"));
    }
    for left in &report.left {
        println!("NOT PACKED {left}");
    }
    if !report.left.is_empty() {
        bail!("{} submissions could not be packed: see the lines above", report.left.len());
    }
    Ok(())
}

/// The old bucket, when the environment names one. All five variables or none.
fn source_bucket_from_environment(download_limit: Option<RateLimit>) -> Result<Option<ReadOnlyS3>> {
    const NAMES: [&str; 5] = [
        "SOURCE_S3_ENDPOINT",
        "SOURCE_S3_REGION",
        "SOURCE_S3_BUCKET",
        "SOURCE_S3_ACCESS_KEY_ID",
        "SOURCE_S3_SECRET_ACCESS_KEY",
    ];
    let values: Vec<String> = NAMES
        .iter()
        .filter_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .collect();
    match values.as_slice() {
        [] => Ok(None),
        [endpoint, region, bucket, access_key, secret_key] => {
            if endpoint.contains('/') {
                bail!("SOURCE_S3_ENDPOINT is a host name, without https:// or a path");
            }
            ReadOnlyS3::new(&format!("https://{endpoint}"), region, bucket, access_key, secret_key, download_limit).map(Some)
        }
        _ => bail!("set all of {} or none", NAMES.join(", ")),
    }
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
