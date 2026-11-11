//! reports/ingest.json: what ingest read, kept and could not use, for people checking a run.

use super::characters::{self, CharacterParams};
use super::distill::{self, DistillParams, DistillStats};
use super::titles::GreetingStats;
use crate::model::{CompletedSource, GiverKind, OfferSource, QuestEpisode, QuestEventKind};
use indexmap::IndexMap;
use rustc_hash::FxHashSet;
use serde::Serialize;

/// At most this many failures are listed (all are counted).
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionCounts {
    pub total: usize,
    /// Every submission is decoded every run.
    pub decoded_this_run: usize,
    pub failed: usize,
    pub failures: Vec<Failure>,
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
}

/// Unique sessions left out of the episodes; a reason only appears once it was counted.
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_events: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_quest_data: Option<usize>,
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
    /// In path order.
    pub failures: Vec<Failure>,
    pub exports: usize,
    pub with_multiple_exports: usize,
    pub copies: usize,
    pub unique: usize,
    pub seen_under_several_contributors: usize,
    pub skipped: Skipped,
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

impl IngestReport {
    pub fn new(generated_at: String, seconds: u64, require_forever_version: bool, tally: Tally, episodes: &[QuestEpisode]) -> Self {
        let mut builds = Builds::default();
        let mut completed = CompletedCounts::default();
        let mut by_source: IndexMap<OfferSource, ListCounts> = IndexMap::new();
        let mut by_kind: IndexMap<QuestEventKind, usize> = IndexMap::new();
        let mut giver_kinds: IndexMap<GiverKind, usize> = IndexMap::new();
        let (mut with_giver, mut with_quest_info, mut with_player_context) = (0, 0, 0);
        for episode in episodes {
            match episode.interface_version {
                None => builds.unknown += 1,
                Some(version) if is_forever_build(version) => builds.forever += 1,
                Some(_) => builds.other += 1,
            }
            match episode.completed.as_ref().map(|completed| completed.source) {
                Some(CompletedSource::Modern) => completed.modern += 1,
                Some(CompletedSource::Legacy) => completed.legacy += 1,
                None => completed.none += 1,
            }
            let changes = episode.completed.iter().flat_map(|completed| &completed.changes);
            if changes
                .into_iter()
                .any(|change| change.t < 30.0 && change.add.as_ref().is_some_and(|add| add.len() > 20))
            {
                completed.early_bulk_adds += 1;
            }
            for offer in &episode.offers {
                let counts = by_source.entry(offer.source).or_default();
                counts.total += 1;
                counts.list_complete += usize::from(offer.list_complete);
            }
            for event in &episode.quest_events {
                *by_kind.entry(event.kind).or_default() += 1;
                if let Some(giver) = event.giver {
                    with_giver += 1;
                    *giver_kinds.entry(giver.kind).or_default() += 1;
                }
            }
            with_quest_info += usize::from(episode.quest_info.is_some());
            with_player_context += usize::from(episode.player.race_id.is_some() && episode.player.class_id.is_some());
        }
        let total_offers = by_source.values().map(|counts| counts.total).sum::<usize>();
        let complete_offers = by_source.values().map(|counts| counts.list_complete).sum::<usize>();
        let distinct = |key: fn(&QuestEpisode) -> &str| episodes.iter().map(key).collect::<FxHashSet<_>>().len();

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
                failures: tally.failures.into_iter().take(LISTED_FAILURES).collect(),
                exports: tally.exports,
                with_multiple_exports: tally.with_multiple_exports,
            },
            sessions: SessionCounts {
                copies: tally.copies,
                unique: tally.unique,
                duplicate_copies_merged: tally.copies - tally.unique,
                seen_under_several_contributors: tally.seen_under_several_contributors,
                kept: episodes.len(),
                skipped: tally.skipped,
            },
            episodes: EpisodeCounts {
                builds,
                with_player_context,
                completed,
                with_quest_info,
                contributors: distinct(|episode| &episode.contributor_id),
                characters: distinct(|episode| &episode.character_key),
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
