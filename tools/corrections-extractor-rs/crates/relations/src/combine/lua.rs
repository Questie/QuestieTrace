//! The Questie corrections module ForeverQuestRelationTraces: every accepted relation, on
//! Forever-new and inherited Classic quests alike, in the layout of the field extractor's
//! corrections modules. QuestieDB loads it after the other trace modules and before authored
//! corrections, so authored fixes still win field by field.
//!
//! Code lines are exactly what changes QuestieDB's data:
//! - List fields keep QuestieDB's targets and add the accepted ones ("add, don't delete"): the
//!   traces see some paths and not others, so a target they never saw is no evidence against it.
//! - Single-value fields (nextQuestInChain and the like) take the accepted value.
//! - Prerequisites are one slot over two fields: Questie ignores preQuestGroup whenever
//!   preQuestSingle is set. See [`prerequisite_fields`] for how accepted prerequisites merge.
//!
//! Everything else the traces accept is written commented out, saying why, so whoever authors
//! corrections sees the traces' whole view next to QuestieDB's:
//! - already in QuestieDB: nothing to add. Writing it anyway would also round-trip values the
//!   ground truth normalizes (negative preQuestGroup entries lose their sign).
//! - an authored fix or a Dynamic Correction sets the field (for prerequisites: either field of
//!   the slot); both override this module.
//! - prerequisites read as OR where QuestieDB has an AND list: OR is resolution's default, not
//!   evidence against AND.
//! - a new nextQuestInChain to a quest whose prerequisites leave this one out. Questie hides a quest
//!   once its next quest is done or in the log, so characters who reached the next quest another
//!   way would lose this one.
//! - a new nextQuestInChain a signal saw contradicted more often than supported.
//!
//! A quest with nothing to apply is written commented out as a whole, so the module never creates
//! an empty row for it.
//!
//! Like authored corrections, each quest's opening line carries its name, here with its Wowhead
//! page. Each field is preceded by the name and page of every quest it names, and ends with an
//! `-- Assumption:` saying why it is believed (see [`super::assumptions`]).

use super::assumptions::assumption;
use super::edges::Truth;
use super::views::QuestView;
use crate::model::{QuestCatalog, QuestId, RelationField, RelationSet};
use crate::text::is_js_whitespace;
use indexmap::IndexMap;
use std::collections::BTreeMap;

pub const LUA_MODULE_NAME: &str = "ForeverQuestRelationTraces";

const PREREQUISITES: [RelationField; 2] = [RelationField::PreQuestSingle, RelationField::PreQuestGroup];

/// Why an accepted relation is written commented out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotApplied {
    /// QuestieDB already has every accepted target (or the same single value).
    AlreadyInQuestieDb,
    /// An authored fix sets the field, or for prerequisites either field of the slot.
    AuthoredFix,
    /// A Dynamic Correction (per faction or class) sets the field, or either prerequisite field.
    DynamicCorrection,
    /// Prerequisites read as OR where QuestieDB requires all of `required` (preQuestGroup).
    QuestieDbRequiresAll { required: Vec<QuestId> },
    /// A new chain to `next`, whose prerequisites as QuestieDB loads them (`required`) leave this quest out.
    NextDoesNotRequire { next: QuestId, required: Vec<QuestId> },
    /// A new chain one signal saw contradicted more often than supported.
    Contradicted { signal: String, support: u32, contradict: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Written {
    /// Code: QuestieDB's targets plus the accepted ones, or the accepted single value.
    Applied,
    /// Code, `{}`: clears QuestieDB's preQuestSingle so the accepted preQuestGroup applies.
    Cleared,
    /// Commented out: the accepted values, not applied.
    Shown(NotApplied),
}

impl Written {
    pub fn is_code(&self) -> bool {
        !matches!(self, Written::Shown(_))
    }
}

/// One field the module writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaField {
    pub field: RelationField,
    /// Applied: the value written. Shown: the accepted values. Cleared: empty.
    pub values: Vec<QuestId>,
    pub written: Written,
}

impl LuaField {
    fn applied(field: RelationField, values: Vec<QuestId>) -> LuaField {
        LuaField {
            field,
            values,
            written: Written::Applied,
        }
    }

    fn shown(field: RelationField, values: &[QuestId], why: NotApplied) -> LuaField {
        LuaField {
            field,
            values: values.to_vec(),
            written: Written::Shown(why),
        }
    }

    fn cleared(field: RelationField) -> LuaField {
        LuaField {
            field,
            values: Vec::new(),
            written: Written::Cleared,
        }
    }
}

/// Per quest, the fields the module writes, in [`RelationField::ALL`] order. Scalar fields hold
/// their one value.
pub type LuaRecords = BTreeMap<QuestId, Vec<LuaField>>;

pub struct RecordSources<'a> {
    /// Accepted relations per quest.
    pub relations: &'a BTreeMap<QuestId, RelationSet>,
    /// What QuestieDB loads today for authored Forever-new and inherited quests (before this
    /// module), merged over every faction x class view.
    pub truth: &'a Truth<'a>,
    /// Relation fields authored fixes set, per quest.
    pub authored_fields: &'a BTreeMap<QuestId, Vec<RelationField>>,
    /// Relation fields Dynamic Corrections set, per quest.
    pub dynamic_fields: &'a BTreeMap<QuestId, Vec<RelationField>>,
    /// The claims behind each accepted chain.
    pub views: &'a IndexMap<QuestId, QuestView<'a>>,
}

impl RecordSources<'_> {
    fn reference(&self, quest_id: QuestId, field: RelationField) -> &[QuestId] {
        values_of(self.truth.get(&quest_id).copied(), field)
    }

    /// Why a later layer keeps this module's value for `fields` from applying, if one does.
    fn overridden(&self, quest_id: QuestId, fields: &[RelationField]) -> Option<NotApplied> {
        let sets = |by_quest: &BTreeMap<QuestId, Vec<RelationField>>| by_quest.get(&quest_id).is_some_and(|set| fields.iter().any(|field| set.contains(field)));
        if sets(self.authored_fields) {
            Some(NotApplied::AuthoredFix)
        } else if sets(self.dynamic_fields) {
            Some(NotApplied::DynamicCorrection)
        } else {
            None
        }
    }
}

fn values_of(set: Option<&RelationSet>, field: RelationField) -> &[QuestId] {
    set.and_then(|set| set.get(&field)).map_or(&[][..], Vec::as_slice)
}

fn is_subset(values: &[QuestId], of: &[QuestId]) -> bool {
    values.iter().all(|value| of.contains(value))
}

/// Both lists' targets, sorted and without repeats.
fn union(reference: &[QuestId], accepted: &[QuestId]) -> Vec<QuestId> {
    let mut values: Vec<QuestId> = reference.iter().chain(accepted).copied().collect();
    values.sort_unstable();
    values.dedup();
    values
}

pub fn relation_records(sources: &RecordSources) -> LuaRecords {
    let mut records: LuaRecords = sources
        .relations
        .iter()
        .map(|(&quest_id, accepted)| (quest_id, quest_fields(quest_id, accepted, sources)))
        .filter(|(_, fields)| !fields.is_empty())
        .collect();
    // A chain is judged by the next quest's prerequisites as QuestieDB will load them, so only
    // once every quest's own fields are decided.
    let mut not_applied = Vec::new();
    for (&quest_id, fields) in &records {
        for (index, field) in fields.iter().enumerate() {
            if field.field == RelationField::NextQuestInChain && field.written == Written::Applied {
                if let Some(why) = chain_problem(quest_id, field.values[0], &records, sources) {
                    not_applied.push((quest_id, index, why));
                }
            }
        }
    }
    for (quest_id, index, why) in not_applied {
        if let Some(field) = records.get_mut(&quest_id).and_then(|fields| fields.get_mut(index)) {
            field.written = Written::Shown(why);
        }
    }
    records
}

fn quest_fields(quest_id: QuestId, accepted: &RelationSet, sources: &RecordSources) -> Vec<LuaField> {
    let mut fields = prerequisite_fields(quest_id, accepted, sources);
    for field in RelationField::ALL.into_iter().filter(|field| !PREREQUISITES.contains(field)) {
        let (accepted, reference) = (values_of(Some(accepted), field), sources.reference(quest_id, field));
        if accepted.is_empty() {
            continue;
        }
        // A single value is new unless it is QuestieDB's one value.
        let nothing_new = if field.is_scalar() {
            reference == accepted
        } else {
            is_subset(accepted, reference)
        };
        fields.push(if nothing_new {
            LuaField::shown(field, accepted, NotApplied::AlreadyInQuestieDb)
        } else if let Some(why) = sources.overridden(quest_id, &[field]) {
            LuaField::shown(field, accepted, why)
        } else if field.is_scalar() {
            LuaField::applied(field, accepted.to_vec())
        } else {
            LuaField::applied(field, union(reference, accepted))
        });
    }
    fields
}

/// The prerequisite slot. Resolution reads a quest's prerequisites as AND (preQuestGroup) only
/// when every one has trusted AND evidence; anything else, including a single prerequisite, comes
/// out as preQuestSingle. So:
/// - nothing new (every accepted target is in either QuestieDB field): shown;
/// - read as AND: extends QuestieDB's AND list; replaces an OR list, clearing preQuestSingle;
/// - read as OR: extends QuestieDB's OR list or starts one. Against an AND list, one prerequisite
///   (the same under either kind) joins it; several are shown, not applied.
fn prerequisite_fields(quest_id: QuestId, accepted: &RelationSet, sources: &RecordSources) -> Vec<LuaField> {
    let (accepted_single, accepted_group) = (
        values_of(Some(accepted), RelationField::PreQuestSingle),
        values_of(Some(accepted), RelationField::PreQuestGroup),
    );
    if accepted_single.is_empty() && accepted_group.is_empty() {
        return Vec::new();
    }
    let shown = |why: NotApplied| -> Vec<LuaField> {
        PREREQUISITES
            .into_iter()
            .map(|field| (field, values_of(Some(accepted), field)))
            .filter(|(_, values)| !values.is_empty())
            .map(|(field, values)| LuaField::shown(field, values, why.clone()))
            .collect()
    };
    let (single, group) = (
        sources.reference(quest_id, RelationField::PreQuestSingle),
        sources.reference(quest_id, RelationField::PreQuestGroup),
    );
    let accepted_all: Vec<QuestId> = accepted_single.iter().chain(accepted_group).copied().collect();
    if accepted_all.iter().all(|target| single.contains(target) || group.contains(target)) {
        return shown(NotApplied::AlreadyInQuestieDb);
    }
    if let Some(why) = sources.overridden(quest_id, &PREREQUISITES) {
        return shown(why);
    }
    if !accepted_group.is_empty() {
        let mut fields = Vec::new();
        if !single.is_empty() {
            fields.push(LuaField::cleared(RelationField::PreQuestSingle));
        }
        fields.push(LuaField::applied(RelationField::PreQuestGroup, union(group, &accepted_all)));
        return fields;
    }
    if single.is_empty() && !group.is_empty() {
        return if accepted_single.len() == 1 {
            vec![LuaField::applied(RelationField::PreQuestGroup, union(group, accepted_single))]
        } else {
            shown(NotApplied::QuestieDbRequiresAll { required: group.to_vec() })
        };
    }
    vec![LuaField::applied(RelationField::PreQuestSingle, union(single, accepted_single))]
}

/// Why a new chain from `quest_id` to `next` must not apply, if it must not.
fn chain_problem(quest_id: QuestId, next: QuestId, records: &LuaRecords, sources: &RecordSources) -> Option<NotApplied> {
    let claims = sources
        .views
        .get(&quest_id)
        .and_then(|view| {
            view.edges
                .iter()
                .find(|edge| edge.is_written() && edge.field == RelationField::NextQuestInChain && edge.target == next)
        })
        .map_or(&[][..], |edge| edge.claims);
    if let Some(claim) = claims
        .iter()
        .filter(|claim| claim.contradict > claim.support)
        .max_by_key(|claim| claim.contradict)
    {
        return Some(NotApplied::Contradicted {
            signal: claim.signal().to_owned(),
            support: claim.support,
            contradict: claim.contradict,
        });
    }
    let required = loaded_prerequisites(next, records, sources);
    (!required.is_empty() && !required.contains(&quest_id)).then_some(NotApplied::NextDoesNotRequire { next, required })
}

/// A quest's prerequisites as QuestieDB will load them with this module: preQuestSingle when set,
/// else preQuestGroup.
fn loaded_prerequisites(quest_id: QuestId, records: &LuaRecords, sources: &RecordSources) -> Vec<QuestId> {
    let written = records.get(&quest_id).map_or(&[][..], Vec::as_slice);
    let loaded = |field: RelationField| -> Vec<QuestId> {
        match written
            .iter()
            .find(|written| written.field == field)
            .map(|written| (&written.written, &written.values))
        {
            Some((Written::Applied, values)) => values.clone(),
            Some((Written::Cleared, _)) => Vec::new(),
            _ => sources.reference(quest_id, field).to_vec(),
        }
    };
    let single = loaded(RelationField::PreQuestSingle);
    if single.is_empty() {
        loaded(RelationField::PreQuestGroup)
    } else {
        single
    }
}

pub struct LuaHeader<'h> {
    pub candidate_files: &'h [&'h str],
    /// Largest inputCount among the candidate files: the episodes the trace signals read.
    pub session_count: usize,
    pub min_score: f64,
    pub generated_at: &'h str,
    /// File name of the review report combine publishes next to this module.
    pub review_file: &'h str,
}

pub fn wowhead_quest_url(quest_id: QuestId) -> String {
    format!("https://wowhead.com/forever/quest={quest_id}")
}

/// Whitespace runs (including line breaks) collapsed to one space, so a comment can never spill
/// into code. None when nothing is left.
fn flat_comment(text: &str) -> Option<String> {
    let flat = text.split(is_js_whitespace).filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ");
    (!flat.is_empty()).then_some(flat)
}

fn trailing_comment(text: Option<&str>) -> String {
    text.and_then(flat_comment).map_or_else(String::new, |flat| format!(" -- {flat}"))
}

fn lua_value(field: RelationField, values: &[QuestId]) -> String {
    if field.is_scalar() {
        values[0].to_string()
    } else {
        let values: Vec<String> = values.iter().map(ToString::to_string).collect();
        format!("{{{}}}", values.join(","))
    }
}

fn joined(ids: &[QuestId]) -> String {
    ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

fn not_applied_reason(field: RelationField, why: &NotApplied) -> String {
    let what = if PREREQUISITES.contains(&field) { "the prerequisites" } else { "this field" };
    match why {
        NotApplied::AlreadyInQuestieDb => "already in QuestieDB".to_owned(),
        NotApplied::AuthoredFix => format!("not applied: an authored fix sets {what}"),
        NotApplied::DynamicCorrection => format!("not applied: a Dynamic Correction (per faction or class) sets {what}"),
        NotApplied::QuestieDbRequiresAll { required } => format!(
            "not applied: QuestieDB requires all of {} (preQuestGroup) and the traces have no AND evidence for these",
            joined(required)
        ),
        NotApplied::NextDoesNotRequire { next, required } => format!(
            "not applied: {next} requires {}, not this quest, and Questie hides a quest once its next quest is done or in the log",
            joined(required)
        ),
        NotApplied::Contradicted { signal, support, contradict } => {
            format!("not applied: {signal} saw it contradicted by {contradict} characters against {support}")
        }
    }
}

/// The comment after a written field.
fn field_comment(written: &LuaField, view: Option<&QuestView>) -> Option<String> {
    let because = view.and_then(|view| assumption(view, written.field));
    match &written.written {
        Written::Cleared if written.field == RelationField::PreQuestSingle => {
            Some("cleared so the accepted preQuestGroup applies: Questie ignores preQuestGroup while preQuestSingle is set".to_owned())
        }
        Written::Cleared => Some("cleared: the accepted preQuestSingle replaces it".to_owned()),
        Written::Applied => {
            let accepted = view.map_or(&[][..], |view| view.relation(written.field));
            let kept: Vec<QuestId> = written.values.iter().copied().filter(|value| !accepted.contains(value)).collect();
            let kept = (!kept.is_empty()).then(|| format!("{} kept from QuestieDB", joined(&kept)));
            let parts: Vec<String> = because.into_iter().chain(kept).collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        Written::Shown(why) => {
            let reason = not_applied_reason(written.field, why);
            Some(because.map_or_else(|| reason.clone(), |because| format!("{reason}; {because}")))
        }
    }
}

/// The module, commented from the same views the review renders.
pub fn forever_relations_lua(records: &LuaRecords, header: &LuaHeader, views: &IndexMap<QuestId, QuestView>, catalog: &QuestCatalog) -> String {
    let name_of = |quest_id: QuestId| catalog.quest(quest_id).and_then(|quest| quest.name.as_deref()).filter(|name| !name.is_empty());
    let module = LUA_MODULE_NAME;
    let mut lines = vec![
        format!("---@class {module}"),
        format!("local {module} = QuestieLoader:CreateModule(\"{module}\")"),
        String::new(),
        "---@type QuestieDB".to_owned(),
        "local QuestieDB = QuestieLoader:ImportModule(\"QuestieDB\")".to_owned(),
        String::new(),
        "-- Generated by Exporters (tools/corrections-extractor)".to_owned(),
        format!(
            "-- Quest relations from `npm run relations -- combine` (min score {}); review: {}, published next to this module",
            header.min_score, header.review_file
        ),
        format!("-- Candidate files: {}", header.candidate_files.join(", ")),
        "-- Each Assumption names the strongest trace observation (independent characters), how many signals agree and the combined score".to_owned(),
        "-- Commented-out fields are accepted relations this module does not apply; each says why".to_owned(),
        format!("-- Sessions aggregated: {}", header.session_count),
        format!("-- Generated at: {}", header.generated_at),
        format!("function {module}:Load()"),
        "    local questKeys = QuestieDB.questKeys".to_owned(),
        String::new(),
        "    return {".to_owned(),
    ];
    for (&quest_id, record) in records {
        // Without a code line the quest is shown, not written: an empty row would create the quest.
        let quest_prefix = if record.iter().any(|written| written.written.is_code()) { "" } else { "-- " };
        let entity = [name_of(quest_id).map(str::to_owned), Some(wowhead_quest_url(quest_id))]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("        {quest_prefix}[{quest_id}] = {{{}", trailing_comment(Some(&entity))));
        let view = views.get(&quest_id);
        for written in record {
            // The review's quest label (`id "name"`), so the ids in the value and the Assumption are easy to match.
            for &target in &written.values {
                let label = match name_of(target) {
                    Some(name) => format!("{target} \"{name}\" {}", wowhead_quest_url(target)),
                    None => format!("{target} {}", wowhead_quest_url(target)),
                };
                if let Some(flat) = flat_comment(&label) {
                    lines.push(format!("            -- {flat}"));
                }
            }
            let value = if written.written == Written::Cleared {
                "{}".to_owned()
            } else {
                lua_value(written.field, &written.values)
            };
            let code = format!("[questKeys.{}] = {value},", written.field);
            let comment = trailing_comment(field_comment(written, view).as_deref());
            let prefix = if written.written.is_code() { "" } else { "-- " };
            lines.push(format!("            {prefix}{code}{comment}"));
        }
        lines.push(format!("        {quest_prefix}}},"));
    }
    lines.push("    }".to_owned());
    lines.push("end".to_owned());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::edges::{Claim, Domain, EdgeKey, EdgeKind, Reference, Source};
    use crate::combine::resolve::Outcome;
    use crate::combine::views::EdgeView;
    use crate::model::CatalogQuest;
    use crate::test_support::{catalog, catalog_quest};

    use RelationField::{ExclusiveTo, NextQuestInChain, PreQuestGroup, PreQuestSingle};

    fn set(entries: &[(RelationField, &[QuestId])]) -> RelationSet {
        entries.iter().map(|&(field, values)| (field, values.to_vec())).collect()
    }

    /// Fields a later layer sets, per quest.
    type Owned<'f> = &'f [(QuestId, &'f [RelationField])];

    fn records_of(
        relations: &[(QuestId, RelationSet)],
        reference: &[(QuestId, RelationSet)],
        (authored, dynamic): (Owned, Owned),
        views: &IndexMap<QuestId, QuestView>,
    ) -> LuaRecords {
        let relations: BTreeMap<QuestId, RelationSet> = relations.iter().cloned().collect();
        let truth: Truth = reference.iter().map(|(quest_id, set)| (*quest_id, set)).collect();
        let by_quest =
            |owned: Owned| -> BTreeMap<QuestId, Vec<RelationField>> { owned.iter().map(|(quest_id, fields)| (*quest_id, fields.to_vec())).collect() };
        relation_records(&RecordSources {
            relations: &relations,
            truth: &truth,
            authored_fields: &by_quest(authored),
            dynamic_fields: &by_quest(dynamic),
            views,
        })
    }

    const NONE_OWNED: (Owned, Owned) = (&[], &[]);

    fn applied(field: RelationField, values: &[QuestId]) -> LuaField {
        LuaField::applied(field, values.to_vec())
    }

    fn shown(field: RelationField, values: &[QuestId], why: NotApplied) -> LuaField {
        LuaField::shown(field, values, why)
    }

    #[test]
    fn adds_to_questiedb_lists_without_deleting_and_clears_an_or_list_for_an_accepted_and_list() {
        let records = records_of(
            &[
                // Classic: the Forever side of an exclusive pair, a target QuestieDB has and the traces did not see.
                (1531, set(&[(ExclusiveTo, &[96136])])),
                (100, set(&[(ExclusiveTo, &[7, 9])])),
                // Classic: "either" in QuestieDB, "both" in the traces.
                (480, set(&[(PreQuestGroup, &[479, 95981])])),
                // Forever-new without reference data.
                (92000, set(&[(PreQuestSingle, &[1]), (NextQuestInChain, &[2])])),
            ],
            &[
                (1531, set(&[(ExclusiveTo, &[1532])])),
                (100, set(&[(ExclusiveTo, &[7, 8])])),
                (480, set(&[(PreQuestSingle, &[479])])),
            ],
            NONE_OWNED,
            &IndexMap::new(),
        );

        assert_eq!(records[&1531], [applied(ExclusiveTo, &[1532, 96136])]);
        assert_eq!(records[&100], [applied(ExclusiveTo, &[7, 8, 9])]);
        assert_eq!(records[&480], [LuaField::cleared(PreQuestSingle), applied(PreQuestGroup, &[479, 95981])]);
        assert_eq!(records[&92000], [applied(PreQuestSingle, &[1]), applied(NextQuestInChain, &[2])]);
    }

    #[test]
    fn never_turns_questiedb_and_lists_into_or_lists_on_prerequisites_read_as_or() {
        let records = records_of(
            &[
                // Resolution writes one prerequisite as preQuestSingle whatever its kind.
                (208, set(&[(PreQuestSingle, &[197])])),
                (209, set(&[(PreQuestSingle, &[197])])),
                // Several read as OR: resolution's default, not evidence against QuestieDB's AND.
                (210, set(&[(PreQuestSingle, &[197, 198])])),
                // OR against OR: the list grows.
                (211, set(&[(PreQuestSingle, &[2])])),
            ],
            &[
                (208, set(&[(PreQuestGroup, &[188, 193, 197])])),
                (209, set(&[(PreQuestGroup, &[188, 193])])),
                (210, set(&[(PreQuestGroup, &[188, 193])])),
                (211, set(&[(PreQuestSingle, &[1])])),
            ],
            NONE_OWNED,
            &IndexMap::new(),
        );

        assert_eq!(records[&208], [shown(PreQuestSingle, &[197], NotApplied::AlreadyInQuestieDb)]);
        assert_eq!(records[&209], [applied(PreQuestGroup, &[188, 193, 197])]);
        assert_eq!(
            records[&210],
            [shown(
                PreQuestSingle,
                &[197, 198],
                NotApplied::QuestieDbRequiresAll { required: vec![188, 193] }
            )]
        );
        assert_eq!(records[&211], [applied(PreQuestSingle, &[1, 2])]);
    }

    #[test]
    fn shows_what_questiedb_already_has_or_a_later_layer_sets_without_applying_it() {
        let records = records_of(
            &[
                // QuestieDB has more than the traces saw: nothing to add.
                (94774, set(&[(ExclusiveTo, &[94775, 94776])])),
                // Authored AND list; an applied preQuestSingle would make Questie ignore it.
                (92000, set(&[(PreQuestSingle, &[3]), (NextQuestInChain, &[3])])),
                // Same prerequisite as the authored one.
                (91745, set(&[(PreQuestSingle, &[91743])])),
                // A legacy faction fix sets the chain per faction; a Static value would never be read.
                (5021, set(&[(NextQuestInChain, &[5022])])),
            ],
            &[
                (94774, set(&[(ExclusiveTo, &[94773, 94775, 94776])])),
                (92000, set(&[(PreQuestGroup, &[1, 5]), (NextQuestInChain, &[2])])),
                (91745, set(&[(PreQuestSingle, &[91743])])),
                (5021, set(&[(NextQuestInChain, &[5022, 5023])])),
            ],
            (
                &[(94774, &[ExclusiveTo]), (92000, &[PreQuestGroup, NextQuestInChain]), (91745, &[PreQuestSingle])],
                &[(5021, &[NextQuestInChain])],
            ),
            &IndexMap::new(),
        );

        assert_eq!(records[&94774], [shown(ExclusiveTo, &[94775, 94776], NotApplied::AlreadyInQuestieDb)]);
        assert_eq!(
            records[&92000],
            [
                shown(PreQuestSingle, &[3], NotApplied::AuthoredFix),
                shown(NextQuestInChain, &[3], NotApplied::AuthoredFix)
            ]
        );
        assert_eq!(records[&91745], [shown(PreQuestSingle, &[91743], NotApplied::AlreadyInQuestieDb)]);
        assert_eq!(records[&5021], [shown(NextQuestInChain, &[5022], NotApplied::DynamicCorrection)]);
    }

    #[test]
    fn applies_a_new_chain_only_into_a_quest_whose_prerequisites_include_it() {
        let records = records_of(
            &[
                // 98299 requires 99: the chain from 98298 would hide 98298 from everyone who skipped it.
                (98298, set(&[(NextQuestInChain, &[98299])])),
                // 480's prerequisites become 479 AND 95981 through this module: the chain holds.
                (95981, set(&[(NextQuestInChain, &[480])])),
                (480, set(&[(PreQuestGroup, &[479, 95981])])),
                // Nothing known about 2's prerequisites: no evidence against the chain.
                (1, set(&[(NextQuestInChain, &[2])])),
                // QuestieDB already has this chain: shown, not judged.
                (544, set(&[(NextQuestInChain, &[545])])),
            ],
            &[
                (98299, set(&[(PreQuestSingle, &[99])])),
                (480, set(&[(PreQuestSingle, &[479])])),
                (544, set(&[(NextQuestInChain, &[545])])),
                (545, set(&[(PreQuestSingle, &[93680])])),
            ],
            NONE_OWNED,
            &IndexMap::new(),
        );

        assert_eq!(
            records[&98298],
            [shown(
                NextQuestInChain,
                &[98299],
                NotApplied::NextDoesNotRequire {
                    next: 98299,
                    required: vec![99]
                }
            )]
        );
        assert_eq!(records[&95981], [applied(NextQuestInChain, &[480])]);
        assert_eq!(records[&1], [applied(NextQuestInChain, &[2])]);
        assert_eq!(records[&544], [shown(NextQuestInChain, &[545], NotApplied::AlreadyInQuestieDb)]);
    }

    fn claim(signal: &'static str, field: RelationField, score: f64, support: u32, contradict: u32) -> Claim<'static> {
        Claim {
            source: Source { signal, field },
            field,
            score,
            support,
            contradict,
            evidence: &[],
        }
    }

    fn view<'s>(quest_id: QuestId, relations: &'s RelationSet, edges: Vec<EdgeView<'s>>) -> QuestView<'s> {
        QuestView {
            quest_id,
            name: None,
            domain: Domain::Forever,
            reference: Reference::None,
            relations,
            edges,
            reference_only: vec![],
            hints: vec![],
            note: None,
        }
    }

    fn written_edge<'s>(kind: EdgeKind, quest: QuestId, field: RelationField, target: QuestId, probability: f64, claims: &'s [Claim<'s>]) -> EdgeView<'s> {
        EdgeView {
            key: EdgeKey::new(kind, quest, target),
            field,
            target,
            probability,
            held_out: false,
            outcome: Outcome::Accepted,
            because: None,
            status: None,
            claims,
            context: &[],
            contributions: &[],
        }
    }

    #[test]
    fn shows_a_new_chain_a_signal_saw_contradicted_more_often_than_supported() {
        let (weak, strong) = (
            [claim("handoff", NextQuestInChain, 0.96, 5, 13)],
            [claim("handoff", NextQuestInChain, 0.96, 31, 2)],
        );
        let (weak_set, strong_set) = (set(&[(NextQuestInChain, &[11])]), set(&[(NextQuestInChain, &[21])]));
        let views = IndexMap::from([
            (
                10,
                view(10, &weak_set, vec![written_edge(EdgeKind::Next, 10, NextQuestInChain, 11, 0.93, &weak)]),
            ),
            (
                20,
                view(20, &strong_set, vec![written_edge(EdgeKind::Next, 20, NextQuestInChain, 21, 0.97, &strong)]),
            ),
        ]);

        let records = records_of(&[(10, weak_set.clone()), (20, strong_set.clone())], &[], NONE_OWNED, &views);

        assert_eq!(
            records[&10],
            [shown(
                NextQuestInChain,
                &[11],
                NotApplied::Contradicted {
                    signal: "handoff".into(),
                    support: 5,
                    contradict: 13
                }
            )]
        );
        assert_eq!(records[&20], [applied(NextQuestInChain, &[21])]);
    }

    fn header() -> LuaHeader<'static> {
        LuaHeader {
            candidate_files: &["handoff", "offer-set"],
            session_count: 12,
            min_score: 0.9,
            generated_at: "1970-01-01T00:00:00.000Z",
            review_file: "relations-review.md",
        }
    }

    #[test]
    fn writes_applied_fields_as_code_and_the_rest_commented_out_with_why() {
        let claims = [claim("handoff", NextQuestInChain, 0.96, 31, 0)];
        let exclusive = [claim("cooccurrence", ExclusiveTo, 0.97, 407, 0)];
        let relations = set(&[(ExclusiveTo, &[96136]), (NextQuestInChain, &[92001])]);
        let quest_view = view(
            92000,
            &relations,
            vec![
                written_edge(EdgeKind::Next, 92000, NextQuestInChain, 92001, 0.97, &claims),
                written_edge(EdgeKind::Exclusive, 92000, ExclusiveTo, 96136, 0.969, &exclusive),
            ],
        );
        let catalog = catalog([
            CatalogQuest {
                name: Some("Book Inventory".into()),
                ..catalog_quest(92000)
            },
            CatalogQuest {
                name: Some("Rascally Rodents".into()),
                ..catalog_quest(92001)
            },
        ]);
        let records = LuaRecords::from([(
            92000,
            vec![
                LuaField::cleared(PreQuestSingle),
                applied(ExclusiveTo, &[1532, 96136]),
                shown(
                    NextQuestInChain,
                    &[92001],
                    NotApplied::NextDoesNotRequire {
                        next: 92001,
                        required: vec![99],
                    },
                ),
            ],
        )]);

        let lua = forever_relations_lua(&records, &header(), &IndexMap::from([(92000, quest_view)]), &catalog);

        assert!(lua
            .contains("-- Quest relations from `npm run relations -- combine` (min score 0.9); review: relations-review.md, published next to this module\n"));
        assert!(lua.contains("-- Candidate files: handoff, offer-set\n"));
        assert!(lua.contains(concat!(
            "        [92000] = { -- Book Inventory https://wowhead.com/forever/quest=92000\n",
            "            [questKeys.preQuestSingle] = {}, -- cleared so the accepted preQuestGroup applies: Questie ignores preQuestGroup while \
             preQuestSingle is set\n",
            "            -- 1532 https://wowhead.com/forever/quest=1532\n",
            "            -- 96136 https://wowhead.com/forever/quest=96136\n",
            "            [questKeys.exclusiveTo] = {1532,96136}, -- Assumption: the server completes it and 96136 together (407 characters), score 0.969; 1532 kept from QuestieDB\n",
            "            -- 92001 \"Rascally Rodents\" https://wowhead.com/forever/quest=92001\n",
            "            -- [questKeys.nextQuestInChain] = 92001, -- not applied: 92001 requires 99, not this quest, and Questie hides a quest once its next \
             quest is done or in the log; Assumption: turning it in led straight to 92001 (31 characters), score 0.970\n",
            "        },\n",
        )));
        assert!(lua.ends_with("    }\nend"));
    }

    #[test]
    fn comments_out_a_quest_with_nothing_to_apply_as_a_whole() {
        let records = LuaRecords::from([(91745, vec![shown(PreQuestSingle, &[91743], NotApplied::AlreadyInQuestieDb)])]);

        let lua = forever_relations_lua(&records, &header(), &IndexMap::new(), &QuestCatalog::default());

        assert!(lua.contains(concat!(
            "        -- [91745] = { -- https://wowhead.com/forever/quest=91745\n",
            "            -- 91743 https://wowhead.com/forever/quest=91743\n",
            "            -- [questKeys.preQuestSingle] = {91743}, -- already in QuestieDB\n",
            "        -- },\n",
        )));
    }
}
