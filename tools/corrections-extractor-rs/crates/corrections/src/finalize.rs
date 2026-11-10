//! Merges the folded observations into one value per entity and field, builds the records and
//! renders the output modules. Ports `finalize` of `extract/index.ts`, `finalizeLookups` of
//! `extract/lookups.ts` and `extract/questRelations.ts`.
//!
//! Merges run on typed values; each exported value is turned into the JS shape the TS writer
//! renders ([`Js`]) only when its record is built.

use crate::accumulator::{Accumulator, LookupAccumulator};
use crate::aggregate::{Num, ObservationsByEntity, ScalarSummary};
use crate::js::format_number;
use crate::lua::{write_corrections, write_lookup, write_quest_xp, Header, Js, LookupTable, Record};
use crate::observation::Observation;
use crate::observe::{MaxIds, MAX_IDS};
use crate::observers::item::drops_merge;
use crate::observers::npc::{friendly_to_faction, level, rank, spawns_merge};
use crate::observers::object::spawns_merge as object_spawns_merge;
use crate::observers::quest::required_races::{merge_quest_factions, merge_required_races, summarize_reactions, ReactionSummary};
use crate::observers::quest::turn_in_poi::turn_in_poi_spawns;
use crate::observers::quest::{finished_by, objectives, quest_xp, reputation_reward, required_level, started_by, trigger_end};
use crate::observers::spawn::SpawnObservation;
use crate::probes::Faction;
use indexmap::{IndexMap, IndexSet};
use std::collections::HashSet;
use std::hash::Hash;

/// Rendered output: file name -> module, and lookup directory -> (locale, module) pairs.
pub struct Bundle {
    pub modules: Vec<(&'static str, String)>,
    pub lookups: Vec<(&'static str, Vec<(String, String)>)>,
}

/// Records under construction: id -> fields in the order they were added.
#[derive(Default)]
struct Records(IndexMap<i64, Record>);

impl Records {
    /// emit/records.ts: the field is set on every entity that has a value for it. Fields appear
    /// in a record (and in the Lua output) in the order they are added.
    fn field<T>(&mut self, name: &str, values: &IndexMap<i64, T>, to_js: impl Fn(&T) -> Js) {
        for (&id, value) in values {
            self.0.entry(id).or_default().push((name.to_owned(), to_js(value)));
        }
    }

    fn below_max_dropped(self, max_id: i64) -> IndexMap<i64, Record> {
        self.0.into_iter().filter(|(id, _)| *id > max_id).collect()
    }
}

fn ids(values: &[i64]) -> Js {
    Js::nums(values.iter().map(|&id| id as f64))
}

/// `{[zoneID]: [[x, y], ...]}`.
fn zone_coordinates(zones: impl IntoIterator<Item = (f64, Vec<[f64; 2]>)>) -> Js {
    Js::object(
        zones
            .into_iter()
            .map(|(zone, coordinates)| (format_number(zone), Js::Array(coordinates.into_iter().map(|[x, y]| Js::nums([x, y])).collect()))),
    )
}

/// The values a merge decided to export (TS `onlyDefinedFacts`).
fn only_defined<T>(values: IndexMap<i64, Option<T>>) -> IndexMap<i64, T> {
    values.into_iter().filter_map(|(id, value)| value.map(|value| (id, value))).collect()
}

/// A merge over each entity's observations of `field` followed by those of `extra` (TS
/// `store.groups([key, extraKey])`): spawns and their quest turn-in spawns. Entities merge
/// independently, so they are spread over threads; the result keeps the entity order.
fn merge_with_extra<R: Send>(
    field: &ObservationsByEntity<SpawnObservation>,
    extra: &IndexMap<i64, Vec<Observation<SpawnObservation>>>,
    merge: impl Fn(i64, &[Observation<SpawnObservation>]) -> R + Sync,
) -> IndexMap<i64, R> {
    let ids: Vec<i64> = field
        .iter()
        .map(|(id, _)| id)
        .chain(extra.keys().copied().filter(|&id| field.get(id).is_none()))
        .collect();
    let merge_one = |id: i64| -> R {
        match (field.get(id), extra.get(&id)) {
            (Some(observed), turn_in) => {
                let mut combined = observed.to_vec();
                combined.extend(turn_in.into_iter().flatten().cloned());
                merge(id, &combined)
            }
            (None, Some(turn_in)) => merge(id, turn_in),
            (None, None) => unreachable!("every id comes from one of the two"),
        }
    };
    // Workers take entities one at a time: a few entities hold most observations.
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut merged: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&id) = ids.get(at) else { return done };
                        done.push((at, merge_one(id)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|worker| worker.join().expect("a merge panicked")).collect()
    });
    merged.sort_unstable_by_key(|(at, _)| *at);
    ids.into_iter().zip(merged.into_iter().map(|(_, result)| result)).collect()
}

fn by_entity<T>(observations: Vec<Observation<T>>) -> IndexMap<i64, Vec<Observation<T>>> {
    let mut grouped: IndexMap<i64, Vec<Observation<T>>> = IndexMap::new();
    for obs in observations {
        grouped.entry(obs.entity_id).or_default().push(obs);
    }
    grouped
}

/// A quest's starters or finishers as exported: creatures and objects (starters also items).
struct Relation {
    creatures: Vec<i64>,
    objects: Vec<i64>,
}

/// questRelations.ts: npc/object questStarts and questEnds, the inverse of the exported quests'
/// startedBy and finishedBy, so both sides agree.
#[derive(Default)]
struct QuestRelations {
    quest_starts: Option<Vec<i64>>,
    quest_ends: Option<Vec<i64>>,
}

impl QuestRelations {
    fn fields(&self) -> Record {
        let mut fields = Vec::new();
        if let Some(starts) = &self.quest_starts {
            fields.push(("questStarts".to_owned(), ids(starts)));
        }
        if let Some(ends) = &self.quest_ends {
            fields.push(("questEnds".to_owned(), ids(ends)));
        }
        fields
    }
}

/// Inverts the exported relations; one pass per field, so questStarts is always created before questEnds.
fn invert_quest_relations(exported: &IndexMap<i64, (Option<Relation>, Option<Relation>)>) -> (IndexMap<i64, QuestRelations>, IndexMap<i64, QuestRelations>) {
    let mut npcs: IndexMap<i64, QuestRelations> = IndexMap::new();
    let mut objects: IndexMap<i64, QuestRelations> = IndexMap::new();
    let mut quest_ids: Vec<i64> = exported.keys().copied().collect();
    quest_ids.sort_unstable();
    for starts in [true, false] {
        for &quest_id in &quest_ids {
            let (started_by, finished_by) = &exported[&quest_id];
            let Some(relation) = (if starts { started_by } else { finished_by }) else {
                continue;
            };
            for (targets, entity_ids) in [(&mut npcs, &relation.creatures), (&mut objects, &relation.objects)] {
                for &id in entity_ids {
                    let relations = targets.entry(id).or_default();
                    let list = if starts { &mut relations.quest_starts } else { &mut relations.quest_ends };
                    list.get_or_insert_with(Vec::new).push(quest_id);
                }
            }
        }
    }
    (npcs, objects)
}

/// A record per entity above `max_id`, plus the quest relations of every entity: quests always
/// resolve their starters and finishers, even those below MAX_IDS.
fn with_quest_relations(records: Records, relations: &IndexMap<i64, QuestRelations>, max_id: i64) -> Vec<(i64, Record)> {
    let mut merged = records.below_max_dropped(max_id);
    for (&id, relation) in relations {
        merged.entry(id).or_default().extend(relation.fields());
    }
    merged.into_iter().collect()
}

pub fn finalize(acc: &Accumulator, source_file_count: usize, generated_at: &str) -> Bundle {
    let max = MAX_IDS;
    let header = Header {
        source_file_count,
        session_count: acc.session_count(),
        generated_at,
    };

    let zone_or_sort = acc.zone_or_sort.aggregate();
    let finished_by = acc.finished_by.merge(finished_by::merge);
    // Quest turn-in POIs are spawns of the quest's single finisher: they join its spawn observations.
    let turn_in = turn_in_poi_spawns(&acc.turn_in_poi, &finished_by);
    let (npc_turn_in, object_turn_in) = (by_entity(turn_in.npcs), by_entity(turn_in.objects));

    let quest_level = acc.quest_level.aggregate();
    let mut quest_xp_rows = Vec::new();
    for (quest_id, observations) in acc.quest_xp.iter() {
        let level = quest_level.get(&quest_id).map(|level| level.0);
        if let (Some(xp), Some(level)) = (quest_xp::merge(observations, level), level) {
            quest_xp_rows.push((quest_id, level, xp));
        }
    }

    let started_by = acc.started_by.merge(started_by::merge);
    let required_races = required_races(acc, &started_by);

    let objective_count = acc.objective_count.aggregate();
    // Objectives only count when every objective the API reports was matched to an id.
    let quest_objectives: IndexMap<i64, objectives::QuestObjectivesValue> = acc
        .objectives
        .merge(objectives::merge)
        .into_iter()
        .filter(|(id, value)| {
            let matched = value.creatures.len() + value.objects.len() + value.items.len();
            objective_count.get(id) == Some(&Num(matched as f64))
        })
        .collect();
    let trigger_end = acc
        .trigger_end
        .merge(|obs| trigger_end::merge(obs, zone_or_sort.get(&obs[0].entity_id).map(|zone| zone.0)));
    // requiredLevel never exceeds questLevel.
    let required_level: IndexMap<i64, f64> = acc
        .required_level
        .merge(required_level::merge)
        .into_iter()
        .map(|(id, required)| match quest_level.get(&id) {
            Some(level) if required > level.0 => (id, level.0),
            _ => (id, required),
        })
        .collect();

    // The order of the field calls is the order of the fields in the Lua output (TS: the order of
    // the QuestFacts / NpcFacts / ... object literals in extract/index.ts).
    let mut quests = Records::default();
    quests.field("name", &acc.quest_name.aggregate_newest(), |name| Js::str(name));
    quests.field("questLevel", &quest_level, |level| Js::Num(level.0));
    quests.field("requiredLevel", &required_level, |level| Js::Num(*level));
    quests.field("zoneOrSort", &zone_or_sort, |zone| Js::Num(zone.0));
    quests.field("objectives", &quest_objectives, |value| {
        let positions = [("1", &value.creatures), ("2", &value.objects), ("3", &value.items)];
        Js::object(
            positions
                .into_iter()
                .filter(|(_, list)| !list.is_empty())
                .map(|(key, list)| (key.to_owned(), Js::nums(list.iter().copied()))),
        )
    });
    quests.field("objectivesText", &acc.objectives_text.aggregate(), |lines| {
        Js::Array(lines.iter().map(Js::str).collect())
    });
    quests.field("triggerEnd", &trigger_end, |value| {
        let locations = value
            .locations
            .iter()
            .map(|(zone, coordinates)| (*zone, coordinates.iter().map(|&(x, y)| [x, y]).collect()));
        Js::Array(vec![Js::str(&value.text), zone_coordinates(locations)])
    });
    quests.field("startedBy", &started_by, |value| {
        Js::object([
            ("creatures".to_owned(), ids(&value.creatures)),
            ("objects".to_owned(), ids(&value.objects)),
            ("items".to_owned(), ids(&value.items)),
        ])
    });
    quests.field("requiredRaces", &required_races, |mask| Js::Num(*mask as f64));
    quests.field("finishedBy", &finished_by, |value| {
        Js::object([("creatures".to_owned(), ids(&value.creatures)), ("objects".to_owned(), ids(&value.objects))])
    });
    quests.field(
        "reputationReward",
        &only_defined(acc.reputation_reward.merge(reputation_reward::merge)),
        |rewards| Js::Array(rewards.iter().map(|&(faction, amount)| Js::nums([faction, amount])).collect()),
    );

    // Quest givers and enders stand still, so they get a single spawn per zone.
    let mut quest_npc_ids = HashSet::new();
    let mut quest_object_ids = HashSet::new();
    for id in quests.0.keys() {
        let starters = started_by.get(id).map(|v| (&v.creatures, &v.objects));
        let finishers = finished_by.get(id).map(|v| (&v.creatures, &v.objects));
        for (creatures, objects) in starters.into_iter().chain(finishers) {
            quest_npc_ids.extend(creatures.iter().copied());
            quest_object_ids.extend(objects.iter().copied());
        }
    }

    let npc_level = only_defined(acc.npc_level.merge(level::merge));
    let npc_spawns = merge_with_extra(&acc.npc_spawns, &npc_turn_in, |id, obs| {
        spawns_merge::merge_spawns(obs, quest_npc_ids.contains(&id))
    });
    let npc_zones = merge_with_extra(&acc.npc_spawns, &npc_turn_in, |_, obs| spawns_merge::merge_zone_id(obs));
    let mut npcs = Records::default();
    npcs.field("name", &acc.npc_name.aggregate_newest(), |name| Js::str(name));
    npcs.field("minLevel", &npc_level, |range| Js::Num(range.min));
    npcs.field("maxLevel", &npc_level, |range| Js::Num(range.max));
    npcs.field("spawns", &npc_spawns, |spawns| zone_coordinates(spawns.iter().cloned()));
    npcs.field("zoneID", &npc_zones, |zone| Js::Num(*zone));
    npcs.field(
        "friendlyToFaction",
        &only_defined(acc.npc_friendly_to_faction.merge(friendly_to_faction::merge)),
        |factions| Js::str(*factions),
    );
    npcs.field("rank", &only_defined(acc.npc_rank.merge(rank::merge)), |rank| Js::Num(*rank as f64));

    let mut items = Records::default();
    items.field("name", &acc.item_name.aggregate_newest(), |name| Js::str(name));
    items.field("npcDrops", &acc.npc_drops.merge(drops_merge::merge_npc_drops), |drops| ids(drops));
    items.field("objectDrops", &acc.object_drops.merge(drops_merge::merge_object_drops), |drops| ids(drops));

    let object_spawns = merge_with_extra(&acc.object_spawns, &object_turn_in, |id, obs| {
        object_spawns_merge::merge_spawns(obs, quest_object_ids.contains(&id))
    });
    let object_zones = merge_with_extra(&acc.object_spawns, &object_turn_in, |_, obs| object_spawns_merge::merge_zone_id(obs));
    let mut objects = Records::default();
    objects.field("name", &acc.object_name.aggregate_newest(), |name| Js::str(name));
    objects.field("spawns", &object_spawns, |spawns| zone_coordinates(spawns.iter().cloned()));
    objects.field("zoneID", &object_zones, |zone| Js::Num(*zone));

    // Quests below MAX_IDS are already known to Questie, but may be started or finished by new
    // npcs/objects: those relations are exported on the quest, and below on the entity.
    let mut exported_quests: Vec<(i64, Record)> = Vec::new();
    let mut exported_relations: IndexMap<i64, (Option<Relation>, Option<Relation>)> = IndexMap::new();
    for (id, record) in quests.0 {
        let starters = started_by.get(&id);
        let finishers = finished_by.get(&id);
        let (record, starts, ends) = if id > max.quest {
            let starts = starters.map(|v| Relation {
                creatures: v.creatures.clone(),
                objects: v.objects.clone(),
            });
            let ends = finishers.map(|v| Relation {
                creatures: v.creatures.clone(),
                objects: v.objects.clone(),
            });
            (record, starts, ends)
        } else {
            relations_to_new_entities(
                starters.map(|v| (&v.creatures, &v.objects)),
                finishers.map(|v| (&v.creatures, &v.objects)),
                &max,
            )
        };
        if !record.is_empty() {
            exported_quests.push((id, record));
            exported_relations.insert(id, (starts, ends));
        }
    }
    let (npc_relations, object_relations) = invert_quest_relations(&exported_relations);

    Bundle {
        modules: vec![
            (
                "foreverNpcTraces.lua",
                write_corrections("ForeverNpcTraces", "npcKeys", &with_quest_relations(npcs, &npc_relations, max.npc), &header),
            ),
            (
                "foreverQuestTraces.lua",
                write_corrections("ForeverQuestTraces", "questKeys", &exported_quests, &header),
            ),
            (
                "foreverItemTraces.lua",
                write_corrections(
                    "ForeverItemTraces",
                    "itemKeys",
                    &items.below_max_dropped(max.item).into_iter().collect::<Vec<_>>(),
                    &header,
                ),
            ),
            (
                "foreverObjectTraces.lua",
                write_corrections(
                    "ForeverObjectTraces",
                    "objectKeys",
                    &with_quest_relations(objects, &object_relations, max.object),
                    &header,
                ),
            ),
            ("xpDB-traces.lua", write_quest_xp(&quest_xp_rows, &header)),
        ],
        lookups: finalize_lookups(&acc.lookups, &max),
    }
}

type Ids<'r> = (&'r Vec<i64>, &'r Vec<i64>);

/// For a quest below MAX_IDS: only its starters and finishers that are new npcs or objects.
fn relations_to_new_entities(starters: Option<Ids>, finishers: Option<Ids>, max: &MaxIds) -> (Record, Option<Relation>, Option<Relation>) {
    let new_only = |(creatures, objects): Ids| Relation {
        creatures: creatures.iter().copied().filter(|&id| id > max.npc).collect(),
        objects: objects.iter().copied().filter(|&id| id > max.object).collect(),
    };
    let mut record = Vec::new();
    let starts = starters.map(new_only).filter(|r| !r.creatures.is_empty() || !r.objects.is_empty());
    if let Some(r) = &starts {
        record.push((
            "startedBy".to_owned(),
            Js::object([
                ("creatures".to_owned(), ids(&r.creatures)),
                ("objects".to_owned(), ids(&r.objects)),
                ("items".to_owned(), ids(&[])),
            ]),
        ));
    }
    let ends = finishers.map(new_only).filter(|r| !r.creatures.is_empty() || !r.objects.is_empty());
    if let Some(r) = &ends {
        record.push((
            "finishedBy".to_owned(),
            Js::object([("creatures".to_owned(), ids(&r.creatures)), ("objects".to_owned(), ids(&r.objects))]),
        ));
    }
    (record, starts, ends)
}

/// requiredRaces from the factions that saw each quest, the reactions of its NPC starters, and
/// the factions that saw the other quests of those starters.
fn required_races(acc: &Accumulator, started_by: &IndexMap<i64, started_by::StartedByValue>) -> IndexMap<i64, u64> {
    let npc_reactions = acc.npc_friendly_to_faction.merge(summarize_reactions);
    let quest_factions = acc.quest_factions.merge(merge_quest_factions);
    let mut factions_by_starter: IndexMap<i64, IndexMap<i64, &IndexSet<Faction>>> = IndexMap::new();
    for (&quest_id, factions) in &quest_factions {
        for &npc_id in started_by.get(&quest_id).map(|s| s.creatures.as_slice()).unwrap_or_default() {
            factions_by_starter.entry(npc_id).or_default().insert(quest_id, factions);
        }
    }
    let mut required_races = IndexMap::new();
    for (&quest_id, seen_factions) in &quest_factions {
        let starters = started_by.get(&quest_id);
        let npc_starters: Vec<ReactionSummary> = match starters {
            Some(s) if s.objects.is_empty() && s.items.is_empty() => s.creatures.iter().map(|id| npc_reactions.get(id).copied().unwrap_or_default()).collect(),
            _ => Vec::new(),
        };
        let mut sibling_factions = IndexSet::new();
        for npc_id in starters.map(|s| s.creatures.as_slice()).unwrap_or_default() {
            for (&sibling_id, factions) in factions_by_starter.get(npc_id).into_iter().flatten() {
                if sibling_id != quest_id {
                    sibling_factions.extend(factions.iter().copied());
                }
            }
        }
        if let Some(mask) = merge_required_races(seen_factions, &npc_starters, &sibling_factions) {
            required_races.insert(quest_id, mask);
        }
    }
    required_races
}

/// lookups.ts `valuesFor`: one value per entity above `max_id` in `locale`.
fn values_for<T: Clone + Eq + Hash>(by_locale: &IndexMap<String, ScalarSummary<T>>, locale: &str, newest_wins: bool, max_id: i64) -> IndexMap<i64, T> {
    let Some(summary) = by_locale.get(locale) else { return IndexMap::new() };
    let values = if newest_wins { summary.aggregate_newest() } else { summary.aggregate() };
    values.into_iter().filter(|(id, _)| *id > max_id).collect()
}

fn sorted_locales<T: Eq + Hash>(by_locale: &IndexMap<String, ScalarSummary<T>>) -> Vec<String> {
    let mut locales: Vec<String> = by_locale.keys().cloned().collect();
    locales.sort();
    locales
}

/// lookups.ts `finalizeLookups`: per entity and locale, a Questie l10n module. Names can change
/// over time (a renamed NPC), so the newest one wins; objective texts use the default policy.
fn finalize_lookups(acc: &LookupAccumulator, max: &MaxIds) -> Vec<(&'static str, Vec<(String, String)>)> {
    let mut npcs = Vec::new();
    for locale in sorted_locales(&acc.npc_name) {
        // Subnames are not traced, so they are always nil.
        let entries: Vec<(i64, Js)> = values_for(&acc.npc_name, &locale, true, max.npc)
            .into_iter()
            .map(|(id, name)| (id, Js::Array(vec![Js::Str(name), Js::Null])))
            .collect();
        if !entries.is_empty() {
            npcs.push((locale.clone(), write_lookup(LookupTable::Npc, &locale, &entries)));
        }
    }
    let mut quests = Vec::new();
    for locale in sorted_locales(&acc.quest_name) {
        let objectives = values_for(&acc.objectives_text, &locale, false, max.quest);
        // Quests without an observed title are left out: Questie needs the title.
        let entries: Vec<(i64, Js)> = values_for(&acc.quest_name, &locale, true, max.quest)
            .into_iter()
            .map(|(id, name)| {
                let text = objectives.get(&id).map_or(Js::Null, |lines| Js::Array(lines.iter().map(Js::str).collect()));
                (id, Js::Array(vec![Js::Str(name), text]))
            })
            .collect();
        if !entries.is_empty() {
            quests.push((locale.clone(), write_lookup(LookupTable::Quest, &locale, &entries)));
        }
    }
    let names = |by_locale: &IndexMap<String, ScalarSummary<String>>, table: LookupTable, max_id: i64| -> Vec<(String, String)> {
        sorted_locales(by_locale)
            .into_iter()
            .filter_map(|locale| {
                let entries: Vec<(i64, Js)> = values_for(by_locale, &locale, true, max_id)
                    .into_iter()
                    .map(|(id, name)| (id, Js::Str(name)))
                    .collect();
                (!entries.is_empty()).then(|| {
                    let module = write_lookup(table, &locale, &entries);
                    (locale, module)
                })
            })
            .collect()
    };
    vec![
        ("lookupNpcs", npcs),
        ("lookupQuests", quests),
        ("lookupItems", names(&acc.item_name, LookupTable::Item, max.item)),
        ("lookupObjects", names(&acc.object_name, LookupTable::Object, max.object)),
    ]
}
