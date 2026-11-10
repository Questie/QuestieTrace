//! Which observers run on a session, and the observations they stage for the fold. Ports
//! `extract/observe.ts` and `extract/observe-lookups.ts`.

use crate::aggregate::{Num, SessionScalars};
use crate::js::format_number;
use crate::observation::{Localized, Observation};
use crate::observers::item::{npc_drops::NpcDrop, object_drops::ObjectDrop};
use crate::observers::npc::friendly_to_faction::FactionReactionValue;
use crate::observers::quest::finished_by::FinishedByFinisher;
use crate::observers::quest::objectives::ObjectiveSlot;
use crate::observers::quest::objectives_text::QuestObjectivesTextValue;
use crate::observers::quest::quest_xp::QuestXpValue;
use crate::observers::quest::reputation_reward::ReputationTurnInValue;
use crate::observers::quest::started_by::StartedByStarter;
use crate::observers::quest::trigger_end::QuestTriggerEndValue;
use crate::observers::quest::turn_in_poi::TurnInPoiValue;
use crate::observers::spawn::SpawnObservation;
use crate::observers::{item, npc, object, quest};
use crate::probes::{session_server_time, Faction};
use std::hash::Hash;
use trace_data::Session;

/// The highest ids Classic knows per entity. Only ids above them are exported, so npc, item
/// and object observations at or below them are dropped right away.
#[derive(Clone, Copy, Debug)]
pub struct MaxIds {
    pub npc: i64,
    pub quest: i64,
    pub item: i64,
    pub object: i64,
}

pub const MAX_IDS: MaxIds = MaxIds {
    npc: 18199,
    quest: 9665,
    item: 25818,
    object: 300_142,
};

/// GetBuildInfo's interface version, as PlayerIdentity.lua records it:
/// `functions.GetBuildInfo.player[0].v`.
fn interface_version(session: &Session) -> Option<f64> {
    let build_info = session.function("GetBuildInfo").filter(|v| v.as_object().is_some())?;
    let first = build_info.get("player")?.as_array()?.first()?;
    first.is_table().then(|| first.get("v")?.as_num()).flatten()
}

/// Sessions from WoW Forever builds (interface 16xxx) are observed. Sessions without
/// GetBuildInfo are included: they may be early Forever traces.
pub fn is_forever_session(session: &Session) -> bool {
    interface_version(session).is_none_or(|version| format_number(version).starts_with("16"))
}

/// One session's observations, ready for the fold. Scalar fields are merged by the default
/// policy (only a per-value summary is kept, so they are pre-merged here, on the observing
/// thread); the others keep every observation.
#[derive(Default)]
pub struct StagedSession {
    pub server_time: Option<f64>,

    // Scalar fields.
    pub zone_or_sort: SessionScalars<Num>,
    pub quest_name: SessionScalars<String>,
    pub quest_level: SessionScalars<Num>,
    pub objectives_text: SessionScalars<QuestObjectivesTextValue>,
    pub objective_count: SessionScalars<Num>,
    pub npc_name: SessionScalars<String>,
    pub item_name: SessionScalars<String>,
    pub object_name: SessionScalars<String>,

    // Fields with a custom merge.
    pub required_level: Vec<Observation<f64>>,
    pub objectives: Vec<Observation<Vec<ObjectiveSlot>>>,
    pub trigger_end: Vec<Observation<QuestTriggerEndValue>>,
    pub started_by: Vec<Observation<StartedByStarter>>,
    pub finished_by: Vec<Observation<FinishedByFinisher>>,
    pub reputation_reward: Vec<Observation<ReputationTurnInValue>>,
    pub quest_xp: Vec<Observation<QuestXpValue>>,
    pub quest_factions: Vec<Observation<Faction>>,
    pub turn_in_poi: Vec<Observation<TurnInPoiValue>>,
    pub npc_spawns: Vec<Observation<SpawnObservation>>,
    pub npc_level: Vec<Observation<f64>>,
    pub npc_friendly_to_faction: Vec<Observation<FactionReactionValue>>,
    pub npc_rank: Vec<Observation<i64>>,
    pub npc_drops: Vec<Observation<NpcDrop>>,
    pub object_drops: Vec<Observation<ObjectDrop>>,
    pub object_spawns: Vec<Observation<SpawnObservation>>,

    // Translations: every locale but enUS, whose names already go into the corrections.
    pub lookups: StagedLookups,
}

/// Translation observations of one session (`observe-lookups.ts`), enUS left out.
#[derive(Default)]
pub struct StagedLookups {
    pub npc_name: SessionScalars<Localized<String>>,
    pub quest_name: SessionScalars<Localized<String>>,
    pub objectives_text: SessionScalars<Localized<QuestObjectivesTextValue>>,
    pub item_name: SessionScalars<Localized<String>>,
    pub object_name: SessionScalars<Localized<String>>,
}

/// Lookups are translations: enUS names already go into forever*Traces.lua.
pub const SKIPPED_LOCALE: &str = "enUS";

/// Entities at or below `max_id` are never exported: their observations are dropped right away.
fn above<T>(observations: Vec<Observation<T>>, max_id: i64) -> Vec<Observation<T>> {
    observations.into_iter().filter(|obs| obs.entity_id > max_id).collect()
}

fn translations<T: Clone + Eq + Hash>(observations: Vec<Observation<Localized<T>>>) -> SessionScalars<Localized<T>> {
    SessionScalars::new(observations.into_iter().filter(|obs| obs.value.locale != SKIPPED_LOCALE).collect())
}

/// Runs every observer over one session; None for sessions that are not observed (not Forever).
/// npcFriendlyToFaction is kept for every NPC: Classic NPCs start Forever quests, and their
/// reactions decide those quests' requiredRaces.
pub fn observe_session(session: &Session) -> Option<StagedSession> {
    let max_ids = MAX_IDS;
    if !is_forever_session(session) {
        return None;
    }
    Some(StagedSession {
        server_time: session_server_time(session),
        zone_or_sort: SessionScalars::new(quest::zone_or_sort::observe(session)),
        quest_name: SessionScalars::new(quest::name::observe(session)),
        quest_level: SessionScalars::new(quest::quest_level::observe(session)),
        objectives_text: SessionScalars::new(quest::objectives_text::observe(session)),
        objective_count: SessionScalars::new(quest::objective_count::observe(session)),
        npc_name: SessionScalars::new(npc::name::observe(session)),
        item_name: SessionScalars::new(item::name::observe(session)),
        object_name: SessionScalars::new(object::name::observe(session)),
        required_level: quest::required_level::observe(session),
        objectives: quest::objectives::per_session(quest::objectives::observe(session)),
        trigger_end: quest::trigger_end::observe(session),
        started_by: quest::started_by::observe(session),
        finished_by: quest::finished_by::observe(session),
        reputation_reward: quest::reputation_reward::observe(session),
        quest_xp: above(quest::quest_xp::observe(session), max_ids.quest),
        quest_factions: quest::required_races::observe_quest_factions(session),
        turn_in_poi: quest::turn_in_poi::observe(session),
        npc_spawns: above(npc::spawns::observe(session), max_ids.npc),
        npc_level: above(npc::level::observe(session), max_ids.npc),
        npc_friendly_to_faction: npc::friendly_to_faction::observe(session),
        npc_rank: above(npc::rank::observe(session), max_ids.npc),
        npc_drops: above(item::npc_drops::observe(session), max_ids.item),
        object_drops: above(item::object_drops::observe(session), max_ids.item),
        object_spawns: above(object::spawns::observe(session), max_ids.object),
        lookups: StagedLookups {
            npc_name: translations(npc::name::observe_localized(session)),
            quest_name: translations(quest::name::observe_localized(session)),
            objectives_text: translations(quest::objectives_text::observe_localized(session)),
            item_name: translations(item::name::observe_localized(session)),
            object_name: translations(object::name::observe_localized(session)),
        },
    })
}
