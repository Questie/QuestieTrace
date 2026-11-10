//! A full extraction run over a local trace-data checkout. Ports the parts of
//! `extract-corrections.ts` that remain without the incremental store.
//!
//! Submissions are read newest first, decoded and observed on worker threads, and folded in
//! order on this thread, so of each session only the first copy met (the
//! newest, so the most complete) is used. Exports are cumulative: one session shows up in
//! many submissions.

use crate::accumulator::Accumulator;
use crate::finalize::{finalize, Bundle};
use crate::lua::iso_timestamp;
use crate::observe::{observe_session, StagedSession};
use crate::pipeline::process_in_order;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use trace_data::source::{in_sample, list_submissions, SubmissionFile};
use trace_data::submission::Submission;
use trace_data::{Inflater, SessionKey};

pub struct Options {
    pub trace_data: PathBuf,
    pub output: PathBuf,
    /// Only this percentage of submissions, the same ones every run.
    pub sample_percent: Option<f64>,
    /// Worker threads decoding and observing submissions.
    pub threads: usize,
    /// Log approximately how much memory the fold holds per field.
    pub stats: bool,
}

/// meta.json: what a run read. Field names and order are those of the TS tool.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub session_count: usize,
    /// Submissions that could be read.
    pub submission_count: usize,
    /// Exports decoded from those submissions; a submission may hold several.
    pub export_count: usize,
    /// Submission files, relative to submissions/, that could not be read.
    pub skipped_files: Vec<SkippedFile>,
    /// Submission files of which only some exports could be decoded.
    pub partial_files: Vec<PartialFile>,
    /// Session copies skipped because a newer copy was already folded in.
    pub duplicate_sessions_skipped: usize,
    pub generated_at: String,
}

#[derive(Serialize, Debug)]
pub struct SkippedFile {
    pub name: String,
    pub error: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PartialFile {
    pub name: String,
    pub dropped_exports: Vec<String>,
}

/// One submission after observing, before folding.
struct ObservedSubmission {
    key: String,
    export_count: usize,
    dropped_exports: Vec<String>,
    error: Option<String>,
    sessions: Vec<ObservedSession>,
}

struct ObservedSession {
    key: Option<SessionKey>,
    /// A copy of a session already used: not observed.
    duplicate: bool,
    /// None for sessions that are not observed (not Forever) or duplicates.
    staged: Option<StagedSession>,
}

/// Decodes and observes one submission. Sessions in `used` were folded from newer
/// submissions: their copies here are duplicates and not observed.
fn observe_submission(file: &SubmissionFile, inflater: &mut Inflater, used: &RwLock<HashSet<SessionKey>>) -> ObservedSubmission {
    let submission = Submission::read(&file.path, inflater);
    let decoded = submission.decode();
    // Only the first copy of a session in a submission is used.
    let mut seen = HashSet::new();
    let mut sessions = Vec::new();
    for trace in &decoded.traces {
        for session in trace.sessions() {
            let key = session.key();
            let duplicate = key.is_some_and(|k| !seen.insert(k) || used.read().expect("no thread panics holding the lock").contains(&k));
            let staged = if duplicate { None } else { observe_session(&session) };
            sessions.push(ObservedSession { key, duplicate, staged });
        }
    }
    ObservedSubmission {
        key: file.key.clone(),
        export_count: decoded.traces.len(),
        dropped_exports: decoded.dropped_exports,
        error: decoded.error,
        sessions,
    }
}

/// What the fold tallies besides the accumulator.
#[derive(Default)]
struct Tally {
    submission_count: usize,
    export_count: usize,
    skipped_files: Vec<SkippedFile>,
    partial_files: Vec<PartialFile>,
    duplicates: usize,
}

fn fold(submission: ObservedSubmission, acc: &mut Accumulator, tally: &mut Tally, used: &RwLock<HashSet<SessionKey>>) {
    if let Some(error) = submission.error {
        tally.skipped_files.push(SkippedFile { name: submission.key, error });
        return;
    }
    tally.submission_count += 1;
    tally.export_count += submission.export_count;
    if !submission.dropped_exports.is_empty() {
        tally.partial_files.push(PartialFile {
            name: submission.key,
            dropped_exports: submission.dropped_exports,
        });
    }
    for session in submission.sessions {
        // A copy marked while observing, or one whose key a submission folded meanwhile used:
        // the observing thread could not know yet.
        if session.duplicate
            || session
                .key
                .is_some_and(|key| !used.write().expect("no thread panics holding the lock").insert(key))
        {
            tally.duplicates += 1;
            continue;
        }
        if let Some(staged) = session.staged {
            acc.add(staged);
        }
    }
}

/// Submissions each worker may observe past the fold: enough that a large submission rarely
/// holds the others up, few enough that observed results do not pile up.
const AHEAD_PER_THREAD: usize = 4;
const PROGRESS_EVERY: usize = 2000;

/// Runs the extraction and writes every output file; meta.json last, as the mark of a complete run.
pub fn run(options: &Options, log: &dyn Fn(&str)) -> Result<Meta, String> {
    let mut files =
        list_submissions(&options.trace_data).map_err(|e| format!("{} has no readable submissions directory: {e}", options.trace_data.display()))?;
    if let Some(percent) = options.sample_percent {
        files.retain(|file| in_sample(&file.key, percent));
    }
    if files.is_empty() {
        return Err("The source lists no submissions; keeping the previous output".into());
    }
    log(&format!("{} submission(s) to read", files.len()));

    let mut acc = Accumulator::default();
    let mut tally = Tally::default();
    let used = RwLock::new(HashSet::new());
    let started = std::time::Instant::now();
    let mut folding = std::time::Duration::ZERO;
    let threads = options.threads.max(1);
    let observed = process_in_order(
        &files,
        threads,
        threads * AHEAD_PER_THREAD,
        Inflater::new,
        |inflater, file| observe_submission(file, inflater, &used),
        |index, observed| {
            let fold_started = std::time::Instant::now();
            fold(observed, &mut acc, &mut tally, &used);
            folding += fold_started.elapsed();
            let done = index + 1;
            if done % PROGRESS_EVERY == 0 || done == files.len() {
                log(&format!("[{done}/{}] submissions observed", files.len()));
            }
        },
    );
    // An observer error aborts the run, keeping the previous output (as the TS tool does).
    if let Err(index) = observed {
        return Err(format!(
            "Observing {} failed (see the panic above); keeping the previous output",
            files[index].key
        ));
    }
    log(&format!("Folding took {:.1}s of that", folding.as_secs_f64()));
    log(&format!("Observed and folded in {:.1}s", started.elapsed().as_secs_f64()));
    if tally.submission_count == 0 {
        return Err("No submission could be loaded; keeping the previous output".into());
    }
    if options.stats {
        let report = acc.memory_report();
        let total: usize = report.iter().map(|line| line.bytes).sum();
        log(&format!("Fold holds about {} MB:", total / 1_000_000));
        for line in report.iter().filter(|line| line.bytes > 0) {
            log(&format!("  {:>8} MB {:>12} rows  {}", line.bytes / 1_000_000, line.rows, line.field));
        }
    }

    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default();
    let generated_at = iso_timestamp(millis);
    let finalize_started = std::time::Instant::now();
    let bundle = finalize(&acc, tally.submission_count, &generated_at);
    log(&format!("Finalized in {:.1}s", finalize_started.elapsed().as_secs_f64()));
    let meta = Meta {
        session_count: acc.session_count(),
        submission_count: tally.submission_count,
        export_count: tally.export_count,
        skipped_files: tally.skipped_files,
        partial_files: tally.partial_files,
        duplicate_sessions_skipped: tally.duplicates,
        generated_at,
    };
    write_outputs(&options.output, &bundle, &meta).map_err(|e| format!("Writing {} failed: {e}", options.output.display()))?;
    Ok(meta)
}

/// WoW locale names; anything else from trace data could leave the output directory.
fn is_locale(locale: &str) -> bool {
    let b = locale.as_bytes();
    b.len() == 4 && b[..2].iter().all(u8::is_ascii_lowercase) && b[2..].iter().all(u8::is_ascii_uppercase)
}

fn write_outputs(dir: &Path, bundle: &Bundle, meta: &Meta) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // meta.json marks a complete run: remove it until everything is written.
    let meta_path = dir.join("meta.json");
    if meta_path.exists() {
        std::fs::remove_file(&meta_path)?;
    }
    for (file_name, content) in &bundle.modules {
        std::fs::write(dir.join(file_name), content)?;
    }
    // Whole lookup directories are replaced, which also drops locales a previous run had.
    let staging = dir.join(".lookups-staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    for (dir_name, files) in &bundle.lookups {
        let target = staging.join(dir_name);
        std::fs::create_dir_all(&target)?;
        for (locale, content) in files {
            if is_locale(locale) {
                std::fs::write(target.join(format!("{locale}.lua")), content)?;
            }
        }
    }
    for (dir_name, _) in &bundle.lookups {
        let target = dir.join(dir_name);
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::rename(staging.join(dir_name), &target)?;
    }
    std::fs::remove_dir_all(&staging)?;
    let mut json = serde_json::to_string_pretty(meta).map_err(std::io::Error::other)?;
    json.truncate(json.trim_end().len());
    std::fs::write(meta_path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{export_string, stream, TempDir};
    use serde_json::{json, Value as Json};

    /// A session that names `quest_id` (above Classic's 9665, so it is exported) `title`.
    fn titled_session(started_at: f64, quest_id: i64, title: &str) -> Json {
        json!({
            "startedAt": started_at,
            "startedAtPrecise": started_at + 0.5,
            "events": [],
            "functions": {
                "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
                "GetQuestID": stream([(3.0, json!(quest_id))]),
                "GetTitleText": stream([(3.0, json!(title))]),
            },
        })
    }

    struct Checkout {
        dir: TempDir,
    }

    impl Checkout {
        fn new() -> Checkout {
            Checkout {
                dir: TempDir::new("trace-data"),
            }
        }

        /// Writes `submissions/<key>` holding `export_string`.
        fn submission(&self, key: &str, export_string: &str) -> &Self {
            let path = self.dir.0.join("submissions").join(key);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, json!({ "id": key, "export_string": export_string, "nickname": "never read" }).to_string()).unwrap();
            self
        }

        fn run(&self, output: &Path, threads: usize) -> Result<Meta, String> {
            let options = Options {
                trace_data: self.dir.0.clone(),
                output: output.to_owned(),
                sample_percent: None,
                threads,
                stats: false,
            };
            run(&options, &|_| {})
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn folds_the_newest_copy_of_each_session_and_counts_what_it_read() {
        let checkout = Checkout::new();
        let broken = "!QuestieTrace:1!abc$def!End:QuestieTrace:1!";
        let old = export_string(&[titled_session(100.0, 96659, "Old Title")]);
        let new = export_string(&[titled_session(100.0, 96659, "New Title")]);
        checkout
            .submission("2026-09/2026-09-01_000000Z_old.json", &old)
            // The newest submission holds the session twice: two exports.
            .submission("2026-10/2026-10-01_000000Z_new.json", &format!("{new}\n{new}"))
            .submission("2026-10/2026-10-02_000000Z_broken.json", broken)
            .submission(
                "2026-10/2026-10-03_000000Z_partial.json",
                &format!("{}{}", &old[..40], export_string(&[titled_session(200.0, 96660, "Other")])),
            );
        let output = TempDir::new("output");

        let meta = checkout.run(&output.0, 2).unwrap();

        assert_eq!((meta.session_count, meta.submission_count, meta.export_count), (2, 3, 4));
        // Of three copies of the session one is used: both of the newest submission's count.
        assert_eq!(meta.duplicate_sessions_skipped, 2);
        assert_eq!(meta.skipped_files.len(), 1);
        assert_eq!(meta.skipped_files[0].name, "2026-10/2026-10-02_000000Z_broken.json");
        assert_eq!(
            meta.skipped_files[0].error,
            "export 1: payload is not valid print-encoded data (character \"$\")"
        );
        assert_eq!(meta.partial_files.len(), 1);
        assert_eq!(meta.partial_files[0].dropped_exports, ["export 1 has no matching end marker"]);

        let quests = read(&output.0.join("foreverQuestTraces.lua"));
        // Session 100 was folded from the newest submission only.
        assert!(
            quests.contains("        [96659] = {\n            [questKeys.name] = \"New Title\","),
            "{quests}"
        );
        assert!(!quests.contains("Old Title"));
        assert!(quests.contains("        [96660] = {\n            [questKeys.name] = \"Other\","));
        assert!(quests.contains("-- Source trace files: 3\n-- Sessions aggregated: 2"));
        let mut on_disk = serde_json::to_string_pretty(&meta).unwrap();
        on_disk.truncate(on_disk.trim_end().len());
        assert_eq!(read(&output.0.join("meta.json")), on_disk);
    }

    #[test]
    fn the_output_does_not_depend_on_the_number_of_threads() {
        let checkout = Checkout::new();
        // Many submissions, each with a copy of a shared session, so copies race between workers.
        for i in 0..60 {
            let sessions = [
                titled_session(1000.0, 96659, &format!("Shared {i}")),
                titled_session(i as f64, 97000 + i, &format!("Own {i}")),
            ];
            checkout.submission(&format!("2026-10/2026-10-01_{i:06}Z_s.json"), &export_string(&sessions));
        }
        let (one, many) = (TempDir::new("one"), TempDir::new("many"));
        let meta_one = checkout.run(&one.0, 1).unwrap();
        let meta_many = checkout.run(&many.0, 8).unwrap();
        assert_eq!((meta_one.session_count, meta_one.duplicate_sessions_skipped), (61, 59));
        assert_eq!((meta_many.session_count, meta_many.duplicate_sessions_skipped), (61, 59));
        let without_time = |text: String| text.lines().filter(|l| !l.starts_with("-- Generated at: ")).collect::<Vec<_>>().join("\n");
        let quests_one = without_time(read(&one.0.join("foreverQuestTraces.lua")));
        assert_eq!(quests_one, without_time(read(&many.0.join("foreverQuestTraces.lua"))));
        // The newest submission's copy of the shared session names its quest; every other copy is skipped.
        assert!(
            quests_one.contains("[96659] = {\n            [questKeys.name] = \"Shared 59\","),
            "{quests_one}"
        );
        assert!(quests_one.contains("[97000] = {\n            [questKeys.name] = \"Own 0\","));
    }

    #[test]
    fn a_run_that_loads_no_submission_keeps_the_previous_output() {
        let checkout = Checkout::new();
        checkout.submission("2026-10/2026-10-01_000000Z_broken.json", "!QuestieTrace:1!abc$def!End:QuestieTrace:1!");
        let output = TempDir::new("output");
        std::fs::write(output.0.join("meta.json"), "previous").unwrap();
        std::fs::create_dir_all(output.0.join("lookupItems")).unwrap();
        std::fs::write(output.0.join("lookupItems/deDE.lua"), "previous").unwrap();

        let error = checkout.run(&output.0, 2).unwrap_err();

        assert_eq!(error, "No submission could be loaded; keeping the previous output");
        assert_eq!(read(&output.0.join("meta.json")), "previous");
        assert_eq!(read(&output.0.join("lookupItems/deDE.lua")), "previous");
    }

    #[test]
    fn replaces_the_lookup_directories_and_leaves_no_staging_behind() {
        let checkout = Checkout::new();
        checkout.submission("2026-10/2026-10-01_000000Z_a.json", &export_string(&[titled_session(1.0, 96659, "Title")]));
        let output = TempDir::new("output");
        std::fs::create_dir_all(output.0.join("lookupItems")).unwrap();
        std::fs::write(output.0.join("lookupItems/frFR.lua"), "a locale the new run has no data for").unwrap();

        checkout.run(&output.0, 2).unwrap();

        assert!(!output.0.join("lookupItems/frFR.lua").exists());
        assert!(output.0.join("lookupItems").is_dir());
        assert!(!output.0.join(".lookups-staging").exists());
        assert!(output.0.join("meta.json").exists());
    }
}
