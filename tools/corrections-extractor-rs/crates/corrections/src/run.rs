//! A full extraction run over a local trace-data checkout. Ports the parts of
//! `extract-corrections.ts` that remain without the incremental store.
//!
//! Submissions are read newest first. A window of them is decoded and observed in parallel,
//! then folded in order on this thread, so of each session only the first copy met (the
//! newest, so the most complete) is used. Exports are cumulative: one session shows up in
//! many submissions.

use crate::finalize::{finalize, Bundle};
use crate::fold::Accumulator;
use crate::lua::iso_timestamp;
use crate::observe::{observe_session, StagedSession, MAX_IDS};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use trace_data::source::{in_sample, list_submissions, SubmissionFile};
use trace_data::submission::Submission;
use trace_data::{Decompressor, SessionKey};

pub struct Options {
    pub trace_data: PathBuf,
    pub output: PathBuf,
    /// Only this percentage of submissions, the same ones every run.
    pub sample_percent: Option<f64>,
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
    submission_id: String,
    contributor_id: String,
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
fn observe_submission(file: &SubmissionFile, decompressor: &mut Decompressor, used: &HashSet<SessionKey>) -> ObservedSubmission {
    let submission = Submission::read(&file.path, decompressor);
    let decoded = submission.decode();
    // Only the first copy of a session in a submission is used.
    let mut seen = HashSet::new();
    let mut sessions = Vec::new();
    for trace in &decoded.traces {
        for session in trace.sessions() {
            let key = session.key();
            let duplicate = key.is_some_and(|k| used.contains(&k) || !seen.insert(k));
            let staged = if duplicate { None } else { observe_session(&session, &MAX_IDS) };
            sessions.push(ObservedSession { key, duplicate, staged });
        }
    }
    ObservedSubmission {
        key: file.key.clone(),
        submission_id: submission.submission_id.clone(),
        contributor_id: submission.contributor_id.clone(),
        export_count: decoded.traces.len(),
        dropped_exports: decoded.dropped_exports,
        error: decoded.error,
        sessions,
    }
}

/// What the fold tallies besides the accumulator.
#[derive(Default)]
struct Tally {
    used_keys: HashSet<SessionKey>,
    submission_ids: Vec<String>,
    export_count: usize,
    skipped_files: Vec<SkippedFile>,
    partial_files: Vec<PartialFile>,
    duplicates: usize,
}

fn fold(submission: ObservedSubmission, acc: &mut Accumulator, tally: &mut Tally) {
    if let Some(error) = submission.error {
        tally.skipped_files.push(SkippedFile { name: submission.key, error });
        return;
    }
    tally.submission_ids.push(submission.submission_id);
    tally.export_count += submission.export_count;
    if !submission.dropped_exports.is_empty() {
        tally.partial_files.push(PartialFile {
            name: submission.key,
            dropped_exports: submission.dropped_exports,
        });
    }
    let contributor = Some(submission.contributor_id.as_str()).filter(|c| !c.is_empty());
    for session in submission.sessions {
        // A copy marked while observing, or one whose key an earlier submission of the same
        // window used: the observing thread could not know.
        if session.duplicate || session.key.is_some_and(|key| !tally.used_keys.insert(key)) {
            tally.duplicates += 1;
            continue;
        }
        if let Some(staged) = session.staged {
            acc.add(staged, contributor);
        }
    }
}

/// Submissions observed in parallel before folding them in order.
fn window_size() -> usize {
    rayon::current_num_threads() * 8
}

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
    let window = window_size();
    for (index, chunk) in files.chunks(window).enumerate() {
        let used = &tally.used_keys;
        let observed: Vec<ObservedSubmission> = chunk
            .par_iter()
            .map_init(Decompressor::new, |d, file| observe_submission(file, d, used))
            .collect();
        for submission in observed {
            fold(submission, &mut acc, &mut tally);
        }
        let done = ((index + 1) * window).min(files.len());
        if done % (window * 16) < window || done == files.len() {
            log(&format!("[{done}/{}] submissions observed", files.len()));
        }
    }
    if tally.submission_ids.is_empty() {
        return Err("No submission could be loaded; keeping the previous output".into());
    }

    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default();
    let generated_at = iso_timestamp(millis);
    let bundle = finalize(&acc, tally.submission_ids.len(), &generated_at);
    let meta = Meta {
        session_count: acc.session_count(),
        submission_count: tally.submission_ids.len(),
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
