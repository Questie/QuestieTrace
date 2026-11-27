//! Merges the copies of one recording session that appear in several submissions.
//!
//! Exports used to be cumulative and always include the in-progress `currentSession`, so one
//! session shows up in many submissions, growing as play continues. Sessions are identified by
//! their capture-start clocks (see [`super::distill::episode_key`]); the copy with the most events
//! is the most complete. (The field extractor keeps the newest copy instead; a newer copy only
//! ever extends an older one, so both rules pick the same copy in practice.)
//!
//! Copies arrive in whatever order the workers finish them. Every tie is broken by where a copy
//! was met ([`CopyOrder`]), the order the TypeScript pipeline merged them in, so the result is the
//! same whatever the arrival order.

use super::distill::DistilledSession;
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::collections::BTreeSet;

/// Where a copy was met: the submission's position in path order (oldest first), then the copy's
/// position among the submission's sessions.
pub type CopyOrder = (usize, usize);

/// The submission a copy came from.
pub struct SubmissionInfo<'s> {
    pub submission_id: &'s str,
    pub contributor_id: &'s str,
    /// ISO 8601, compared as text.
    pub received_at: &'s str,
    /// Position in path order.
    pub index: usize,
}

pub struct MergedSession {
    /// The most complete copy, carrying the first-met session name of any copy.
    pub session: DistilledSession,
    best: (String, CopyOrder),
    /// Contributor of the earliest received submission containing the session.
    pub contributor_id: String,
    pub first_received_at: String,
    first_received_index: usize,
    pub seen_under_several_contributors: bool,
    pub submission_ids: BTreeSet<String>,
    /// The first-met name: the name only exists on saved copies, but a bigger in-progress copy
    /// still gets it.
    name: Option<(CopyOrder, String)>,
}

/// How good a copy is; the greatest wins. The copy with the most events, then a saved one, then
/// the one from the submission with the smallest id, then the first met.
fn rank<'s>(session: &DistilledSession, (submission_id, order): &'s (String, CopyOrder)) -> (usize, bool, Reverse<&'s str>, Reverse<CopyOrder>) {
    (session.event_count, session.saved, Reverse(submission_id.as_str()), Reverse(*order))
}

impl MergedSession {
    fn new(submission: &SubmissionInfo, order: CopyOrder, session: DistilledSession) -> Self {
        MergedSession {
            name: session.episode.session_name.clone().map(|name| (order, name)),
            session,
            best: (submission.submission_id.to_owned(), order),
            contributor_id: submission.contributor_id.to_owned(),
            first_received_at: submission.received_at.to_owned(),
            first_received_index: submission.index,
            seen_under_several_contributors: false,
            submission_ids: BTreeSet::from([submission.submission_id.to_owned()]),
        }
    }

    fn add(&mut self, submission: &SubmissionInfo, order: CopyOrder, copy: DistilledSession) {
        self.submission_ids.insert(submission.submission_id.to_owned());
        // Any second contributor differs from the one held at the time it arrives.
        if submission.contributor_id != self.contributor_id {
            self.seen_under_several_contributors = true;
        }
        if (submission.received_at, submission.index) < (self.first_received_at.as_str(), self.first_received_index) {
            self.first_received_at = submission.received_at.to_owned();
            self.first_received_index = submission.index;
            self.contributor_id = submission.contributor_id.to_owned();
        }
        if let Some(name) = &copy.episode.session_name {
            if self.name.as_ref().is_none_or(|(first, _)| order < *first) {
                self.name = Some((order, name.clone()));
            }
        }
        let candidate = (submission.submission_id.to_owned(), order);
        if rank(&copy, &candidate) > rank(&self.session, &self.best) {
            self.session = copy;
            self.best = candidate;
        }
        self.session.episode.session_name = self.name.as_ref().map(|(_, name)| name.clone());
    }
}

#[derive(Default)]
pub struct SessionMerger {
    pub merged: FxHashMap<String, MergedSession>,
    /// Every copy added.
    pub copies: usize,
}

impl SessionMerger {
    pub fn add(&mut self, submission: &SubmissionInfo, order: CopyOrder, session: DistilledSession) {
        self.copies += 1;
        match self.merged.get_mut(&session.key) {
            Some(merged) => merged.add(submission, order, session),
            None => {
                self.merged.insert(session.key.clone(), MergedSession::new(submission, order, session));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::distill::{DistillStats, EpisodeDraft};
    use crate::model::PlayerContext;

    fn copy(event_count: usize, saved: bool) -> DistilledSession {
        DistilledSession {
            key: "session-1".into(),
            event_count,
            saved,
            start_server_time: None,
            episode: EpisodeDraft {
                session_name: saved.then(|| "2026-10-01_12-00-00".into()),
                interface_version: None,
                locale: None,
                player: PlayerContext::default(),
                duration: event_count as f64,
                levels: vec![],
                completed: None,
                quest_log: vec![],
                quest_events: vec![],
                offers: vec![],
                quest_info: None,
            },
            pending_greetings: vec![],
            titles: vec![],
            stats: DistillStats::default(),
        }
    }

    fn submission<'s>(submission_id: &'s str, contributor_id: &'s str, received_at: &'s str, index: usize) -> SubmissionInfo<'s> {
        SubmissionInfo {
            submission_id,
            contributor_id,
            received_at,
            index,
        }
    }

    #[test]
    fn keeps_the_most_complete_copy_and_remembers_every_submission_it_came_in() {
        let mut merger = SessionMerger::default();
        merger.add(&submission("b", "late-contributor", "2026-10-02", 1), (1, 0), copy(80, false));
        merger.add(&submission("a", "first-contributor", "2026-10-01", 0), (0, 0), copy(50, true));
        merger.add(&submission("c", "late-contributor", "2026-10-03", 2), (2, 0), copy(120, false));

        let merged = &merger.merged["session-1"];
        assert_eq!(merger.copies, 3);
        assert_eq!(merged.session.event_count, 120);
        // The name only exists on the saved copy; the bigger in-progress copy still gets it.
        assert_eq!(merged.session.episode.session_name.as_deref(), Some("2026-10-01_12-00-00"));
        assert_eq!(merged.submission_ids.iter().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(merged.contributor_id, "first-contributor");
        assert!(merged.seen_under_several_contributors);
    }

    #[test]
    fn prefers_the_saved_copy_when_an_in_progress_copy_has_as_many_events() {
        let mut merger = SessionMerger::default();
        merger.add(&submission("a", "c", "2026-10-01", 0), (0, 0), copy(80, false));
        merger.add(&submission("b", "c", "2026-10-02", 1), (1, 0), copy(80, true));
        assert!(merger.merged["session-1"].session.saved);
        assert!(!merger.merged["session-1"].seen_under_several_contributors);
    }

    #[test]
    fn the_result_does_not_depend_on_the_order_copies_arrive_in() {
        // Equal copies differ only in duration; ties go to the smaller submission id, then to the
        // copy met first, and equal receive times to the submission met first.
        let mut copies = vec![(submission("b", "x", "2026-10-01", 0), (0, 0), copy(80, false))];
        copies.push((submission("b", "y", "2026-10-01", 1), (1, 0), copy(80, false)));
        copies.push((submission("a", "z", "2026-10-02", 2), (2, 1), copy(80, false)));
        copies.push((submission("a", "z", "2026-10-02", 2), (2, 0), copy(80, false)));
        for (i, (_, order, session)) in copies.iter_mut().enumerate() {
            session.episode.duration = i as f64;
            if order.0 > 0 {
                session.episode.session_name = Some(format!("2026-10-0{}_12-00-00", order.0 + order.1));
            }
        }
        let outcome = |arrival: &[usize]| {
            let mut merger = SessionMerger::default();
            for &i in arrival {
                let (info, order, session) = &copies[i];
                merger.add(info, *order, session.clone());
            }
            let merged = &merger.merged["session-1"];
            (
                merged.session.episode.duration,
                merged.session.episode.session_name.clone(),
                merged.contributor_id.clone(),
            )
        };
        // "a" is the smaller id and (2, 0) met first of its two copies; the first named copy is
        // (1, 0); the earliest receive time is shared by x and y, x's submission was met first.
        let expected = (3.0, Some("2026-10-01_12-00-00".to_string()), "x".to_string());
        for arrival in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1], [1, 3, 0, 2]] {
            assert_eq!(outcome(&arrival), expected, "arrival order {arrival:?}");
        }
    }
}
