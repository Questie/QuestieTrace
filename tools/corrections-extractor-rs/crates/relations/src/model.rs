//! The contracts between the pipeline's areas, as their JSON files store them:
//!
//! ```text
//! trace-data submissions --ingest--> episodes.jsonl (QuestEpisode per line)
//! QuestieDB              --catalog-> catalog.json (QuestCatalog) + groundtruth.json (GroundTruth)
//! episodes + catalog     --signals-> candidates/<signal>.json (CandidateFile)
//! all candidates         --combine-> foreverQuestRelationTraces.lua, relations-review.md, relations-meta.json
//! ```
//!
//! Field names match the TypeScript pipeline (Exporters `src/relations/core/types.ts`) so either
//! side can read the other's files. Maps keyed by quest id are `BTreeMap`s: ascending ids, the
//! order the files list them in.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type QuestId = i64;
/// Seconds since the start of the recording session. Never compare across episodes.
pub type SessionTime = f64;

// ---------------------------------------------------------------------------
// Relationship fields (the Questie questKeys we infer)
// ---------------------------------------------------------------------------

/// Questie quest relationship fields, as `QuestieDB.IsDoable` evaluates them for quest Q:
///
/// - `PreQuestSingle`: Q is available once ANY listed quest is completed.
/// - `PreQuestGroup`: Q is available once ALL listed quests are completed (an entry also counts
///   as done when one of its own exclusiveTo siblings is). Ignored when preQuestSingle is set.
/// - `ExclusiveTo`: Q is hidden while ANY listed quest is completed or in the log. One-directional
///   in Questie, so both sides must be emitted.
/// - `NextQuestInChain`: Q is hidden once that quest is completed or in the log.
/// - `ParentQuest`: Q is only available while that quest is in the log.
/// - `BreadcrumbForQuestId`: on breadcrumb Q, Q is hidden once its target is completed or in the log.
/// - `Breadcrumbs`: on target Q, Q is hidden while any listed breadcrumb is in the log.
/// - `AvailableUntilCompleted`: Q is hidden once that quest is completed.
/// - `AvailableStartingWith`: Q is only available once that quest is completed or in the log.
/// - `DisabledByQuest`: Q is hidden while that quest is in the log.
///
/// Declaration order is the order the TS pipeline lists them in (`RELATION_FIELDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelationField {
    PreQuestSingle,
    PreQuestGroup,
    ExclusiveTo,
    NextQuestInChain,
    ParentQuest,
    BreadcrumbForQuestId,
    Breadcrumbs,
    AvailableUntilCompleted,
    AvailableStartingWith,
    DisabledByQuest,
}

impl RelationField {
    pub const ALL: [RelationField; 10] = [
        RelationField::PreQuestSingle,
        RelationField::PreQuestGroup,
        RelationField::ExclusiveTo,
        RelationField::NextQuestInChain,
        RelationField::ParentQuest,
        RelationField::BreadcrumbForQuestId,
        RelationField::Breadcrumbs,
        RelationField::AvailableUntilCompleted,
        RelationField::AvailableStartingWith,
        RelationField::DisabledByQuest,
    ];

    /// Questie stores these as a single quest id rather than a list. Candidates still model them
    /// as one candidate per target.
    pub fn is_scalar(self) -> bool {
        matches!(
            self,
            RelationField::NextQuestInChain
                | RelationField::ParentQuest
                | RelationField::BreadcrumbForQuestId
                | RelationField::AvailableUntilCompleted
                | RelationField::AvailableStartingWith
                | RelationField::DisabledByQuest
        )
    }

    /// The questKey name, as in the JSON files and the Lua output.
    pub fn name(self) -> &'static str {
        match self {
            RelationField::PreQuestSingle => "preQuestSingle",
            RelationField::PreQuestGroup => "preQuestGroup",
            RelationField::ExclusiveTo => "exclusiveTo",
            RelationField::NextQuestInChain => "nextQuestInChain",
            RelationField::ParentQuest => "parentQuest",
            RelationField::BreadcrumbForQuestId => "breadcrumbForQuestId",
            RelationField::Breadcrumbs => "breadcrumbs",
            RelationField::AvailableUntilCompleted => "availableUntilCompleted",
            RelationField::AvailableStartingWith => "availableStartingWith",
            RelationField::DisabledByQuest => "disabledByQuest",
        }
    }
}

impl std::fmt::Display for RelationField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Relationship values for one quest. Scalar fields are normalized to one-element lists, and
/// negative preQuestGroup entries (Questie: "this quest, without the exclusiveTo substitute") to
/// their absolute value.
pub type RelationSet = BTreeMap<RelationField, Vec<QuestId>>;

// ---------------------------------------------------------------------------
// Episodes (produced by ingest, consumed by the signals)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimedValue<T> {
    pub t: SessionTime,
    pub v: T,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GiverKind {
    Npc,
    Object,
    Item,
}

impl GiverKind {
    /// As the JSON files and evidence texts name it.
    pub fn name(self) -> &'static str {
        match self {
            GiverKind::Npc => "npc",
            GiverKind::Object => "object",
            GiverKind::Item => "item",
        }
    }
}

/// The entity a quest dialog or offer list belongs to. Players are never givers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GiverRef {
    pub kind: GiverKind,
    pub id: i64,
}

impl GiverRef {
    /// "npc 1234", as evidence names a giver.
    pub fn label(self) -> String {
        format!("{} {}", self.kind.name(), self.id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Faction {
    Alliance,
    Horde,
    Neutral,
}

/// Character context. Pseudonymous only.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerContext {
    /// Blizzard ChrRaces id (UnitRace 3rd return); its Questie bit is in `crate::race_masks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_id: Option<i64>,
    /// Blizzard ChrClasses id (UnitClass 3rd return). Questie class bit = 2^(classId - 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faction: Option<Faction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletedSource {
    /// C_QuestLog.GetAllCompletedQuestIDs; preferred.
    Modern,
    Legacy,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompletedChange {
    pub t: SessionTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<Vec<QuestId>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<Vec<QuestId>>,
}

/// The character's completed-quest set over the session: `initial` is the full history when the
/// capture started, `changes` the deltas afterwards. Removals happen for daily/weekly resets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompletedTimeline {
    pub source: CompletedSource,
    pub initial: Vec<QuestId>,
    pub changes: Vec<CompletedChange>,
}

impl CompletedTimeline {
    /// See [`QuestEpisode::shrink_to_fit`].
    pub fn shrink_to_fit(&mut self) {
        self.initial.shrink_to_fit();
        self.changes.shrink_to_fit();
        for change in &mut self.changes {
            change.add.iter_mut().chain(&mut change.remove).for_each(Vec::shrink_to_fit);
        }
    }
}

/// - `Detail`: the accept dialog was shown (QUEST_DETAIL).
/// - `Accepted` / `TurnedIn`: QUEST_ACCEPTED / QUEST_TURNED_IN.
/// - `Progress` / `Complete`: the turn-in dialog pages (QUEST_PROGRESS / QUEST_COMPLETE).
/// - `Abandoned`: removed from the log without a turn-in and without becoming completed.
/// - `Removed`: removed from the log, for a reason ingest could not tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestEventKind {
    Detail,
    Accepted,
    Progress,
    Complete,
    TurnedIn,
    Abandoned,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestEvent {
    pub t: SessionTime,
    pub kind: QuestEventKind,
    pub quest_id: QuestId,
    /// Who the dialog belonged to, when ingest could resolve it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub giver: Option<GiverRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfferedQuest {
    pub id: QuestId,
    /// Gray/low-level quest. The client still lists these; only the color changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trivial: Option<bool>,
    /// 1 normal, 2 daily, 3 weekly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeatable: Option<bool>,
}

/// - `Gossip`: C_GossipInfo.GetAvailableQuests/GetActiveQuests at GOSSIP_SHOW.
/// - `Greeting`: the QUEST_GREETING frame (GetAvailableQuestInfo / GetAvailableTitle per index).
/// - `Detail`: a QUEST_DETAIL that was not reached by picking from a gossip/greeting list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OfferSource {
    Gossip,
    Greeting,
    Detail,
}

/// The quests a giver offered at one moment. Servers omit unavailable quests entirely (they are
/// not shown grayed out), so a complete list is evidence about what was NOT available.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferSnapshot {
    pub t: SessionTime,
    pub source: OfferSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub giver: Option<GiverRef>,
    pub available: Vec<OfferedQuest>,
    /// Greeting titles ingest could not map to a quest id (legacy traces record titles only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_titles: Option<Vec<String>>,
    /// Quests the giver showed as in progress / ready to turn in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<Vec<QuestId>>,
    /// Whether `available` is believed to be the giver's ENTIRE available list at `t`. Only
    /// complete lists may be used as evidence that a quest was not offered.
    pub list_complete: bool,
}

impl OfferSnapshot {
    /// See [`QuestEpisode::shrink_to_fit`].
    pub fn shrink_to_fit(&mut self) {
        self.available.shrink_to_fit();
        self.unresolved_titles.iter_mut().for_each(Vec::shrink_to_fit);
        self.active.iter_mut().for_each(Vec::shrink_to_fit);
    }
}

/// Client-side facts about a quest that newer traces record directly (Forever-only APIs).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestClientInfo {
    /// Raw IsBreadcrumbQuest(questID). Undocumented API: a hint, to verify against behaviour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_breadcrumb: Option<bool>,
    /// C_QuestLine.GetQuestLineInfo(questID).questLineID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest_line_id: Option<i64>,
    /// C_QuestInfoSystem.GetQuestClassification(questID) (6 = Questline).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<i64>,
}

/// One deduplicated recording session, reduced to what relationship inference needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestEpisode {
    /// Stable, unique, pseudonymous id of this session.
    pub key: String,
    /// Pseudonymous contributor id from the trace-data submission.
    pub contributor_id: String,
    /// Pseudonymous id grouping the sessions ingest believes belong to one character.
    pub character_key: String,
    /// Submissions that contained this session (duplicates are merged).
    pub submission_ids: Vec<String>,
    /// Session name as recorded (a local "YYYY-MM-DD_HH-MM-SS" stamp); absent for about a third.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// 0-based position among the character's sessions, oldest first (by progress, then capture
    /// start, then name). Use this, not the name, to order a character's sessions.
    pub session_order: u32,
    /// GetBuildInfo interface version; Forever builds start with 16. Absent in old traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    pub player: PlayerContext,
    pub duration: SessionTime,
    /// Player level change points; the first is the level at (or near) t = 0.
    pub levels: Vec<TimedValue<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<CompletedTimeline>,
    /// Active quest log change points (quest ids, no headers).
    pub quest_log: Vec<TimedValue<Vec<QuestId>>>,
    /// Quest dialog and log events in time order.
    pub quest_events: Vec<QuestEvent>,
    /// Giver offer lists in time order.
    pub offers: Vec<OfferSnapshot>,
    /// Per-quest client facts. Absent in traces older than the addon change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest_info: Option<BTreeMap<QuestId, QuestClientInfo>>,
}

impl QuestEpisode {
    /// Frees the room its lists grew into. Lists grow by doubling while being built or read (a
    /// serde read cannot know a list's length up front), so before this an episode holds about
    /// 40% more than its contents, and every step holds every episode.
    pub fn shrink_to_fit(&mut self) {
        self.submission_ids.shrink_to_fit();
        self.levels.shrink_to_fit();
        self.completed.iter_mut().for_each(CompletedTimeline::shrink_to_fit);
        self.quest_log.shrink_to_fit();
        for entry in &mut self.quest_log {
            entry.v.shrink_to_fit();
        }
        self.quest_events.shrink_to_fit();
        self.offers.shrink_to_fit();
        self.offers.iter_mut().for_each(OfferSnapshot::shrink_to_fit);
    }
}

// ---------------------------------------------------------------------------
// Catalog and ground truth (produced by catalog, consumed by signals and scoring)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Starters {
    pub npcs: Vec<i64>,
    pub objects: Vec<i64>,
    pub items: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Finishers {
    pub npcs: Vec<i64>,
    pub objects: Vec<i64>,
}

/// Non-relationship facts about a quest as Forever sees it (Classic base, then Forever legacy,
/// generated, traces and authored corrections). Signals gate evidence with these: a quest the
/// character could not take for level/race/class/skill reasons says nothing about relationships.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogQuest {
    pub id: QuestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest_level: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_level: Option<i64>,
    /// Questie race bitmask; absent or 0 means every race. Uses bits above 32.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_races: Option<u64>,
    /// Questie class bitmask; absent or 0 means every class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_classes: Option<u64>,
    /// (skillId, value)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_skill: Option<(i64, i64)>,
    /// (factionId, value)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_min_rep: Option<(i64, i64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_max_rep: Option<(i64, i64)>,
    /// Spell the character must know; negative means it must NOT know it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_spell: Option<i64>,
    /// Bitmask: 1 = repeatable, 2 = needs event, 4 = monthly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub special_flags: Option<i64>,
    pub starters: Starters,
    pub finishers: Finishers,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestCatalog {
    pub generated_at: String,
    /// Source files in layering order.
    pub sources: Vec<String>,
    pub quests: BTreeMap<QuestId, CatalogQuest>,
}

impl CatalogQuest {
    /// specialFlags bit 1. Repeatable quests never show as completed, so they cannot gate
    /// anything a trace can observe.
    pub fn is_repeatable(&self) -> bool {
        self.special_flags.is_some_and(|flags| flags & 1 != 0)
    }

    /// specialFlags bit 2: only available during a world event.
    pub fn needs_event(&self) -> bool {
        self.special_flags.is_some_and(|flags| flags & 2 != 0)
    }
}

impl QuestCatalog {
    pub fn quest(&self, id: QuestId) -> Option<&CatalogQuest> {
        self.quests.get(&id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    /// Hand-authored relations for Forever-new quests (QuestieDB foreverQuestFixes).
    AuthoredForever,
    /// Every quest Forever inherits from Era, with relations as Forever loads them ({} when none).
    InheritedClassic,
}

impl Tier {
    pub const ALL: [Tier; 2] = [Tier::AuthoredForever, Tier::InheritedClassic];
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundTruthTiers {
    pub authored_forever: BTreeMap<QuestId, RelationSet>,
    pub inherited_classic: BTreeMap<QuestId, RelationSet>,
}

impl GroundTruthTiers {
    pub fn get(&self, tier: Tier) -> &BTreeMap<QuestId, RelationSet> {
        match tier {
            Tier::AuthoredForever => &self.authored_forever,
            Tier::InheritedClassic => &self.inherited_classic,
        }
    }
}

/// Reference relationship data to score signals against. Signals must NOT read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundTruth {
    pub generated_at: String,
    pub sources: Vec<String>,
    pub tiers: GroundTruthTiers,
}

// ---------------------------------------------------------------------------
// Candidates (produced by signals and sources, consumed by combine and scoring)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// QuestEpisode.key, or a source reference such as "wowhead:quest/92579".
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<SessionTime>,
    /// One line a reviewer can check, e.g. "turned in 91741 at npc 1234, 92124 offered 0.8s later".
    pub text: String,
}

/// One claimed edge: `quest_id`'s `field` contains (or, for scalar fields, equals) `target`.
/// A list value such as preQuestSingle = {A, B} is two candidates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationCandidate {
    pub quest_id: QuestId,
    pub field: RelationField,
    pub target: QuestId,
    /// Signal-specific confidence in [0, 1].
    pub score: f64,
    /// Independent characters whose observations support the edge.
    pub support: u32,
    /// Independent characters whose observations contradict the edge.
    pub contradict: u32,
    /// A few concrete examples (about 5).
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl RelationCandidate {
    /// The sort key of the order candidate files list candidates in: quest, field name, target.
    /// Fields compare by name, not in [`RelationField`] order, as the TS pipeline sorts them (its
    /// localeCompare agrees with byte order on these ASCII camelCase names). Two signals keep
    /// their TS versions' own orders instead: offer-set (quest, best score first, target) and
    /// breadcrumbs (best score first, quest).
    pub fn canonical_order(&self) -> (QuestId, &'static str, QuestId) {
        (self.quest_id, self.field.name(), self.target)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateFile {
    /// Signal or source name, matching the file name: `candidates/<signal>.json`
    pub signal: String,
    pub generated_at: String,
    /// Episodes (or source records) the signal looked at.
    pub input_count: usize,
    /// Tunables the signal ran with, for reproducibility.
    pub params: serde_json::Value,
    /// Quests the signal had enough data to judge, whether or not it produced candidates for
    /// them. Used for recall.
    pub covered_quest_ids: Vec<QuestId>,
    pub candidates: Vec<RelationCandidate>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_order_sorts_fields_by_name_not_by_declaration() {
        let candidate = |quest_id: QuestId, field: RelationField, target: QuestId| RelationCandidate {
            quest_id,
            field,
            target,
            score: 0.5,
            support: 1,
            contradict: 0,
            evidence: vec![],
            note: None,
        };
        let mut candidates = [
            candidate(2, RelationField::ExclusiveTo, 1),
            candidate(1, RelationField::PreQuestSingle, 5),
            candidate(1, RelationField::ExclusiveTo, 9),
            candidate(1, RelationField::ExclusiveTo, 3),
        ];
        candidates.sort_by_key(RelationCandidate::canonical_order);
        let order: Vec<_> = candidates.iter().map(|c| (c.quest_id, c.field, c.target)).collect();
        // preQuestSingle is declared before exclusiveTo but sorts after it by name, as in the TS files.
        assert_eq!(
            order,
            [
                (1, RelationField::ExclusiveTo, 3),
                (1, RelationField::ExclusiveTo, 9),
                (1, RelationField::PreQuestSingle, 5),
                (2, RelationField::ExclusiveTo, 1),
            ]
        );
    }
}
