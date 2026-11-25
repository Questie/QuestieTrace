//! Ingest: trace-data submissions -> one [`QuestEpisode`] per recording session, its copies across
//! submissions merged, its character worked out.
//!
//! 1. Every submission is decoded and its sessions distilled on all cores (`submission`,
//!    `distill`); only the small distilled values leave a worker, so decoded trees never pile up.
//!    On one thread, as results arrive, every title pairing is indexed (`titles`) and every
//!    session copy is written to a spill file, partitioned by session key (`spill`).
//! 2. Partition by partition, the copies of each session are merged (`merge`), legacy greeting
//!    titles resolved against the index of every submission, and sessions without quest data or
//!    from the beta (`origin`) dropped. The sessions kept are spilled again, partitioned by
//!    contributor.
//! 3. Partition by partition, in contributor order, sessions are grouped into characters and
//!    ordered (`characters`), characters met on a server that is not Blizzard's (`origin`) are
//!    dropped, and the episodes of the others streamed to episodes.jsonl and the episode store,
//!    then reports/ingest.json is written (`report`).
//!
//! Ingest holds one partition at a time, never every session: memory stays flat as the archive
//! grows. Nothing is cached between runs: every run decodes every submission.

mod characters;
mod distill;
mod merge;
mod origin;
mod report;
mod spill;
mod streams;
mod submission;
mod titles;

#[cfg(test)]
mod test_trace;

use crate::config::Config;
use crate::episode_store::StoreWriter;
use crate::io;
use crate::model::QuestEpisode;
use anyhow::{bail, Context, Result};
use characters::{assign_characters, order_sessions, CharacterInput};
use distill::{DistillStats, EpisodeIdentity};
use merge::{MergedSession, SessionMerger, SubmissionInfo};
use origin::Era;
use rayon::prelude::*;
use report::{EpisodeSummary, IngestReport, OtherServerCharacter, Skipped, Tally};
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use spill::{contributor_partition, copy_partition, KeptSession, Partitions, SpilledCopy, Written};
use std::collections::BTreeMap;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::mpsc;
use std::time::Instant;
use submission::{distill_submission, DistilledSubmission, SubmissionError};
use titles::{resolve_greeting, GreetingStats, TitleIndex};
use trace_data::{Arena, Inflater};
use trace_pack::source::{self, StoredSubmission};

/// What ingest reports. The episodes themselves are already written by [`run`]: as JSON lines
/// for people and the TS tools, and as the episode store the steps read.
pub struct IngestOutput {
    /// reports/ingest.json
    pub report: serde_json::Value,
}

impl IngestOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
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
    let mut files = source::list(trace_data_dir).with_context(|| format!("listing {}/submissions", trace_data_dir.display()))?;
    if files.is_empty() {
        bail!("{}/submissions has no submissions", trace_data_dir.display());
    }
    // Oldest first, the order the TS pipeline merged in: ties between copies go to the earlier one.
    files.sort_by(|a, b| a.key.cmp(&b.key));
    eprintln!("ingest: decoding {} submissions", files.len());

    let spill_dir = config.relations_dir.join("ingest-spill");
    let mut read = ReadTally::default();
    let mut copies = Partitions::<SpilledCopy>::create(&spill_dir.join("copies"))?;
    let mut spill_error = None;
    let require_forever = config.require_forever_version;
    let distill =
        |file: &StoredSubmission, inflater: &mut Inflater, arena: &mut Arena| distill_submission(file.read(inflater), require_forever, inflater, arena);
    distill_all(&files, distill, |index, submission| {
        // After a failed write the rest is only drained: the run fails below.
        if spill_error.is_none() {
            if let Err(error) = read.add(index, submission, &mut copies) {
                spill_error = Some(error);
            }
        }
        if read.submissions % 1000 == 0 {
            eprintln!("ingest: {}/{} decoded ({:.0}s)", read.submissions, files.len(), started.elapsed().as_secs_f64());
        }
    });
    if let Some(error) = spill_error {
        return Err(error);
    }
    let copies = copies.finish()?;
    eprintln!("ingest: decoded in {:.0}s, merging copies", started.elapsed().as_secs_f64());

    let (kept, mut merged) = merge_copies(&copies, &read.titles, &spill_dir.join("kept"))?;
    copies.remove()?;
    eprintln!("ingest: merged in {:.0}s, building episodes", started.elapsed().as_secs_f64());

    let (summary, other_servers) = write_episodes(config, &kept)?;
    merged.skipped.other_server = Some(other_servers.iter().map(|character| character.sessions).sum()).filter(|&sessions| sessions > 0);
    kept.remove()?;
    std::fs::remove_dir(&spill_dir).with_context(|| format!("removing {}", spill_dir.display()))?;

    let ReadTally {
        submissions,
        mut failures,
        exports,
        with_multiple_exports,
        titles: _,
    } = read;
    failures.sort_by_key(|(index, _, _)| *index);
    let tally = Tally {
        submissions,
        failures: failures.into_iter().map(|(_, submission_id, error)| (submission_id, error)).collect(),
        exports,
        with_multiple_exports,
        copies: merged.copies,
        unique: merged.unique,
        seen_under_several_contributors: merged.seen_under_several_contributors,
        skipped: merged.skipped,
        other_servers,
        greetings: merged.greetings,
        distill: merged.distill,
    };
    let seconds = started.elapsed().as_secs_f64().round() as u64;
    let report = IngestReport::new(config.generated_at.clone(), seconds, config.require_forever_version, tally, summary);
    eprintln!(
        "ingest: {} episodes of {} characters, {} submissions failed",
        report.sessions.kept, report.episodes.characters, report.submissions.failed
    );
    report.sessions.log_left_out();
    Ok(IngestOutput {
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
    files: &[StoredSubmission],
    distill: impl Fn(&StoredSubmission, &mut Inflater, &mut Arena) -> DistilledSubmission + Sync,
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
                        DistilledSubmission::crashed(Path::new(&file.key))
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

/// What the later passes need from every submission read; the sessions themselves go to the
/// spill files.
#[derive(Default)]
struct ReadTally {
    submissions: usize,
    /// (position in path order, submission id, why).
    failures: Vec<(usize, String, SubmissionError)>,
    exports: usize,
    with_multiple_exports: usize,
    titles: TitleIndex,
}

impl ReadTally {
    fn add(&mut self, index: usize, submission: DistilledSubmission, copies: &mut Partitions<SpilledCopy>) -> Result<()> {
        self.submissions += 1;
        if let Some(error) = submission.error {
            self.failures.push((index, submission.submission_id.clone(), error));
        }
        self.exports += submission.exports;
        self.with_multiple_exports += usize::from(submission.exports > 1);
        for (position, mut session) in submission.sessions.into_iter().enumerate() {
            // Only sessions that can become episodes say which quest a title names, as far as this
            // copy of the session shows it: a copy that met no NPC does not show its server.
            if origin::era(session.start_server_time) == Era::Live && session.server_id.is_none_or(origin::is_blizzard) {
                let locale = session.episode.locale.as_deref();
                for observation in &session.titles {
                    self.titles.add(locale, session.episode.player.faction, observation);
                    self.titles.note_sighting(locale, &observation.title, &submission.contributor_id);
                }
                for pending in &session.pending_greetings {
                    for title in pending.available.iter().chain(&pending.active) {
                        self.titles.note_sighting(locale, title, &submission.contributor_id);
                    }
                }
            }
            // Indexed above; nothing later reads them.
            session.titles = Vec::new();
            let copy = SpilledCopy {
                submission_id: submission.submission_id.clone(),
                contributor_id: submission.contributor_id.clone(),
                received_at: submission.received_at.clone(),
                index,
                position,
                session,
            };
            copies.push(copy_partition(&copy.session.key), &copy)?;
        }
        Ok(())
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
fn session_order_times(sessions: &[KeptSession]) -> Vec<f64> {
    let mut offsets: FxHashMap<&str, Vec<f64>> = FxHashMap::default();
    for kept in sessions {
        let (start, episode) = (kept.start_server_time, &kept.episode);
        if let (Some(start), Some(saved)) = (start, session_name_time(episode.session_name.as_deref())) {
            offsets.entry(&kept.contributor_id).or_default().push(saved - (start + episode.duration));
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
        .map(|kept| {
            let episode = &kept.episode;
            let order = kept
                .start_server_time
                .unwrap_or_else(|| match session_name_time(episode.session_name.as_deref()) {
                    Some(saved) => saved - medians.get(kept.contributor_id.as_str()).copied().unwrap_or(0.0) - episode.duration,
                    None => parse_received_at(&kept.first_received_at).unwrap_or(f64::NAN) - episode.duration,
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
/// What merging the copies counted, for the report.
#[derive(Default)]
struct MergeCounts {
    copies: usize,
    unique: usize,
    seen_under_several_contributors: usize,
    skipped: Skipped,
    greetings: GreetingStats,
    distill: DistillStats,
}

/// Pass 2: merges the copies of each session, one copy partition at a time, resolves legacy
/// greeting titles against `titles` (built from every submission) and spills the sessions kept,
/// partitioned by contributor.
fn merge_copies(copies: &Written<SpilledCopy>, titles: &TitleIndex, dir: &Path) -> Result<(Written<KeptSession>, MergeCounts)> {
    let mut kept = Partitions::<KeptSession>::create(dir)?;
    let mut counts = MergeCounts::default();
    for &partition in &copies.partitions {
        let mut merger = SessionMerger::default();
        for copy in copies.read(partition)? {
            let info = SubmissionInfo {
                submission_id: &copy.submission_id,
                contributor_id: &copy.contributor_id,
                received_at: &copy.received_at,
                index: copy.index,
            };
            merger.add(&info, (copy.index, copy.position), copy.session);
        }
        counts.copies += merger.copies;
        counts.unique += merger.merged.len();
        let mut merged: Vec<MergedSession> = merger.merged.into_values().collect();
        merged.sort_by(|a, b| a.session.key.cmp(&b.session.key));
        for mut merged in merged {
            counts.seen_under_several_contributors += usize::from(merged.seen_under_several_contributors);
            if merged.session.event_count == 0 {
                *counts.skipped.no_events.get_or_insert(0) += 1;
                continue;
            }
            if !has_quest_data(&merged) {
                *counts.skipped.no_quest_data.get_or_insert(0) += 1;
                continue;
            }
            match origin::era(merged.session.start_server_time) {
                Era::Live => {}
                Era::Beta => {
                    *counts.skipped.beta.get_or_insert(0) += 1;
                    continue;
                }
                Era::Undated => {
                    *counts.skipped.undated.get_or_insert(0) += 1;
                    continue;
                }
            }
            let session = &mut merged.session;
            let context = (session.episode.locale.as_deref(), session.episode.player.faction);
            for pending in &session.pending_greetings {
                resolve_greeting(&mut session.episode.offers[pending.offer], pending, context, titles, &mut counts.greetings);
            }
            counts.distill += session.stats;
            let session = KeptSession {
                key: merged.session.key,
                contributor_id: merged.contributor_id,
                first_received_at: merged.first_received_at,
                submission_ids: merged.submission_ids.into_iter().collect(),
                start_server_time: merged.session.start_server_time,
                server_id: merged.session.server_id,
                episode: merged.session.episode,
            };
            kept.push(contributor_partition(&session.contributor_id), &session)?;
        }
    }
    Ok((kept.finish()?, counts))
}

/// Pass 3: turns each contributor partition into episodes, in contributor order, and streams them
/// to episodes.jsonl and the episode store. Also the characters left out.
fn write_episodes(config: &Config, kept: &Written<KeptSession>) -> Result<(EpisodeSummary, Vec<OtherServerCharacter>)> {
    let mut store = StoreWriter::create(&config.episode_store(), config.bucket_bytes()?)?;
    let mut summary = EpisodeSummary::default();
    let mut other_servers = Vec::new();
    io::write_atomic(&config.episodes(), |out| {
        for &partition in &kept.partitions {
            let (episodes, left_out) = contributor_episodes(kept.read(partition)?);
            other_servers.extend(left_out);
            for episode in episodes {
                summary.add(&episode);
                serde_json::to_writer(&mut *out, &episode)?;
                out.write_all(b"\n")?;
                store.push(&episode)?;
            }
        }
        Ok(())
    })?;
    store.finish()?;
    Ok((summary, other_servers))
}

/// The episodes of every session of a set of whole contributors: grouped into characters,
/// ordered, and sorted by contributor, character and session order. Of a character met on a server
/// that is not Blizzard's, only sessions met on one of Blizzard's stay (grouping can put two
/// characters together): those without an NPC do not show their server.
fn contributor_episodes(mut sessions: Vec<KeptSession>) -> (Vec<QuestEpisode>, Vec<OtherServerCharacter>) {
    sessions.sort_by(|a, b| a.key.cmp(&b.key));
    let order = session_order_times(&sessions);
    let inputs: Vec<CharacterInput> = sessions
        .iter()
        .zip(&order)
        .map(|(session, &order)| {
            let episode = &session.episode;
            CharacterInput {
                episode_key: &session.key,
                contributor_id: &session.contributor_id,
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
    let met_elsewhere = characters_met_elsewhere(&sessions, &characters);
    let left_out =
        |session: &KeptSession, character_key: &String| met_elsewhere.contains_key(character_key) && !session.server_id.is_some_and(origin::is_blizzard);
    let left_out_characters: Vec<OtherServerCharacter> = met_elsewhere
        .iter()
        .map(|(character_key, &server_id)| OtherServerCharacter {
            character_key: character_key.clone(),
            server_id,
            sessions: sessions
                .iter()
                .zip(&characters)
                .filter(|(session, key)| *key == character_key && left_out(session, key))
                .count(),
        })
        .collect();

    let mut episodes: Vec<QuestEpisode> = sessions
        .into_iter()
        .zip(characters)
        .zip(session_orders)
        .filter(|((session, character_key), _)| !left_out(session, character_key))
        .map(|((session, character_key), session_order)| {
            let identity = EpisodeIdentity {
                key: session.key,
                contributor_id: session.contributor_id,
                character_key,
                submission_ids: session.submission_ids,
                session_order,
            };
            session.episode.into_episode(identity)
        })
        .collect();
    // Contributor ids and keys are lowercase hex, so comparing them as text orders them like the
    // TS localeCompare.
    episodes
        .sort_by(|a, b| (&a.contributor_id, &a.character_key, a.session_order, &a.key).cmp(&(&b.contributor_id, &b.character_key, b.session_order, &b.key)));
    (episodes, left_out_characters)
}

/// The characters met on a server that is not Blizzard's, with the server's id. `characters` holds
/// each session's character key.
fn characters_met_elsewhere(sessions: &[KeptSession], characters: &[String]) -> BTreeMap<String, u32> {
    let mut met: BTreeMap<String, u32> = BTreeMap::new();
    for (session, character_key) in sessions.iter().zip(characters) {
        if let Some(server_id) = session.server_id.filter(|&id| !origin::is_blizzard(id)) {
            met.entry(character_key.clone()).or_insert(server_id);
        }
    }
    met
}

#[cfg(test)]
mod tests {
    use super::test_trace::{gossip_quest, TraceBuilder};
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

    /// A minute before and after the live realms opened (see `origin`).
    const IN_BETA: f64 = origin::LIVE_SINCE - 60.0;
    const LIVE: f64 = origin::LIVE_SINCE + 86_400.0;

    /// A session that saw one offer, as distilled.
    fn session(key: &str, start_server_time: Option<f64>) -> distill::DistilledSession {
        distill::DistilledSession {
            key: key.into(),
            start_server_time,
            ..TraceBuilder::new().gossip(951, &[gossip_quest(91743)], &[]).build()
        }
    }

    #[test]
    fn sessions_from_the_beta_and_undated_ones_are_left_out_and_counted() {
        let dir = std::env::temp_dir().join(format!("ingest-origin-test-{}", std::process::id()));
        let mut copies = Partitions::<SpilledCopy>::create(&dir.join("copies")).unwrap();
        let sessions = [session("live", Some(LIVE)), session("beta", Some(IN_BETA)), session("undated", None)];
        for (position, session) in sessions.into_iter().enumerate() {
            let copy = SpilledCopy {
                submission_id: "s1".into(),
                contributor_id: "c1".into(),
                received_at: "2026-11-06T00:00:00Z".into(),
                index: 0,
                position,
                session,
            };
            copies.push(copy_partition(&copy.session.key), &copy).unwrap();
        }
        let (kept, counts) = merge_copies(&copies.finish().unwrap(), &TitleIndex::default(), &dir.join("kept")).unwrap();

        let keys: Vec<String> = kept
            .partitions
            .iter()
            .flat_map(|&partition| kept.read(partition).unwrap())
            .map(|session| session.key)
            .collect();
        assert_eq!(keys, ["live"]);
        assert_eq!((counts.unique, counts.skipped.beta, counts.skipped.undated), (3, Some(1), Some(1)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A session of contributor "c1" to become an episode.
    fn kept(key: &str, hours: f64, race_id: i64, server_id: Option<u32>) -> KeptSession {
        let mut session = session(key, Some(LIVE + hours * 3600.0));
        session.episode.player.race_id = Some(race_id);
        KeptSession {
            key: session.key,
            contributor_id: "c1".into(),
            first_received_at: "2026-11-06T00:00:00Z".into(),
            submission_ids: vec!["s1".into()],
            start_server_time: session.start_server_time,
            server_id,
            episode: session.episode,
        }
    }

    #[test]
    fn a_character_met_on_another_server_loses_its_sessions_and_no_other_character_does() {
        // The contributor's orc was met on server 70; a later session of its saw no NPC. The
        // troll's sessions are on a live server or show none.
        let (episodes, left_out) = contributor_episodes(vec![
            kept("orc-1", 0.0, 2, Some(70)),
            kept("troll-1", 1.0, 8, Some(6823)),
            kept("orc-2", 2.0, 2, None),
            kept("troll-2", 3.0, 8, None),
        ]);

        assert_eq!(episodes.iter().map(|episode| episode.key.as_str()).collect::<Vec<_>>(), ["troll-1", "troll-2"]);
        assert_eq!(left_out.len(), 1);
        assert_eq!((left_out[0].server_id, left_out[0].sessions), (70, 2));
        assert_ne!(left_out[0].character_key, episodes[0].character_key);
    }

    #[test]
    fn a_session_on_a_live_server_stays_when_grouping_put_it_with_one_from_another() {
        // Two orcs of one contributor, one of them on server 70, taken for one character.
        let (episodes, left_out) = contributor_episodes(vec![
            kept("other-1", 0.0, 2, Some(70)),
            kept("unknown", 1.0, 2, None),
            kept("live-1", 2.0, 2, Some(6823)),
            kept("other-2", 3.0, 2, Some(70)),
            kept("live-2", 4.0, 2, Some(6901)),
        ]);

        assert_eq!(episodes.iter().map(|episode| episode.key.as_str()).collect::<Vec<_>>(), ["live-1", "live-2"]);
        assert_eq!((left_out.len(), left_out[0].sessions), (1, 3), "what is left out and what stays adds up");
    }

    #[test]
    fn titles_of_beta_and_other_server_sessions_name_no_quest() {
        let with_title = |start, server_id| distill::DistilledSession {
            server_id,
            titles: vec![distill::TitleObservation {
                title: "Rascally Rodents".into(),
                quest_id: 91743,
                giver: None,
            }],
            ..session("titles", start)
        };
        let resolved = |session: distill::DistilledSession| {
            let dir = std::env::temp_dir().join(format!("ingest-titles-test-{}", std::process::id()));
            let mut copies = Partitions::<SpilledCopy>::create(&dir).unwrap();
            let mut read = ReadTally::default();
            let submission = DistilledSubmission {
                sessions: vec![session],
                error: None,
                ..DistilledSubmission::crashed(Path::new("s1.json"))
            };
            read.add(0, submission, &mut copies).unwrap();
            copies.finish().unwrap().remove().unwrap();
            // The builder's player is a Horde orc on an enUS client.
            read.titles.resolve(Some("enUS"), Some(crate::model::Faction::Horde), None, "Rascally Rodents")
        };
        assert_eq!(resolved(with_title(Some(LIVE), Some(6823))), Some(91743));
        assert_eq!(resolved(with_title(Some(LIVE), None)), Some(91743), "a session that met no NPC");
        assert_eq!(resolved(with_title(Some(IN_BETA), Some(6823))), None);
        assert_eq!(resolved(with_title(Some(LIVE), Some(70))), None);
    }

    /// Packs must not be a way around what ingest leaves out. A submission read from a pack
    /// distills to exactly what its file distills to, down to the server time that marks a
    /// session as recorded in the beta, and everything after the read is the same code.
    #[test]
    fn a_submission_read_from_a_pack_distills_as_its_file_does_beta_sessions_included() {
        use trace_pack::pack::{build_frame, PackOptions, PackWriter, Scratch};
        use trace_pack::record::Encoding;
        use trace_pack::values::TpPolicy;

        let recorded_at = |server_time: f64| {
            let builder = TraceBuilder::new().sample("GetServerTime", serde_json::json!(server_time));
            builder.gossip(951, &[gossip_quest(91743)], &[]).session
        };
        let export = test_trace::export_string(&[recorded_at(IN_BETA), recorded_at(LIVE)]);
        let name = "2026-10/2026-10-20_120000Z_s1.json";
        let file = serde_json::json!({ "id": "s1", "contributor_id": "c1", "received_at": "2026-10-20T12:00:00Z", "export_string": export });
        let file = file.to_string().into_bytes();

        // The same submission as a file of a checkout and in a pack, value-encoded.
        let dir = std::env::temp_dir().join(format!("ingest-pack-source-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (checkout, packed) = (dir.join("checkout"), dir.join("packed"));
        std::fs::create_dir_all(checkout.join("submissions/2026-10")).unwrap();
        std::fs::write(checkout.join("submissions").join(name), &file).unwrap();
        let options = PackOptions {
            tp: TpPolicy::SchemaVersions(vec![12]),
            ..PackOptions::default()
        };
        let frame = build_frame(name, &file, &options, &mut Scratch::new()).unwrap();
        assert_eq!(frame.entry.exports.iter().map(|(encoding, _)| *encoding).collect::<Vec<_>>(), [Encoding::Value]);
        let mut writer = PackWriter::new();
        writer.add(frame);
        std::fs::create_dir_all(packed.join("packs/live")).unwrap();
        std::fs::write(packed.join("packs/live/a.pack"), writer.finish(options.level).unwrap()).unwrap();

        let distilled = |directory: &Path| {
            let listed = source::list(directory).unwrap();
            assert_eq!(listed.iter().map(|stored| stored.key.as_str()).collect::<Vec<_>>(), [name]);
            let (mut inflater, mut arena) = (Inflater::new(), Arena::new());
            distill_submission(listed[0].read(&mut inflater), false, &mut inflater, &mut arena)
        };
        let (from_file, from_pack) = (distilled(&checkout), distilled(&packed));
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(format!("{:?}", from_pack.sessions), format!("{:?}", from_file.sessions));
        assert_eq!(
            (&from_pack.submission_id, &from_pack.contributor_id, from_pack.exports),
            (&from_file.submission_id, &from_file.contributor_id, 1)
        );
        let eras: Vec<Era> = from_pack.sessions.iter().map(|session| origin::era(session.start_server_time)).collect();
        assert_eq!(eras, [Era::Beta, Era::Live]);
    }

    #[test]
    fn a_submission_whose_reading_panics_fails_alone_and_the_run_goes_on() {
        let files: Vec<StoredSubmission> = (0..40)
            .map(|i| {
                let name = format!("2026-10-01_000000Z_{i:02}.json");
                StoredSubmission::file(format!("2026-10/{name}"), std::path::Path::new("/nowhere").join(name))
            })
            .collect();
        let distill = |file: &StoredSubmission, _: &mut Inflater, _: &mut Arena| {
            assert!(!file.key.ends_with("_13.json"), "a bug met reading submission 13");
            DistilledSubmission {
                exports: 1,
                error: None,
                ..DistilledSubmission::crashed(Path::new(&file.key))
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
