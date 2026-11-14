//! Test helpers: sessions described as a few readable facts.

use super::history::{build_histories, unstable_quests, HistoryParams};
use super::population::Population;
use crate::model::*;
use crate::test_support::{catalog, catalog_quest, event, make_episode};

pub struct SessionSpec {
    /// QuestEpisode::session_order: position among the character's sessions, oldest first.
    pub order: u32,
    pub race: i64,
    pub level: i64,
    /// (t, level): dinged at t.
    pub level_ups: Vec<(SessionTime, i64)>,
    /// Completed before the session started.
    pub initial: Vec<QuestId>,
    /// In the log when the session started.
    pub holding: Vec<QuestId>,
    /// (t, quest): accepted at t.
    pub accepts: Vec<(SessionTime, QuestId)>,
    /// (t, quest): turned in at t; the completed set gains it `delta_delay` seconds later.
    pub turn_ins: Vec<(SessionTime, QuestId)>,
    /// Seconds between a turn-in and the completed set showing it (the client samples the set late).
    pub delta_delay: SessionTime,
    /// (t, quest): the completed set gains it at t without a turn-in (server flag).
    pub flags: Vec<(SessionTime, QuestId)>,
}

impl Default for SessionSpec {
    /// A level 5 human warrior's first session, with nothing completed.
    fn default() -> Self {
        SessionSpec {
            order: 0,
            race: 1,
            level: 5,
            level_ups: Vec::new(),
            initial: Vec::new(),
            holding: Vec::new(),
            accepts: Vec::new(),
            turn_ins: Vec::new(),
            delta_delay: 0.5,
            flags: Vec::new(),
        }
    }
}

const HORDE_RACES: [i64; 5] = [2, 5, 6, 8, 96];

/// One session of character `character_key`; the quest log follows the accepts and turn-ins.
pub fn session(character_key: &str, spec: SessionSpec) -> QuestEpisode {
    // Completed-set additions, by time.
    let mut adds: Vec<(SessionTime, Vec<QuestId>)> = Vec::new();
    let mut add = |t: SessionTime, quest_id: QuestId| match adds.iter_mut().find(|(at, _)| *at == t) {
        Some((_, ids)) => ids.push(quest_id),
        None => adds.push((t, vec![quest_id])),
    };
    let mut quest_events = Vec::new();
    for &(t, quest_id) in &spec.turn_ins {
        quest_events.push(event(t, QuestEventKind::TurnedIn, quest_id, None));
        add(t + spec.delta_delay, quest_id);
    }
    for &(t, quest_id) in &spec.flags {
        add(t, quest_id);
    }
    for &(t, quest_id) in &spec.accepts {
        quest_events.push(event(t, QuestEventKind::Accepted, quest_id, None));
    }
    quest_events.sort_by(|a, b| a.t.total_cmp(&b.t));
    adds.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut log = spec.holding.clone();
    let mut quest_log = vec![TimedValue { t: 0.0, v: log.clone() }];
    for event in &quest_events {
        if event.kind == QuestEventKind::Accepted {
            if !log.contains(&event.quest_id) {
                log.push(event.quest_id);
            }
        } else {
            log.retain(|&id| id != event.quest_id);
        }
        quest_log.push(TimedValue { t: event.t, v: log.clone() });
    }

    let faction = if HORDE_RACES.contains(&spec.race) {
        Faction::Horde
    } else {
        Faction::Alliance
    };
    let mut levels = vec![TimedValue { t: 0.0, v: spec.level }];
    levels.extend(spec.level_ups.iter().map(|&(t, v)| TimedValue { t, v }));
    QuestEpisode {
        character_key: character_key.to_owned(),
        session_order: spec.order,
        player: PlayerContext {
            race_id: Some(spec.race),
            class_id: Some(1),
            faction: Some(faction),
        },
        levels,
        completed: Some(CompletedTimeline {
            source: CompletedSource::Modern,
            initial: spec.initial,
            changes: adds
                .into_iter()
                .map(|(t, ids)| CompletedChange {
                    t,
                    add: Some(ids),
                    remove: None,
                })
                .collect(),
        }),
        quest_log,
        quest_events,
        ..make_episode()
    }
}

/// A catalog of plain quests (no requirements, no starters). Use `test_support::catalog` for others.
pub fn catalog_of(ids: impl IntoIterator<Item = QuestId>) -> QuestCatalog {
    catalog(ids.into_iter().map(catalog_quest))
}

/// The population the signal would build from `episodes`.
pub fn population_of<'e>(episodes: &'e [QuestEpisode], catalog: &'e QuestCatalog) -> Population<'e> {
    let episodes: Vec<&QuestEpisode> = episodes.iter().collect();
    let ignore = unstable_quests(&episodes, catalog);
    let (histories, _) = build_histories(&episodes, &HistoryParams { max_initial_loss: 0.1 }, &ignore);
    Population::new(histories, catalog, &ignore)
}
