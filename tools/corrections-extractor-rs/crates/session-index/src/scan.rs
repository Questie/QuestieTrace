//! Reads every submission of a trace-data checkout into its session copies, on every core.
//! Reads only: the checkout is never written to.

use crate::index::{CopyLocation, Identity, ScannedSubmission, SessionCopy, SubmissionContents};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use trace_data::source::{list_submissions, SubmissionFile};
use trace_data::submission::{Decoded, Submission};
use trace_data::{Arena, Inflater, Session, Value};

const PROGRESS_EVERY: usize = 5000;

/// The most arena memory a worker keeps between submissions: a reset arena keeps its largest
/// block, so without a cap every worker would hold on to the largest submission it decoded.
/// The value the corrections extractor measured best.
const ARENA_KEPT_BYTES: usize = 16 << 20;

/// Every submission under `<trace_data_dir>/submissions/`, newest first.
pub fn scan_checkout(trace_data_dir: &Path, log: &(dyn Fn(&str) + Sync)) -> Result<Vec<ScannedSubmission>, String> {
    let files = list_submissions(trace_data_dir).map_err(|e| format!("{} has no readable submissions directory: {e}", trace_data_dir.display()))?;
    log(&format!("{} submission(s) to read", files.len()));
    let done = AtomicUsize::new(0);
    let scanned = files
        .par_iter()
        .map_init(
            || (Inflater::new(), Arena::new()),
            |(inflater, arena), file| {
                let scanned = scan_submission(file, inflater, arena);
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                if done % PROGRESS_EVERY == 0 {
                    log(&format!("[{done}/{}] submissions read", files.len()));
                }
                scanned
            },
        )
        .collect();
    Ok(scanned)
}

/// Reads one submission. `inflater` and `arena` are a worker's, reused from one submission to
/// the next.
pub fn scan_submission(file: &SubmissionFile, inflater: &mut Inflater, arena: &mut Arena) -> ScannedSubmission {
    let submission = Submission::read(&file.path, inflater);
    let contents = session_copies(&file.key, &submission.decode(arena));
    submission.recycle(inflater);
    arena.reset();
    if arena.allocated_bytes() > ARENA_KEPT_BYTES {
        *arena = Arena::new();
    }
    ScannedSubmission {
        submission: file.key.clone(),
        contents,
    }
}

fn session_copies(submission: &str, decoded: &Decoded) -> Result<SubmissionContents, String> {
    if let Some(error) = &decoded.error {
        return Err(error.clone());
    }
    let mut contents = SubmissionContents {
        dropped_exports: decoded.dropped_exports.len(),
        ..SubmissionContents::default()
    };
    for trace in &decoded.traces {
        // What `Trace::sessions()` reads, with each record's position and the invalid ones counted.
        let Some(records) = trace.root().get("sessions").and_then(Value::as_array) else {
            contents.invalid_sessions += 1;
            continue;
        };
        for (position, record) in records.iter().enumerate() {
            let Some(session) = Session::new(record) else {
                contents.invalid_sessions += 1;
                continue;
            };
            let first_event = session.event(0).map(|event| (event.name(), event.t(), event.tp()));
            contents.copies.push(SessionCopy {
                location: CopyLocation {
                    submission: submission.to_owned(),
                    export: trace.export_index(),
                    session: position,
                },
                identity: session.key().map(|key| Identity::new(key, first_event)),
                events: session.event_count(),
                function_bytes: function_bytes(&session),
                stopped: session.field("stoppedAt").is_some(),
            });
        }
    }
    Ok(contents)
}

/// The encoded size of a session's function samples, read without decoding them.
fn function_bytes(session: &Session) -> usize {
    let streams = session.functions().entries().map(|(_, stream)| stream);
    streams
        .chain(session.field("functionsDelta"))
        .map(|samples| samples.encoded_len().expect("trace-data decodes function samples lazily"))
        .sum()
}
