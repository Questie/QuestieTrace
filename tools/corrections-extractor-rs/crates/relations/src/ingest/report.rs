//! reports/ingest.json: what ingest read, kept and could not use, for people checking a run.

use super::characters::{self, CharacterParams};
use super::distill::{self, DistillParams, DistillStats};
use super::submission::SubmissionError;
use super::titles::GreetingStats;
use crate::model::{CompletedSource, GiverKind, OfferSource, QuestEpisode, QuestEventKind};
use indexmap::IndexMap;
use rustc_hash::FxHashSet;
use serde::Serialize;

/// At most this many failures and crashes are listed (all are counted).
const LISTED_CRASHES: usize = 5;
const LISTED_FAILURES: usize = 20;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub generated_at: String,
    pub seconds: u64,
    pub params: Params,
    pub submissions: SubmissionCounts,
    pub sessions: SessionCounts,
    pub episodes: EpisodeCounts,
    pub offers: OfferCounts,
    pub greetings: GreetingStats,
    pub quest_events: QuestEventCounts,
    pub distill_counts: DistillStats,
}

/// The tunables episodes were made with. (The TS report also has `cacheVersion`, the key of its
/// per-submission cache, which this port does not have; whether only Forever sessions were read
/// was the suffix of that key and is its own field here.)
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Params {
    pub require_forever_version: bool,
    pub distill: &'static DistillParams,
    pub characters: &'static CharacterParams,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub submission_id: String,
    pub error: String,
}

impl Failure {
    fn new((submission_id, error): &(String, SubmissionError)) -> Self {
        let error = match error {
            SubmissionError::Unreadable(reason) => reason.clone(),
            SubmissionError::Crashed => "panicked while reading (a bug; see the run's log)".into(),
        };
        Failure {
            submission_id: submission_id.clone(),
            error,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionCounts {
    pub total: usize,
    /// Every submission is decoded every run.
    pub decoded_this_run: usize,
    pub failed: usize,
    pub failures: Vec<Failure>,
    /// "<submission id>: panicked" for the first few submissions whose reading panicked (a bug;
    /// the panics are in the run's log). They are failures too.
    pub worker_crashes: Vec<String>,
    pub exports: usize,
    pub with_multiple_exports: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCounts {
    pub copies: usize,
    pub unique: usize,
    pub duplicate_copies_merged: usize,
    pub seen_under_several_contributors: usize,
    pub kept: usize,
    pub skipped: Skipped,
    /// The characters `skipped.other_server` counts the sessions of.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_servers: Vec<OtherServerCharacter>,
}

impl SessionCounts {
    /// Says what was left out because of where it was recorded. Beta sessions are expected;
    /// anything else here means somebody should look at the recordings.
    pub fn log_left_out(&self) {
        if let Some(beta) = self.skipped.beta {
            eprintln!("ingest: left out {beta} beta sessions");
        }
        if let Some(undated) = self.skipped.undated {
            eprintln!("ingest: WARNING: left out {undated} sessions without a server time, so the beta cannot be told from the live game");
        }
        for OtherServerCharacter {
            character_key,
            server_id,
            sessions,
        } in &self.other_servers
        {
            eprintln!(
                "ingest: WARNING: left out character {character_key} ({sessions} sessions): recorded on server {server_id}, which is not one of Blizzard's"
            );
        }
    }
}

/// A character recorded on a server that is not Blizzard's; none of its sessions became episodes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherServerCharacter {
    pub character_key: String,
    pub server_id: u32,
    pub sessions: usize,
}

/// Unique sessions left out of the episodes; a reason only appears once it was counted.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_events: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_quest_data: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beta: Option<usize>,
    /// Without a server time, so from the beta or the live game.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undated: Option<usize>,
    /// Every session of the characters met on a server that is not Blizzard's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_server: Option<usize>,
}

#[derive(Default, Serialize)]
pub struct Builds {
    pub forever: usize,
    pub other: usize,
    pub unknown: usize,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedCounts {
    pub modern: usize,
    pub legacy: usize,
    pub none: usize,
    /// A large add right after capture start means the history had not loaded yet at t = 0.
    pub early_bulk_adds: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeCounts {
    pub builds: Builds,
    pub with_player_context: usize,
    pub completed: CompletedCounts,
    pub with_quest_info: usize,
    pub contributors: usize,
    pub characters: usize,
}

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCounts {
    pub total: usize,
    pub list_complete: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferCounts {
    pub by_source: IndexMap<OfferSource, ListCounts>,
    pub total: usize,
    pub list_complete: usize,
    pub list_complete_ratio: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestEventCounts {
    pub by_kind: IndexMap<QuestEventKind, usize>,
    pub with_giver: usize,
    pub giver_kinds: IndexMap<GiverKind, usize>,
}

/// What the merge and the episode build counted, for [`IngestReport::new`].
pub struct Tally {
    pub submissions: usize,
    /// (submission id, why), in path order.
    pub failures: Vec<(String, SubmissionError)>,
    pub exports: usize,
    pub with_multiple_exports: usize,
    pub copies: usize,
    pub unique: usize,
    pub seen_under_several_contributors: usize,
    pub skipped: Skipped,
    pub other_servers: Vec<OtherServerCharacter>,
    pub greetings: GreetingStats,
    /// Summed over the kept sessions.
    pub distill: DistillStats,
}

/// Interface versions of Forever builds start with 16.
fn is_forever_build(version: i64) -> bool {
    version.to_string().starts_with("16")
}

/// Rounded to 3 decimals.
fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// What reports/ingest.json counts about the episodes, added up one episode at a time in ingest's
/// order (the maps keep first-met order).
#[derive(Default)]
pub struct EpisodeSummary {
    episodes: usize,
    builds: Builds,
    completed: CompletedCounts,
    by_source: IndexMap<OfferSource, ListCounts>,
    by_kind: IndexMap<QuestEventKind, usize>,
    giver_kinds: IndexMap<GiverKind, usize>,
    with_giver: usize,
    with_quest_info: usize,
    with_player_context: usize,
    contributors: FxHashSet<String>,
    characters: FxHashSet<String>,
}

impl EpisodeSummary {
    pub fn add(&mut self, episode: &QuestEpisode) {
        self.episodes += 1;
        match episode.interface_version {
            None => self.builds.unknown += 1,
            Some(version) if is_forever_build(version) => self.builds.forever += 1,
            Some(_) => self.builds.other += 1,
        }
        match episode.completed.as_ref().map(|completed| completed.source) {
            Some(CompletedSource::Modern) => self.completed.modern += 1,
            Some(CompletedSource::Legacy) => self.completed.legacy += 1,
            None => self.completed.none += 1,
        }
        let changes = episode.completed.iter().flat_map(|completed| &completed.changes);
        if changes
            .into_iter()
            .any(|change| change.t < 30.0 && change.add.as_ref().is_some_and(|add| add.len() > 20))
        {
            self.completed.early_bulk_adds += 1;
        }
        for offer in &episode.offers {
            let counts = self.by_source.entry(offer.source).or_default();
            counts.total += 1;
            counts.list_complete += usize::from(offer.list_complete);
        }
        for event in &episode.quest_events {
            *self.by_kind.entry(event.kind).or_default() += 1;
            if let Some(giver) = event.giver {
                self.with_giver += 1;
                *self.giver_kinds.entry(giver.kind).or_default() += 1;
            }
        }
        self.with_quest_info += usize::from(episode.quest_info.is_some());
        self.with_player_context += usize::from(episode.player.race_id.is_some() && episode.player.class_id.is_some());
        if !self.contributors.contains(&episode.contributor_id) {
            self.contributors.insert(episode.contributor_id.clone());
        }
        if !self.characters.contains(&episode.character_key) {
            self.characters.insert(episode.character_key.clone());
        }
    }
}

impl IngestReport {
    pub fn new(generated_at: String, seconds: u64, require_forever_version: bool, tally: Tally, summary: EpisodeSummary) -> Self {
        let EpisodeSummary {
            episodes,
            builds,
            completed,
            by_source,
            by_kind,
            giver_kinds,
            with_giver,
            with_quest_info,
            with_player_context,
            contributors,
            characters,
        } = summary;
        let total_offers = by_source.values().map(|counts| counts.total).sum::<usize>();
        let complete_offers = by_source.values().map(|counts| counts.list_complete).sum::<usize>();

        IngestReport {
            generated_at,
            seconds,
            params: Params {
                require_forever_version,
                distill: &distill::PARAMS,
                characters: &characters::PARAMS,
            },
            submissions: SubmissionCounts {
                total: tally.submissions,
                decoded_this_run: tally.submissions,
                failed: tally.failures.len(),
                failures: tally.failures.iter().take(LISTED_FAILURES).map(Failure::new).collect(),
                worker_crashes: tally
                    .failures
                    .iter()
                    .filter(|(_, error)| *error == SubmissionError::Crashed)
                    .take(LISTED_CRASHES)
                    .map(|(submission_id, _)| format!("{submission_id}: panicked"))
                    .collect(),
                exports: tally.exports,
                with_multiple_exports: tally.with_multiple_exports,
            },
            sessions: SessionCounts {
                copies: tally.copies,
                unique: tally.unique,
                duplicate_copies_merged: tally.copies - tally.unique,
                seen_under_several_contributors: tally.seen_under_several_contributors,
                kept: episodes,
                skipped: tally.skipped,
                other_servers: tally.other_servers,
            },
            episodes: EpisodeCounts {
                builds,
                with_player_context,
                completed,
                with_quest_info,
                contributors: contributors.len(),
                characters: characters.len(),
            },
            offers: OfferCounts {
                by_source,
                total: total_offers,
                list_complete: complete_offers,
                list_complete_ratio: round(complete_offers as f64 / total_offers.max(1) as f64),
            },
            greetings: tally.greetings,
            quest_events: QuestEventCounts {
                by_kind,
                with_giver,
                giver_kinds,
            },
            distill_counts: tally.distill,
        }
    }
}
