//! Ingest: trace-data submissions -> one [`QuestEpisode`] per recording session, its copies across
//! submissions merged, its character worked out.
//!
//! 1. Every submission is decoded and its sessions distilled on all cores ([`submission`],
//!    [`distill`]); only the small distilled values leave a worker, so decoded trees never pile up.
//! 2. On one thread, as results arrive: the copies of each session are merged ([`merge`]) and every
//!    title pairing is indexed ([`titles`]).
//! 3. Legacy greeting titles are resolved, sessions without quest data dropped, sessions grouped
//!    into characters and ordered ([`characters`]), and the episodes written with
//!    reports/ingest.json ([`report`]).
//!
//! Nothing is cached between runs: every run decodes every submission.

pub mod characters;
pub mod distill;
pub mod merge;
pub mod report;
pub mod streams;
pub mod submission;
pub mod titles;

#[cfg(test)]
mod test_trace;

use crate::config::Config;
use crate::io;
use crate::model::QuestEpisode;
use anyhow::{bail, Context, Result};
use characters::{assign_characters, order_sessions, CharacterInput};
use distill::{DistillStats, EpisodeIdentity};
use merge::{MergedSession, SessionMerger, SubmissionInfo};
use rayon::prelude::*;
use report::{IngestReport, Skipped, Tally};
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Instant;
use submission::{distill_submission, DistilledSubmission, SubmissionError};
use titles::{resolve_greeting, GreetingStats, TitleIndex};
use trace_data::source::{list_submissions, SubmissionFile};
use trace_data::{Arena, Inflater};

pub struct IngestOutput {
    pub episodes: Vec<QuestEpisode>,
    /// reports/ingest.json
    pub report: serde_json::Value,
}

impl IngestOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        io::write_episodes(&config.episodes(), &self.episodes)?;
        io::write_json(&config.report("ingest"), &self.report)
    }
}

/// The first 16 hex digits of SHA-256(`text`): the pseudonymous keys episodes carry.
fn short_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn run(config: &Config) -> Result<IngestOutput> {
    let started = Instant::now();
    let trace_data_dir = config.trace_data_dir()?;
    let mut files = list_submissions(trace_data_dir).with_context(|| format!("listing {}/submissions", trace_data_dir.display()))?;
    if files.is_empty() {
        bail!("{}/submissions has no submissions", trace_data_dir.display());
    }
    // Oldest first, the order the TS pipeline merged in: ties between copies go to the earlier one.
    files.sort_by(|a, b| a.key.cmp(&b.key));
    eprintln!("ingest: decoding {} submissions", files.len());

    let mut read = ReadTally::default();
    let require_forever = config.require_forever_version;
    let distill = |file: &SubmissionFile, inflater: &mut Inflater, arena: &mut Arena| distill_submission(&file.path, require_forever, inflater, arena);
    distill_all(&files, distill, |index, submission| {
        read.add(index, submission);
        if read.submissions % 1000 == 0 {
            eprintln!("ingest: {}/{} decoded ({:.0}s)", read.submissions, files.len(), started.elapsed().as_secs_f64());
        }
    });
    eprintln!("ingest: decoded in {:.0}s, building episodes", started.elapsed().as_secs_f64());

    let (episodes, tally) = build_episodes(read);
    let seconds = started.elapsed().as_secs_f64().round() as u64;
    let report = IngestReport::new(config.generated_at.clone(), seconds, config.require_forever_version, tally, &episodes);
    eprintln!(
        "ingest: {} episodes of {} characters, {} submissions failed",
        episodes.len(),
        report.episodes.characters,
        report.submissions.failed
    );
    Ok(IngestOutput {
        episodes,
        report: serde_json::to_value(&report)?,
    })
}

/// Results waiting for the merge at most; workers block beyond that.
const RESULTS_IN_FLIGHT: usize = 64;

/// Runs `distill` (decode and distill one submission) on every file on all cores and hands each
/// result, with its position in `files`, to `merge` on this thread in whatever order they finish
/// (the merge does not depend on that order). A submission whose `distill` panics becomes a
/// [`SubmissionError::Crashed`] failure of its own; the run goes on.
fn distill_all(
    files: &[SubmissionFile],
    distill: impl Fn(&SubmissionFile, &mut Inflater, &mut Arena) -> DistilledSubmission + Sync,
    mut merge: impl FnMut(usize, DistilledSubmission),
) {
    let (results, received) = mpsc::sync_channel(RESULTS_IN_FLIGHT);
    let distill = &distill;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            files.par_iter().enumerate().for_each_init(
                || (Inflater::new(), Arena::new()),
                |(inflater, arena), (index, file)| {
                    let distilled = catch_unwind(AssertUnwindSafe(|| distill(file, inflater, arena))).unwrap_or_else(|_| {
                        // Nothing borrows the crashed submission's values any more.
                        arena.reset();
                        DistilledSubmission::crashed(&file.path)
                    });
                    // Only fails once the merge stopped listening, which it does by panicking.
                    let _ = results.send((index, distilled));
                },
            );
        });
        for (index, distilled) in received {
            merge(index, distilled);
        }
    });
}

/// What phase 2 needs from every submission read.
#[derive(Default)]
struct ReadTally {
    submissions: usize,
    /// (position in path order, submission id, why).
    failures: Vec<(usize, String, SubmissionError)>,
    exports: usize,
    with_multiple_exports: usize,
    merger: SessionMerger,
    titles: TitleIndex,
}

impl ReadTally {
    fn add(&mut self, index: usize, submission: DistilledSubmission) {
        self.submissions += 1;
        if let Some(error) = submission.error {
            self.failures.push((index, submission.submission_id.clone(), error));
        }
        self.exports += submission.exports;
        self.with_multiple_exports += usize::from(submission.exports > 1);
        let info = SubmissionInfo {
            submission_id: &submission.submission_id,
            contributor_id: &submission.contributor_id,
            received_at: &submission.received_at,
            index,
        };
        for (position, mut session) in submission.sessions.into_iter().enumerate() {
            let locale = session.episode.locale.as_deref();
            for observation in &session.titles {
                self.titles.add(locale, session.episode.player.faction, observation);
                self.titles.note_sighting(locale, &observation.title, info.contributor_id);
            }
            for pending in &session.pending_greetings {
                for title in pending.available.iter().chain(&pending.active) {
                    self.titles.note_sighting(locale, title, info.contributor_id);
                }
            }
            // Indexed above; holding them for every kept session only costs memory.
            session.titles = Vec::new();
            self.merger.add(&info, (index, position), session);
        }
    }
}

fn has_quest_data(merged: &MergedSession) -> bool {
    let episode = &merged.session.episode;
    episode.completed.is_some() || !episode.quest_events.is_empty() || !episode.offers.is_empty() || episode.quest_log.iter().any(|entry| !entry.v.is_empty())
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Epoch seconds of a UTC date and time; out-of-range months, days and times carry over, as with
/// JS `Date.UTC`.
fn utc_seconds(year: i64, month: i64, day: i64, hours: i64, minutes: i64, seconds: i64) -> f64 {
    let (year, month) = (year + (month - 1).div_euclid(12), (month - 1).rem_euclid(12) + 1);
    let days = days_from_civil(year, month, 1) + day - 1;
    (days * 86_400 + hours * 3600 + minutes * 60 + seconds) as f64
}

/// The session name is the local wall-clock time the session was saved, "YYYY-MM-DD_HH-MM-SS".
fn session_name_time(name: Option<&str>) -> Option<f64> {
    let name = name.filter(|name| distill::is_session_name(name))?;
    let field = |range: std::ops::Range<usize>| name[range].parse::<i64>().expect("digits");
    Some(utc_seconds(field(0..4), field(5..7), field(8..10), field(11..13), field(14..16), field(17..19)))
}

/// Epoch seconds of an ISO 8601 time as trace-data writes `received_at`
/// ("2026-10-01T00:05:00.708876249Z"), to the millisecond like JS `Date.parse`. None for anything
/// else.
fn parse_received_at(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let digits = text.get(range)?;
        digits.bytes().all(|b| b.is_ascii_digit()).then(|| digits.parse().ok())?
    };
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let mut rest = text.get(19..)?;
    let mut millis = 0.0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let ms: String = fraction[..digits].chars().chain("00".chars()).take(3).collect();
        millis = ms.parse::<f64>().ok()? / 1000.0;
        rest = &fraction[digits..];
    }
    let offset_minutes = match rest {
        "Z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let (hours, minutes) = rest[1..].split_once(':')?;
            if hours.len() != 2 || minutes.len() != 2 {
                return None;
            }
            sign * (hours.parse::<i64>().ok()? * 60 + minutes.parse::<i64>().ok()?)
        }
    };
    let seconds = utc_seconds(number(0..4)?, number(5..7)?, number(8..10)?, number(11..13)?, number(14..16)?, number(17..19)?);
    Some(seconds + millis - (offset_minutes * 60) as f64)
}

/// Approximate epoch start of each session, for ordering a contributor's sessions. GetServerTime
/// at capture start when recorded (nearly always); otherwise the save stamp shifted by the
/// contributor's usual local-clock offset, or the first submission's receive time.
fn session_order_times(sessions: &[MergedSession]) -> Vec<f64> {
    let mut offsets: FxHashMap<&str, Vec<f64>> = FxHashMap::default();
    for merged in sessions {
        let (start, episode) = (merged.session.start_server_time, &merged.session.episode);
        if let (Some(start), Some(saved)) = (start, session_name_time(episode.session_name.as_deref())) {
            offsets.entry(&merged.contributor_id).or_default().push(saved - (start + episode.duration));
        }
    }
    let medians: FxHashMap<&str, f64> = offsets
        .into_iter()
        .map(|(contributor, mut values)| {
            values.sort_by(f64::total_cmp);
            (contributor, values[values.len() >> 1])
        })
        .collect();
    sessions
        .iter()
        .map(|merged| {
            let episode = &merged.session.episode;
            let order = merged
                .session
                .start_server_time
                .unwrap_or_else(|| match session_name_time(episode.session_name.as_deref()) {
                    Some(saved) => saved - medians.get(merged.contributor_id.as_str()).copied().unwrap_or(0.0) - episode.duration,
                    None => parse_received_at(&merged.first_received_at).unwrap_or(f64::NAN) - episode.duration,
                });
            if order.is_finite() {
                order
            } else {
                0.0
            }
        })
        .collect()
}

/// Phase 2: drops sessions without quest data, resolves greeting titles, groups characters and
/// builds the sorted episodes.
fn build_episodes(read: ReadTally) -> (Vec<QuestEpisode>, Tally) {
    let ReadTally {
        submissions,
        mut failures,
        exports,
        with_multiple_exports,
        merger,
        titles,
    } = read;
    let (copies, unique) = (merger.copies, merger.merged.len());
    let mut merged: Vec<MergedSession> = merger.merged.into_values().collect();
    merged.sort_by(|a, b| a.session.key.cmp(&b.session.key));
    let seen_under_several_contributors = merged.iter().filter(|merged| merged.seen_under_several_contributors).count();

    let mut skipped = Skipped::default();
    let mut kept: Vec<MergedSession> = Vec::new();
    for session in merged {
        if session.session.event_count == 0 {
            *skipped.no_events.get_or_insert(0) += 1;
        } else if !has_quest_data(&session) {
            *skipped.no_quest_data.get_or_insert(0) += 1;
        } else {
            kept.push(session);
        }
    }

    let mut greetings = GreetingStats::default();
    let mut distill = DistillStats::default();
    for merged in &mut kept {
        let session = &mut merged.session;
        let context = (session.episode.locale.as_deref(), session.episode.player.faction);
        for pending in &session.pending_greetings {
            resolve_greeting(&mut session.episode.offers[pending.offer], pending, context, &titles, &mut greetings);
        }
        distill += session.stats;
    }

    let order = session_order_times(&kept);
    let inputs: Vec<CharacterInput> = kept
        .iter()
        .zip(&order)
        .map(|(merged, &order)| {
            let (session, episode) = (&merged.session, &merged.session.episode);
            CharacterInput {
                episode_key: &session.key,
                contributor_id: &merged.contributor_id,
                player: &episode.player,
                start: session.start_server_time,
                duration: episode.duration,
                order,
                session_name: episode.session_name.as_deref(),
                first_level: episode.levels.first().map(|level| level.v),
                last_level: episode.levels.last().map(|level| level.v),
                completed: episode.completed.as_ref(),
            }
        })
        .collect();
    let characters = assign_characters(&inputs);
    let session_orders = order_sessions(&inputs, &characters);
    drop(inputs);

    let mut episodes: Vec<QuestEpisode> = kept
        .into_iter()
        .zip(characters)
        .zip(session_orders)
        .map(|((merged, character_key), session_order)| {
            let identity = EpisodeIdentity {
                key: merged.session.key,
                contributor_id: merged.contributor_id,
                character_key,
                submission_ids: merged.submission_ids.into_iter().collect(),
                session_order,
            };
            merged.session.episode.into_episode(identity)
        })
        .collect();
    // Contributor ids and keys are lowercase hex, so comparing them as text orders them like the
    // TS localeCompare.
    episodes
        .sort_by(|a, b| (&a.contributor_id, &a.character_key, a.session_order, &a.key).cmp(&(&b.contributor_id, &b.character_key, b.session_order, &b.key)));

    failures.sort_by_key(|(index, _, _)| *index);
    let tally = Tally {
        submissions,
        failures: failures.into_iter().map(|(_, submission_id, error)| (submission_id, error)).collect(),
        exports,
        with_multiple_exports,
        copies,
        unique,
        seen_under_several_contributors,
        skipped,
        greetings,
        distill,
    };
    (episodes, tally)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_receive_times_and_session_names_as_js_date_parse_and_date_utc_do() {
        // node: Date.parse(text) / 1000, which keeps milliseconds and drops finer digits.
        assert_eq!(parse_received_at("2026-10-01T00:05:00.708876249Z"), Some(1790813100.708));
        assert_eq!(parse_received_at("2026-10-01T00:05:00.708876249+02:00"), Some(1790805900.708));
        assert_eq!(parse_received_at("2026-10-01T00:05:00Z"), Some(1790813100.0));
        assert_eq!(parse_received_at(""), None);
        // A multi-byte character where the time zone starts is no time, not a crash.
        assert_eq!(parse_received_at("2026-10-01T00:05:0\u{e9}Z"), None);
        // node: Date.UTC(2026, 9, 1, 12, 0, 0) / 1000; out-of-range fields carry over.
        assert_eq!(session_name_time(Some("2026-10-01_12-00-00")), Some(1790856000.0));
        assert_eq!(session_name_time(Some("2026-13-31_24-00-00")), Some(1801440000.0));
        assert_eq!(session_name_time(Some("raid night")), None);
    }

    #[test]
    fn a_submission_whose_reading_panics_fails_alone_and_the_run_goes_on() {
        let files: Vec<SubmissionFile> = (0..40)
            .map(|i| {
                let name = format!("2026-10-01_000000Z_{i:02}.json");
                SubmissionFile {
                    key: format!("2026-10/{name}"),
                    path: std::path::Path::new("/nowhere").join(name),
                }
            })
            .collect();
        let distill = |file: &SubmissionFile, _: &mut Inflater, _: &mut Arena| {
            assert!(!file.key.ends_with("_13.json"), "a bug met reading submission 13");
            DistilledSubmission {
                exports: 1,
                error: None,
                ..DistilledSubmission::crashed(&file.path)
            }
        };
        let mut results = std::collections::BTreeMap::new();
        distill_all(&files, distill, |index, submission| {
            results.insert(index, (submission.submission_id, submission.error));
        });
        assert_eq!(results.len(), 40, "every submission has a result");
        assert_eq!(results[&13], ("13".to_string(), Some(SubmissionError::Crashed)));
        assert!(results.iter().all(|(&index, (_, error))| index == 13 || error.is_none()));
    }
}
