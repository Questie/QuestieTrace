//! Turns materialized QuestieDB rows ([`super::materialize`]) into the catalog and ground-truth
//! contracts ([`QuestCatalog`], [`GroundTruth`]).
//!
//! Layering already happened in QuestieDB's registry. What is decided here:
//! - Quests a Dynamic Correction touches have one row per faction x class view. List values
//!   (starters, finishers, relations) are the union over views, because each view is what some
//!   real character sees. Other fields take the first view that has a value, so a requirement
//!   one view has is never dropped: a requirement-free quest looks takeable by every character,
//!   and signals would count its absence from offer lists as evidence.
//! - Questie treats 0 and empty lists as "no value", so both are dropped (when a row is read, see
//!   [`QuestRow`]).
//! - Ground-truth tiers follow provenance, not quest id: inherited Era quests (data/Forever and
//!   Forever/legacy/) versus Forever-new quests.

use super::materialize::MaterializedQuests;
use crate::model::{CatalogQuest, Finishers, GroundTruth, GroundTruthTiers, QuestCatalog, QuestId, RelationField, RelationSet, Starters};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The catalog's own Questie quest fields, as `forever-quests.lua` is asked for them.
const CATALOG_FIELDS: [&str; 12] = [
    "name",
    "startedBy",
    "finishedBy",
    "requiredLevel",
    "questLevel",
    "requiredRaces",
    "requiredClasses",
    "requiredSkill",
    "requiredMinRep",
    "requiredMaxRep",
    "requiredSpell",
    "specialFlags",
];

/// Every Questie quest field the catalog step asks QuestieDB for.
pub fn exported_fields() -> Vec<&'static str> {
    CATALOG_FIELDS.into_iter().chain(RelationField::ALL.map(RelationField::name)).collect()
}

/// One materialized row (a quest's static row or one of its faction x class views), normalized
/// as Questie reads it: 0, `{0, 0}` and empty lists are no value, null holes and values of the
/// wrong type are skipped.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(from = "Map<String, Value>")]
pub struct QuestRow {
    pub name: Option<String>,
    pub quest_level: Option<i64>,
    pub required_level: Option<i64>,
    pub required_races: Option<u64>,
    pub required_classes: Option<u64>,
    pub required_skill: Option<(i64, i64)>,
    pub required_min_rep: Option<(i64, i64)>,
    pub required_max_rep: Option<(i64, i64)>,
    pub required_spell: Option<i64>,
    pub special_flags: Option<i64>,
    pub starters: Starters,
    pub finishers: Finishers,
    /// Only the fields that have targets.
    pub relations: RelationSet,
}

impl From<Map<String, Value>> for QuestRow {
    fn from(row: Map<String, Value>) -> QuestRow {
        let field = |name: &str| row.get(name).unwrap_or(&Value::Null);
        // Questie's startedBy / finishedBy are positional groups: {npcs?, objects?, items?}.
        let givers = |name: &str, group: usize| field(name).get(group).map(id_list).unwrap_or_default();
        QuestRow {
            name: field("name").as_str().map(str::to_owned),
            quest_level: field("questLevel").as_i64(),
            required_level: field("requiredLevel").as_i64(),
            required_races: non_zero(field("requiredRaces")).and_then(|value| u64::try_from(value).ok()),
            required_classes: non_zero(field("requiredClasses")).and_then(|value| u64::try_from(value).ok()),
            required_skill: pair(field("requiredSkill")),
            required_min_rep: pair(field("requiredMinRep")),
            required_max_rep: pair(field("requiredMaxRep")),
            required_spell: non_zero(field("requiredSpell")),
            special_flags: non_zero(field("specialFlags")),
            starters: Starters {
                npcs: givers("startedBy", 0),
                objects: givers("startedBy", 1),
                items: givers("startedBy", 2),
            },
            finishers: Finishers {
                npcs: givers("finishedBy", 0),
                objects: givers("finishedBy", 1),
            },
            relations: RelationField::ALL
                .into_iter()
                .map(|relation| (relation, relation_targets(relation, field(relation.name()))))
                .filter(|(_, targets)| !targets.is_empty())
                .collect(),
        }
    }
}

fn non_zero(value: &Value) -> Option<i64> {
    value.as_i64().filter(|&number| number != 0)
}

/// Non-zero ids from a Questie id list.
fn id_list(value: &Value) -> Vec<QuestId> {
    value.as_array().map(|ids| ids.iter().filter_map(non_zero).collect()).unwrap_or_default()
}

/// Questie pairs such as requiredSkill {skillId, value}; {0, 0} reads back as nil.
fn pair(value: &Value) -> Option<(i64, i64)> {
    match value.as_array()?.as_slice() {
        [first, second] => Some((first.as_i64()?, second.as_i64()?)).filter(|&pair| pair != (0, 0)),
        _ => None,
    }
}

/// Targets of one relation field. Scalars become one-element lists.
fn relation_targets(field: RelationField, value: &Value) -> Vec<QuestId> {
    if field.is_scalar() {
        return non_zero(value).into_iter().collect();
    }
    // A negative preQuestGroup entry still means that quest, only without the exclusiveTo
    // substitute (Questie's IsPreQuestGroupFulfilled), so the edge target is its absolute value.
    id_list(value).into_iter().map(i64::abs).collect()
}

fn push_unique(target: &mut Vec<i64>, values: &[i64]) {
    for &value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

/// The union of one list over `rows`, in first-seen order.
fn union(rows: &[QuestRow], list: impl Fn(&QuestRow) -> &[i64]) -> Vec<i64> {
    let mut ids = Vec::new();
    for row in rows {
        push_unique(&mut ids, list(row));
    }
    ids
}

/// Relationship values for a quest across its views. Empty fields are omitted.
pub fn relation_set_of(rows: &[QuestRow]) -> RelationSet {
    RelationField::ALL
        .into_iter()
        .map(|field| (field, union(rows, |row| row.relations.get(&field).map_or(&[], Vec::as_slice))))
        .filter(|(_, targets)| !targets.is_empty())
        .collect()
}

/// The first view's value, so one view's requirement is never dropped.
fn first<T: Clone>(rows: &[QuestRow], value: impl Fn(&QuestRow) -> &Option<T>) -> Option<T> {
    rows.iter().find_map(|row| value(row).clone())
}

pub fn catalog_quest_of(id: QuestId, rows: &[QuestRow]) -> CatalogQuest {
    CatalogQuest {
        id,
        name: first(rows, |row| &row.name),
        quest_level: first(rows, |row| &row.quest_level),
        required_level: first(rows, |row| &row.required_level),
        required_races: first(rows, |row| &row.required_races),
        required_classes: first(rows, |row| &row.required_classes),
        required_skill: first(rows, |row| &row.required_skill),
        required_min_rep: first(rows, |row| &row.required_min_rep),
        required_max_rep: first(rows, |row| &row.required_max_rep),
        required_spell: first(rows, |row| &row.required_spell),
        special_flags: first(rows, |row| &row.special_flags),
        starters: Starters {
            npcs: union(rows, |row| &row.starters.npcs),
            objects: union(rows, |row| &row.starters.objects),
            items: union(rows, |row| &row.starters.items),
        },
        finishers: Finishers {
            npcs: union(rows, |row| &row.finishers.npcs),
            objects: union(rows, |row| &row.finishers.objects),
        },
    }
}

pub fn build_catalog(materialized: &MaterializedQuests, generated_at: &str) -> QuestCatalog {
    let quests = materialized
        .quest_ids()
        .filter_map(|id| {
            let rows = materialized.rows_of(id);
            (!rows.is_empty()).then(|| (id, catalog_quest_of(id, rows)))
        })
        .collect();
    QuestCatalog {
        generated_at: generated_at.to_owned(),
        sources: materialized.sources.clone(),
        quests,
    }
}

fn has_authored_relation(materialized: &MaterializedQuests, id: QuestId) -> bool {
    let fields = materialized.authored_fields.get(&id).map_or(&[][..], Vec::as_slice);
    fields.iter().any(|name| RelationField::ALL.iter().any(|field| field.name() == name))
}

/// - authoredForever: Forever-new quests whose relations an authored provider sets, adds to or
///   removes from, with the effective values (possibly {} when the authored fix deletes them).
/// - inheritedClassic: every inherited Era quest Forever loads, whatever its id, {} included so
///   "no relation" is known. Authored fixes on these quests show up as their effective values.
pub fn build_ground_truth(materialized: &MaterializedQuests, generated_at: &str) -> GroundTruth {
    let inherited: BTreeSet<QuestId> = materialized.inherited_ids.iter().copied().collect();
    let mut authored_forever = BTreeMap::new();
    let mut inherited_classic = BTreeMap::new();
    for id in materialized.quest_ids() {
        let rows = materialized.rows_of(id);
        if rows.is_empty() {
            continue;
        }
        if inherited.contains(&id) {
            inherited_classic.insert(id, relation_set_of(rows));
        } else if has_authored_relation(materialized, id) {
            authored_forever.insert(id, relation_set_of(rows));
        }
    }
    GroundTruth {
        generated_at: generated_at.to_owned(),
        sources: materialized.sources.clone(),
        tiers: GroundTruthTiers {
            authored_forever,
            inherited_classic,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(value: Value) -> QuestRow {
        serde_json::from_value(value).unwrap()
    }

    fn relations(entries: &[(RelationField, &[QuestId])]) -> RelationSet {
        entries.iter().map(|&(field, targets)| (field, targets.to_vec())).collect()
    }

    #[test]
    fn relation_set_turns_scalars_into_lists_and_drops_questies_empty_values() {
        let rows = [row(json!({
            "nextQuestInChain": 5, "parentQuest": 0, "preQuestSingle": [], "exclusiveTo": [7, 0, 8], "preQuestGroup": [3, -4]
        }))];
        use RelationField::*;
        assert_eq!(
            relation_set_of(&rows),
            relations(&[(NextQuestInChain, &[5]), (ExclusiveTo, &[7, 8]), (PreQuestGroup, &[3, 4])])
        );
    }

    #[test]
    fn relation_set_unions_the_targets_of_faction_and_class_views() {
        let rows = [
            row(json!({ "nextQuestInChain": 3370, "preQuestSingle": [1] })),
            row(json!({ "nextQuestInChain": 3369, "preQuestSingle": [1] })),
        ];
        use RelationField::*;
        assert_eq!(relation_set_of(&rows), relations(&[(NextQuestInChain, &[3370, 3369]), (PreQuestSingle, &[1])]));
    }

    #[test]
    fn catalog_quest_unions_givers_and_never_drops_a_requirement_only_some_views_have() {
        let quest = catalog_quest_of(
            7562,
            &[
                row(
                    json!({ "name": "Mor'zul", "questLevel": 58, "requiredLevel": 60, "requiredRaces": 0, "requiredSkill": [0, 0],
                            "startedBy": [[5520, 6382], null, [9]], "finishedBy": [[14436]] }),
                ),
                row(
                    json!({ "name": "Mor'zul", "questLevel": 58, "requiredLevel": 55, "requiredRaces": 0, "requiredSkill": [186, 100],
                            "startedBy": [[5753, 5520]], "finishedBy": [[14436]] }),
                ),
            ],
        );
        assert_eq!(
            quest,
            CatalogQuest {
                id: 7562,
                name: Some("Mor'zul".into()),
                quest_level: Some(58),
                required_level: Some(60),
                required_skill: Some((186, 100)),
                starters: Starters {
                    npcs: vec![5520, 6382, 5753],
                    objects: vec![],
                    items: vec![9]
                },
                finishers: Finishers {
                    npcs: vec![14436],
                    objects: vec![]
                },
                ..CatalogQuest::default()
            }
        );
    }

    #[test]
    fn ground_truth_tiers_follow_provenance_not_quest_ids() {
        let quests = [
            (100, json!({})),
            (490, json!({})),
            (65593, json!({ "nextQuestInChain": 65594 })),
            (9666, json!({ "preQuestSingle": [100] })),
            (9667, json!({ "requiredRaces": 2 })),
            (9668, json!({ "nextQuestInChain": 9666 })),
            (9669, json!({})),
        ];
        let authored = [(490, "name"), (9666, "preQuestSingle"), (9667, "requiredRaces"), (9669, "exclusiveTo")];
        let materialized = MaterializedQuests {
            quests: quests.into_iter().map(|(id, value)| (id, row(value))).collect(),
            authored_fields: authored.into_iter().map(|(id, field)| (id, vec![field.to_owned()])).collect(),
            inherited_ids: vec![100, 65593],
            ..MaterializedQuests::default()
        };
        let tiers = build_ground_truth(&materialized, "").tiers;
        // 100: no relations is still known. 65593: inherited from a legacy fix despite its id.
        assert_eq!(
            tiers.inherited_classic,
            BTreeMap::from([(100, RelationSet::new()), (65593, relations(&[(RelationField::NextQuestInChain, &[65594])]))])
        );
        // 490: Forever-new despite its id, no authored relation. 9667: authored, but not a relation.
        // 9668: relation from elsewhere. 9669: authored deletion.
        assert_eq!(
            tiers.authored_forever,
            BTreeMap::from([(9666, relations(&[(RelationField::PreQuestSingle, &[100])])), (9669, RelationSet::new())])
        );
    }

    #[test]
    fn dynamic_views_replace_the_static_row() {
        let materialized = MaterializedQuests {
            quests: BTreeMap::from([(10, row(json!({ "nextQuestInChain": 5 })))]),
            dynamic_views: BTreeMap::from([(10, vec![row(json!({ "nextQuestInChain": 6 })), row(json!({ "nextQuestInChain": 7 }))])]),
            inherited_ids: vec![10],
            ..MaterializedQuests::default()
        };
        let truth = build_ground_truth(&materialized, "");
        assert_eq!(truth.tiers.inherited_classic[&10], relations(&[(RelationField::NextQuestInChain, &[6, 7])]));
    }
}
