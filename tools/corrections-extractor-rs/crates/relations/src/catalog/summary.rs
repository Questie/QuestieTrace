//! Counts printed after `relations catalog` and saved to reports/catalog.json, so a QuestieDB
//! update that shifts coverage or relation counts is visible at a glance.

use super::build::relation_set_of;
use super::materialize::MaterializedQuests;
use crate::model::{CatalogQuest, GroundTruth, QuestCatalog, QuestId, RelationField, RelationSet, Tier};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Counts split by provenance: inherited Era quests versus Forever-new ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ByProvenance {
    pub inherited: usize,
    pub forever_new: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct FieldCount {
    /// Quests that have the field.
    pub quests: usize,
    /// Targets over those quests.
    pub edges: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierSummary {
    pub quests: usize,
    pub with_relations: usize,
    pub fields: BTreeMap<RelationField, FieldCount>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSummary {
    pub generated_at: String,
    pub sources: Vec<String>,
    pub quests: ByProvenance,
    pub with_starters: ByProvenance,
    pub with_finishers: ByProvenance,
    /// Quests whose rows differ by faction or class (Dynamic Corrections).
    pub dynamic_quests: usize,
    pub tiers: BTreeMap<Tier, TierSummary>,
    /// Forever-new quests with relations that no authored provider set; left out of the ground truth.
    pub forever_relations_outside_authored: Vec<QuestId>,
}

fn tier_summary(tier: &BTreeMap<QuestId, RelationSet>) -> TierSummary {
    let mut summary = TierSummary::default();
    for set in tier.values() {
        summary.quests += 1;
        if !set.is_empty() {
            summary.with_relations += 1;
        }
        for (&field, targets) in set {
            let count = summary.fields.entry(field).or_default();
            count.quests += 1;
            count.edges += targets.len();
        }
    }
    summary
}

pub fn summarize(materialized: &MaterializedQuests, catalog: &QuestCatalog, truth: &GroundTruth) -> CatalogSummary {
    let inherited: BTreeSet<QuestId> = materialized.inherited_ids.iter().copied().collect();
    let count = |predicate: &dyn Fn(&CatalogQuest) -> bool| {
        let mut counts = ByProvenance::default();
        for quest in catalog.quests.values().filter(|quest| predicate(quest)) {
            if inherited.contains(&quest.id) {
                counts.inherited += 1;
            } else {
                counts.forever_new += 1;
            }
        }
        counts
    };
    let has_starters = |quest: &CatalogQuest| {
        [&quest.starters.npcs, &quest.starters.objects, &quest.starters.items]
            .iter()
            .any(|ids| !ids.is_empty())
    };
    let has_finishers = |quest: &CatalogQuest| !quest.finishers.npcs.is_empty() || !quest.finishers.objects.is_empty();
    let outside = catalog
        .quests
        .values()
        .filter(|quest| !inherited.contains(&quest.id) && !truth.tiers.authored_forever.contains_key(&quest.id))
        .filter(|quest| !relation_set_of(materialized.rows_of(quest.id)).is_empty())
        .map(|quest| quest.id)
        .collect();
    CatalogSummary {
        generated_at: catalog.generated_at.clone(),
        sources: catalog.sources.clone(),
        quests: count(&|_| true),
        with_starters: count(&has_starters),
        with_finishers: count(&has_finishers),
        dynamic_quests: materialized.dynamic_views.len(),
        tiers: Tier::ALL.into_iter().map(|tier| (tier, tier_summary(truth.tiers.get(tier)))).collect(),
        forever_relations_outside_authored: outside,
    }
}

impl std::fmt::Display for CatalogSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let split = |counts: ByProvenance| format!("inherited {}, forever-new {}", counts.inherited, counts.forever_new);
        writeln!(f, "quests: {}", split(self.quests))?;
        writeln!(f, "with starters: {}", split(self.with_starters))?;
        writeln!(f, "with finishers: {}", split(self.with_finishers))?;
        write!(f, "faction/class-dependent (Dynamic Corrections): {}", self.dynamic_quests)?;
        for (tier, summary) in &self.tiers {
            let name = match tier {
                Tier::AuthoredForever => "authoredForever",
                Tier::InheritedClassic => "inheritedClassic",
            };
            write!(f, "\n{name}: {} quests, {} with relations", summary.quests, summary.with_relations)?;
            for (field, count) in &summary.fields {
                write!(f, "\n  {:<24} {:>5} quests {:>6} edges", field.name(), count.quests, count.edges)?;
            }
        }
        if !self.forever_relations_outside_authored.is_empty() {
            let ids: Vec<String> = self.forever_relations_outside_authored.iter().map(QuestId::to_string).collect();
            write!(f, "\nforever-new quests with non-authored relations (not ground truth): {}", ids.join(", "))?;
        }
        Ok(())
    }
}
