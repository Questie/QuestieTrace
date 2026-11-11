//! `relations <command>`: quest relationship inference. See the crate docs and RELATIONS.md.

use anyhow::{bail, Context, Result};
use relations::combine::CombineInput;
use relations::config::Config;
use relations::model::{CandidateFile, GroundTruth, QuestCatalog};
use relations::signals::{self, SignalInput};
use relations::{catalog, combine, ingest, io, score, sources};
use std::process::ExitCode;
use std::time::Instant;

// The system allocator, unlike the corrections extractor: decoding goes into a per-worker arena
// (see ingest), so few allocations reach it. On the full archive, ingest took 385 s and 8.1 GB with
// it against 435 s and 8.5 GB with mimalloc.

const USAGE: &str = "\
Usage: relations <command>

  pipeline             every step in order: ingest, catalog, source:wowhead, signals, combine
  ingest               trace-data submissions -> episodes.jsonl
  catalog              QuestieDB -> catalog.json + groundtruth.json (runs QuestieDB's Lua loader)
  source:wowhead       scraper databases -> candidates/wowhead-*.json (review context only)
  signal:<name>        one signal: episodes + catalog -> candidates/<name>.json
  combine              all candidates -> foreverQuestRelationTraces.lua, relations-review.md,
                       relations-meta.json
  score <signal> [--min-score N]
                       precision/recall of candidates/<signal>.json against the ground truth

Environment (the same as the TypeScript pipeline's):
  TRACE_DATA_DIR, QUESTIEDB_DIR, SCRAPER_DIR, RELATIONS_TRACE_MODULES_DIR
  RELATIONS_DIR (working files, default .relations), RELATIONS_OUTPUT_DIR (default
  $RELATIONS_DIR/output), RELATIONS_REQUIRE_FOREVER_VERSION=1";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config::from_env(relations::now_iso());
    match run(&config, &args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(config: &Config, args: &[String]) -> Result<()> {
    let Some(command) = args.first() else {
        println!("{}", usage());
        return Ok(());
    };
    match command.as_str() {
        "pipeline" => pipeline(config),
        "ingest" => timed("ingest", || ingest::run(config)?.write(config)),
        "catalog" => timed("catalog", || catalog::run(config)?.write(config)),
        "source:wowhead" => timed("source:wowhead", || sources::wowhead::run(config)?.write(config)),
        "combine" => timed("combine", || {
            let catalog: QuestCatalog = io::read_json(&config.catalog(), "catalog")?;
            let ground_truth: GroundTruth = io::read_json(&config.ground_truth(), "catalog")?;
            let episodes = io::read_episodes(&config.episodes())?;
            let candidate_files = io::read_candidate_files(&config.candidates_dir())?;
            let input = CombineInput {
                candidate_files,
                catalog: &catalog,
                ground_truth: &ground_truth,
                episodes: &episodes,
            };
            combine::run(config, input)?.write(config)
        }),
        "score" => {
            let signal = args.get(1).context("usage: score <signal> [--min-score N]")?;
            let min_score = match args.iter().position(|arg| arg == "--min-score") {
                Some(at) => args
                    .get(at + 1)
                    .and_then(|value| value.parse::<f64>().ok())
                    .context("--min-score needs a number")?,
                None => 0.0,
            };
            let file: CandidateFile = io::read_json(&config.candidates(signal), &format!("signal:{signal}"))?;
            let truth: GroundTruth = io::read_json(&config.ground_truth(), "catalog")?;
            let report = score::score_candidates(&file, &truth, min_score);
            io::write_json(&config.report(&format!("score-{signal}")), &report)?;
            println!("{}", score::format_score_report(&report));
            Ok(())
        }
        other => match other
            .strip_prefix("signal:")
            .and_then(|name| signals::registry().into_iter().find(|s| s.name() == name))
        {
            Some(signal) => timed(other, || {
                let catalog: QuestCatalog = io::read_json(&config.catalog(), "catalog")?;
                let episodes = io::read_episodes(&config.episodes())?;
                let input = SignalInput {
                    episodes: &episodes,
                    catalog: &catalog,
                    generated_at: &config.generated_at,
                };
                signal.run(&input).write(config)
            }),
            None => bail!("unknown command {other:?}\n\n{}", usage()),
        },
    }
}

/// The usage text, with one line per registered signal.
fn usage() -> String {
    let signals: Vec<String> = signals::registry().iter().map(|s| format!("  signal:{:<15} {}", s.name(), s.describe())).collect();
    format!("{USAGE}\n\nSignals:\n{}", signals.join("\n"))
}

fn timed(step: &str, work: impl FnOnce() -> Result<()>) -> Result<()> {
    let started = Instant::now();
    work().with_context(|| format!("{step} failed"))?;
    eprintln!("== {step} done in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}

/// Every step in one process: each step's output goes straight to the next and is also written,
/// so a run can be inspected or resumed step by step.
fn pipeline(config: &Config) -> Result<()> {
    let started = Instant::now();
    let mut step = Instant::now();
    let mut done = |name: &str| {
        eprintln!("== {name} done in {:.1}s", step.elapsed().as_secs_f64());
        step = Instant::now();
    };

    let ingested = ingest::run(config).context("ingest failed")?;
    ingested.write(config)?;
    done("ingest");

    let cataloged = catalog::run(config).context("catalog failed")?;
    cataloged.write(config)?;
    done("catalog");

    let wowhead = sources::wowhead::run(config).context("source:wowhead failed")?;
    wowhead.write(config)?;
    done("source:wowhead");

    let input = SignalInput {
        episodes: &ingested.episodes,
        catalog: &cataloged.catalog,
        generated_at: &config.generated_at,
    };
    // Each signal's result is written and dropped before the next one runs.
    signals::run_all(&input, |output| output.write(config))?;
    done("signals");

    // Combine reads every candidates file there is, like the TS pipeline, so files of signals run
    // separately before count too.
    let candidate_files = io::read_candidate_files(&config.candidates_dir())?;
    let input = CombineInput {
        candidate_files,
        catalog: &cataloged.catalog,
        ground_truth: &cataloged.ground_truth,
        episodes: &ingested.episodes,
    };
    combine::run(config, input).context("combine failed")?.write(config)?;
    done("combine");
    eprintln!("== pipeline done in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}
