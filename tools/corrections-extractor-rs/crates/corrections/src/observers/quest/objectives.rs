//! questKeys.objectives - Questie field 10. Port of `extract/observers/quest/objectives.ts`.
//!
//! A positional table {creatureObjective, objectObjective, itemObjective, reputationObjective,
//! killCreditObjective, spellObjective}; only the first three are populated here.
//!
//! C_QuestLog.GetQuestObjectives exposes each objective's type, text and progress, but not the
//! entity ids Questie needs. Objectives are matched by name against ids seen anywhere in the
//! same session (kills and loot usually happen well after the quest was accepted):
//!   item    -> GetLootSlotLink + GetLootSlotInfo, the turn-in window's
//!              GetQuestItemLink("required", i), and GetMerchantItemLink
//!   monster -> UnitName + UnitGUID
//!   object  -> UnitName + UnitGUID
//!
//! Blizzard's "object" type also covers in-place actions (emotes, casting a quest spell) whose
//! text never names a GameObject; those simply match nothing.

use crate::aggregate::Num;
use crate::guid::{parse_guid, GuidKind};
use crate::item_link::parse_item_link;
use crate::js;
use crate::observation::{Confidence, Observation, SessionId};
use crate::probes::{locale_at, stream_value_at, unit_name_at};
use indexmap::{IndexMap, IndexSet};
use trace_data::stream::{emulated_list, entry_t, entry_v, get_stream, param_keys, value_at, Stream};
use trace_data::{KeyRef, Session, Value};

const OBJECTIVES: &str = "C_QuestLog.GetQuestObjectives";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ObjectiveKind {
    Item,
    Monster,
    Object,
}

impl ObjectiveKind {
    /// The objective `type` the API reports; None for the types without an entity id.
    fn from_type(objective_type: Option<&Value>) -> Option<ObjectiveKind> {
        match objective_type?.as_str()? {
            "item" => Some(ObjectiveKind::Item),
            "monster" => Some(ObjectiveKind::Monster),
            "object" => Some(ObjectiveKind::Object),
            _ => None,
        }
    }
}

/// One objective of one GetQuestObjectives sample, matched to an entity id.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectiveMatch {
    pub kind: ObjectiveKind,
    /// The name that matched: the objective text without its progress (and maybe its verb).
    pub name: String,
    pub text: String,
    /// npcID, objectID or itemID.
    pub id: f64,
    /// Position in the C_QuestLog.GetQuestObjectives result (0-based).
    pub index: usize,
}

/// Questie's positional objectives table. JS shape `{1: creatures, 2: objects, 3: items}`, each
/// key only present when its list is non-empty (TS `QuestObjectivesValue`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuestObjectivesValue {
    pub creatures: Vec<f64>,
    pub objects: Vec<f64>,
    pub items: Vec<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum EvidenceKind {
    Item,
    Npc,
    Object,
}

/// Whether evidence wins over same-named alternatives for a quest (TS `preferredFor`).
#[derive(Clone, Copy)]
enum Preference {
    Never,
    /// An item GetLootSlotInfo flags as a quest item.
    Always,
    /// A unit seen while GetQuestID() reported this quest: a weak "which dialog was open".
    ForQuest(f64),
}

impl Preference {
    fn prefers(self, quest_id: f64) -> bool {
        match self {
            Preference::Never => false,
            Preference::Always => true,
            Preference::ForQuest(dialog_quest_id) => dialog_quest_id == quest_id,
        }
    }
}

struct Evidence {
    kind: EvidenceKind,
    /// `normalizeName(name)`, computed once instead of per comparison.
    normalized_name: String,
    id: f64,
    preference: Preference,
    /// The quest this item starts (GetLootSlotInfo). It ranks below same-named alternatives for
    /// that quest, but stays a valid fallback.
    starts_quest_id: Option<f64>,
    /// The quest whose turn-in window listed this item as required. Wins over all other evidence.
    required_for_quest_id: Option<f64>,
}

/// `name.trim().toLocaleLowerCase()`.
fn normalize_name(name: &str) -> String {
    js::trim(name).to_lowercase()
}

/// Removes `\d+` at the start of `s`; None without a digit.
fn strip_digits(s: &str) -> Option<&str> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| &s[digits..])
}

fn trim_start_ws(s: &str) -> &str {
    s.trim_start_matches(js::is_whitespace)
}

fn trim_end_ws(s: &str) -> &str {
    s.trim_end_matches(js::is_whitespace)
}

/// `s.replace(/^\s*\d+\s*\/\s*\d+\s*/, "")`: the progress prefix of "0/8 Juvenile Vuldren slain".
fn strip_progress_prefix(s: &str) -> &str {
    let stripped = strip_digits(trim_start_ws(s))
        .and_then(|rest| trim_start_ws(rest).strip_prefix('/'))
        .and_then(|rest| strip_digits(trim_start_ws(rest)))
        .map(trim_start_ws);
    stripped.unwrap_or(s)
}

/// `s.replace(/\s*:\s*\d+\s*\/\s*\d+\s*$/, "")`: the progress suffix of "Anaya's Pendant: 0/1".
fn strip_progress_suffix(s: &str) -> &str {
    // A match holds no ':' after its own, so only the last colon can start one; the leftmost
    // match then starts at the whitespace before that colon.
    let Some(colon) = s.rfind(':') else { return s };
    let tail = strip_digits(trim_start_ws(&s[colon + 1..]))
        .and_then(|rest| trim_start_ws(rest).strip_prefix('/'))
        .and_then(|rest| strip_digits(trim_start_ws(rest)))
        .map(trim_start_ws);
    match tail {
        Some("") => trim_end_ws(&s[..colon]),
        _ => s,
    }
}

/// `s.replace(/\s+slain$/i, "")`.
fn strip_slain_suffix(s: &str) -> &str {
    const SLAIN: &str = "slain";
    // Non-unicode /i only folds ASCII letters into "slain".
    let Some(before) = s
        .len()
        .checked_sub(SLAIN.len())
        .filter(|&at| s.as_bytes()[at..].eq_ignore_ascii_case(SLAIN.as_bytes()))
        .map(|at| &s[..at])
    else {
        return s;
    };
    let trimmed = trim_end_ws(before);
    // \s+ needs at least one whitespace character.
    if trimmed.len() == before.len() {
        s
    } else {
        trimmed
    }
}

/// `s.replace(/\s+\S+ed$/i, "")`: the last word, when it ends in "ed" after at least one
/// character, together with the whitespace before it.
fn strip_trailing_past_tense_word(s: &str) -> &str {
    let Some(space) = s.rfind(js::is_whitespace) else { return s };
    let word_start = space + s[space..].chars().next().map_or(0, char::len_utf8);
    let word = &s.as_bytes()[word_start..];
    if word.len() > 2 && word[word.len() - 2..].eq_ignore_ascii_case(b"ed") {
        trim_end_ws(&s[..word_start])
    } else {
        s
    }
}

/// The objective's name without its progress counter and, for kill objectives, without a
/// trailing " slain". enUS Classic writes monster/object progress as a prefix ("0/8 Juvenile
/// Vuldren slain", "0/1 Destroy the Demon Seed") and item progress as a suffix ("Anaya's
/// Pendant: 0/1"); both are stripped since only one can match. " slain" is only stripped for
/// monsters, so an item or object name ending in "slain" stays intact.
fn objective_name(text: &str, kind: ObjectiveKind) -> Option<String> {
    let without_suffix = strip_progress_suffix(strip_progress_prefix(text));
    let without_kill_suffix = if kind == ObjectiveKind::Monster {
        strip_slain_suffix(without_suffix)
    } else {
        without_suffix
    };
    let trimmed = js::trim(without_kill_suffix);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// "slain" is only the most common kill verb: Blizzard also writes "0/6 Roiling Winds destroyed"
/// or "0/5 Sickly Gazelle cured". When the full name matches nothing, monster and object
/// objectives retry without a trailing past-tense word. Items are excluded: "Cured Ham".
fn name_without_trailing_verb(name: &str, kind: ObjectiveKind) -> Option<String> {
    if kind == ObjectiveKind::Item {
        return None;
    }
    let stripped = js::trim(strip_trailing_past_tense_word(name));
    (!stripped.is_empty() && stripped != name).then(|| stripped.to_owned())
}

/// `GetQuestID()` at or before `t`. Unlike `probes::quest_id_at`, NaN counts as a quest id.
fn quest_id_at(session: &Session, t: f64) -> Option<f64> {
    stream_value_at(session, "GetQuestID", &[], t).and_then(Value::as_num).filter(|&id| id != 0.0)
}

fn item_evidence(name: &str, id: f64, preference: Preference, starts_quest_id: Option<f64>, required_for_quest_id: Option<f64>) -> Evidence {
    Evidence {
        kind: EvidenceKind::Item,
        normalized_name: normalize_name(name),
        id,
        preference,
        starts_quest_id,
        required_for_quest_id,
    }
}

fn collect_loot_evidence(session: &Session) -> Vec<Evidence> {
    let Some(link_streams) = session.function("GetLootSlotLink") else {
        return Vec::new();
    };

    // Older traces store both as one flat stream, newer ones as streams keyed by loot slot.
    // Keyed streams are paired by slot: filtering each root separately would shift info from
    // another slot onto links without samples.
    let info_root = session.function("GetLootSlotInfo");
    let groups: Vec<(Stream, Stream)> = match link_streams.as_array() {
        Some(links) => vec![(links, info_root.and_then(Value::as_array).unwrap_or(&[]))],
        None => param_keys(link_streams)
            .iter()
            .map(|slot| {
                let links = get_stream(session, "GetLootSlotLink", &[KeyRef::Str(slot)]).unwrap_or(&[]);
                let infos = match info_root {
                    Some(root) if !root.is_array() => get_stream(session, "GetLootSlotInfo", &[KeyRef::Str(slot)]).unwrap_or(&[]),
                    _ => &[],
                };
                (links, infos)
            })
            .filter(|(links, _)| !links.is_empty())
            .collect(),
    };

    let mut evidence = Vec::new();
    for (links, infos) in groups {
        for link_entry in links {
            let Some(parsed) = entry_v(link_entry).and_then(Value::as_str).and_then(|link| parse_item_link(Some(link))) else {
                continue;
            };
            // GetLootSlotInfo returns (icon, name, quantity, currencyID, quality, locked,
            // isQuestItem, questID, isActive); questID is the quest the item starts.
            let info = if infos.is_empty() {
                None
            } else {
                emulated_list(value_at(infos, entry_t(link_entry)))
            };
            let info_value = |index: usize| info.as_ref().and_then(|info| info.get(index).copied().flatten());
            let is_quest_item = info_value(6).and_then(Value::as_bool) == Some(true);
            let starts_quest_id = info_value(7).and_then(Value::as_num).filter(|&id| id != 0.0);
            let preference = if is_quest_item { Preference::Always } else { Preference::Never };
            evidence.push(item_evidence(parsed.name, parsed.item_id, preference, starts_quest_id, None));
        }
    }
    evidence
}

/// Entries of the streams of `function_name` keyed by index under `prefix`, e.g.
/// GetMerchantItemLink[index] or GetQuestItemLink.required[index].
fn link_entries_by_index<'v, 'a>(session: &Session<'v, 'a>, function_name: &str, prefix: &[&str]) -> Vec<&'v Value<'a>> {
    let mut root = session.function(function_name);
    for &key in prefix {
        root = root.filter(|node| !node.is_array()).and_then(|node| node.get(key));
    }
    let Some(root) = root.and_then(Value::as_object) else { return Vec::new() };
    // getStream(session, functionName, ...prefix, index) is the value under each key, when it
    // is a leaf stream.
    root.entries().filter_map(|(_, node)| node.as_array()).flatten().collect()
}

/// Required items of a quest's turn-in window (attributed to the quest from GetQuestID) and
/// items a merchant sells, e.g. vendor-bought objectives like Empty Vial.
fn collect_link_evidence(session: &Session) -> Vec<Evidence> {
    let mut evidence = Vec::new();
    for entry in link_entries_by_index(session, "GetQuestItemLink", &["required"]) {
        let Some(parsed) = entry_v(entry).and_then(Value::as_str).and_then(|link| parse_item_link(Some(link))) else {
            continue;
        };
        let required_for = quest_id_at(session, entry_t(entry));
        evidence.push(item_evidence(parsed.name, parsed.item_id, Preference::Never, None, required_for));
    }
    for entry in link_entries_by_index(session, "GetMerchantItemLink", &[]) {
        let Some(parsed) = entry_v(entry).and_then(Value::as_str).and_then(|link| parse_item_link(Some(link))) else {
            continue;
        };
        evidence.push(item_evidence(parsed.name, parsed.item_id, Preference::Never, None, None));
    }
    evidence
}

fn collect_unit_evidence(session: &Session) -> Vec<Evidence> {
    let mut evidence = Vec::new();
    for token in ["target", "npc", "questnpc"] {
        let Some(guid_stream) = get_stream(session, "UnitGUID", &[KeyRef::Str(token)]) else {
            continue;
        };
        for entry in guid_stream {
            let Some(guid) = entry_v(entry).and_then(Value::as_str) else { continue };
            let parsed = parse_guid(Some(guid));
            let kind = match parsed.kind {
                GuidKind::Npc => EvidenceKind::Npc,
                GuidKind::Object => EvidenceKind::Object,
                _ => continue,
            };
            let Some(id) = parsed.id.filter(|&id| id != 0.0) else { continue };
            let t = entry_t(entry);
            let Some(name) = unit_name_at(session, token, t) else { continue };
            let preference = quest_id_at(session, t).map_or(Preference::Never, Preference::ForQuest);
            evidence.push(Evidence {
                kind,
                normalized_name: normalize_name(&name),
                id,
                preference,
                starts_quest_id: None,
                required_for_quest_id: None,
            });
        }
    }
    evidence
}

/// The best evidence with exactly the objective's normalized name, ranked: items the quest's
/// turn-in window requires, then preferred evidence, then other evidence, then items that start
/// the quest (some quests ask to bring back the item that started them). Remaining ties go to
/// the lowest id.
fn candidate_for_objective(kind: ObjectiveKind, name: &str, evidence: &[Evidence], quest_id: f64) -> Option<f64> {
    let evidence_kind = match kind {
        ObjectiveKind::Item => EvidenceKind::Item,
        ObjectiveKind::Monster => EvidenceKind::Npc,
        ObjectiveKind::Object => EvidenceKind::Object,
    };
    let wanted_name = normalize_name(name);
    let mut best: Option<(f64, u8)> = None;
    for item in evidence {
        if item.kind != evidence_kind || item.normalized_name != wanted_name {
            continue;
        }
        let rank = if item.required_for_quest_id == Some(quest_id) {
            3
        } else if item.preference.prefers(quest_id) {
            2
        } else if item.starts_quest_id == Some(quest_id) {
            0
        } else {
            1
        };
        if best.is_none_or(|(best_id, best_rank)| rank > best_rank || (rank == best_rank && item.id < best_id)) {
            best = Some((item.id, rank));
        }
    }
    best.map(|(id, _)| id)
}

pub fn observe(session: &Session) -> Vec<Observation<Vec<ObjectiveMatch>>> {
    let mut observations = Vec::new();
    let Some(objectives_root) = session.function(OBJECTIVES).filter(|root| !root.is_array()) else {
        return observations;
    };
    // TS collects the evidence before this check; it is pure, so collecting after is the same.
    let item_evidence: Vec<Evidence> = collect_loot_evidence(session).into_iter().chain(collect_link_evidence(session)).collect();
    let unit_evidence = collect_unit_evidence(session);

    for quest_id_key in param_keys(objectives_root) {
        let quest_id = js::number(&quest_id_key);
        if !quest_id.is_finite() {
            continue;
        }
        for entry in get_stream(session, OBJECTIVES, &[KeyRef::Str(&quest_id_key)]).unwrap_or(&[]) {
            let t = entry_t(entry);
            // Older traces may not contain GetLocale: no locale is the legacy enUS default.
            if locale_at(session, t).is_some_and(|locale| locale != "enUS") {
                continue;
            }

            // Unfiltered, so positions keep matching the API's objective indices.
            let objective_list = emulated_list(entry_v(entry)).unwrap_or_default();
            let mut matches = Vec::new();
            for (index, objective) in objective_list.into_iter().enumerate() {
                let Some(objective) = objective.filter(|o| o.as_object().is_some()) else {
                    continue;
                };
                let kind = ObjectiveKind::from_type(objective.get("type"));
                let text = objective.get("text").and_then(Value::as_str).filter(|text| !text.is_empty());
                let (Some(kind), Some(text)) = (kind, text) else { continue };
                let Some(name) = objective_name(text, kind) else { continue };

                let source = if kind == ObjectiveKind::Item { &item_evidence } else { &unit_evidence };
                if let Some(id) = candidate_for_objective(kind, &name, source, quest_id) {
                    matches.push(ObjectiveMatch {
                        kind,
                        name,
                        text: text.to_owned(),
                        id,
                        index,
                    });
                    continue;
                }
                let Some(short_name) = name_without_trailing_verb(&name, kind) else {
                    continue;
                };
                if let Some(id) = candidate_for_objective(kind, &short_name, source, quest_id) {
                    matches.push(ObjectiveMatch {
                        kind,
                        name: short_name,
                        text: text.to_owned(),
                        id,
                        index,
                    });
                }
            }

            if !matches.is_empty() {
                observations.push(Observation::new(quest_id as i64, matches, Confidence::Low, t));
            }
        }
    }
    observations
}

/// Objective index -> matched id -> sessions that matched it.
/// What the merge needs of a match: which objective (kind and position) and which id. The
/// matched name and text only explain the match; the fold keeps just this.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ObjectiveSlot {
    pub kind: ObjectiveKind,
    pub index: usize,
    pub id: Num,
}

impl ObjectiveMatch {
    pub fn slot(&self) -> ObjectiveSlot {
        ObjectiveSlot {
            kind: self.kind,
            index: self.index,
            id: Num(self.id),
        }
    }
}

/// One session's observations reduced to what the merge reads: per quest, each slot once (the
/// merge counts sessions per slot, so a slot seen again in the same session adds nothing), in
/// the order first seen. All `observations` must come from one session.
pub fn per_session(observations: Vec<Observation<Vec<ObjectiveMatch>>>) -> Vec<Observation<Vec<ObjectiveSlot>>> {
    let mut by_quest: IndexMap<i64, (Observation<()>, IndexSet<ObjectiveSlot>)> = IndexMap::new();
    for observation in observations {
        let slots = observation.value.iter().map(ObjectiveMatch::slot);
        let (_, seen) = by_quest
            .entry(observation.entity_id)
            .or_insert_with(|| (observation.clone().map(|_| ()), IndexSet::new()));
        seen.extend(slots);
    }
    by_quest
        .into_values()
        .map(|(first, slots)| first.map(|()| slots.into_iter().collect()))
        .collect()
}

type SessionsBySlot = IndexMap<usize, IndexMap<Num, IndexSet<SessionId>>>;

/// Merges matched objectives across observations into Questie's positional buckets.
///
/// Ids are ordered by their objective's index in the GetQuestObjectives result, since
/// unmatched objectives leave gaps in each observation's matches. Observations may resolve one
/// objective to different ids (creatures sharing a name), so each objective gets one id: the
/// one matched in the most sessions, lowest id on ties.
pub fn merge(observations: &[Observation<Vec<ObjectiveSlot>>]) -> QuestObjectivesValue {
    let (mut monsters, mut objects, mut items) = (SessionsBySlot::new(), SessionsBySlot::new(), SessionsBySlot::new());
    for observation in observations {
        for objective in &observation.value {
            let slots = match objective.kind {
                ObjectiveKind::Monster => &mut monsters,
                ObjectiveKind::Object => &mut objects,
                ObjectiveKind::Item => &mut items,
            };
            slots
                .entry(objective.index)
                .or_default()
                .entry(objective.id)
                .or_default()
                .insert(observation.session);
        }
    }

    let pick = |candidates: &IndexMap<Num, IndexSet<SessionId>>| -> f64 {
        let mut ranked: Vec<(&Num, &IndexSet<SessionId>)> = candidates.iter().collect();
        // Ids are finite, so total_cmp orders them like `a - b`.
        ranked.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0 .0.total_cmp(&b.0 .0)));
        ranked[0].0 .0
    };
    let ordered = |slots: SessionsBySlot| -> Vec<f64> {
        let mut slots: Vec<_> = slots.into_iter().collect();
        slots.sort_by_key(|(index, _)| *index);
        let ids: IndexSet<Num> = slots.iter().map(|(_, candidates)| Num(pick(candidates))).collect();
        ids.into_iter().map(|id| id.0).collect()
    };

    QuestObjectivesValue {
        creatures: ordered(monsters),
        objects: ordered(objects),
        items: ordered(items),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{packed, session_json, stream, with_session};
    use serde_json::{json, Value as Json};

    const ITEM_LINK: &str = "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r";

    fn observed(functions: Json) -> Vec<Observation<Vec<ObjectiveMatch>>> {
        with_session(&session_json(functions, vec![]), observe)
    }

    /// The matches of the first observation.
    fn first_matches(functions: Json) -> Vec<ObjectiveMatch> {
        observed(functions).into_iter().next().expect("an observation").value
    }

    fn first_id(functions: Json) -> f64 {
        first_matches(functions)[0].id
    }

    fn matched(kind: ObjectiveKind, name: &str, text: &str, id: f64, index: usize) -> ObjectiveMatch {
        ObjectiveMatch {
            kind,
            name: name.to_owned(),
            text: text.to_owned(),
            id,
            index,
        }
    }

    fn objectives(list: Json) -> Json {
        stream([(3.0, list)])
    }

    fn unit_name(name: &str) -> Json {
        packed(&[json!(name)])
    }

    #[test]
    fn matches_an_item_objective_with_suffix_progress_to_a_quest_item_link_and_slot_info() {
        let observations = observed(json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "item", "text": "Tough Wolf Meat: 0/8" }])) },
            "GetLootSlotLink": { "1": stream([(4.0, json!(ITEM_LINK))]) },
            "GetLootSlotInfo": { "1": stream([(4.0, json!({ "2": "Tough Wolf Meat", "7": true, "n": 10 }))]) },
        }));
        let observed: Vec<_> = observations.into_iter().map(|o| (o.entity_id, o.value, o.confidence, o.t)).collect();
        let expected = vec![matched(ObjectiveKind::Item, "Tough Wolf Meat", "Tough Wolf Meat: 0/8", 750.0, 0)];
        assert_eq!(observed, [(33, expected, Confidence::Low, 3.0)]);
    }

    #[test]
    fn pairs_keyed_loot_streams_by_slot_and_leaves_missing_info_streams_empty() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "item", "text": "Tough Wolf Meat: 0/8" }])) },
            "GetLootSlotLink": { "1": stream([(4.0, json!(ITEM_LINK))]), "2": stream([(5.0, json!(ITEM_LINK))]) },
            "GetLootSlotInfo": { "2": stream([(5.0, json!({ "2": "Tough Wolf Meat", "7": true, "n": 10 }))]) },
        }));
        assert_eq!(id, 750.0);
    }

    #[test]
    fn matches_monster_and_object_objectives_with_prefix_progress_and_kill_suffix_to_unit_name_and_guid() {
        let matches = first_matches(json!({
            "GetLocale": { "player": stream([(0.0, json!("enUS"))]) },
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([
                { "type": "monster", "text": "0/10 Kobold Vermin slain" },
                { "type": "object", "text": "0/1 Suspicious Cache" },
            ])) },
            "UnitGUID": {
                "npc": stream([(3.0, json!("Creature-0-5208-0-7-299-000123"))]),
                "target": stream([(3.0, json!("GameObject-0-5208-0-7-2843-0000399"))]),
            },
            "UnitName": {
                "npc": stream([(3.0, unit_name("Kobold Vermin"))]),
                "target": stream([(3.0, unit_name("Suspicious Cache"))]),
            },
        }));
        assert_eq!(
            matches,
            [
                matched(ObjectiveKind::Monster, "Kobold Vermin", "0/10 Kobold Vermin slain", 299.0, 0),
                matched(ObjectiveKind::Object, "Suspicious Cache", "0/1 Suspicious Cache", 2843.0, 1),
            ]
        );
    }

    #[test]
    fn keeps_the_api_index_of_a_matched_objective_that_follows_an_unmatched_one() {
        let matches = first_matches(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([
                { "type": "item", "text": "Twisted Water Totem: 0/5" },
                { "type": "item", "text": "Tough Wolf Meat: 0/8" },
            ])) },
            "GetLootSlotLink": { "1": stream([(4.0, json!(ITEM_LINK))]) },
        }));
        assert_eq!(matches, [matched(ObjectiveKind::Item, "Tough Wolf Meat", "Tough Wolf Meat: 0/8", 750.0, 1)]);
    }

    #[test]
    fn does_not_match_singular_and_plural_name_variations() {
        let observations = observed(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "monster", "text": "0/10 Kobold Workers slain" }])) },
            "UnitGUID": { "target": stream([(3.0, json!("Creature-0-5208-0-7-299-000123"))]) },
            "UnitName": { "target": stream([(3.0, unit_name("Kobold Worker"))]) },
        }));
        assert_eq!(observations, []);
    }

    #[test]
    fn matches_evidence_observed_anywhere_in_the_session() {
        // The kill happens long after the objective sample: no time window around t=3.
        let matches = first_matches(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "monster", "text": "0/1 Kobold Vermin slain" }])) },
            "UnitGUID": { "target": stream([(250.0, json!("Creature-0-5208-0-7-299-000123"))]) },
            "UnitName": { "target": stream([(250.0, unit_name("Kobold Vermin"))]) },
        }));
        assert_eq!(
            matches[0],
            matched(ObjectiveKind::Monster, "Kobold Vermin", "0/1 Kobold Vermin slain", 299.0, 0)
        );
    }

    #[test]
    fn matches_unit_evidence_attributed_to_another_quest() {
        // GetQuestID reported another quest at the kill; the exact name still matches.
        let matches = first_matches(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "monster", "text": "0/1 Kobold Vermin slain" }])) },
            "GetQuestID": stream([(3.0, json!(999))]),
            "UnitGUID": { "target": stream([(3.0, json!("Creature-0-5208-0-7-299-000123"))]) },
            "UnitName": { "target": stream([(3.0, unit_name("Kobold Vermin"))]) },
        }));
        assert_eq!(
            matches[0],
            matched(ObjectiveKind::Monster, "Kobold Vermin", "0/1 Kobold Vermin slain", 299.0, 0)
        );
    }

    #[test]
    fn prefers_same_quest_unit_evidence_over_cross_quest_evidence() {
        // Both units match; the one seen while quest 33's dialog showed (299) beats 300.
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "monster", "text": "0/1 Kobold Vermin slain" }])) },
            "GetQuestID": stream([(3.0, json!(33)), (5.0, json!(999))]),
            "UnitGUID": {
                "target": stream([(3.0, json!("Creature-0-5208-0-7-299-000123"))]),
                "npc": stream([(5.0, json!("Creature-0-5208-0-7-300-000456"))]),
            },
            "UnitName": {
                "target": stream([(3.0, unit_name("Kobold Vermin"))]),
                "npc": stream([(5.0, unit_name("Kobold Vermin"))]),
            },
        }));
        assert_eq!(id, 299.0);
    }

    #[test]
    fn falls_back_to_an_item_that_starts_the_quest_when_no_other_item_matches() {
        // GetLootSlotInfo: 7 = isQuestItem, 8 = questID of the quest the item starts, 9 = isActive.
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "item", "text": "Tough Wolf Meat: 0/8" }])) },
            "GetLootSlotLink": { "1": stream([(4.0, json!(ITEM_LINK))]) },
            "GetLootSlotInfo": { "1": stream([(4.0, json!({ "2": "Tough Wolf Meat", "7": false, "8": 33, "9": false, "n": 10 }))]) },
        }));
        assert_eq!(id, 750.0);
    }

    #[test]
    fn prefers_any_other_same_named_item_over_one_that_starts_the_quest() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "86613": objectives(json!([{ "type": "item", "text": "0/6 Excavation Tools" }])) },
            "GetLootSlotLink": { "1": stream([
                (1.0, json!("|cnIQ1:|Hitem:278049::::::::15:1487:::::::::|h[Excavation Tools]|h|r")),
                (5.0, json!("|cnIQ1:|Hitem:278398::::::::15:1487:::::::::|h[Excavation Tools]|h|r")),
            ]) },
            "GetLootSlotInfo": { "1": stream([
                (1.0, json!({ "2": "Excavation Tools", "7": false, "8": 86613, "9": false, "n": 10 })),
                (5.0, json!({ "2": "Excavation Tools", "7": false, "n": 10 })),
            ]) },
        }));
        assert_eq!(id, 278398.0);
    }

    #[test]
    fn prefers_a_quest_item_over_a_same_named_starter_item_with_a_lower_id() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "86613": objectives(json!([{ "type": "item", "text": "0/6 Excavation Tools" }])) },
            "GetLootSlotLink": { "1": stream([
                (1.0, json!("|cnIQ1:|Hitem:278049::::::::15:1487:::::::::|h[Excavation Tools]|h|r")),
                (5.0, json!("|cnIQ1:|Hitem:278398::::::::15:1487:::::::::|h[Excavation Tools]|h|r")),
            ]) },
            "GetLootSlotInfo": { "1": stream([
                (1.0, json!({ "2": "Excavation Tools", "7": false, "8": 86000, "9": false, "n": 10 })),
                (5.0, json!({ "2": "Excavation Tools", "7": true, "n": 10 })),
            ]) },
        }));
        assert_eq!(id, 278398.0);
    }

    #[test]
    fn matches_an_item_objective_to_a_required_item_of_the_turn_in_window() {
        let matches = first_matches(json!({
            "C_QuestLog.GetQuestObjectives": { "92109": objectives(json!([{ "type": "item", "text": "0/5 Empty Vial" }])) },
            "GetQuestID": stream([(9.0, json!(92109))]),
            "GetQuestItemLink": { "required": { "1": stream([(9.0, json!("|cnIQ1:|Hitem:3371::::::::10:1485::::::::::|h[Empty Vial]|h|r"))]) } },
        }));
        assert_eq!(matches, [matched(ObjectiveKind::Item, "Empty Vial", "0/5 Empty Vial", 3371.0, 0)]);
    }

    #[test]
    fn matches_an_item_objective_to_an_item_a_merchant_sells() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "92109": objectives(json!([{ "type": "item", "text": "0/5 Empty Vial" }])) },
            "GetMerchantItemLink": { "4": stream([(5.0, json!("|cnIQ1:|Hitem:3371::::::::10:1485::::::::::|h[Empty Vial]|h|r"))]) },
        }));
        assert_eq!(id, 3371.0);
    }

    #[test]
    fn prefers_a_required_item_of_the_quest_over_a_same_named_looted_item() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "item", "text": "Tough Wolf Meat: 0/8" }])) },
            "GetLootSlotLink": { "1": stream([(4.0, json!(ITEM_LINK))]) },
            "GetQuestID": stream([(9.0, json!(33))]),
            "GetQuestItemLink": { "required": { "1": stream([(9.0, json!("|Hitem:9999::::::::1::::::::::|h[Tough Wolf Meat]|h|r"))]) } },
        }));
        assert_eq!(id, 9999.0);
    }

    #[test]
    fn supports_legacy_flat_loot_streams_and_traces_without_get_locale() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "item", "text": "Tough Wolf Meat: 0/8" }])) },
            "GetLootSlotLink": stream([(4.0, json!(ITEM_LINK))]),
            "GetLootSlotInfo": stream([(4.0, json!({ "2": "Tough Wolf Meat", "7": true, "n": 10 }))]),
        }));
        assert_eq!(id, 750.0);
    }

    #[test]
    fn skips_non_en_us_objective_text() {
        let observations = observed(json!({
            "GetLocale": { "player": stream([(0.0, json!("deDE"))]) },
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "item", "text": "Wolfsfleisch: 0/8" }])) },
        }));
        assert_eq!(observations, []);
    }

    #[test]
    fn matches_a_kill_objective_that_ends_in_another_past_tense_verb() {
        let matches = first_matches(json!({
            "C_QuestLog.GetQuestObjectives": { "92465": objectives(json!([{ "type": "monster", "text": "0/6 Roiling Winds destroyed" }])) },
            "UnitGUID": { "target": stream([(3.0, json!("Creature-0-5208-0-7-251143-000123"))]) },
            "UnitName": { "target": stream([(3.0, unit_name("Roiling Winds"))]) },
        }));
        assert_eq!(
            matches,
            [matched(ObjectiveKind::Monster, "Roiling Winds", "0/6 Roiling Winds destroyed", 251143.0, 0)]
        );
    }

    #[test]
    fn prefers_the_full_objective_name_over_one_without_its_trailing_verb() {
        let id = first_id(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "object", "text": "0/1 Ancient Seed Planted" }])) },
            "UnitGUID": {
                "target": stream([(3.0, json!("GameObject-0-5208-0-7-100-0000399"))]),
                "npc": stream([(3.0, json!("GameObject-0-5208-0-7-200-0000399"))]),
            },
            "UnitName": {
                "target": stream([(3.0, unit_name("Ancient Seed"))]),
                "npc": stream([(3.0, unit_name("Ancient Seed Planted"))]),
            },
        }));
        assert_eq!(id, 200.0);
    }

    #[test]
    fn does_not_strip_a_trailing_slain_from_non_monster_objective_names() {
        let matches = first_matches(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([{ "type": "object", "text": "0/1 Bones of the Slain" }])) },
            "UnitGUID": { "target": stream([(3.0, json!("GameObject-0-5208-0-7-2843-0000399"))]) },
            "UnitName": { "target": stream([(3.0, unit_name("Bones of the Slain"))]) },
        }));
        assert_eq!(
            matches[0],
            matched(ObjectiveKind::Object, "Bones of the Slain", "0/1 Bones of the Slain", 2843.0, 0)
        );
    }

    #[test]
    fn skips_objective_types_that_never_carry_an_entity_id() {
        let observations = observed(json!({
            "C_QuestLog.GetQuestObjectives": { "33": objectives(json!([
                { "type": "log", "text": "Some flavor text" },
                { "type": "event", "text": "0/1 Something happened" },
                { "type": "", "text": "0/1 Untyped" },
            ])) },
        }));
        assert_eq!(observations, []);
    }

    /// Port of `__fixtures__/objectives.realTrace.fixture.ts`: a trimmed excerpt of a real
    /// session. Quest 92461 "Slay 8 Vuldren Juveniles in Thendal Grove.": the player targets a
    /// "Juvenile Vuldren" (creature 250873) well after the objective was first sampled at 0/8.
    fn objectives_real_trace_fixture() -> Json {
        let objective = |text: &str, fulfilled: u32| json!([{ "type": "monster", "numRequired": 8, "finished": false, "text": text, "objectiveType": 0, "numFulfilled": fulfilled }]);
        json!({
            "startedAt": 0,
            "startedAtPrecise": 0,
            "events": [],
            "functions": {
                "GetLocale": { "player": [{ "t": 0, "tp": 0.005454300029669, "v": "enUS" }] },
                "C_QuestLog.GetQuestObjectives": {
                    "92461": [
                        { "t": 31.988000000012, "tp": 31.720352599979, "v": objective("0/8 Juvenile Vuldren slain", 0) },
                        { "t": 297.72100000002, "tp": 297.2064575, "v": objective("1/8 Juvenile Vuldren slain", 1) },
                    ],
                },
                "UnitGUID": {
                    "target": [
                        // An unrelated NPC targeted earlier in the session (noise).
                        { "t": 288.20600000001, "tp": 287.69539800001, "v": "Creature-0-6782-2991-380-251368-000031C01F" },
                        // The quest-objective kill: targeted well after the 0/8 objective sample.
                        { "t": 296.804, "tp": 296.29263139999, "v": "Creature-0-6782-2991-380-250873-0000326AFF" },
                    ],
                },
                "UnitName": {
                    "target": [
                        { "t": 288.20600000001, "tp": 287.69539800001, "v": { "1": "Elatrell Featherlight", "n": 2 } },
                        { "t": 296.804, "tp": 296.29263139999, "v": { "1": "Juvenile Vuldren", "n": 2 } },
                    ],
                },
            },
        })
    }

    const JUVENILE_VULDREN_CREATURE_ID: f64 = 250873.0;
    const SLAY_VULDREN_QUEST_ID: i64 = 92461;

    #[test]
    fn matches_a_real_captured_objective_end_to_end() {
        let observations = with_session(&objectives_real_trace_fixture(), observe);
        // Both the 0/8 and 1/8 samples resolve to the same creature: the objective and evidence
        // times need not align.
        assert_eq!(observations.len(), 2);
        for observation in observations {
            assert_eq!(observation.entity_id, SLAY_VULDREN_QUEST_ID);
            let [only] = observation.value.as_slice() else {
                panic!("one match, got {:?}", observation.value)
            };
            assert_eq!(
                (only.kind, only.name.as_str(), only.id, only.index),
                (ObjectiveKind::Monster, "Juvenile Vuldren", JUVENILE_VULDREN_CREATURE_ID, 0)
            );
            assert!(only.text.contains("Juvenile Vuldren slain"), "text {:?}", only.text);
        }
    }

    #[test]
    fn objective_names_strip_progress_and_kill_suffixes_like_the_ts_regexes() {
        assert_eq!(
            objective_name(" 3 / 8  Juvenile Vuldren   SLAIN", ObjectiveKind::Monster).as_deref(),
            Some("Juvenile Vuldren")
        );
        assert_eq!(
            objective_name("Anaya's Pendant : 0 / 1 ", ObjectiveKind::Item).as_deref(),
            Some("Anaya's Pendant")
        );
        // Only a trailing counter after the last colon is progress.
        assert_eq!(
            objective_name("Note: 0/1 left over", ObjectiveKind::Item).as_deref(),
            Some("Note: 0/1 left over")
        );
        // \s+ is required before "slain".
        assert_eq!(objective_name("0/1 Unslain", ObjectiveKind::Monster).as_deref(), Some("Unslain"));
        assert_eq!(objective_name("0/1 ", ObjectiveKind::Object), None);
        assert_eq!(
            name_without_trailing_verb("Sickly Gazelle CURED", ObjectiveKind::Monster).as_deref(),
            Some("Sickly Gazelle")
        );
        // \S+ needs a character before "ed", and an item keeps its name.
        assert_eq!(name_without_trailing_verb("Roiling ed", ObjectiveKind::Monster), None);
        assert_eq!(name_without_trailing_verb("Cured Ham", ObjectiveKind::Item), None);
    }

    fn obs(value: Vec<ObjectiveMatch>, t: f64, session: u32) -> Observation<Vec<ObjectiveSlot>> {
        Observation {
            entity_id: 33,
            value: value.iter().map(ObjectiveMatch::slot).collect(),
            confidence: Confidence::Low,
            t,
            session: SessionId(session),
        }
    }

    fn item(id: f64, index: usize) -> ObjectiveMatch {
        matched(ObjectiveKind::Item, &format!("Item {id}"), &format!("Item {id}: 0/1"), id, index)
    }

    fn value(creatures: &[f64], objects: &[f64], items: &[f64]) -> QuestObjectivesValue {
        QuestObjectivesValue {
            creatures: creatures.to_vec(),
            objects: objects.to_vec(),
            items: items.to_vec(),
        }
    }

    #[test]
    fn merge_fills_only_the_buckets_that_have_data() {
        let matches = vec![
            matched(ObjectiveKind::Object, "Suspicious Cache", "0/1 Suspicious Cache", 2843.0, 0),
            matched(ObjectiveKind::Item, "Tough Wolf Meat", "Tough Wolf Meat: 0/8", 750.0, 1),
        ];
        assert_eq!(merge(&[obs(matches, 1.0, 0)]), value(&[], &[2843.0], &[750.0]));
    }

    #[test]
    fn merge_dedupes_ids_of_one_objective_seen_in_several_observations() {
        let matches = vec![matched(ObjectiveKind::Monster, "Kobold Vermin", "0/10 Kobold Vermin slain", 299.0, 0)];
        assert_eq!(merge(&[obs(matches.clone(), 1.0, 0), obs(matches, 2.0, 0)]), value(&[299.0], &[], &[]));
    }

    #[test]
    fn merge_emits_one_id_per_objective_when_observations_disagree() {
        let hecklefang = |id| matched(ObjectiveKind::Monster, "Hecklefang Stalker", "0/7 Hecklefang Stalker slain", id, 0);
        let stormhide = |id| matched(ObjectiveKind::Monster, "Stormhide", "0/7 Stormhide slain", id, 1);
        let result = merge(&[
            obs(vec![hecklefang(4128.0), stormhide(3456.0)], 1.0, 1),
            obs(vec![hecklefang(4128.0), stormhide(3238.0)], 2.0, 2),
            obs(vec![stormhide(3238.0)], 3.0, 3),
        ]);
        assert_eq!(result, value(&[4128.0, 3238.0], &[], &[]));
    }

    #[test]
    fn merge_prefers_the_lowest_id_seen_in_equally_many_sessions() {
        let result = merge(&[obs(vec![item(20.0, 0)], 1.0, 1), obs(vec![item(10.0, 0)], 2.0, 2)]);
        assert_eq!(result.items, [10.0]);
    }

    #[test]
    fn merge_counts_repeated_observations_of_one_session_once() {
        let result = merge(&[
            obs(vec![item(10.0, 0)], 1.0, 1),
            obs(vec![item(10.0, 0)], 2.0, 1),
            obs(vec![item(10.0, 0)], 3.0, 1),
            obs(vec![item(20.0, 0)], 4.0, 2),
            obs(vec![item(20.0, 0)], 5.0, 3),
        ]);
        assert_eq!(result.items, [20.0]);
    }

    #[test]
    fn merge_of_no_observations_is_empty() {
        assert_eq!(merge(&[]), QuestObjectivesValue::default());
    }

    #[test]
    fn merge_orders_ids_by_api_objective_index_not_numerically() {
        let result = merge(&[obs(vec![item(251291.0, 0), item(251284.0, 1), item(251261.0, 2)], 1.0, 0)]);
        assert_eq!(result.items, [251291.0, 251284.0, 251261.0]);
    }

    #[test]
    fn merge_keeps_api_order_when_observations_matched_only_some_objectives() {
        // Water(0), Earth(1), Fire(2), Air(3): evidence for each totem shows up at different times.
        let result = merge(&[
            obs(vec![item(274046.0, 2)], 1.0, 0),
            obs(vec![item(274044.0, 1), item(274046.0, 2), item(274047.0, 3)], 2.0, 0),
            obs(vec![item(274045.0, 0), item(274044.0, 1), item(274046.0, 2), item(274047.0, 3)], 3.0, 1),
        ]);
        assert_eq!(result.items, [274045.0, 274044.0, 274046.0, 274047.0]);
    }

    #[test]
    fn merge_keeps_api_order_for_each_objective_type_independently() {
        let matches = vec![
            matched(ObjectiveKind::Monster, "Mob B", "0/1 Mob B slain", 200.0, 0),
            matched(ObjectiveKind::Object, "Obj A", "0/1 Obj A", 100.0, 1),
            matched(ObjectiveKind::Monster, "Mob A", "0/1 Mob A slain", 100.0, 2),
            matched(ObjectiveKind::Item, "Item B", "Item B: 0/1", 300.0, 3),
            matched(ObjectiveKind::Object, "Obj B", "0/1 Obj B", 200.0, 4),
            matched(ObjectiveKind::Item, "Item A", "Item A: 0/1", 100.0, 5),
        ];
        assert_eq!(merge(&[obs(matches, 1.0, 0)]), value(&[200.0, 100.0], &[100.0, 200.0], &[300.0, 100.0]));
    }

    #[test]
    fn per_session_keeps_each_slot_of_a_quest_once_and_merges_alike() {
        let at = |entity_id: i64, matches: Vec<ObjectiveMatch>, t: f64| Observation {
            entity_id,
            value: matches,
            confidence: Confidence::Low,
            t,
            session: SessionId::UNSET,
        };
        let observations = vec![
            at(33, vec![item(750.0, 0), item(751.0, 1)], 1.0),
            at(44, vec![item(900.0, 0)], 2.0),
            at(33, vec![item(751.0, 1), item(752.0, 1)], 3.0),
        ];
        let reduced = per_session(observations.clone());
        let slots = |o: &Observation<Vec<ObjectiveSlot>>| (o.entity_id, o.t, o.value.iter().map(|s| (s.index, s.id.0)).collect::<Vec<_>>());
        assert_eq!(
            reduced.iter().map(slots).collect::<Vec<_>>(),
            [(33, 1.0, vec![(0, 750.0), (1, 751.0), (1, 752.0)]), (44, 2.0, vec![(0, 900.0)])]
        );
        // The merge cannot tell: it counts sessions per slot.
        let quest_33: Vec<_> = observations
            .iter()
            .filter(|o| o.entity_id == 33)
            .map(|o| o.clone().map(|m| m.iter().map(ObjectiveMatch::slot).collect()))
            .collect();
        assert_eq!(merge(&reduced[..1]), merge(&quest_33));
    }
}
