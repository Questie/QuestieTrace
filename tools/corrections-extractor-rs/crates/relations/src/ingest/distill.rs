//! Distills one raw session into an episode draft: the character context, completed / quest-log /
//! level timelines, quest dialog events and giver offer lists.
//!
//! All output times are session-relative `tp` (GetTimePreciseSec), not `t`: `t` is shared by
//! everything in one frame, so it cannot order a turn-in before the follow-up detail it causes.
//! See [`super::streams`] for how samples are matched to the event that caused them.

use super::short_hash;
use super::streams::{
    as_integer, as_positive_int, entries_between, entry_time, event_arg, event_time, finite_entry_time, finite_event_time, last_index_before, packed,
    positive_int, sampled_before, value_at_or_before, value_before,
};
use crate::model::{
    CompletedChange, CompletedSource, CompletedTimeline, Faction, GiverKind, GiverRef, OfferSnapshot, OfferSource, OfferedQuest, PlayerContext,
    QuestClientInfo, QuestEpisode, QuestEvent, QuestEventKind, QuestId, SessionTime, TimedValue,
};
use crate::text::js_trim;
use indexmap::IndexSet;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use std::collections::BTreeMap;
use trace_data::stream::{entry_t, entry_v, get_stream, Stream};
use trace_data::value::{format_number, to_json};
use trace_data::{Event, JsKey, KeyRef, Session, Value};

/// Tunables, reported in ingest.json so episode semantics are reproducible.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistillParams {
    /// A detail this soon after a turn-in is taken as the server's follow-up offer, not a fresh list.
    pub after_turn_in_seconds: f64,
    /// A gossip/greeting list from the same giver this recent means a detail was not opened directly.
    pub list_context_seconds: f64,
    /// Any other dialog with a compatible giver this recent means a detail was not a fresh right-click.
    pub quiet_before_detail_seconds: f64,
    /// A detail this soon after an accept or a dialog closing in an earlier frame was pushed by the
    /// server, not clicked.
    pub pushed_detail_seconds: f64,
    /// An accept this soon after a list, of a quest the list lacks, was already in flight: the list
    /// may omit it.
    pub in_flight_accept_seconds: f64,
    /// An empty gossip page this soon after a non-empty one from the same giver, with no quest event
    /// between, is a sub-menu.
    pub sub_menu_seconds: f64,
    /// How far back accepted/turnedIn look for the dialog of the same quest to inherit its giver.
    pub carry_giver_seconds: f64,
    /// A detail/turn-in dialog whose quest id never showed in GetQuestID borrows it from the next
    /// accept/turn-in this soon...
    pub follow_up_seconds: f64,
    /// ...and the accept must answer the dialog's close this soon (the server confirms an accept
    /// after the window closes).
    pub follow_up_after_close_seconds: f64,
    /// QUEST_REMOVED this close to the quest's own turn-in or completion is the turn-in, not an abandon.
    pub removal_turn_in_seconds: f64,
    /// QUEST_REMOVED this close to another quest's accept/turn-in may be server-side (breadcrumb or
    /// chain replacement).
    pub removal_other_quest_seconds: f64,
    /// A completed-set removal at least this large that is re-added soon after is an empty read
    /// during a loading screen.
    pub completed_read_gap_min_quests: usize,
    pub completed_read_gap_seconds: f64,
    pub completed_read_gap_re_added_ratio: f64,
}

pub const PARAMS: DistillParams = DistillParams {
    after_turn_in_seconds: 5.0,
    list_context_seconds: 120.0,
    quiet_before_detail_seconds: 2.0,
    pushed_detail_seconds: 2.0,
    in_flight_accept_seconds: 2.0,
    sub_menu_seconds: 60.0,
    carry_giver_seconds: 60.0,
    follow_up_seconds: 5.0,
    follow_up_after_close_seconds: 1.5,
    removal_turn_in_seconds: 5.0,
    removal_other_quest_seconds: 2.0,
    completed_read_gap_min_quests: 20,
    completed_read_gap_seconds: 30.0,
    completed_read_gap_re_added_ratio: 0.9,
};

/// How far a recorded duration may run past the last event before it is distrusted.
const DURATION_SLACK_SECONDS: f64 = 3600.0;

// ---------------------------------------------------------------------------
// What a session distills to
// ---------------------------------------------------------------------------

/// A [`crate::model::QuestEpisode`] before merge: the key, contributor, character, submissions
/// and session order need every submission and are filled in later.
#[derive(Clone, Debug, PartialEq)]
pub struct EpisodeDraft {
    pub session_name: Option<String>,
    pub interface_version: Option<i64>,
    pub locale: Option<String>,
    pub player: PlayerContext,
    pub duration: SessionTime,
    pub levels: Vec<TimedValue<i64>>,
    pub completed: Option<CompletedTimeline>,
    pub quest_log: Vec<TimedValue<Vec<QuestId>>>,
    pub quest_events: Vec<QuestEvent>,
    pub offers: Vec<OfferSnapshot>,
    pub quest_info: Option<BTreeMap<QuestId, QuestClientInfo>>,
}

/// What merging the copies of a session and grouping characters add to its draft.
pub struct EpisodeIdentity {
    pub key: String,
    pub contributor_id: String,
    pub character_key: String,
    /// Sorted.
    pub submission_ids: Vec<String>,
    pub session_order: u32,
}

impl EpisodeDraft {
    pub fn into_episode(self, identity: EpisodeIdentity) -> QuestEpisode {
        QuestEpisode {
            key: identity.key,
            contributor_id: identity.contributor_id,
            character_key: identity.character_key,
            submission_ids: identity.submission_ids,
            session_name: self.session_name,
            session_order: identity.session_order,
            interface_version: self.interface_version,
            locale: self.locale,
            player: self.player,
            duration: self.duration,
            levels: self.levels,
            completed: self.completed,
            quest_log: self.quest_log,
            quest_events: self.quest_events,
            offers: self.offers,
            quest_info: self.quest_info,
        }
    }

    /// Frees the room vectors grew into. Drafts are held until every submission is read (on the
    /// 5% sample a third of their memory was such room), so this is done once, when distilled.
    fn shrink_to_fit(&mut self) {
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

/// A legacy greeting list that only recorded titles. Resolved to quest ids once the
/// (locale, title) -> quest id pairings of all submissions are known (see [`super::titles`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PendingGreeting {
    /// Index into the episode's offers.
    pub offer: usize,
    pub available: Vec<String>,
    pub active: Vec<String>,
    /// GetNumAvailableQuests; fewer readable titles than this means the list cannot be complete.
    pub expected: f64,
    /// The list cannot be complete whatever the titles resolve to (see `mark_in_flight_accepts`).
    pub incomplete: bool,
}

/// One (title, quest id) pairing the session saw in a dialog, gossip list or the quest log, in
/// the session's locale and faction. Only used to resolve legacy greeting titles; never written
/// to episodes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TitleObservation {
    pub title: String,
    pub quest_id: QuestId,
    /// Set when the pairing was seen at a known giver.
    pub giver: Option<GiverRef>,
}

/// Dialog reads distill could not use, counted per session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistillStats {
    /// QUEST_DETAIL whose quest id never showed in GetQuestID nor in a following QUEST_ACCEPTED.
    pub detail_unresolved: u64,
    /// QUEST_PROGRESS/QUEST_COMPLETE likewise, checked against QUEST_TURNED_IN.
    pub progress_unresolved: u64,
    pub greeting_without_count: u64,
    pub gossip_without_read: u64,
    /// Item-started details; Forever records no item id for them, so they have no giver.
    pub item_started_details: u64,
    /// Dialog quest ids taken from the accept/turn-in that answered the dialog.
    pub borrowed_quest_ids: u64,
    /// GOSSIP_SHOWs without their own read whose non-empty lists were last read at another giver
    /// (never complete).
    pub gossip_inherited_from_other_giver: u64,
    /// Empty completed-quest reads (bulk removal + re-add) collapsed out of the timeline.
    pub completed_read_gaps: u64,
}

impl std::ops::AddAssign for DistillStats {
    fn add_assign(&mut self, other: DistillStats) {
        self.detail_unresolved += other.detail_unresolved;
        self.progress_unresolved += other.progress_unresolved;
        self.greeting_without_count += other.greeting_without_count;
        self.gossip_without_read += other.gossip_without_read;
        self.item_started_details += other.item_started_details;
        self.borrowed_quest_ids += other.borrowed_quest_ids;
        self.gossip_inherited_from_other_giver += other.gossip_inherited_from_other_giver;
        self.completed_read_gaps += other.completed_read_gaps;
    }
}

/// One session of one submission, distilled.
#[derive(Clone, Debug)]
pub struct DistilledSession {
    /// Stable episode key (see [`episode_key`]).
    pub key: String,
    /// How complete this copy is; the copy with the most events wins a merge.
    pub event_count: usize,
    /// Stopped/saved sessions carry a name; in-progress `currentSession` copies do not.
    pub saved: bool,
    /// GetServerTime at capture start (epoch seconds). Orders a contributor's sessions; never
    /// written to episodes.
    pub start_server_time: Option<f64>,
    pub episode: EpisodeDraft,
    pub pending_greetings: Vec<PendingGreeting>,
    pub titles: Vec<TitleObservation>,
    pub stats: DistillStats,
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// The episode key of a session record: a hash of its two capture-start clocks, which every copy
/// of the session shares (exports are cumulative, so one session shows up in many submissions,
/// growing as play continues). A session without both clocks cannot be matched with its copies,
/// so it is keyed by its content instead: only identical copies merge.
pub fn episode_key(record: &Value) -> String {
    let clock = |name: &str| record.get(name).and_then(Value::as_num).filter(|n| n.is_finite());
    match (clock("startedAt"), clock("startedAtPrecise")) {
        // + 0.0 turns -0 into 0, as JS String(-0) does.
        (Some(started_at), Some(precise)) => short_hash(&format!("{}|{}", format_number(started_at + 0.0), format_number(precise + 0.0))),
        _ => short_hash(&to_json(record)),
    }
}

/// The recorded client interface number (GetBuildInfo's 4th return, or the number alone).
pub fn interface_version(session: &Session) -> Option<i64> {
    let entries = get_stream(session, "GetBuildInfo", &[KeyRef::Str("player")]).or_else(|| get_stream(session, "GetBuildInfo", &[]))?;
    let value = entries.iter().find_map(entry_v)?;
    let version = match value.as_num() {
        Some(version) => version,
        None if value.is_table() => value.get(4)?.as_num()?,
        None => return None,
    };
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    (version.fract() == 0.0 && version > 0.0 && version <= MAX_SAFE_INTEGER).then_some(version as i64)
}

/// An explicitly recorded Forever build (interface 16xxx). A missing version is not inferred.
pub fn is_recorded_forever(session: &Session) -> bool {
    interface_version(session).is_some_and(|version| (16_000..=16_999).contains(&version))
}

// ---------------------------------------------------------------------------
// Character context and timelines
// ---------------------------------------------------------------------------

fn first_player_value<'v, 'a>(session: &Session<'v, 'a>, name: &str) -> Option<&'v Value<'a>> {
    get_stream(session, name, &[KeyRef::Str("player")])?.iter().find_map(entry_v)
}

fn read_player(session: &Session) -> PlayerContext {
    let faction = packed(first_player_value(session, "UnitFactionGroup"), 1).and_then(Value::as_str);
    PlayerContext {
        race_id: as_positive_int(packed(first_player_value(session, "UnitRace"), 3)),
        class_id: as_positive_int(packed(first_player_value(session, "UnitClass"), 3)),
        faction: match faction {
            Some("Alliance") => Some(Faction::Alliance),
            Some("Horde") => Some(Faction::Horde),
            Some("Neutral") => Some(Faction::Neutral),
            _ => None,
        },
    }
}

fn read_levels(session: &Session) -> Vec<TimedValue<i64>> {
    let mut levels: Vec<TimedValue<i64>> = Vec::new();
    for entry in get_stream(session, "UnitLevel", &[KeyRef::Str("player")]).unwrap_or_default() {
        let (Some(t), Some(level)) = (finite_entry_time(entry), as_positive_int(entry_v(entry))) else {
            continue;
        };
        if levels.last().is_none_or(|last| last.v != level) {
            levels.push(TimedValue { t, v: level });
        }
    }
    levels
}

/// The positive integers of an id list; anything but an array is no list.
fn quest_ids(list: Option<&Value>) -> Vec<QuestId> {
    let items = list.and_then(Value::as_array).unwrap_or_default();
    items.iter().filter_map(|id| as_positive_int(Some(id))).collect()
}

/// GetAllCompletedQuestIDs can read empty during a loading screen, which records a bulk removal
/// of the whole history followed seconds later by its re-add. Left in, long-done quests would look
/// not completed for those seconds. Drops the removal and the matching re-adds; ids that really
/// were removed (not re-added) and genuinely new adds keep their times.
fn collapse_read_gaps(mut changes: Vec<CompletedChange>, stats: &mut DistillStats) -> Vec<CompletedChange> {
    for i in 0..changes.len() {
        let removed = changes[i].remove.clone().unwrap_or_default();
        if removed.len() < PARAMS.completed_read_gap_min_quests {
            continue;
        }
        let mut pending: FxHashSet<QuestId> = removed.iter().copied().collect();
        // (change index, quest id) of every re-add.
        let mut re_adds: Vec<(usize, QuestId)> = Vec::new();
        let soon_after = (i + 1..changes.len()).take_while(|&j| changes[j].t - changes[i].t <= PARAMS.completed_read_gap_seconds);
        for j in soon_after {
            for &id in changes[j].add.iter().flatten() {
                if pending.remove(&id) {
                    re_adds.push((j, id));
                }
            }
        }
        if (re_adds.len() as f64) < PARAMS.completed_read_gap_re_added_ratio * removed.len() as f64 {
            continue;
        }
        stats.completed_read_gaps += 1;
        let restored: FxHashSet<QuestId> = re_adds.iter().map(|&(_, id)| id).collect();
        changes[i].remove = Some(removed.into_iter().filter(|id| !restored.contains(id)).collect());
        for &(j, _) in &re_adds {
            if let Some(add) = &mut changes[j].add {
                add.retain(|id| !restored.contains(id));
            }
        }
    }
    let non_empty = |list: Option<Vec<QuestId>>| list.filter(|ids| !ids.is_empty());
    changes
        .into_iter()
        .map(|change| CompletedChange {
            t: change.t,
            add: non_empty(change.add),
            remove: non_empty(change.remove),
        })
        .filter(|change| change.add.is_some() || change.remove.is_some())
        .collect()
}

fn read_completed(session: &Session, stats: &mut DistillStats) -> Option<CompletedTimeline> {
    let modern = session.function_delta("C_QuestLog.GetAllCompletedQuestIDs");
    let stream = modern.or_else(|| session.function_delta("GetQuestsCompleted"))?;
    let initial = stream.get("initial").filter(|initial| initial.is_array())?;
    let mut changes = Vec::new();
    for delta in stream.get("delta").and_then(Value::as_array).unwrap_or_default() {
        let (add, remove) = (quest_ids(delta.get("add")), quest_ids(delta.get("remove")));
        let Some(t) = finite_entry_time(delta).filter(|_| !add.is_empty() || !remove.is_empty()) else {
            continue;
        };
        changes.push(CompletedChange {
            t,
            add: (!add.is_empty()).then_some(add),
            remove: (!remove.is_empty()).then_some(remove),
        });
    }
    let mut initial = quest_ids(Some(initial));
    initial.sort_unstable();
    Some(CompletedTimeline {
        // A modern stream, when there is one, is the stream read above.
        source: if modern.is_some() { CompletedSource::Modern } else { CompletedSource::Legacy },
        initial,
        changes: collapse_read_gaps(changes, stats),
    })
}

/// Quest log change points; the raw stream also changes when only the order changes.
fn read_quest_log(session: &Session) -> Vec<TimedValue<Vec<QuestId>>> {
    let mut log: Vec<TimedValue<Vec<QuestId>>> = Vec::new();
    for entry in get_stream(session, "QuestLog", &[]).unwrap_or_default() {
        let (Some(t), Some(rows)) = (finite_entry_time(entry), entry_v(entry).filter(|v| v.is_array())) else {
            continue;
        };
        let mut ids = quest_ids(Some(rows));
        ids.sort_unstable();
        ids.dedup();
        if log.last().is_some_and(|last| last.v == ids) {
            continue;
        }
        log.push(TimedValue { t, v: ids });
    }
    log
}

/// A quest-id key of a per-quest stream table.
fn quest_id_key(key: &JsKey) -> Option<QuestId> {
    match key {
        JsKey::Index(id) => (*id > 0).then_some(i64::from(*id)),
        JsKey::Name(name) => name.parse::<QuestId>().ok().filter(|id| *id > 0),
    }
}

/// Forever-only per-quest streams, keyed by quest id. Absent in traces older than the addon change.
fn read_quest_info(session: &Session) -> Option<BTreeMap<QuestId, QuestClientInfo>> {
    let mut info: BTreeMap<QuestId, QuestClientInfo> = BTreeMap::new();
    let mut each = |name: &str, apply: &dyn Fn(&mut QuestClientInfo, &Value)| {
        let Some(root) = session.function(name).and_then(Value::as_object) else {
            return;
        };
        for (key, stream) in root.entries() {
            let Some(quest_id) = quest_id_key(key) else { continue };
            // The latest value the stream recorded.
            let Some(value) = stream.as_array().and_then(|entries| entries.iter().rev().find_map(entry_v)) else {
                continue;
            };
            apply(info.entry(quest_id).or_default(), value);
        }
    };
    each("IsBreadcrumbQuest", &|target, value| {
        if let Some(is_breadcrumb) = value.as_bool() {
            target.is_breadcrumb = Some(is_breadcrumb);
        }
    });
    each("C_QuestLine.GetQuestLineInfo", &|target, value| {
        if let Some(quest_line_id) = as_positive_int(value.get("questLineID")) {
            target.quest_line_id = Some(quest_line_id);
        }
    });
    each("C_QuestInfoSystem.GetQuestClassification", &|target, value| {
        if let Some(classification) = as_integer(Some(value)) {
            target.classification = Some(classification);
        }
    });
    info.retain(|_, quest| *quest != QuestClientInfo::default());
    (!info.is_empty()).then_some(info)
}

// ---------------------------------------------------------------------------
// Dialog steps: quest dialog / gossip events resolved to quest ids and givers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ListKind {
    Gossip,
    Greeting,
}

/// Titles of a legacy greeting list, resolved after all submissions are read.
struct PendingTitles {
    available: Vec<String>,
    active: Vec<String>,
    expected: f64,
    incomplete: bool,
}

struct ListStep {
    kind: ListKind,
    tp: f64,
    giver: Option<GiverRef>,
    offer: OfferSnapshot,
    pending: Option<PendingTitles>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DialogKind {
    Detail,
    Progress,
    Complete,
}

#[derive(Default)]
struct DialogStep {
    tp: f64,
    quest_id: Option<QuestId>,
    /// GetTitleText paired with the quest id, for greeting title resolution.
    title: Option<String>,
    /// Giver read from the dialog's own UnitGUID (questnpc/npc).
    direct_giver: Option<GiverRef>,
    /// QUEST_DETAIL questStartItemID.
    item_id: Option<i64>,
    /// A dialog closed (in an earlier frame) moments before this one opened.
    after_close: bool,
    /// tp of the first QUEST_FINISHED/GOSSIP_CLOSED after the dialog opened.
    close_tp: Option<f64>,
    /// Started from an item. Forever leaves questStartItemID at 0 and only puts an Item GUID,
    /// which carries no item id, in questnpc/npc; such details get no giver rather than a
    /// guessed one.
    from_item: bool,
    /// The offer a detail stands for, when it is not picked from a list.
    offer: Option<OfferSnapshot>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum QuestStepKind {
    Accepted,
    TurnedIn,
    Removed,
}

struct QuestStep {
    kind: QuestStepKind,
    tp: f64,
    quest_id: QuestId,
    replay: bool,
}

enum Step {
    List(ListStep),
    Dialog(DialogKind, DialogStep),
    Quest(QuestStep),
}

impl Step {
    fn tp(&self) -> f64 {
        match self {
            Step::List(list) => list.tp,
            Step::Dialog(_, dialog) => dialog.tp,
            Step::Quest(quest) => quest.tp,
        }
    }

    fn giver(&self) -> Option<GiverRef> {
        match self {
            Step::List(list) => list.giver,
            Step::Dialog(_, dialog) => dialog.direct_giver,
            Step::Quest(_) => None,
        }
    }

    fn is_list_or_dialog(&self) -> bool {
        !matches!(self, Step::Quest(_))
    }

    fn quest(&self, kind: QuestStepKind) -> Option<&QuestStep> {
        match self {
            Step::Quest(quest) if quest.kind == kind => Some(quest),
            _ => None,
        }
    }

    /// A progress/complete dialog (a turn-in dialog) of `quest_id`.
    fn is_turn_in_dialog_of(&self, quest_id: QuestId) -> bool {
        matches!(self, Step::Dialog(kind, dialog) if *kind != DialogKind::Detail && dialog.quest_id == Some(quest_id))
    }

    fn offer(&self) -> Option<&OfferSnapshot> {
        match self {
            Step::List(list) => Some(&list.offer),
            Step::Dialog(_, dialog) => dialog.offer.as_ref(),
            Step::Quest(_) => None,
        }
    }

    fn offer_mut(&mut self) -> Option<&mut OfferSnapshot> {
        match self {
            Step::List(list) => Some(&mut list.offer),
            Step::Dialog(_, dialog) => dialog.offer.as_mut(),
            Step::Quest(_) => None,
        }
    }
}

/// Only creatures, vehicles and game objects give quests; pets and players never do.
fn giver_from_guid(guid: Option<&Value>) -> Option<GiverRef> {
    let guid = guid?.as_str()?;
    let kind = match guid.split_once('-')?.0 {
        "Creature" | "Vehicle" => GiverKind::Npc,
        "GameObject" => GiverKind::Object,
        _ => return None,
    };
    // The entity id is the second-to-last "-"-separated segment.
    let parts: Vec<&str> = guid.split('-').collect();
    if parts.len() < 3 {
        return None;
    }
    let id = positive_int(parts[parts.len() - 2].parse::<f64>().ok()?)?;
    Some(GiverRef { kind, id })
}

/// A string that is not blank by JS rules, as given (untrimmed).
fn non_empty_title<'v>(value: Option<&'v Value>) -> Option<&'v str> {
    value?.as_str().filter(|title| !js_trim(title).is_empty())
}

fn same_giver(a: Option<GiverRef>, b: Option<GiverRef>) -> bool {
    a.is_some() && a == b
}

/// Unknown givers are compatible with anything: used where matching too much is the safe side.
fn compatible_giver(a: Option<GiverRef>, b: Option<GiverRef>) -> bool {
    a.is_none() || b.is_none() || a == b
}

fn is_dialog_open(name: Option<&str>) -> bool {
    matches!(
        name,
        Some("QUEST_DETAIL" | "QUEST_PROGRESS" | "QUEST_COMPLETE" | "QUEST_GREETING" | "GOSSIP_SHOW")
    )
}

fn is_dialog_close(name: Option<&str>) -> bool {
    matches!(name, Some("QUEST_FINISHED" | "GOSSIP_CLOSED"))
}

struct SessionReader<'s, 'v, 'a> {
    session: &'s Session<'v, 'a>,
    events: Vec<Event<'v, 'a>>,
    /// For event i: tp of the next dialog open/close event, the end of the samples that describe
    /// event i's dialog.
    next_dialog_tp: Vec<f64>,
    /// For event i: tp of the previous dialog open event.
    prev_open_tp: Vec<f64>,
    quest_id_stream: Option<Stream<'v, 'a>>,
    title_stream: Option<Stream<'v, 'a>>,
}

impl<'s, 'v, 'a> SessionReader<'s, 'v, 'a> {
    fn new(session: &'s Session<'v, 'a>) -> Self {
        let events: Vec<Event> = session.events().collect();
        let mut next_dialog_tp = vec![f64::INFINITY; events.len()];
        let mut next = f64::INFINITY;
        for (i, event) in events.iter().enumerate().rev() {
            next_dialog_tp[i] = next;
            if is_dialog_open(event.name()) || is_dialog_close(event.name()) {
                next = event_time(event);
            }
        }
        let mut prev_open_tp = vec![f64::NEG_INFINITY; events.len()];
        let mut previous = f64::NEG_INFINITY;
        for (i, event) in events.iter().enumerate() {
            prev_open_tp[i] = previous;
            if is_dialog_open(event.name()) {
                previous = event_time(event);
            }
        }
        SessionReader {
            session,
            events,
            next_dialog_tp,
            prev_open_tp,
            quest_id_stream: get_stream(session, "GetQuestID", &[]),
            title_stream: get_stream(session, "GetTitleText", &[]),
        }
    }

    fn stream(&self, name: &str, params: &[KeyRef]) -> Option<Stream<'v, 'a>> {
        get_stream(self.session, name, params)
    }

    /// tp of the first QUEST_FINISHED/GOSSIP_CLOSED after event i.
    fn close_after(&self, i: usize) -> Option<f64> {
        self.events[i + 1..].iter().find(|event| is_dialog_close(event.name())).map(event_time)
    }

    /// Whether a QUEST_FINISHED/GOSSIP_CLOSED from an earlier frame happened within `seconds`
    /// before event i.
    fn closed_shortly_before(&self, i: usize, seconds: f64) -> bool {
        let event = &self.events[i];
        self.events[..i]
            .iter()
            .rev()
            .take_while(|earlier| event_time(event) - event_time(earlier) <= seconds)
            .any(|earlier| is_dialog_close(earlier.name()) && earlier.t() < event.t())
    }

    /// The value a stream held once event i's dialog had been sampled.
    fn dialog_value(&self, i: usize, name: &str, params: &[KeyRef]) -> Option<&'v Value<'a>> {
        value_before(self.stream(name, params), self.next_dialog_tp[i])
    }

    fn dialog_sampled(&self, i: usize, name: &str) -> bool {
        sampled_before(self.stream(name, &[]), self.next_dialog_tp[i])
    }

    /// Gossip lists are stored change-only, so a GOSSIP_SHOW without its own sample either re-read
    /// the same lists or failed to read them, and the two look alike. Inheriting is believable
    /// when the earlier read was at the same giver, or when the lists are empty (every vendor and
    /// flight master in a row); a non-empty list carried over from another giver is not.
    fn gossip_read_is_own(&self, i: usize, giver: Option<GiverRef>) -> bool {
        let at = event_time(&self.events[i]);
        let streams = [
            self.stream("C_GossipInfo.GetAvailableQuests", &[]),
            self.stream("C_GossipInfo.GetActiveQuests", &[]),
        ];
        if streams.iter().any(|stream| !entries_between(*stream, at, self.next_dialog_tp[i]).is_empty()) {
            return true;
        }
        let empty = |list: Option<&Value>| list.and_then(Value::as_array).is_none_or(<[Value]>::is_empty);
        if streams.iter().all(|stream| empty(value_before(*stream, at))) {
            return true;
        }
        let last_read = streams
            .iter()
            .filter_map(|stream| {
                let stream = (*stream)?;
                Some(entry_time(&stream[last_index_before(stream, at, false)?]))
            })
            .fold(f64::NEG_INFINITY, f64::max);
        // UnitInteraction samples the npc token just before the lists, in the same handler.
        let npc = value_at_or_before(self.stream("UnitGUID", &[KeyRef::Str("npc")]), last_read);
        same_giver(giver, giver_from_guid(npc))
    }

    /// The list indices 1, 2, ... up to `count` (a list's recorded length) that per-index streams
    /// `functions[name][index]` can answer for: indices above the highest one they recorded read
    /// nothing. Bounding by it also keeps a damaged count from looping (nearly) forever.
    fn indices_read(&self, names: [&str; 2], count: f64) -> impl Iterator<Item = u32> {
        let recorded = names.iter().filter_map(|name| self.session.function(name)?.as_object());
        let highest = recorded
            .flat_map(|streams| streams.entries())
            .filter_map(|(key, _)| match key {
                JsKey::Index(index) => Some(*index),
                JsKey::Name(_) => None,
            })
            .max()
            .unwrap_or(0);
        (1..=highest).take_while(move |&index| f64::from(index) <= count)
    }

    fn giver_at(&self, i: usize, tokens: [&str; 2]) -> Option<GiverRef> {
        tokens
            .iter()
            .find_map(|&token| giver_from_guid(self.dialog_value(i, "UnitGUID", &[KeyRef::Str(token)])))
    }

    /// GetQuestID (and its title) for the dialog opened by event i. Prefers samples taken for the
    /// event itself; falls back to samples earlier in the same frame, because the client can
    /// already be in the new dialog while it is still dispatching the previous dialog's
    /// QUEST_FINISHED events (and an auto-accept addon can close it again before QUEST_DETAIL
    /// reaches us).
    fn dialog_quest(&self, i: usize) -> Option<(QuestId, Option<String>)> {
        let event = &self.events[i];
        let at = event_time(event);
        // A sample that read a quest: GetQuestID is 0 once the dialog closed.
        let quest_of = |sample: &'v Value<'a>| Some((sample, as_positive_int(entry_v(sample))?));
        let own_samples = entries_between(self.quest_id_stream, at, self.next_dialog_tp[i]);
        let earlier_in_frame = || {
            let earlier = entries_between(self.quest_id_stream, self.prev_open_tp[i], at);
            earlier.iter().rev().filter(|sample| entry_t(sample) == event.t()).find_map(quest_of)
        };
        if let Some((sample, quest_id)) = own_samples.iter().rev().find_map(quest_of).or_else(earlier_in_frame) {
            return Some((quest_id, self.title_at(entry_time(sample), true)));
        }
        // The dialog's own reads saw no quest (it was already closed): an older value would be
        // another dialog's.
        if !own_samples.is_empty() {
            return None;
        }
        // Nothing new was sampled: the dialog still shows what an earlier sample of it read.
        let in_effect = as_positive_int(value_before(self.quest_id_stream, at))?;
        Some((in_effect, self.title_at(at, false)))
    }

    /// GetTitleText from the same sample as a GetQuestID entry (both are read in one call).
    fn title_at(&self, tp: f64, inclusive: bool) -> Option<String> {
        let value = if inclusive {
            value_at_or_before(self.title_stream, tp)
        } else {
            value_before(self.title_stream, tp)
        };
        non_empty_title(value).map(str::to_owned)
    }
}

fn offered_from_gossip(entry: &Value) -> Option<OfferedQuest> {
    entry.as_object()?;
    Some(OfferedQuest {
        id: as_positive_int(entry.get("questID"))?,
        trivial: entry.get("isTrivial").and_then(Value::as_bool),
        frequency: as_integer(entry.get("frequency")),
        repeatable: entry.get("repeatable").and_then(Value::as_bool),
    })
}

fn gossip_step<'v, 'a>(reader: &SessionReader<'_, 'v, 'a>, i: usize, stats: &mut DistillStats, titles: &mut TitleSink) -> ListStep {
    let event = &reader.events[i];
    let tp = event_time(event);
    let giver = reader.giver_at(i, ["npc", "questnpc"]);
    let raw_available = reader.dialog_value(i, "C_GossipInfo.GetAvailableQuests", &[]);
    let raw_active = reader.dialog_value(i, "C_GossipInfo.GetActiveQuests", &[]);
    let read = reader.dialog_sampled(i, "C_GossipInfo.GetAvailableQuests") && raw_available.is_some_and(Value::is_array);
    if !read {
        stats.gossip_without_read += 1;
    }
    let inherited_from_other_giver = read && !reader.gossip_read_is_own(i, giver);
    if inherited_from_other_giver {
        stats.gossip_inherited_from_other_giver += 1;
    }

    // A list carried over from another giver describes that giver, not this one: keep nothing.
    let (available_entries, active_entries) = if inherited_from_other_giver {
        (Default::default(), Default::default())
    } else {
        let list = |raw: Option<&'v Value<'a>>| raw.and_then(Value::as_array).unwrap_or_default();
        (list(raw_available), list(raw_active))
    };
    let available: Vec<OfferedQuest> = available_entries.iter().filter_map(offered_from_gossip).collect();
    let active: Vec<QuestId> = active_entries.iter().filter_map(offered_from_gossip).map(|quest| quest.id).collect();
    for entry in available_entries.iter().chain(active_entries) {
        titles.add(non_empty_title(entry.get("title")), as_positive_int(entry.get("questID")), giver);
    }

    // The count API is sampled by a different tracker; disagreement means one read was stale.
    let count = reader.dialog_value(i, "C_GossipInfo.GetNumAvailableQuests", &[]).and_then(Value::as_num);
    let count_agrees = count.is_none_or(|count| count == available.len() as f64);
    // GOSSIP_SHOW and GOSSIP_CLOSED in one frame: an auto-quest addon handled the gossip at once.
    // A read that still saw quests happened before the close; an empty one may describe the closed
    // window instead of the giver.
    let closed_same_frame = reader.events[i + 1..]
        .iter()
        .take_while(|later| later.t() == event.t())
        .any(|later| later.is("GOSSIP_CLOSED"));
    let empty_after_close = closed_same_frame && available.is_empty() && active.is_empty();

    ListStep {
        kind: ListKind::Gossip,
        tp,
        giver,
        offer: OfferSnapshot {
            t: tp,
            source: OfferSource::Gossip,
            giver,
            available,
            unresolved_titles: None,
            active: Some(active),
            list_complete: read && !inherited_from_other_giver && count_agrees && !empty_after_close,
        },
        pending: None,
    }
}

/// QUEST_GREETING lists. Newer addon builds record the quest id behind each index
/// (GetAvailableQuestInfo's 5th return, GetActiveQuestID); older builds, and clients whose
/// GetAvailableQuestInfo has no quest id, only have titles, which are resolved once every
/// submission's title pairings are known.
fn greeting_step(reader: &SessionReader, i: usize, stats: &mut DistillStats, titles: &mut TitleSink) -> ListStep {
    let tp = event_time(&reader.events[i]);
    let giver = reader.giver_at(i, ["npc", "questnpc"]);
    let offer = |available: Vec<OfferedQuest>, active: Vec<QuestId>, list_complete: bool| OfferSnapshot {
        t: tp,
        source: OfferSource::Greeting,
        giver,
        available,
        unresolved_titles: None,
        active: Some(active),
        list_complete,
    };
    let Some(num_available) = reader.dialog_value(i, "GetNumAvailableQuests", &[]).and_then(Value::as_num) else {
        stats.greeting_without_count += 1;
        return ListStep {
            kind: ListKind::Greeting,
            tp,
            giver,
            offer: offer(Vec::new(), Vec::new(), false),
            pending: None,
        };
    };
    let num_active = reader.dialog_value(i, "GetNumActiveQuests", &[]).and_then(Value::as_num).unwrap_or(0.0);

    let mut available: Vec<OfferedQuest> = Vec::new();
    let mut available_titles: Vec<String> = Vec::new();
    let read_available = reader.indices_read(["GetAvailableQuestInfo", "GetAvailableTitle"], num_available);
    for index in read_available {
        let info = reader.dialog_value(i, "GetAvailableQuestInfo", &[KeyRef::from(index)]);
        let title = non_empty_title(reader.dialog_value(i, "GetAvailableTitle", &[KeyRef::from(index)]));
        let Some(id) = as_positive_int(packed(info, 5)) else {
            available_titles.extend(title.map(str::to_owned));
            continue;
        };
        titles.add(title, Some(id), giver);
        available.push(OfferedQuest {
            id,
            trivial: packed(info, 1).and_then(Value::as_bool),
            frequency: as_integer(packed(info, 2)),
            repeatable: packed(info, 3).and_then(Value::as_bool),
        });
    }

    let mut active: Vec<QuestId> = Vec::new();
    let mut active_titles: Vec<String> = Vec::new();
    for index in reader.indices_read(["GetActiveQuestID", "GetActiveTitle"], num_active) {
        let id = as_positive_int(reader.dialog_value(i, "GetActiveQuestID", &[KeyRef::from(index)]));
        let title = non_empty_title(packed(reader.dialog_value(i, "GetActiveTitle", &[KeyRef::from(index)]), 1));
        match id {
            Some(id) => {
                titles.add(title, Some(id), giver);
                active.push(id);
            }
            None => active_titles.extend(title.map(str::to_owned)),
        }
    }

    let distinct: FxHashSet<QuestId> = available.iter().map(|quest| quest.id).collect();
    let list_complete = available.len() as f64 == num_available && distinct.len() == available.len();
    if available_titles.is_empty() && active_titles.is_empty() {
        return ListStep {
            kind: ListKind::Greeting,
            tp,
            giver,
            offer: offer(available, active, list_complete),
            pending: None,
        };
    }
    // Completeness is decided again once the titles are resolved.
    ListStep {
        kind: ListKind::Greeting,
        tp,
        giver,
        offer: offer(available, active, false),
        pending: Some(PendingTitles {
            available: available_titles,
            active: active_titles,
            expected: num_available,
            incomplete: false,
        }),
    }
}

fn dialog_step(reader: &SessionReader, i: usize, kind: DialogKind, stats: &mut DistillStats) -> DialogStep {
    let (quest_id, title) = reader.dialog_quest(i).unzip();
    let mut step = DialogStep {
        tp: event_time(&reader.events[i]),
        quest_id,
        title: title.flatten(),
        close_tp: reader.close_after(i),
        ..DialogStep::default()
    };
    if kind == DialogKind::Detail {
        step.after_close = reader.closed_shortly_before(i, PARAMS.pushed_detail_seconds);
        step.item_id = as_positive_int(event_arg(&reader.events[i], 1));
        let item_guid = ["questnpc", "npc"].iter().any(|&token| {
            let guid = reader.dialog_value(i, "UnitGUID", &[KeyRef::Str(token)]).and_then(Value::as_str);
            guid.is_some_and(|guid| guid.starts_with("Item-"))
        });
        if step.item_id.is_some() || item_guid {
            step.from_item = true;
            stats.item_started_details += 1;
            return step;
        }
    }
    step.direct_giver = reader.giver_at(i, ["questnpc", "npc"]);
    step
}

/// QUEST_ACCEPTED is (logIndex, questID) on Era and (questID) on Forever.
fn accepted_quest_id(event: &Event) -> Option<QuestId> {
    let count = event.arg_count().unwrap_or(0.0);
    as_positive_int(event_arg(event, if count >= 2.0 { 2 } else { 1 }))
}

// ---------------------------------------------------------------------------
// Title observations (only for resolving legacy greeting titles)
// ---------------------------------------------------------------------------

/// The session's distinct (title, quest id, giver) pairings. Without a locale nothing is
/// collected: titles are only comparable within one locale.
struct TitleSink {
    has_locale: bool,
    seen: IndexSet<TitleObservation>,
}

impl TitleSink {
    fn add(&mut self, title: Option<&str>, quest_id: Option<QuestId>, giver: Option<GiverRef>) {
        if let (true, Some(title), Some(quest_id)) = (self.has_locale, title, quest_id) {
            self.seen.insert(TitleObservation {
                title: title.to_owned(),
                quest_id,
                giver,
            });
        }
    }

    /// Quest log rows: C_QuestLog.GetInfo values (modern) and GetQuestLogTitle tuples (legacy).
    fn add_quest_log(&mut self, session: &Session) {
        let values = |name: &str| {
            let streams = session
                .function(name)
                .and_then(Value::as_object)
                .map(|root| root.entries().map(|(_, stream)| stream));
            streams
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(entry_v)
                .collect::<Vec<_>>()
        };
        for info in values("C_QuestLog.GetInfo") {
            if info.as_object().is_some() && info.get("isHeader").and_then(Value::as_bool) != Some(true) {
                self.add(non_empty_title(info.get("title")), as_positive_int(info.get("questID")), None);
            }
        }
        for row in values("GetQuestLogTitle") {
            if packed(Some(row), 4).and_then(Value::as_bool) != Some(true) {
                self.add(non_empty_title(packed(Some(row), 1)), as_positive_int(packed(Some(row), 8)), None);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

fn list_contains(list: &ListStep, detail: &DialogStep) -> bool {
    if detail.quest_id.is_some_and(|id| list.offer.available.iter().any(|quest| quest.id == id)) {
        return true;
    }
    match (&detail.title, &list.pending) {
        (Some(title), Some(pending)) => pending.available.contains(title),
        _ => false,
    }
}

#[derive(Default)]
struct DetailContext {
    /// Index of the gossip/greeting list the detail was picked from.
    from_list: Option<usize>,
    /// A gossip/greeting list from this giver was shown recently, so the detail was not opened
    /// directly.
    list_nearby: bool,
    /// Index of the quest turned in just before, whose follow-up this detail likely is.
    after_turn_in: Option<usize>,
    /// Some other dialog with a compatible giver was open moments before.
    busy: bool,
    /// The detail came right after an accept or another dialog closing: the server pushed it (e.g.
    /// the giver's next quest after an accept), so it says nothing about the giver's whole list.
    pushed: bool,
}

/// `event_giver[j]` is the giver of the quest event step j produced, if any.
fn detail_context(steps: &[Step], index: usize, detail: &DialogStep, event_giver: &[Option<GiverRef>]) -> DetailContext {
    let mut context = DetailContext {
        pushed: detail.after_close,
        ..DetailContext::default()
    };
    // The giver this visit is with: the detail's own, else the latest one seen before it.
    let mut anchor = detail.direct_giver;
    let mut other_giver_seen = false;
    for j in (0..index).rev() {
        let step = &steps[j];
        let age = detail.tp - step.tp();
        if age > PARAMS.list_context_seconds {
            break;
        }
        let giver = step.giver();
        anchor = anchor.or(giver);
        // Lists behind an interaction with a different known giver belong to an earlier visit.
        if giver.is_some() && !same_giver(giver, anchor) {
            other_giver_seen = true;
        }
        if let Step::List(list) = step {
            if !other_giver_seen && compatible_giver(list.giver, anchor) {
                context.list_nearby = true;
                if context.from_list.is_none() && list_contains(list, detail) {
                    context.from_list = Some(j);
                }
            }
        }
        if age <= PARAMS.quiet_before_detail_seconds && step.is_list_or_dialog() && compatible_giver(giver, detail.direct_giver) {
            context.busy = true;
        }
        if let Some(accepted) = step.quest(QuestStepKind::Accepted) {
            if Some(accepted.quest_id) != detail.quest_id && age <= PARAMS.pushed_detail_seconds {
                context.pushed = true;
            }
        }
        if step.quest(QuestStepKind::TurnedIn).is_some()
            && age <= PARAMS.after_turn_in_seconds
            && context.after_turn_in.is_none()
            && compatible_giver(event_giver[j], detail.direct_giver)
        {
            context.after_turn_in = Some(j);
        }
    }
    context
}

/// The most recent step before `index` matching `matches`, within the carry-over window.
fn find_back(steps: &[Step], index: usize, matches: impl Fn(&Step) -> bool) -> Option<usize> {
    let tp = steps[index].tp();
    (0..index)
        .rev()
        .take_while(|&j| tp - steps[j].tp() <= PARAMS.carry_giver_seconds)
        .find(|&j| matches(&steps[j]))
}

/// Fills dialog quest ids that GetQuestID never showed from the accept/turn-in that closes the
/// dialog. Clicking Accept closes the window and the server's QUEST_ACCEPTED follows within a
/// moment; an accept arriving long after the close answers something else (the detail was
/// declined).
fn borrow_quest_ids(steps: &mut [Step], stats: &mut DistillStats) {
    for index in 0..steps.len() {
        let Step::Dialog(kind, dialog) = &steps[index] else { continue };
        if dialog.quest_id.is_some() {
            continue;
        }
        let (kind, tp, close_tp) = (*kind, dialog.tp, dialog.close_tp);
        let closing = if kind == DialogKind::Detail {
            QuestStepKind::Accepted
        } else {
            QuestStepKind::TurnedIn
        };
        // The first accept/turn-in soon after, before any other dialog, if it answered the close.
        let borrowed = steps[index + 1..]
            .iter()
            .take_while(|next| next.tp() - tp <= PARAMS.follow_up_seconds && !next.is_list_or_dialog())
            .find_map(|next| next.quest(closing))
            .filter(|answer| close_tp.is_none_or(|close_tp| answer.tp - close_tp <= PARAMS.follow_up_after_close_seconds))
            .map(|answer| answer.quest_id);
        let Step::Dialog(_, dialog) = &mut steps[index] else {
            unreachable!("checked above")
        };
        match borrowed {
            Some(quest_id) => {
                dialog.quest_id = Some(quest_id);
                stats.borrowed_quest_ids += 1;
            }
            None if kind == DialogKind::Detail => stats.detail_unresolved += 1,
            None => stats.progress_unresolved += 1,
        }
    }
}

fn detail_offer(t: f64, giver: Option<GiverRef>, quest_id: QuestId, list_complete: bool) -> OfferSnapshot {
    OfferSnapshot {
        t,
        source: OfferSource::Detail,
        giver,
        available: vec![OfferedQuest {
            id: quest_id,
            trivial: None,
            frequency: None,
            repeatable: None,
        }],
        unresolved_titles: None,
        active: None,
        list_complete,
    }
}

/// The quest event step `index` stands for, if any, and for a detail not picked from a list the
/// offer it stands for. `event_giver[j]` is the giver of the event of each earlier step j.
fn step_event(
    steps: &[Step],
    index: usize,
    event_giver: &[Option<GiverRef>],
    completed_adds: &FxHashMap<QuestId, Vec<f64>>,
) -> Option<(QuestEvent, Option<OfferSnapshot>)> {
    let t = steps[index].tp();
    let event = |kind: QuestEventKind, quest_id: QuestId, giver: Option<GiverRef>| QuestEvent { t, kind, quest_id, giver };
    match &steps[index] {
        Step::List(_) => None,
        Step::Dialog(kind, dialog) => {
            let quest_id = dialog.quest_id?;
            match kind {
                DialogKind::Progress | DialogKind::Complete => {
                    // progress -> complete of one quest is a single visit; the giver carries over.
                    let earlier = find_back(steps, index, |step| step.is_turn_in_dialog_of(quest_id));
                    let giver = dialog.direct_giver.or_else(|| earlier.and_then(|j| event_giver[j]));
                    let kind = if *kind == DialogKind::Progress {
                        QuestEventKind::Progress
                    } else {
                        QuestEventKind::Complete
                    };
                    Some((event(kind, quest_id, giver), None))
                }
                DialogKind::Detail if dialog.from_item => {
                    let item = dialog.item_id.map(|id| GiverRef { kind: GiverKind::Item, id });
                    Some((event(QuestEventKind::Detail, quest_id, item), Some(detail_offer(t, item, quest_id, false))))
                }
                DialogKind::Detail => {
                    let context = detail_context(steps, index, dialog, event_giver);
                    let giver = dialog
                        .direct_giver
                        .or_else(|| context.from_list.and_then(|j| steps[j].giver()))
                        .or_else(|| context.after_turn_in.and_then(|j| event_giver[j]));
                    // Opened by a fresh right-click on a giver with nothing else to say: the server
                    // only does that when this is the giver's single available quest, so the
                    // one-quest list is complete.
                    let standalone =
                        dialog.direct_giver.is_some() && !context.list_nearby && context.after_turn_in.is_none() && !context.busy && !context.pushed;
                    let offer = context.from_list.is_none().then(|| detail_offer(t, giver, quest_id, standalone));
                    Some((event(QuestEventKind::Detail, quest_id, giver), offer))
                }
            }
        }
        Step::Quest(quest) => {
            let quest_id = quest.quest_id;
            match quest.kind {
                QuestStepKind::Accepted => {
                    let detail = find_back(
                        steps,
                        index,
                        |step| matches!(step, Step::Dialog(DialogKind::Detail, detail) if detail.quest_id == Some(quest_id)),
                    );
                    Some((event(QuestEventKind::Accepted, quest_id, detail.and_then(|j| event_giver[j])), None))
                }
                QuestStepKind::TurnedIn => {
                    let dialog = find_back(steps, index, |step| step.is_turn_in_dialog_of(quest_id));
                    Some((event(QuestEventKind::TurnedIn, quest_id, dialog.and_then(|j| event_giver[j])), None))
                }
                QuestStepKind::Removed => classify_removal(steps, quest, completed_adds).map(|kind| (event(kind, quest_id, None), None)),
            }
        }
    }
}

/// After an accept the server re-sends the giver's menu before QUEST_ACCEPTED (and the quest log
/// update) reaches the client, so the list already lacks the quest being accepted while the log
/// does not show it yet. Such a list cannot vouch for what was missing.
fn mark_in_flight_accepts(steps: &mut [Step]) {
    for index in 0..steps.len() {
        let Some(offer) = steps[index].offer() else { continue };
        let has_pending = matches!(&steps[index], Step::List(ListStep { pending: Some(_), .. }));
        if !offer.list_complete && !has_pending {
            continue;
        }
        let listed: FxHashSet<QuestId> = offer.available.iter().map(|quest| quest.id).collect();
        let tp = steps[index].tp();
        let in_flight = steps[index + 1..]
            .iter()
            .take_while(|next| next.tp() - tp <= PARAMS.in_flight_accept_seconds && !next.is_list_or_dialog())
            .any(|next| next.quest(QuestStepKind::Accepted).is_some_and(|accept| !listed.contains(&accept.quest_id)));
        if !in_flight {
            continue;
        }
        if let Some(offer) = steps[index].offer_mut() {
            offer.list_complete = false;
        }
        if let Step::List(ListStep { pending: Some(pending), .. }) = &mut steps[index] {
            pending.incomplete = true;
        }
    }
}

/// Picking a gossip option shows another gossip page, which lists no quests even though the giver
/// has some. An empty page right after a non-empty one from the same giver, with no quest event in
/// between, is such a sub-menu rather than an empty list.
fn mark_sub_menus(steps: &mut [Step]) {
    let has_quests =
        |list: &ListStep| !list.offer.available.is_empty() || list.offer.active.as_ref().is_some_and(|active| !active.is_empty()) || list.pending.is_some();
    let mut previous: Option<usize> = None;
    let mut quest_activity = false;
    for index in 0..steps.len() {
        let Step::List(list) = &steps[index] else {
            quest_activity = true;
            continue;
        };
        let parent = previous.and_then(|p| match &steps[p] {
            Step::List(parent) => Some(parent),
            _ => None,
        });
        let sub_menu = list.kind == ListKind::Gossip
            && !has_quests(list)
            && !quest_activity
            && parent.is_some_and(|parent| has_quests(parent) && same_giver(parent.giver, list.giver) && list.tp - parent.tp <= PARAMS.sub_menu_seconds);
        if sub_menu {
            if let Step::List(list) = &mut steps[index] {
                list.offer.list_complete = false;
            }
            // Further pages still belong to the same menu.
            continue;
        }
        previous = Some(index);
        quest_activity = false;
    }
}

/// QUEST_REMOVED: None when it is the quest's own turn-in (or completion), `Removed` when
/// something else may have removed it (replay quests; another quest accepted/turned in moments
/// before or after, e.g. the server dropping a breadcrumb), otherwise `Abandoned`.
fn classify_removal(steps: &[Step], removal: &QuestStep, completed_adds: &FxHashMap<QuestId, Vec<f64>>) -> Option<QuestEventKind> {
    let near = |tp: f64, window: f64| (tp - removal.tp).abs() <= window;
    let turn_in_window = PARAMS.removal_turn_in_seconds;
    if completed_adds
        .get(&removal.quest_id)
        .is_some_and(|times| times.iter().any(|&tp| near(tp, turn_in_window)))
    {
        return None;
    }
    let mut other_quest_activity = false;
    for step in steps {
        let Step::Quest(quest) = step else { continue };
        if quest.kind == QuestStepKind::Removed {
            continue;
        }
        if quest.kind == QuestStepKind::TurnedIn && quest.quest_id == removal.quest_id && near(quest.tp, turn_in_window) {
            return None;
        }
        if quest.quest_id != removal.quest_id && near(quest.tp, PARAMS.removal_other_quest_seconds) {
            other_quest_activity = true;
        }
    }
    Some(if removal.replay || other_quest_activity {
        QuestEventKind::Removed
    } else {
        QuestEventKind::Abandoned
    })
}

/// Session names are "YYYY-MM-DD_HH-MM-SS"; older builds also took free text from /qlt, which is
/// never copied.
pub fn is_session_name(name: &str) -> bool {
    const PATTERN: &[u8; 19] = b"dddd-dd-dd_dd-dd-dd";
    name.len() == PATTERN.len() && name.bytes().zip(PATTERN).all(|(c, &p)| if p == b'd' { c.is_ascii_digit() } else { c == p })
}

/// Distills one session; `key` is its [`episode_key`].
pub fn distill_session(session: &Session, key: String) -> DistilledSession {
    let mut stats = DistillStats::default();
    let reader = SessionReader::new(session);
    let locale = first_player_value(session, "GetLocale").and_then(Value::as_str);
    let player = read_player(session);
    let mut titles = TitleSink {
        has_locale: locale.is_some_and(|locale| !locale.is_empty()),
        seen: IndexSet::new(),
    };
    titles.add_quest_log(session);

    let mut steps: Vec<Step> = Vec::new();
    for (i, event) in reader.events.iter().enumerate() {
        // Steps become quest events and offers on the session's timeline: they need a time.
        let Some(tp) = finite_event_time(event) else { continue };
        let quest =
            |kind: QuestStepKind, quest_id: Option<QuestId>, replay: bool| quest_id.map(|quest_id| Step::Quest(QuestStep { kind, tp, quest_id, replay }));
        let step = match event.name() {
            Some("GOSSIP_SHOW") => Some(Step::List(gossip_step(&reader, i, &mut stats, &mut titles))),
            Some("QUEST_GREETING") => Some(Step::List(greeting_step(&reader, i, &mut stats, &mut titles))),
            Some("QUEST_DETAIL") => Some(Step::Dialog(DialogKind::Detail, dialog_step(&reader, i, DialogKind::Detail, &mut stats))),
            Some("QUEST_PROGRESS") => Some(Step::Dialog(DialogKind::Progress, dialog_step(&reader, i, DialogKind::Progress, &mut stats))),
            Some("QUEST_COMPLETE") => Some(Step::Dialog(DialogKind::Complete, dialog_step(&reader, i, DialogKind::Complete, &mut stats))),
            Some("QUEST_ACCEPTED") => quest(QuestStepKind::Accepted, accepted_quest_id(event), false),
            Some("QUEST_TURNED_IN") => quest(QuestStepKind::TurnedIn, as_positive_int(event_arg(event, 1)), false),
            Some("QUEST_REMOVED") => {
                let replay = event_arg(event, 2).and_then(Value::as_bool) == Some(true);
                quest(QuestStepKind::Removed, as_positive_int(event_arg(event, 1)), replay)
            }
            _ => None,
        };
        steps.extend(step);
    }
    borrow_quest_ids(&mut steps, &mut stats);

    let completed = read_completed(session, &mut stats);
    let mut completed_adds: FxHashMap<QuestId, Vec<f64>> = FxHashMap::default();
    for change in completed.iter().flat_map(|completed| &completed.changes) {
        for &id in change.add.iter().flatten() {
            completed_adds.entry(id).or_default().push(change.t);
        }
    }

    // Steps keep their offers until every completeness check has run.
    let mut quest_events = Vec::new();
    let mut event_giver: Vec<Option<GiverRef>> = vec![None; steps.len()];
    for index in 0..steps.len() {
        let Some((event, offer)) = step_event(&steps, index, &event_giver, &completed_adds) else {
            continue;
        };
        event_giver[index] = event.giver;
        quest_events.push(event);
        if let (Some(offer), Step::Dialog(_, dialog)) = (offer, &mut steps[index]) {
            dialog.offer = Some(offer);
        }
    }
    mark_in_flight_accepts(&mut steps);
    mark_sub_menus(&mut steps);

    let mut offers = Vec::new();
    let mut pending_greetings = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        match step {
            Step::List(list) => {
                if let Some(pending) = list.pending {
                    pending_greetings.push(PendingGreeting {
                        offer: offers.len(),
                        available: pending.available,
                        active: pending.active,
                        expected: pending.expected,
                        incomplete: pending.incomplete,
                    });
                }
                offers.push(list.offer);
            }
            Step::Dialog(_, dialog) => {
                if let Some(quest_id) = dialog.quest_id {
                    titles.add(dialog.title.as_deref(), Some(quest_id), event_giver[index]);
                }
                offers.extend(dialog.offer);
            }
            Step::Quest(_) => {}
        }
    }

    let last_event_tp = reader.events.iter().rev().find_map(finite_event_time).unwrap_or(0.0);
    let recorded = session
        .field("durationPrecise")
        .and_then(Value::as_num)
        .or_else(|| session.field("duration").and_then(Value::as_num));
    // A session recovered after a client restart gets its stop time from the new clock. The range
    // check also turns away a non-finite recording.
    let duration = recorded
        .filter(|&recorded| recorded >= last_event_tp && recorded <= last_event_tp + DURATION_SLACK_SECONDS)
        .unwrap_or(last_event_tp);
    let server_time = get_stream(session, "GetServerTime", &[]).and_then(<[Value]>::first);
    let start_server_time = server_time.and_then(|entry| Some(entry_v(entry)?.as_num()? - finite_entry_time(entry)?).filter(|t| t.is_finite()));
    let name = session.field("name").and_then(Value::as_str);

    let mut episode = EpisodeDraft {
        session_name: name.filter(|name| is_session_name(name)).map(str::to_owned),
        interface_version: interface_version(session),
        locale: locale.map(str::to_owned),
        player,
        duration,
        levels: read_levels(session),
        completed,
        quest_log: read_quest_log(session),
        quest_events,
        offers,
        quest_info: read_quest_info(session),
    };
    episode.shrink_to_fit();
    DistilledSession {
        key,
        event_count: reader.events.len(),
        saved: name.is_some(),
        start_server_time,
        episode,
        pending_greetings,
        titles: titles.seen.into_iter().collect(),
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::test_trace::{creature, decode_json, gossip_quest, TraceBuilder};
    use serde_json::{json, Value as Json};
    use QuestEventKind::*;

    fn npc(id: i64) -> Option<GiverRef> {
        Some(GiverRef { kind: GiverKind::Npc, id })
    }

    fn kinds(result: &DistilledSession) -> Vec<(QuestEventKind, QuestId)> {
        result.episode.quest_events.iter().map(|event| (event.kind, event.quest_id)).collect()
    }

    fn events(result: &DistilledSession) -> Vec<(QuestEventKind, QuestId, Option<GiverRef>)> {
        result
            .episode
            .quest_events
            .iter()
            .map(|event| (event.kind, event.quest_id, event.giver))
            .collect()
    }

    fn quest(id: QuestId) -> OfferedQuest {
        OfferedQuest {
            id,
            trivial: None,
            frequency: None,
            repeatable: None,
        }
    }

    /// A quest as `gossip_quest` lists it.
    fn listed(id: QuestId) -> OfferedQuest {
        OfferedQuest {
            trivial: Some(false),
            frequency: Some(0),
            repeatable: Some(false),
            ..quest(id)
        }
    }

    fn offer(source: OfferSource, giver: Option<GiverRef>, available: Vec<OfferedQuest>, active: Option<Vec<QuestId>>, complete: bool) -> OfferSnapshot {
        OfferSnapshot {
            t: 0.0,
            source,
            giver,
            available,
            unresolved_titles: None,
            active,
            list_complete: complete,
        }
    }

    /// The offers with their times zeroed: tests compare what was offered, not when.
    fn offers(result: &DistilledSession) -> Vec<OfferSnapshot> {
        result.episode.offers.iter().map(|offer| OfferSnapshot { t: 0.0, ..offer.clone() }).collect()
    }

    fn closed_gossip(builder: TraceBuilder, t: f64) -> TraceBuilder {
        builder
            .frame(t)
            .event("GOSSIP_CLOSED", &[json!(false)])
            .sample_param("UnitGUID", "npc", Json::Null)
    }

    // --- offers ---

    #[test]
    fn turns_each_gossip_show_into_a_complete_snapshot_of_the_givers_lists() {
        let builder = TraceBuilder::new()
            .frame(10.0)
            .gossip(500, &[gossip_quest(1), gossip_quest(2)], &[gossip_quest(3)]);
        let result = closed_gossip(builder, 12.0).build();
        let expected = offer(OfferSource::Gossip, npc(500), vec![listed(1), listed(2)], Some(vec![3]), true);
        assert_eq!(offers(&result), [expected]);
    }

    #[test]
    fn does_not_trust_an_empty_gossip_read_from_a_window_an_addon_closed_in_the_same_frame() {
        let result = TraceBuilder::new()
            .frame(10.0)
            .gossip(500, &[], &[])
            .event("GOSSIP_CLOSED", &[json!(false)])
            .build();
        assert!(result.episode.offers[0].available.is_empty());
        assert!(!result.episode.offers[0].list_complete);
    }

    #[test]
    fn does_not_reuse_another_givers_non_empty_list_for_a_gossip_show_that_recorded_no_read_of_its_own() {
        // Lists are change-only: identical lists and failed reads both leave no sample.
        let builder = closed_gossip(TraceBuilder::new().frame(10.0).gossip(500, &[gossip_quest(1)], &[]), 11.0);
        let builder = builder
            .frame(20.0)
            .event("GOSSIP_SHOW", &[Json::Null])
            .sample_param("UnitGUID", "npc", json!(creature(501)));
        let builder = closed_gossip(builder, 21.0);
        let builder = builder
            .frame(30.0)
            .event("GOSSIP_SHOW", &[Json::Null])
            .sample_param("UnitGUID", "npc", json!(creature(500)));
        let result = builder.build();
        let summary: Vec<(i64, usize, bool)> = result
            .episode
            .offers
            .iter()
            .map(|offer| (offer.giver.map_or(0, |giver| giver.id), offer.available.len(), offer.list_complete))
            .collect();
        // The same giver again: an unchanged re-read is the likely story.
        assert_eq!(summary, [(500, 1, true), (501, 0, false), (500, 1, true)]);
        assert_eq!(result.stats.gossip_inherited_from_other_giver, 1);
    }

    #[test]
    fn does_not_trust_a_gossip_list_whose_count_api_disagrees() {
        let builder = TraceBuilder::new().frame(10.0).gossip(500, &[gossip_quest(1)], &[]);
        let result = builder.sample("C_GossipInfo.GetNumAvailableQuests", json!(2)).build();
        assert!(!result.episode.offers[0].list_complete);
    }

    #[test]
    fn reads_greeting_quest_ids_per_index_falling_back_to_the_title_where_the_client_gives_no_id() {
        let result = TraceBuilder::new()
            .frame(10.0)
            .event("QUEST_GREETING", &[])
            .sample_param("UnitGUID", "npc", json!(creature(240)))
            .sample("GetNumAvailableQuests", json!(2))
            .sample("GetNumActiveQuests", json!(1))
            .sample_param(
                "GetAvailableQuestInfo",
                "1",
                json!({ "1": false, "2": 0, "3": false, "4": false, "5": 147, "6": false, "n": 6 }),
            )
            .sample_param("GetAvailableTitle", "1", json!("Manhunt"))
            // Clients without the quest id return only the first four values.
            .sample_param("GetAvailableQuestInfo", "2", json!({ "1": false, "2": 0, "3": false, "4": false, "n": 4 }))
            .sample_param("GetAvailableTitle", "2", json!("Westbrook Garrison Needs Help!"))
            .sample_param("GetActiveQuestID", "1", json!(123))
            .sample_param("GetActiveTitle", "1", json!({ "1": "The Collector", "2": true, "n": 2 }))
            .build();
        // Completeness is decided once the remaining title is resolved.
        let expected = offer(OfferSource::Greeting, npc(240), vec![listed(147)], Some(vec![123]), false);
        assert_eq!(offers(&result), [expected]);
        let pending = PendingGreeting {
            offer: 0,
            available: vec!["Westbrook Garrison Needs Help!".into()],
            active: vec![],
            expected: 2.0,
            incomplete: false,
        };
        assert_eq!(result.pending_greetings, [pending]);
        let manhunt = TitleObservation {
            title: "Manhunt".into(),
            quest_id: 147,
            giver: npc(240),
        };
        assert!(result.titles.contains(&manhunt), "{:?}", result.titles);
    }

    #[test]
    fn reads_a_greeting_with_a_damaged_huge_count_without_walking_every_index_up_to_it() {
        // Walking 1..=count would take hours (and overflow a u32 index); only recorded indices matter.
        let result = TraceBuilder::new()
            .frame(10.0)
            .event("QUEST_GREETING", &[])
            .sample_param("UnitGUID", "npc", json!(creature(240)))
            .sample("GetNumAvailableQuests", json!(1e15))
            .sample("GetNumActiveQuests", json!(1e15))
            .sample_param(
                "GetAvailableQuestInfo",
                "1",
                json!({ "1": false, "2": 0, "3": false, "4": false, "5": 147, "n": 5 }),
            )
            .sample_param("GetActiveQuestID", "2", json!(123))
            .build();
        let expected = offer(OfferSource::Greeting, npc(240), vec![listed(147)], Some(vec![123]), false);
        assert_eq!(offers(&result), [expected]);
    }

    #[test]
    fn treats_a_detail_opened_straight_from_an_npc_as_that_npcs_complete_one_quest_list() {
        let builder = TraceBuilder::new().frame(10.0).dialog("QUEST_DETAIL", 7, Some(500), &[json!(0)]);
        let result = builder.frame(11.0).close().event("QUEST_ACCEPTED", &[json!(7)]).build();
        assert_eq!(offers(&result), [offer(OfferSource::Detail, npc(500), vec![quest(7)], None, true)]);
    }

    #[test]
    fn only_calls_a_detail_standalone_when_nothing_else_explains_why_it_opened() {
        let standalone = |setup: fn(TraceBuilder) -> TraceBuilder| {
            let result = setup(TraceBuilder::new())
                .frame(200.0)
                .dialog("QUEST_DETAIL", 7, Some(500), &[json!(0)])
                .build();
            result.episode.offers.last().expect("a detail offer").list_complete
        };
        assert!(standalone(|b| b));
        // The same giver showed a list (without quest 7) recently: a gossip option opened the detail.
        assert!(!standalone(|b| b.frame(150.0).gossip(500, &[gossip_quest(8)], &[])));
        // ...but not when that list is older than the window, or another giver was visited since.
        assert!(standalone(|b| b.frame(79.0).gossip(500, &[gossip_quest(8)], &[])));
        assert!(standalone(|b| b.frame(150.0).gossip(500, &[gossip_quest(8)], &[]).frame(160.0).gossip(
            501,
            &[gossip_quest(9)],
            &[]
        )));
        // Another dialog with this giver was open moments before.
        assert!(!standalone(|b| b.frame(199.0).dialog("QUEST_PROGRESS", 6, Some(500), &[])));
        // Pushed by the server right after an accept (the giver's next quest), not opened by a click.
        assert!(!standalone(|b| {
            b.frame(150.0)
                .dialog("QUEST_DETAIL", 6, Some(500), &[json!(0)])
                .frame(199.0)
                .close()
                .event("QUEST_ACCEPTED", &[json!(6)])
        }));
    }

    #[test]
    fn does_not_trust_a_list_shown_while_the_accept_of_a_quest_it_lacks_was_still_in_flight() {
        // The server re-sends the menu before QUEST_ACCEPTED and the quest log update arrive.
        let builder = TraceBuilder::new().frame(10.0).gossip(500, &[gossip_quest(871)], &[]);
        let result = builder.frame(10.1).event("QUEST_ACCEPTED", &[json!(6541)]).build();
        assert!(!result.episode.offers[0].list_complete);
    }

    #[test]
    fn does_not_trust_an_empty_gossip_page_that_follows_a_non_empty_one_from_the_same_giver() {
        let result = TraceBuilder::new()
            .frame(10.0)
            .gossip(500, &[gossip_quest(1)], &[])
            .frame(14.0)
            .gossip(500, &[], &[])
            .build();
        let complete: Vec<bool> = result.episode.offers.iter().map(|offer| offer.list_complete).collect();
        assert_eq!(complete, [true, false], "the empty page is a sub-menu");
    }

    #[test]
    fn adds_no_detail_offer_for_a_quest_picked_from_a_gossip_list_and_the_detail_inherits_the_lists_giver() {
        let builder = closed_gossip(TraceBuilder::new().frame(10.0).gossip(500, &[gossip_quest(7), gossip_quest(8)], &[]), 12.0);
        // An auto-accept addon closed the detail before questnpc/npc were sampled.
        let result = builder.event("QUEST_DETAIL", &[json!(0)]).sample("GetQuestID", json!(7)).build();
        let sources: Vec<OfferSource> = result.episode.offers.iter().map(|offer| offer.source).collect();
        assert_eq!(sources, [OfferSource::Gossip]);
        assert_eq!(events(&result), [(Detail, 7, npc(500))]);
    }

    #[test]
    fn never_guesses_a_giver_for_an_item_started_detail() {
        // Forever: questStartItemID stays 0 and questnpc holds an Item GUID without the item id,
        // while the NPC targeted moments before must not be taken as the giver.
        let result = TraceBuilder::new()
            .frame(9.0)
            .event("PLAYER_TARGET_CHANGED", &[])
            .sample_param("UnitGUID", "target", json!(creature(600)))
            .frame(10.0)
            .event("QUEST_DETAIL", &[json!(0)])
            .sample("GetQuestID", json!(7))
            .sample_param("UnitGUID", "questnpc", json!("Item-4619-0-400000003542D74B"))
            .build();
        assert_eq!(events(&result), [(Detail, 7, None)]);
        assert_eq!(offers(&result), [offer(OfferSource::Detail, None, vec![quest(7)], None, false)]);
        assert_eq!(result.stats.item_started_details, 1);
    }

    #[test]
    fn treats_a_detail_right_after_a_turn_in_as_the_finishers_follow_up_offer_not_a_complete_list() {
        let result = TraceBuilder::new()
            .frame(10.0)
            .dialog("QUEST_COMPLETE", 6, Some(500), &[])
            .frame(11.0)
            .event("QUEST_TURNED_IN", &[json!(6), json!(100), json!(0)])
            .frame(11.2)
            .close()
            // The follow-up detail opens in the same frame, before questnpc/npc are readable again.
            .event("QUEST_DETAIL", &[json!(0)])
            .sample("GetQuestID", json!(7))
            .build();
        assert_eq!(events(&result), [(Complete, 6, npc(500)), (TurnedIn, 6, npc(500)), (Detail, 7, npc(500))]);
        assert_eq!(offers(&result), [offer(OfferSource::Detail, npc(500), vec![quest(7)], None, false)]);
    }

    // --- quest ids and givers ---

    #[test]
    fn reads_the_details_quest_id_from_earlier_in_the_same_frame_when_an_addon_already_closed_it() {
        // Real ordering seen in traces: the client is already in the new dialog while it dispatches
        // the old dialog's QUEST_FINISHED events, and QUEST_DETAIL arrives after it was auto-accepted.
        let result = TraceBuilder::new()
            .frame(10.0)
            .event("QUEST_FINISHED", &[])
            .quest_dialog_read(52, "Border Patrol")
            .event("QUEST_FINISHED", &[])
            .quest_dialog_read(0, "")
            .event("QUEST_DETAIL", &[json!(0)])
            .frame(10.4)
            .event("QUEST_ACCEPTED", &[json!(52)])
            .build();
        assert_eq!(kinds(&result), [(Detail, 52), (Accepted, 52)]);
        let border_patrol = TitleObservation {
            title: "Border Patrol".into(),
            quest_id: 52,
            giver: None,
        };
        assert!(result.titles.contains(&border_patrol), "{:?}", result.titles);
    }

    #[test]
    fn never_takes_the_targeted_unit_as_the_giver_when_the_dialogs_own_npc_was_not_read() {
        // A party member's shared quest opens the same kind of detail while some mob is targeted.
        let result = TraceBuilder::new()
            .frame(10.0)
            .event("PLAYER_TARGET_CHANGED", &[])
            .sample_param("UnitGUID", "target", json!(creature(600)))
            .frame(11.0)
            .event("QUEST_DETAIL", &[json!(0)])
            .sample("GetQuestID", json!(7))
            .frame(11.5)
            .event("QUEST_ACCEPTED", &[json!(7)])
            .build();
        assert_eq!(events(&result), [(Detail, 7, None), (Accepted, 7, None)]);
        assert_eq!(result.episode.offers[0].giver, None);
    }

    #[test]
    fn borrows_an_unread_details_quest_id_only_from_the_accept_that_answers_its_close() {
        let detail = |accept_at: f64| {
            let builder = TraceBuilder::new()
                .frame(10.0)
                .event("QUEST_DETAIL", &[json!(0)])
                .sample("GetQuestID", json!(0));
            let builder = builder.frame(10.5).event("QUEST_FINISHED", &[]);
            builder.frame(accept_at).event("QUEST_ACCEPTED", &[json!(7)]).build()
        };
        let answered = detail(10.8);
        assert_eq!(kinds(&answered), [(Detail, 7), (Accepted, 7)]);
        assert_eq!(answered.stats.borrowed_quest_ids, 1);
        // Declined, then something else was accepted a few seconds later.
        let declined = detail(13.6);
        assert_eq!(kinds(&declined), [(Accepted, 7)]);
        assert_eq!(declined.stats.detail_unresolved, 1);
    }

    #[test]
    fn carries_givers_from_the_detail_to_quest_accepted_and_from_the_turn_in_dialog_to_quest_turned_in() {
        let result = TraceBuilder::new()
            .frame(10.0)
            .dialog("QUEST_DETAIL", 7, Some(500), &[json!(0)])
            .frame(11.0)
            .close()
            .event("QUEST_ACCEPTED", &[json!(3), json!(7)]) // Era: (logIndex, questID)
            .frame(50.0)
            .dialog("QUEST_PROGRESS", 7, Some(501), &[])
            .frame(51.0)
            .dialog("QUEST_COMPLETE", 7, Some(501), &[])
            .frame(52.0)
            .event("QUEST_TURNED_IN", &[json!(7), json!(100), json!(0)])
            .build();
        assert_eq!(
            events(&result),
            [
                (Detail, 7, npc(500)),
                (Accepted, 7, npc(500)),
                (Progress, 7, npc(501)),
                (Complete, 7, npc(501)),
                (TurnedIn, 7, npc(501))
            ]
        );
    }

    // --- privacy ---

    #[test]
    fn never_makes_a_player_or_a_pet_the_giver_from_any_unit_token() {
        // Older traces may still hold player GUIDs; the addon only filters them on newer builds.
        let player = json!("Player-1234-0053656F");
        let result = TraceBuilder::new()
            .frame(10.0)
            .event("PLAYER_TARGET_CHANGED", &[])
            .sample_param("UnitGUID", "target", player.clone())
            .frame(11.0)
            .event("GOSSIP_SHOW", &[Json::Null])
            .sample_param("UnitGUID", "npc", json!("Pet-0-4615-0-138441-165189-02002A657B"))
            .sample("C_GossipInfo.GetAvailableQuests", json!([gossip_quest(7)]))
            .frame(12.0)
            .event("QUEST_DETAIL", &[json!(0)])
            .sample("GetQuestID", json!(7))
            .sample_param("UnitGUID", "questnpc", player.clone())
            .sample_param("UnitGUID", "npc", player)
            .build();
        // The gossip list (at the pet) and the detail picked from it are still read, giverless.
        let sources: Vec<OfferSource> = result.episode.offers.iter().map(|offer| offer.source).collect();
        assert_eq!(sources, [OfferSource::Gossip]);
        assert_eq!(result.episode.offers[0].giver, None);
        assert_eq!(events(&result), [(Detail, 7, None)]);
        assert!(!result.titles.is_empty() && result.titles.iter().all(|title| title.giver.is_none()));
        let written = serde_json::to_string(&(&result.episode.offers, &result.episode.quest_events)).unwrap();
        for leaked in ["Player-", "Pet-", "1234", "165189"] {
            assert!(!written.contains(leaked), "{leaked} in {written}");
        }
    }

    #[test]
    fn keeps_only_timestamp_session_names_since_older_builds_accepted_free_text() {
        let named = |name: &str| {
            let mut builder = TraceBuilder::new();
            builder.session["name"] = json!(name);
            let result = builder.build();
            assert!(result.saved, "any name marks a saved session");
            result.episode.session_name
        };
        assert_eq!(named("2026-10-01_12-00-00").as_deref(), Some("2026-10-01_12-00-00"));
        assert_eq!(named("raid with Someone"), None);
    }

    // --- client quest flags, completed timeline, removals ---

    #[test]
    fn keeps_the_latest_flag_values_per_quest() {
        let result = TraceBuilder::new()
            .frame(10.0)
            .sample_param("IsBreadcrumbQuest", "92579", json!(false))
            .frame(20.0)
            .sample_param("IsBreadcrumbQuest", "92579", json!(true))
            .sample_param("C_QuestLine.GetQuestLineInfo", "92579", json!({ "questLineID": 5678, "questID": 92579 }))
            .sample_param("C_QuestInfoSystem.GetQuestClassification", "92580", json!(6))
            .build();
        let info = |is_breadcrumb, quest_line_id, classification| QuestClientInfo {
            is_breadcrumb,
            quest_line_id,
            classification,
        };
        let expected = BTreeMap::from([(92579, info(Some(true), Some(5678), None)), (92580, info(None, None, Some(6)))]);
        assert_eq!(result.episode.quest_info, Some(expected));
    }

    #[test]
    fn collapses_an_empty_completed_quest_read_but_keeps_real_changes() {
        let history: Vec<QuestId> = (1..=30).collect();
        let removed: Vec<QuestId> = (1..=31).collect();
        let re_added: Vec<QuestId> = (1..=29).chain([31, 99]).collect();
        let mut builder = TraceBuilder::new();
        builder.session["functionsDelta"]["C_QuestLog.GetAllCompletedQuestIDs"] = json!({
            "t": 0, "tp": 0, "initial": history,
            "delta": [
                { "t": 5, "tp": 5, "add": [31] },
                // Loading screen: everything but quest 30 comes back, plus a quest really completed meanwhile.
                { "t": 100, "tp": 100, "remove": removed },
                { "t": 103, "tp": 103, "add": re_added },
            ],
        });
        let result = builder.build();
        let change = |t: f64, add: Option<Vec<QuestId>>, remove: Option<Vec<QuestId>>| CompletedChange { t, add, remove };
        let completed = result.episode.completed.expect("a completed timeline");
        assert_eq!((completed.source, completed.initial), (CompletedSource::Modern, history));
        assert_eq!(
            completed.changes,
            [
                change(5.0, Some(vec![31]), None),
                change(100.0, None, Some(vec![30])),
                change(103.0, Some(vec![99]), None)
            ]
        );
        assert_eq!(result.stats.completed_read_gaps, 1);
    }

    #[test]
    fn drops_the_removal_that_belongs_to_a_turn_in_or_a_completion() {
        let turned_in = TraceBuilder::new().frame(10.0).event("QUEST_TURNED_IN", &[json!(7), json!(0), json!(0)]);
        let turned_in = turned_in.frame(10.2).event("QUEST_REMOVED", &[json!(7), json!(false)]).build();
        assert_eq!(kinds(&turned_in), [(TurnedIn, 7)]);
        let mut completed_without_turn_in = TraceBuilder::new().frame(10.0).event("QUEST_REMOVED", &[json!(7), json!(false)]);
        completed_without_turn_in.session["functionsDelta"]["C_QuestLog.GetAllCompletedQuestIDs"] =
            json!({ "t": 0, "tp": 0, "initial": [], "delta": [{ "t": 11, "tp": 11, "add": [7] }] });
        assert_eq!(kinds(&completed_without_turn_in.build()), []);
    }

    #[test]
    fn calls_a_lone_removal_an_abandon_and_one_next_to_another_quests_accept_undecidable() {
        let lone = TraceBuilder::new().frame(10.0).event("QUEST_REMOVED", &[json!(7), json!(false)]).build();
        assert_eq!(kinds(&lone), [(Abandoned, 7)]);
        let next_to_accept = TraceBuilder::new().frame(10.0).event("QUEST_ACCEPTED", &[json!(8)]);
        let next_to_accept = next_to_accept.frame(11.0).event("QUEST_REMOVED", &[json!(7), json!(false)]).build();
        assert_eq!(kinds(&next_to_accept), [(Accepted, 8), (Removed, 7)]);
    }

    // --- times ---

    #[test]
    fn leaves_out_what_has_no_time_so_a_written_episode_reads_back_as_it_was() {
        // A damaged trace: an accept, a level sample, a quest log entry and a completed change
        // without `t` or `tp`, after a timed accept and level sample.
        let mut builder = TraceBuilder::new()
            .frame(10.0)
            .event("QUEST_ACCEPTED", &[json!(7)])
            .sample_param("UnitLevel", "player", json!(5));
        let timeless = |value: Json| json!({ "v": value });
        builder.session["events"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "e": "QUEST_ACCEPTED", "a": { "1": 8, "n": 1 } }));
        builder.session["functions"]["UnitLevel"]["player"]
            .as_array_mut()
            .unwrap()
            .push(timeless(json!(6)));
        builder.session["functions"]["QuestLog"] = json!([timeless(json!([8]))]);
        builder.session["functionsDelta"]["C_QuestLog.GetAllCompletedQuestIDs"] = json!({ "initial": [], "delta": [{ "add": [8] }] });
        let result = builder.build();

        assert_eq!(kinds(&result), [(Accepted, 7)]);
        assert_eq!(result.episode.levels.iter().map(|level| level.v).collect::<Vec<_>>(), [5]);
        assert!(result.episode.quest_log.is_empty());
        assert_eq!(result.episode.completed.as_ref().map(|completed| completed.changes.len()), Some(0));
        // The session lasts until its last event that has a time.
        assert_eq!(result.episode.duration, result.episode.quest_events[0].t);

        // JSON has no NaN: a time that is not finite would be written as null and fail the read.
        let identity = EpisodeIdentity {
            key: result.key.clone(),
            contributor_id: "contributor".into(),
            character_key: "character".into(),
            submission_ids: vec!["submission".into()],
            session_order: 0,
        };
        let episode = result.episode.into_episode(identity);
        let path = std::env::temp_dir().join(format!("relations-timeless-{}.jsonl", std::process::id()));
        crate::io::write_episodes(&path, std::slice::from_ref(&episode)).unwrap();
        let read = crate::io::read_episodes(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read.unwrap(), [episode]);
    }

    // --- identity ---

    #[test]
    fn keys_every_copy_of_a_session_by_its_capture_start_clocks_and_one_without_them_by_its_content() {
        let copy = TraceBuilder::new().frame(10.0).event("QUEST_ACCEPTED", &[json!(7)]).session;
        let grown = TraceBuilder::new()
            .frame(10.0)
            .event("QUEST_ACCEPTED", &[json!(7)])
            .frame(20.0)
            .event("QUEST_TURNED_IN", &[json!(7)]);
        let grown = grown.session;
        let without_clocks = |session: &Json| {
            let mut session = session.clone();
            let record = session.as_object_mut().expect("a record");
            record.remove("startedAt");
            record.remove("startedAtPrecise");
            session
        };
        let key = |record: &Json| decode_json(record, episode_key);
        assert_eq!(key(&grown), key(&copy));
        assert_eq!(key(&without_clocks(&copy)), key(&without_clocks(&copy.clone())));
        assert_ne!(key(&without_clocks(&grown)), key(&without_clocks(&copy)));
    }

    #[test]
    fn the_clock_key_is_the_ts_key_so_episode_keys_survive_the_port() {
        // node: createHash("sha256").update(`${startedAt}|${startedAtPrecise}`).digest("hex").slice(0, 16)
        let key = |started_at: f64, precise: f64| decode_json(&json!({ "startedAt": started_at, "startedAtPrecise": precise }), episode_key);
        assert_eq!(key(1000.0, 2000.5), "998582ba005aef3d");
        // JS prints -0 as "0", and a float in its shortest round-trip form.
        assert_eq!(key(-0.0, 0.1 + 0.2), "bf078009c4ccc11e");
        assert_eq!(key(1e21, 123456.789), "0c13a485128d09db");
    }
}
