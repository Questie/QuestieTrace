// Counts printed after `npm run relations -- catalog` and saved to reports/catalog.json, so a
// QuestieDB update that shifts coverage or relation counts is visible at a glance.

import type { TierName } from "../core/score";
import type { CatalogQuest, GroundTruth, QuestCatalog, RelationField } from "../core/types";
import { RELATION_FIELDS } from "../core/types";
import { relationSetOf, rowsOf } from "./build";
import type { MaterializedQuests } from "./materialize";

/** Counts split by provenance: inherited Era quests versus Forever-new ones. */
interface ByProvenance {
  inherited: number;
  foreverNew: number;
}

interface TierSummary {
  quests: number;
  withRelations: number;
  /** Per field: quests that have it and total targets. */
  fields: Partial<Record<RelationField, { quests: number; edges: number }>>;
}

export interface CatalogSummary {
  generatedAt: string;
  sources: string[];
  quests: ByProvenance;
  withStarters: ByProvenance;
  withFinishers: ByProvenance;
  /** Quests whose rows differ by faction or class (Dynamic Corrections). */
  dynamicQuests: number;
  tiers: Record<TierName, TierSummary>;
  /** Forever-new quests with relations that no authored provider set; left out of the ground truth. */
  foreverRelationsOutsideAuthored: number[];
}

function tierSummary(tier: GroundTruth["tiers"][TierName]): TierSummary {
  const summary: TierSummary = { quests: 0, withRelations: 0, fields: {} };
  for (const set of Object.values(tier)) {
    summary.quests++;
    if (Object.keys(set).length > 0) summary.withRelations++;
    for (const field of RELATION_FIELDS) {
      const targets = set[field];
      if (!targets) continue;
      const count = (summary.fields[field] ??= { quests: 0, edges: 0 });
      count.quests++;
      count.edges += targets.length;
    }
  }
  return summary;
}

export function summarize(materialized: MaterializedQuests, catalog: QuestCatalog, truth: GroundTruth): CatalogSummary {
  const inherited = new Set(materialized.inheritedIds);
  const count = (predicate: (quest: CatalogQuest) => boolean): ByProvenance => {
    const counts = { inherited: 0, foreverNew: 0 };
    for (const quest of Object.values(catalog.quests)) {
      if (predicate(quest)) counts[inherited.has(quest.id) ? "inherited" : "foreverNew"]++;
    }
    return counts;
  };
  const hasAny = (groups: Record<string, number[]>) => Object.values(groups).some((ids) => ids.length > 0);
  const outside = Object.values(catalog.quests)
    .filter((quest) => !inherited.has(quest.id) && truth.tiers.authoredForever[quest.id] === undefined)
    .filter((quest) => Object.keys(relationSetOf(rowsOf(materialized, String(quest.id)))).length > 0)
    .map((quest) => quest.id);
  return {
    generatedAt: catalog.generatedAt,
    sources: catalog.sources,
    quests: count(() => true),
    withStarters: count((quest) => hasAny(quest.starters)),
    withFinishers: count((quest) => hasAny(quest.finishers)),
    dynamicQuests: Object.keys(materialized.dynamicViews).length,
    tiers: { authoredForever: tierSummary(truth.tiers.authoredForever), inheritedClassic: tierSummary(truth.tiers.inheritedClassic) },
    foreverRelationsOutsideAuthored: outside,
  };
}

export function formatSummary(summary: CatalogSummary): string {
  const split = (counts: ByProvenance) => `inherited ${counts.inherited}, forever-new ${counts.foreverNew}`;
  const lines = [
    `quests: ${split(summary.quests)}`,
    `with starters: ${split(summary.withStarters)}`,
    `with finishers: ${split(summary.withFinishers)}`,
    `faction/class-dependent (Dynamic Corrections): ${summary.dynamicQuests}`,
  ];
  for (const [name, tier] of Object.entries(summary.tiers)) {
    lines.push(`${name}: ${tier.quests} quests, ${tier.withRelations} with relations`);
    for (const field of RELATION_FIELDS) {
      const fieldCount = tier.fields[field];
      if (fieldCount) {
        lines.push(`  ${field.padEnd(24)} ${String(fieldCount.quests).padStart(5)} quests ${String(fieldCount.edges).padStart(6)} edges`);
      }
    }
  }
  if (summary.foreverRelationsOutsideAuthored.length > 0) {
    lines.push(`forever-new quests with non-authored relations (not ground truth): ${summary.foreverRelationsOutsideAuthored.join(", ")}`);
  }
  return lines.join("\n");
}
