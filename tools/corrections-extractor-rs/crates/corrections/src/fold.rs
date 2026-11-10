//! What the fold collects from observed sessions, in fold order: per-value summaries of the
//! scalar and translation fields, and every observation of the fields with a custom merge.

use crate::aggregate::{Num, ObservationsByEntity, ScalarSummary, SessionScalars};
use crate::observation::{Localized, Observation, SessionId, Sessions};
use crate::observe::{StagedLookups, StagedSession};
use crate::observers::item::{npc_drops::NpcDrop, object_drops::ObjectDrop};
use crate::observers::npc::friendly_to_faction::FactionReactionValue;
use crate::observers::quest::finished_by::FinishedByFinisher;
use crate::observers::quest::objectives::ObjectiveMatch;
use crate::observers::quest::objectives_text::QuestObjectivesTextValue;
use crate::observers::quest::quest_xp::QuestXpValue;
use crate::observers::quest::reputation_reward::ReputationTurnInValue;
use crate::observers::quest::started_by::StartedByStarter;
use crate::observers::quest::trigger_end::QuestTriggerEndValue;
use crate::observers::quest::turn_in_poi::TurnInPoiValue;
use crate::observers::spawn::SpawnObservation;
use crate::probes::Faction;
use indexmap::IndexMap;
use std::hash::Hash;

#[derive(Default)]
pub struct Accumulator {
    pub sessions: Sessions,

    pub zone_or_sort: ScalarSummary<Num>,
    pub quest_name: ScalarSummary<String>,
    pub quest_level: ScalarSummary<Num>,
    pub objectives_text: ScalarSummary<QuestObjectivesTextValue>,
    pub objective_count: ScalarSummary<Num>,
    pub npc_name: ScalarSummary<String>,
    pub item_name: ScalarSummary<String>,
    pub object_name: ScalarSummary<String>,

    pub required_level: ObservationsByEntity<f64>,
    pub objectives: ObservationsByEntity<Vec<ObjectiveMatch>>,
    pub trigger_end: ObservationsByEntity<QuestTriggerEndValue>,
    pub started_by: ObservationsByEntity<StartedByStarter>,
    pub finished_by: ObservationsByEntity<FinishedByFinisher>,
    pub reputation_reward: ObservationsByEntity<ReputationTurnInValue>,
    pub quest_xp: ObservationsByEntity<QuestXpValue>,
    pub quest_factions: ObservationsByEntity<Faction>,
    /// In fold order across quests: turn-in spawns join their finisher's spawns in this order.
    pub turn_in_poi: Vec<Observation<TurnInPoiValue>>,
    pub npc_spawns: ObservationsByEntity<SpawnObservation>,
    pub npc_level: ObservationsByEntity<f64>,
    pub npc_friendly_to_faction: ObservationsByEntity<FactionReactionValue>,
    pub npc_rank: ObservationsByEntity<i64>,
    pub npc_drops: ObservationsByEntity<NpcDrop>,
    pub object_drops: ObservationsByEntity<ObjectDrop>,
    pub object_spawns: ObservationsByEntity<SpawnObservation>,

    pub lookups: LookupAccumulator,
}

/// Translations: per field, a scalar summary per locale.
#[derive(Default)]
pub struct LookupAccumulator {
    pub npc_name: IndexMap<String, ScalarSummary<String>>,
    pub quest_name: IndexMap<String, ScalarSummary<String>>,
    pub objectives_text: IndexMap<String, ScalarSummary<QuestObjectivesTextValue>>,
    pub item_name: IndexMap<String, ScalarSummary<String>>,
    pub object_name: IndexMap<String, ScalarSummary<String>>,
}

fn assign<T>(observations: Vec<Observation<T>>, session: SessionId) -> impl Iterator<Item = Observation<T>> {
    observations.into_iter().map(move |mut obs| {
        obs.session = session;
        obs
    })
}

fn add_localized<T: Clone + Eq + Hash>(
    by_locale: &mut IndexMap<String, ScalarSummary<T>>,
    scalars: SessionScalars<Localized<T>>,
    session: SessionId,
    sessions: &Sessions,
) {
    for (locale, scalars) in scalars.by_locale() {
        by_locale.entry(locale).or_default().add_session(scalars, session, sessions);
    }
}

impl LookupAccumulator {
    fn add(&mut self, staged: StagedLookups, session: SessionId, sessions: &Sessions) {
        add_localized(&mut self.npc_name, staged.npc_name, session, sessions);
        add_localized(&mut self.quest_name, staged.quest_name, session, sessions);
        add_localized(&mut self.objectives_text, staged.objectives_text, session, sessions);
        add_localized(&mut self.item_name, staged.item_name, session, sessions);
        add_localized(&mut self.object_name, staged.object_name, session, sessions);
    }
}

impl Accumulator {
    /// Folded sessions so far.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Adds one observed session; sessions must come in fold order.
    pub fn add(&mut self, staged: StagedSession, contributor: Option<&str>) {
        let s = self.sessions.add(contributor, staged.server_time);
        let sessions = &self.sessions;
        self.zone_or_sort.add_session(staged.zone_or_sort, s, sessions);
        self.quest_name.add_session(staged.quest_name, s, sessions);
        self.quest_level.add_session(staged.quest_level, s, sessions);
        self.objectives_text.add_session(staged.objectives_text, s, sessions);
        self.objective_count.add_session(staged.objective_count, s, sessions);
        self.npc_name.add_session(staged.npc_name, s, sessions);
        self.item_name.add_session(staged.item_name, s, sessions);
        self.object_name.add_session(staged.object_name, s, sessions);

        self.required_level.add(assign(staged.required_level, s));
        self.objectives.add(assign(staged.objectives, s));
        self.trigger_end.add(assign(staged.trigger_end, s));
        self.started_by.add(assign(staged.started_by, s));
        self.finished_by.add(assign(staged.finished_by, s));
        self.reputation_reward.add(assign(staged.reputation_reward, s));
        self.quest_xp.add(assign(staged.quest_xp, s));
        self.quest_factions.add(assign(staged.quest_factions, s));
        self.turn_in_poi.extend(assign(staged.turn_in_poi, s));
        self.npc_spawns.add(assign(staged.npc_spawns, s));
        self.npc_level.add(assign(staged.npc_level, s));
        self.npc_friendly_to_faction.add(assign(staged.npc_friendly_to_faction, s));
        self.npc_rank.add(assign(staged.npc_rank, s));
        self.npc_drops.add(assign(staged.npc_drops, s));
        self.object_drops.add(assign(staged.object_drops, s));
        self.object_spawns.add(assign(staged.object_spawns, s));

        self.lookups.add(staged.lookups, s, sessions);
    }
}
