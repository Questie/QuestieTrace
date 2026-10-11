// Turns materialized QuestieDB rows (./materialize.ts) into the catalog and ground-truth
// contracts (QuestCatalog, GroundTruth in ../core/types.ts).
//
// Layering already happened in QuestieDB's registry. What is decided here:
// - Quests a Dynamic Correction touches have one row per faction x class view. List values
//   (starters, finishers, relations) are the union over views, because each view is what some
//   real character sees. Other fields take the first view that has a value, so a requirement
//   one view has is never dropped: a requirement-free quest looks takeable by every character,
//   and signals would count its absence from offer lists as evidence.
// - Questie treats 0 and empty lists as "no value", so both are dropped.
// - Ground-truth tiers follow provenance, not quest id: inherited Era quests (data/Forever and
//   Forever/legacy/) versus Forever-new quests.

import type { CatalogQuest, GroundTruth, QuestCatalog, RelationField, RelationSet } from "../core/types";
import { RELATION_FIELDS, SCALAR_RELATION_FIELDS } from "../core/types";
import type { MaterializedQuests, RawQuestRow } from "./materialize";

const CATALOG_FIELDS = [
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
] as const;

/** Every Questie quest field the catalog step asks QuestieDB for. */
export const EXPORTED_FIELDS: readonly string[] = [...CATALOG_FIELDS, ...RELATION_FIELDS];

/** The rows real characters read for a quest: its dynamic views, or else its one static row. */
export function rowsOf(materialized: MaterializedQuests, id: string): RawQuestRow[] {
  const views = materialized.dynamicViews[id];
  if (views) return views;
  const row = materialized.quests[id];
  return row ? [row] : [];
}

function questIds(materialized: MaterializedQuests): string[] {
  const ids = new Set([...Object.keys(materialized.quests), ...Object.keys(materialized.dynamicViews)]);
  return [...ids].sort((a, b) => Number(a) - Number(b));
}

// ---------------------------------------------------------------------------
// Value normalization
// ---------------------------------------------------------------------------

function pushUnique(target: number[], values: Iterable<number>): void {
  for (const value of values) if (!target.includes(value)) target.push(value);
}

/** Non-zero ids from a Questie id list (null holes and zeroes skipped). */
function idList(value: unknown): number[] {
  return Array.isArray(value) ? value.filter((id): id is number => typeof id === "number" && id !== 0) : [];
}

/** Targets of one relation field. Scalars become one-element lists. */
function relationTargets(field: RelationField, value: unknown): number[] {
  if (SCALAR_RELATION_FIELDS.has(field)) return typeof value === "number" && value !== 0 ? [value] : [];
  // A negative preQuestGroup entry still means that quest, only without the exclusiveTo
  // substitute (Questie's IsPreQuestGroupFulfilled), so the edge target is its absolute value.
  return idList(value).map(Math.abs);
}

/** Relationship values for a quest across its views. Empty fields are omitted. */
export function relationSetOf(rows: readonly RawQuestRow[]): RelationSet {
  const set: RelationSet = {};
  for (const field of RELATION_FIELDS) {
    const targets: number[] = [];
    for (const row of rows) pushUnique(targets, relationTargets(field, row[field]));
    if (targets.length > 0) set[field] = targets;
  }
  return set;
}

/** The first view's value that `normalize` accepts, so one view's requirement is never dropped. */
function firstValue<T>(rows: readonly RawQuestRow[], field: string, normalize: (value: unknown) => T | undefined): T | undefined {
  for (const row of rows) {
    const value = normalize(row[field]);
    if (value !== undefined) return value;
  }
  return undefined;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function number(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined;
}

function nonZeroNumber(value: unknown): number | undefined {
  return typeof value === "number" && value !== 0 ? value : undefined;
}

/** Questie pairs such as requiredSkill {skillId, value}; {0, 0} reads back as nil. */
function pair(value: unknown): [number, number] | undefined {
  if (!Array.isArray(value) || value.length !== 2) return undefined;
  const [first, second] = value as unknown[];
  if (typeof first !== "number" || typeof second !== "number" || (first === 0 && second === 0)) return undefined;
  return [first, second];
}

/** Union of one group of Questie's startedBy/finishedBy ({npcs?, objects?, items?}) across views. */
function giverGroup(rows: readonly RawQuestRow[], field: "startedBy" | "finishedBy", group: number): number[] {
  const ids: number[] = [];
  for (const row of rows) {
    const givers = row[field];
    if (Array.isArray(givers)) pushUnique(ids, idList(givers[group]));
  }
  return ids;
}

export function catalogQuestOf(id: number, rows: readonly RawQuestRow[]): CatalogQuest {
  return {
    id,
    name: firstValue(rows, "name", text),
    questLevel: firstValue(rows, "questLevel", number),
    requiredLevel: firstValue(rows, "requiredLevel", number),
    requiredRaces: firstValue(rows, "requiredRaces", nonZeroNumber),
    requiredClasses: firstValue(rows, "requiredClasses", nonZeroNumber),
    requiredSkill: firstValue(rows, "requiredSkill", pair),
    requiredMinRep: firstValue(rows, "requiredMinRep", pair),
    requiredMaxRep: firstValue(rows, "requiredMaxRep", pair),
    requiredSpell: firstValue(rows, "requiredSpell", nonZeroNumber),
    specialFlags: firstValue(rows, "specialFlags", nonZeroNumber),
    starters: {
      npcs: giverGroup(rows, "startedBy", 0),
      objects: giverGroup(rows, "startedBy", 1),
      items: giverGroup(rows, "startedBy", 2),
    },
    finishers: { npcs: giverGroup(rows, "finishedBy", 0), objects: giverGroup(rows, "finishedBy", 1) },
  };
}

// ---------------------------------------------------------------------------
// Catalog and ground truth
// ---------------------------------------------------------------------------

export function buildCatalog(materialized: MaterializedQuests, generatedAt: string): QuestCatalog {
  const quests: Record<string, CatalogQuest> = {};
  for (const id of questIds(materialized)) {
    const rows = rowsOf(materialized, id);
    if (rows.length > 0) quests[id] = catalogQuestOf(Number(id), rows);
  }
  return { generatedAt, sources: materialized.sources, quests };
}

function hasAuthoredRelation(materialized: MaterializedQuests, id: string): boolean {
  return (materialized.authoredFields[id] ?? []).some((field) => (RELATION_FIELDS as readonly string[]).includes(field));
}

/**
 * - authoredForever: Forever-new quests whose relations an authored provider sets, adds to or
 *   removes from, with the effective values (possibly {} when the authored fix deletes them).
 * - inheritedClassic: every inherited Era quest Forever loads, whatever its id, {} included so
 *   "no relation" is known. Authored fixes on these quests show up as their effective values.
 */
export function buildGroundTruth(materialized: MaterializedQuests, generatedAt: string): GroundTruth {
  const inherited = new Set(materialized.inheritedIds.map(String));
  const authoredForever: Record<string, RelationSet> = {};
  const inheritedClassic: Record<string, RelationSet> = {};
  for (const id of questIds(materialized)) {
    const rows = rowsOf(materialized, id);
    if (rows.length === 0) continue;
    if (inherited.has(id)) {
      inheritedClassic[id] = relationSetOf(rows);
    } else if (hasAuthoredRelation(materialized, id)) {
      authoredForever[id] = relationSetOf(rows);
    }
  }
  return { generatedAt, sources: materialized.sources, tiers: { authoredForever, inheritedClassic } };
}
