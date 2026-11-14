//! One submission: decode its export(s) and distill every session, keeping only small owned
//! values so the decoded tree can be freed right away.

use super::distill::{distill_session, episode_key, is_recorded_forever, DistilledSession};
use std::path::Path;
use trace_data::submission::{submission_id_from_path, Submission};
use trace_data::{Arena, Inflater, Session, Value};

/// One submission, distilled.
pub struct DistilledSubmission {
    pub submission_id: String,
    /// Pseudonymous; empty when missing.
    pub contributor_id: String,
    /// ISO 8601; empty when missing.
    pub received_at: String,
    /// Exports that decoded.
    pub exports: usize,
    /// Why the submission gave nothing; `sessions` is then empty.
    pub error: Option<SubmissionError>,
    pub sessions: Vec<DistilledSession>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubmissionError {
    /// It could not be read or decoded: the reader's reason.
    Unreadable(String),
    /// Reading it panicked: a bug, whose message is in the run's log. Only the submission is
    /// lost, as with a crashed worker in the TS pipeline.
    Crashed,
}

impl DistilledSubmission {
    /// The stand-in for a submission whose reading panicked; only its file name is known.
    pub fn crashed(path: &Path) -> Self {
        DistilledSubmission {
            submission_id: submission_id_from_path(path),
            contributor_id: String::new(),
            received_at: String::new(),
            exports: 0,
            error: Some(SubmissionError::Crashed),
            sessions: Vec::new(),
        }
    }
}

/// The most arena memory a worker keeps between submissions. A reset arena keeps its largest
/// block, so without a cap every worker would hold on to the largest submission it ever decoded
/// (the corrections extractor measured the same cap as fastest and smallest).
const ARENA_KEPT_BYTES: usize = 16 << 20;

/// Reads, decodes and distills the submission at `path`. With `require_forever`, only sessions
/// that recorded a Forever build are kept: versioned dumps never infer a missing build.
pub fn distill_submission(path: &Path, require_forever: bool, inflater: &mut Inflater, arena: &mut Arena) -> DistilledSubmission {
    let submission = Submission::read(path, inflater);
    let (exports, error, sessions) = {
        let decoded = submission.decode(arena);
        let sessions = decoded
            .traces
            .iter()
            .flat_map(|trace| distill_sessions(trace.root(), require_forever))
            .collect();
        (decoded.traces.len(), decoded.error, sessions)
    };
    let distilled = DistilledSubmission {
        submission_id: submission.submission_id.clone(),
        contributor_id: submission.contributor_id.clone(),
        received_at: submission.received_at.clone(),
        exports,
        error: error.map(SubmissionError::Unreadable),
        sessions,
    };
    // The next submission inflates into this one's buffers and decodes into this arena.
    submission.recycle(inflater);
    arena.reset();
    if arena.allocated_bytes() > ARENA_KEPT_BYTES {
        *arena = Arena::new();
    }
    distilled
}

/// The sessions of one decoded export, distilled in order. Entries that are no session record
/// (an object with an `events` array and a `functions` object) are skipped.
pub fn distill_sessions(export: &Value, require_forever: bool) -> Vec<DistilledSession> {
    let records = export.get("sessions").and_then(Value::as_array).unwrap_or_default();
    records
        .iter()
        .filter_map(|record| {
            let session = Session::new(record)?;
            if require_forever && !is_recorded_forever(&session) {
                return None;
            }
            Some(distill_session(&session, episode_key(record)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_trace::decode_json;
    use serde_json::{json, Value as Json};

    fn session(build_info: Json) -> Json {
        json!({
            "schemaVersion": 9, "startedAt": 1, "startedAtPrecise": 1, "events": [{ "t": 0, "tp": 0, "e": "X" }],
            "functions": { "GetBuildInfo": build_info },
        })
    }

    fn versions(require_forever: bool) -> Vec<Option<i64>> {
        let player = |version: Json| json!({ "player": [{ "t": 0, "tp": 0, "v": version }] });
        let sessions = json!({ "sessions": [
            session(player(json!(16001))),
            session(player(json!(11508))),
            session(json!({ "_": [] })),
            // The parameterless capture, and the full packed GetBuildInfo tuple.
            session(json!([{ "t": 0, "tp": 0, "v": 16001 }])),
            session(player(json!({ "1": "1.15.8", "2": "64000", "3": "Oct 1 2026", "4": 16002, "n": 4 }))),
        ]});
        decode_json(&sessions, |export| {
            distill_sessions(export, require_forever)
                .iter()
                .map(|session| session.episode.interface_version)
                .collect()
        })
    }

    #[test]
    fn strict_ingest_keeps_only_sessions_that_recorded_a_forever_build() {
        assert_eq!(versions(false), [Some(16001), Some(11508), None, Some(16001), Some(16002)]);
        assert_eq!(versions(true), [Some(16001), Some(16001), Some(16002)]);
    }
}
