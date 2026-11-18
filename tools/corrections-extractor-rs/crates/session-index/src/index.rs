//! Groups session copies by session, picks the copy to keep, and tells which submissions hold
//! nothing that is not kept elsewhere.

use serde::Serialize;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use trace_data::SessionKey;

/// What tells a session apart from every other: its capture-start clocks ([`SessionKey`], what
/// the extractors match copies by) and its first event, which every copy of a session shares.
/// The first event keeps apart sessions whose clocks happen to agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Identity {
    pub key: SessionKey,
    first_event: Option<FirstEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FirstEvent {
    name: u64,
    t_micros: Option<i64>,
    tp_micros: Option<i64>,
}

impl Identity {
    /// `first_event`: the session's first event's name, `t` and `tp`. Times are compared to the
    /// microsecond: a saved copy went through SavedVariables, which keeps fewer digits
    /// (`tp` 0.0057977999999820895 comes back as 0.00579779999998209).
    pub fn new(key: SessionKey, first_event: Option<(Option<&str>, f64, f64)>) -> Identity {
        let micros = |seconds: f64| seconds.is_finite().then(|| (seconds * 1e6).round() as i64);
        let first_event = first_event.map(|(name, t, tp)| {
            let mut hasher = DefaultHasher::new();
            name.hash(&mut hasher);
            FirstEvent {
                name: hasher.finish(),
                t_micros: micros(t),
                tp_micros: micros(tp),
            }
        });
        Identity { key, first_event }
    }
}

/// Where a session copy is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CopyLocation {
    /// The submission file: `<yyyy-mm>/<file>.json` under `submissions/`.
    pub submission: String,
    /// Position among the export markers of the submission's `export_string` (0-based).
    pub export: usize,
    /// Position in that export's `sessions` array (0-based).
    pub session: usize,
}

/// One copy of a session, as one submission holds it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCopy {
    #[serde(flatten)]
    pub location: CopyLocation,
    /// None when the session lacks a capture-start clock: its copies cannot be told apart from
    /// other sessions.
    #[serde(skip)]
    pub identity: Option<Identity>,
    /// Recorded events.
    pub events: usize,
    /// Encoded size of the function samples (`functions` and `functionsDelta`).
    pub function_bytes: usize,
    /// Stopped (it has `stoppedAt`): a stopped session never records again.
    pub stopped: bool,
}

/// What one submission file held.
#[derive(Debug)]
pub struct ScannedSubmission {
    /// `<yyyy-mm>/<file>.json` under `submissions/`.
    pub submission: String,
    /// Its session copies, or why nothing in it could be decoded.
    pub contents: Result<SubmissionContents, String>,
}

#[derive(Debug, Default)]
pub struct SubmissionContents {
    /// In export order, then session order.
    pub copies: Vec<SessionCopy>,
    /// Exports in it that could not be decoded.
    pub dropped_exports: usize,
    /// Records in its decoded exports that are no valid session (`Session::new`), counting an
    /// export without a `sessions` array as one.
    pub invalid_sessions: usize,
}

/// One recorded session: the copy kept and every other copy of it.
#[derive(Debug)]
pub struct UniqueSession {
    pub kept: SessionCopy,
    /// By location.
    pub superseded: Vec<SessionCopy>,
}

/// Whether a submission holds anything that is not kept from elsewhere.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SubmissionStatus {
    /// Nothing in it could be decoded.
    Unreadable { error: String },
    /// Every export decoded, and none holds a session record.
    Empty,
    /// Every export decoded, every record in them is a valid session, and every one of them is
    /// superseded by a copy kept from another submission.
    Redundant { copies: usize },
    /// Holds a kept copy, or something this index cannot read (an export that did not decode, a
    /// record that is no valid session), which may hold anything.
    Needed {
        copies: usize,
        kept_copies: usize,
        dropped_exports: usize,
        invalid_sessions: usize,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SubmissionEntry {
    pub submission: String,
    #[serde(flatten)]
    pub status: SubmissionStatus,
}

/// Every session once, and the status of every submission.
#[derive(Debug)]
pub struct Index {
    /// By where the kept copy is.
    pub sessions: Vec<UniqueSession>,
    /// By submission.
    pub submissions: Vec<SubmissionEntry>,
}

/// Totals of an [`Index`].
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub submissions: usize,
    pub unreadable_submissions: usize,
    pub empty_submissions: usize,
    pub redundant_submissions: usize,
    pub needed_submissions: usize,
    pub session_copies: usize,
    pub unique_sessions: usize,
    /// Copies that are not kept.
    pub superseded_copies: usize,
    /// Of the superseded copies, those holding less than the kept copy (fewer events or less
    /// function data); the others repeat it.
    pub smaller_copies: usize,
    /// Sessions without a capture-start clock: each copy counts as its own session.
    pub sessions_without_key: usize,
    /// Sessions whose clocks another session has too (told apart by their first event). The
    /// extractors match copies by clocks alone, so they would mix these up.
    pub sessions_sharing_clocks: usize,
    /// Session records that are no valid session.
    pub invalid_sessions: usize,
}

/// How good a copy is; the greatest is kept.
///
/// 1. The most events.
/// 2. A stopped copy: the session's final state.
/// 3. The most function data: between events a session keeps sampling (the player's position,
///    for one), so in-progress copies with as many events can still differ.
/// 4. The newest submission (keys start with the receive time). Exports were cumulative, so the
///    newest submission tends to hold the older ones' sessions too, and keeping its copies leaves
///    the older submissions redundant.
/// 5. The first copy in that submission.
fn rank(copy: &SessionCopy) -> (usize, bool, usize, &str, Reverse<(usize, usize)>) {
    let location = &copy.location;
    (
        copy.events,
        copy.stopped,
        copy.function_bytes,
        &location.submission,
        Reverse((location.export, location.session)),
    )
}

impl Index {
    pub fn build(scanned: Vec<ScannedSubmission>) -> Index {
        let mut sessions = Vec::new();
        let mut copies_of: HashMap<Identity, Vec<SessionCopy>> = HashMap::new();
        // Every submission, with what it held once its copies are taken out.
        let mut held = Vec::with_capacity(scanned.len());
        for ScannedSubmission { submission, contents } in scanned {
            let counts = contents.map(|contents| {
                let counts = (contents.copies.len(), contents.dropped_exports, contents.invalid_sessions);
                for copy in contents.copies {
                    match copy.identity {
                        Some(identity) => copies_of.entry(identity).or_default().push(copy),
                        None => sessions.push(UniqueSession {
                            kept: copy,
                            superseded: Vec::new(),
                        }),
                    }
                }
                counts
            });
            held.push((submission, counts));
        }
        for mut copies in copies_of.into_values() {
            let best = (0..copies.len())
                .max_by(|&a, &b| rank(&copies[a]).cmp(&rank(&copies[b])))
                .expect("a session has a copy");
            let kept = copies.swap_remove(best);
            copies.sort_by(|a, b| a.location.cmp(&b.location));
            sessions.push(UniqueSession { kept, superseded: copies });
        }
        sessions.sort_by(|a, b| a.kept.location.cmp(&b.kept.location));

        let mut kept_copies_in: HashMap<&str, usize> = HashMap::new();
        for session in &sessions {
            *kept_copies_in.entry(&session.kept.location.submission).or_default() += 1;
        }
        let mut submissions: Vec<SubmissionEntry> = held
            .into_iter()
            .map(|(submission, counts)| {
                let status = match counts {
                    Err(error) => SubmissionStatus::Unreadable { error },
                    Ok((copies, dropped_exports, invalid_sessions)) => {
                        let kept_copies = kept_copies_in.get(submission.as_str()).copied().unwrap_or(0);
                        classify(copies, kept_copies, dropped_exports, invalid_sessions)
                    }
                };
                SubmissionEntry { submission, status }
            })
            .collect();
        submissions.sort_by(|a, b| a.submission.cmp(&b.submission));
        Index { sessions, submissions }
    }

    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            submissions: self.submissions.len(),
            unique_sessions: self.sessions.len(),
            ..Summary::default()
        };
        for entry in &self.submissions {
            match entry.status {
                SubmissionStatus::Unreadable { .. } => summary.unreadable_submissions += 1,
                SubmissionStatus::Empty => summary.empty_submissions += 1,
                SubmissionStatus::Redundant { .. } => summary.redundant_submissions += 1,
                SubmissionStatus::Needed { invalid_sessions, .. } => {
                    summary.needed_submissions += 1;
                    summary.invalid_sessions += invalid_sessions;
                }
            }
        }
        let mut sessions_per_key: HashMap<SessionKey, usize> = HashMap::new();
        for session in &self.sessions {
            let kept = &session.kept;
            summary.session_copies += 1 + session.superseded.len();
            summary.superseded_copies += session.superseded.len();
            summary.smaller_copies += session
                .superseded
                .iter()
                .filter(|copy| copy.events < kept.events || copy.function_bytes < kept.function_bytes)
                .count();
            match kept.identity {
                Some(identity) => *sessions_per_key.entry(identity.key).or_default() += 1,
                None => summary.sessions_without_key += 1,
            }
        }
        summary.sessions_sharing_clocks = sessions_per_key.values().filter(|&&count| count > 1).sum();
        summary
    }
}

fn classify(copies: usize, kept_copies: usize, dropped_exports: usize, invalid_sessions: usize) -> SubmissionStatus {
    if kept_copies > 0 || dropped_exports > 0 || invalid_sessions > 0 {
        SubmissionStatus::Needed {
            copies,
            kept_copies,
            dropped_exports,
            invalid_sessions,
        }
    } else if copies == 0 {
        SubmissionStatus::Empty
    } else {
        SubmissionStatus::Redundant { copies }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::session_key;

    /// The identity of a session started at `clock` (both clocks) whose first event is a login at 0.
    fn session(clock: f64) -> Option<Identity> {
        Some(Identity::new(session_key(clock, clock), Some((Some("PLAYER_LOGIN"), 0.0, 0.0))))
    }

    /// A copy with one event and no function data, in progress.
    fn copy(submission: &str, export: usize, session: usize, identity: Option<Identity>) -> SessionCopy {
        SessionCopy {
            location: CopyLocation {
                submission: submission.into(),
                export,
                session,
            },
            identity,
            events: 1,
            function_bytes: 0,
            stopped: false,
        }
    }

    fn read(submission: &str, copies: Vec<SessionCopy>) -> ScannedSubmission {
        ScannedSubmission {
            submission: submission.into(),
            contents: Ok(SubmissionContents {
                copies,
                ..SubmissionContents::default()
            }),
        }
    }

    /// Where the kept copy of the only session with `identity` is.
    fn kept(index: &Index, identity: Option<Identity>) -> &CopyLocation {
        let mut matching = index.sessions.iter().filter(|session| session.kept.identity == identity);
        let session = matching.next().expect("the session is indexed");
        assert!(matching.next().is_none(), "the session is indexed once");
        &session.kept.location
    }

    #[test]
    fn keeps_the_copy_with_the_most_events_even_when_a_newer_submission_holds_a_stopped_one() {
        let s = session(1000.0);
        let index = Index::build(vec![
            read(
                "2026-10/c.json",
                vec![SessionCopy {
                    events: 20,
                    stopped: true,
                    ..copy("2026-10/c.json", 0, 0, s)
                }],
            ),
            read(
                "2026-10/a.json",
                vec![SessionCopy {
                    events: 10,
                    ..copy("2026-10/a.json", 0, 0, s)
                }],
            ),
            read(
                "2026-10/b.json",
                vec![SessionCopy {
                    events: 30,
                    ..copy("2026-10/b.json", 0, 0, s)
                }],
            ),
        ]);

        assert_eq!(index.sessions.len(), 1);
        assert_eq!(kept(&index, s).submission, "2026-10/b.json");
        let superseded: Vec<&str> = index.sessions[0].superseded.iter().map(|c| c.location.submission.as_str()).collect();
        assert_eq!(superseded, ["2026-10/a.json", "2026-10/c.json"]);
    }

    #[test]
    fn among_copies_with_as_many_events_keeps_a_stopped_one_then_the_most_function_data_then_the_newest_then_the_first() {
        let (stopped, sampled, newest, first) = (session(1.0), session(2.0), session(3.0), session(4.0));
        let index = Index::build(vec![
            read(
                "2026-10/old.json",
                vec![
                    SessionCopy {
                        stopped: true,
                        ..copy("2026-10/old.json", 0, 0, stopped)
                    },
                    SessionCopy {
                        function_bytes: 900,
                        ..copy("2026-10/old.json", 0, 1, sampled)
                    },
                    copy("2026-10/old.json", 0, 2, newest),
                ],
            ),
            read(
                "2026-11/new.json",
                vec![
                    // More function data, but not stopped.
                    SessionCopy {
                        function_bytes: 50,
                        ..copy("2026-11/new.json", 0, 0, stopped)
                    },
                    SessionCopy {
                        function_bytes: 800,
                        ..copy("2026-11/new.json", 0, 1, sampled)
                    },
                    copy("2026-11/new.json", 0, 2, newest),
                    // The same session in two exports of one submission.
                    copy("2026-11/new.json", 1, 0, first),
                    copy("2026-11/new.json", 0, 3, first),
                ],
            ),
        ]);

        assert_eq!(kept(&index, stopped).submission, "2026-10/old.json");
        assert_eq!(kept(&index, sampled).submission, "2026-10/old.json");
        assert_eq!(kept(&index, newest).submission, "2026-11/new.json");
        assert_eq!((kept(&index, first).export, kept(&index, first).session), (0, 3));
    }

    #[test]
    fn sessions_with_the_same_clocks_but_another_first_event_are_different_sessions() {
        let key = session_key(5.0, 6.25);
        let login = |tp: f64| Some(Identity::new(key, Some((Some("PLAYER_LOGIN"), 0.0, tp))));
        let other = Some(Identity::new(key, Some((Some("QUEST_ACCEPTED"), 0.0, 0.0057977999999820895))));
        let index = Index::build(vec![
            // The in-progress copy and the saved copy of one session: SavedVariables kept fewer
            // digits of `tp`.
            read("2026-10/live.json", vec![copy("2026-10/live.json", 0, 0, login(0.0057977999999820895))]),
            read("2026-10/saved.json", vec![copy("2026-10/saved.json", 0, 0, login(0.00579779999998209))]),
            read("2026-10/other.json", vec![copy("2026-10/other.json", 0, 0, other)]),
        ]);

        assert_eq!(index.sessions.len(), 2);
        assert_eq!(kept(&index, other).submission, "2026-10/other.json");
        assert_eq!(index.sessions.iter().map(|s| s.superseded.len()).sum::<usize>(), 1);
        assert_eq!(index.summary().sessions_sharing_clocks, 2);
    }

    #[test]
    fn a_submission_is_redundant_only_when_every_record_in_it_is_a_superseded_copy() {
        let (s, t) = (session(1.0), session(2.0));
        let with = |submission: &str, copies: Vec<SessionCopy>, dropped_exports: usize, invalid_sessions: usize| ScannedSubmission {
            submission: submission.into(),
            contents: Ok(SubmissionContents {
                copies,
                dropped_exports,
                invalid_sessions,
            }),
        };
        let index = Index::build(vec![
            // Superseded copies of both sessions.
            read("2026-10/a.json", vec![copy("2026-10/a.json", 0, 0, s), copy("2026-10/a.json", 0, 1, t)]),
            // As superseded, but one of its exports did not decode.
            with("2026-10/b.json", vec![copy("2026-10/b.json", 0, 0, s)], 1, 0),
            // As superseded, but next to a record that is no valid session.
            with("2026-10/c.json", vec![copy("2026-10/c.json", 0, 0, s)], 0, 1),
            read(
                "2026-10/d.json",
                vec![
                    SessionCopy {
                        events: 2,
                        ..copy("2026-10/d.json", 0, 0, s)
                    },
                    copy("2026-10/d.json", 0, 1, t),
                ],
            ),
            read(
                "2026-10/e.json",
                vec![SessionCopy {
                    events: 2,
                    ..copy("2026-10/e.json", 0, 0, t)
                }],
            ),
            read("2026-10/f.json", vec![]),
            ScannedSubmission {
                submission: "2026-10/g.json".into(),
                contents: Err("no export_string".into()),
            },
        ]);

        let needed = |copies, kept_copies, dropped_exports, invalid_sessions| SubmissionStatus::Needed {
            copies,
            kept_copies,
            dropped_exports,
            invalid_sessions,
        };
        let statuses: Vec<(&str, &SubmissionStatus)> = index.submissions.iter().map(|e| (e.submission.as_str(), &e.status)).collect();
        assert_eq!(
            statuses,
            [
                ("2026-10/a.json", &SubmissionStatus::Redundant { copies: 2 }),
                ("2026-10/b.json", &needed(1, 0, 1, 0)),
                ("2026-10/c.json", &needed(1, 0, 0, 1)),
                // Keeps s; its copy of t is superseded by e's.
                ("2026-10/d.json", &needed(2, 1, 0, 0)),
                ("2026-10/e.json", &needed(1, 1, 0, 0)),
                ("2026-10/f.json", &SubmissionStatus::Empty),
                (
                    "2026-10/g.json",
                    &SubmissionStatus::Unreadable {
                        error: "no export_string".into()
                    }
                ),
            ]
        );
    }

    #[test]
    fn copies_without_a_key_are_never_matched_so_each_is_kept() {
        let index = Index::build(vec![
            read("2026-10/a.json", vec![copy("2026-10/a.json", 0, 0, None)]),
            read("2026-10/b.json", vec![copy("2026-10/b.json", 0, 0, None)]),
        ]);

        assert_eq!(index.sessions.len(), 2);
        assert!(index
            .submissions
            .iter()
            .all(|entry| matches!(entry.status, SubmissionStatus::Needed { kept_copies: 1, .. })));
        assert_eq!(index.summary().sessions_without_key, 2);
    }

    #[test]
    fn lists_sessions_by_where_their_kept_copy_is() {
        let copies = (0..8).map(|i| copy("2026-10/a.json", i / 4, i % 4, session(i as f64))).collect();
        let index = Index::build(vec![read("2026-10/a.json", copies)]);

        let positions: Vec<(usize, usize)> = index.sessions.iter().map(|s| (s.kept.location.export, s.kept.location.session)).collect();
        assert_eq!(positions, [(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1), (1, 2), (1, 3)]);
    }

    #[test]
    fn the_summary_tells_smaller_copies_from_repeated_ones() {
        let s = session(1.0);
        let index = Index::build(vec![
            read(
                "2026-10/a.json",
                vec![SessionCopy {
                    events: 4,
                    ..copy("2026-10/a.json", 0, 0, s)
                }],
            ),
            read(
                "2026-10/b.json",
                vec![SessionCopy {
                    events: 9,
                    function_bytes: 70,
                    ..copy("2026-10/b.json", 0, 0, s)
                }],
            ),
            read(
                "2026-10/c.json",
                vec![SessionCopy {
                    events: 9,
                    function_bytes: 80,
                    ..copy("2026-10/c.json", 0, 0, s)
                }],
            ),
            read(
                "2026-10/d.json",
                vec![SessionCopy {
                    events: 9,
                    function_bytes: 80,
                    ..copy("2026-10/d.json", 0, 0, s)
                }],
            ),
            read("2026-10/e.json", vec![]),
        ]);

        assert_eq!(
            index.summary(),
            Summary {
                submissions: 5,
                unreadable_submissions: 0,
                empty_submissions: 1,
                redundant_submissions: 3,
                needed_submissions: 1,
                session_copies: 4,
                unique_sessions: 1,
                superseded_copies: 3,
                // a (fewer events) and b (less function data); c repeats d, which is kept.
                smaller_copies: 2,
                sessions_without_key: 0,
                sessions_sharing_clocks: 0,
                invalid_sessions: 0,
            }
        );
    }
}
