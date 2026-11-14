// Top-level orchestration: sessions -> per-entity Questie "forever*Traces"
// correction modules.
//
// Wires up all four entity kinds (npc/quest/item/object). Fields are added
// incrementally (see observers/<entity>/ + emit/<entity>.ts).
//
// This produces CORRECTIONS layered on top of Questie's base DB (matching
// `Database/Custom/Fixes/foreverNPCFixes.lua`'s shape), not a full DB dump:
// only fields we actually have an aggregated Fact for are emitted.
//
// Only sessions from WoW Forever builds (interfaceVersion starting with "16")
// are processed. See PlayerIdentity.lua for how interfaceVersion is captured:

import type { SessionRecord } from "../core/types";
import { aggregateNewestScalarSummary, highestConfidence, aggregateScalarSummary, addScalarGroup, addToScalarSummary, type Fact, type ScalarGroup, type ScalarSummary } from "./aggregate";
import type { QuestObjectivesValue } from "./merge/quest/objectives";
import type { Observation } from "./observation";
import { ObservationStore } from "./observation-store";
import { emitItemRecords } from "./emit/item";
import { emitNpcRecords } from "./emit/npc";
import { emitObjectRecords } from "./emit/object";
import { emitQuestRecords } from "./emit/quest";
import { mergeNpcDrops, mergeObjectDrops } from "./merge/item/drops";
import { mergeLevelRange as mergeNpcLevelRange, mergeSpawns as mergeNpcSpawns, mergeZoneID as mergeNpcZoneID, mergeFriendlyToFaction, mergeRank as mergeNpcRank } from './merge/npc';
import { invertQuestRelations, type QuestRelations } from "./questRelations";
import { mergeSpawns as mergeObjectSpawns, mergeZoneID as mergeObjectZoneID } from "./merge/object";
import { mergeStartedBy, mergeFinishedBy, turnInPoiSpawns } from "./merge/quest";
import type { TurnInPoiValue } from "./observers/quest/turnInPoi";
import type { SpawnObservationValue } from "./observers/npc/spawns";
import { mergeQuestObjectives, mergeQuestTriggerEnds } from "./merge/quest";
import type { QuestTriggerEndValue } from "./observers/quest";
import { mergeReputationReward } from './merge/quest';
import { mergeQuestXp } from "./merge/quest/questXp";
import type { QuestXpValue } from "./observers/quest/questXp";
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";
import { writeQuestXpLua } from "./schema/quest-xp-writer";
import {mergeRequiredLevel} from './merge/quest/requiredLevel';
import { mergeQuestFactions, mergeRequiredRaces, summarizeReactions, type ReactionSummary } from "./merge/quest/requiredRaces";
import type { Faction } from "./observers/npc/friendlyToFaction";
import { MAX_IDS, observeSession, SCALAR_OBSERVERS, type ObservedValue, type ObserverKey, type ScalarObserverKey, type StagedSession } from "./observe";

export { isForeverSession, MAX_IDS, observeSession, type StagedSession } from "./observe";

export interface ExtractOptions {
  sourceFileNames: string[];
  /** Injectable for deterministic tests; defaults to `new Date()`. */
  now?: Date;
  /** Maximum known IDs per entity from classic data. Entities at or below
   * these thresholds are skipped -- only "new Forever IDs" are exported.
   * Injectable for tests. Only for extractAll: finalize uses the ones the
   * accumulator was created with (see createAccumulator). */
  maxIds?: Partial<typeof MAX_IDS>;
}

export interface FactBundle {
  npcFixes: string;
  questFixes: string;
  itemFixes: string;
  objectFixes: string;
  questXp: string;
}

/**
 * What was collected so far: per-value summaries of the scalar fields, and the raw
 * observations of the other fields in `observations`. Holding only observations (not the
 * sessions they came from) lets callers fold in one trace file at a time.
 */
export interface ExtractAccumulator {
  sessionCount: number;
  maxIds: typeof MAX_IDS;
  scalars: { [K in ScalarObserverKey]: ScalarSummary<ObservedValue<(typeof SCALAR_OBSERVERS)[K]>> };
  observations: ObservationStore;
}

/** `storeDir` keeps the raw observations on disk instead of in memory (see ObservationStore). */
export function createAccumulator(maxIds: Partial<typeof MAX_IDS> = {}, storeDir?: string): ExtractAccumulator {
  const scalars = {} as Record<ScalarObserverKey, ScalarSummary<unknown>>;
  for (const key of Object.keys(SCALAR_OBSERVERS) as ScalarObserverKey[]) {
    scalars[key] = new Map();
  }
  return {
    sessionCount: 0,
    maxIds: { ...MAX_IDS, ...maxIds },
    scalars: scalars as ExtractAccumulator["scalars"],
    observations: new ObservationStore(storeDir),
  };
}


/** Adds a summarized value group of a scalar field (see addScalarGroup). */
export function addStagedScalarGroup(acc: ExtractAccumulator, field: string, entityId: number, group: Omit<ScalarGroup<unknown>, "lastSessionKey">): void {
  const summary = acc.scalars[field as ScalarObserverKey] as ScalarSummary<unknown> | undefined;
  if (!summary) throw new Error(`Unknown scalar field ${field}`);
  addScalarGroup(summary, entityId, group);
}

/** Adds sessions observed with the accumulator's maxIds, in order. */
export function addStagedSessions(acc: ExtractAccumulator, staged: StagedSession[]): void {
  for (const session of staged) {
    acc.sessionCount++;
    const sessionKey = session.sessionKey ?? `#${acc.sessionCount}`;
    const { contributorId } = session;
    for (const [key, observations] of session.scalars) {
      for (const observation of observations) {
        observation.provenance.sessionKey = sessionKey;
        observation.provenance.sessionTime = session.sessionTime;
        if (contributorId !== undefined) observation.provenance.contributorId = contributorId;
      }
      addToScalarSummary(acc.scalars[key] as ScalarSummary<unknown>, observations);
    }
    for (const [key, observations] of session.observations) {
      for (const observation of observations) {
        observation.provenance.sessionKey = sessionKey;
        observation.provenance.sessionTime = session.sessionTime;
        if (contributorId !== undefined) observation.provenance.contributorId = contributorId;
      }
      acc.observations.add(key, observations);
    }
  }
}

/**
 * Runs every observer over `sessions` and adds the results to `acc` (see observeSession).
 * All observers run before anything is added, so a throwing observer leaves `acc` untouched.
 */
export function foldSessions(acc: ExtractAccumulator, sessions: SessionRecord[]): void {
  const staged = sessions.map((session) => observeSession(session, acc.maxIds)).filter((s): s is StagedSession => s !== undefined);
  addStagedSessions(acc, staged);
}

export function extractAll(sessions: SessionRecord[], options: ExtractOptions): FactBundle {
  const acc = createAccumulator(options.maxIds);
  foldSessions(acc, sessions);
  return finalize(acc, options);
}

/** aggregateField over a stored field, followed per entity by the observations of `extraKey`. */
function aggregateStoredField<T, R>(
  store: ObservationStore,
  key: ObserverKey,
  merge: (observations: Observation<T>[]) => R,
  extraKey?: string,
): Map<number, Fact<R>> {
  const facts = new Map<number, Fact<R>>();
  for (const byEntity of store.groups<T>(extraKey === undefined ? [key] : [key, extraKey])) {
    for (const [entityId, observations] of byEntity) {
      facts.set(entityId, {
        entityId,
        value: merge(observations),
        confidence: highestConfidence(observations),
        observationCount: observations.length,
        alternativeCount: 0,
      });
    }
  }
  return facts;
}

/** Drops objectives of quests whose matched id count differs from the objective count the API reports. */
function withAllObjectivesMatched(
  objectives: Map<number, Fact<QuestObjectivesValue>>,
  objectiveCounts: Map<number, Fact<number>>,
): Map<number, Fact<QuestObjectivesValue>> {
  const complete = new Map<number, Fact<QuestObjectivesValue>>();
  for (const [questId, fact] of objectives) {
    const matched = Object.values(fact.value).reduce((sum, ids) => sum + ids.length, 0);
    if (matched === objectiveCounts.get(questId)?.value) complete.set(questId, fact);
  }
  return complete;
}

/** Drops facts whose merge decided not to export a value. */
function onlyDefinedFacts<T>(facts: Map<number, Fact<T | undefined>>): Map<number, Fact<T>> {
  const out = new Map<number, Fact<T>>();
  for (const [id, fact] of facts) {
    if (fact.value !== undefined) out.set(id, fact as Fact<T>);
  }
  return out;
}

function mapFacts<T, R>(facts: Map<number, Fact<T>>, map: (value: T) => R): Map<number, Fact<R>> {
  const out = new Map<number, Fact<R>>();
  for (const [id, fact] of facts) out.set(id, { ...fact, value: map(fact.value) });
  return out;
}

export function finalize(acc: ExtractAccumulator, options: Omit<ExtractOptions, "maxIds">): FactBundle {
  const { scalars, observations: store } = acc;

  const header = {
    sourceFileNames: options.sourceFileNames,
    sessionCount: acc.sessionCount,
    generatedAt: options.now ?? new Date(),
  };

  const effectiveMaxIds = acc.maxIds;
  function filterBelowMax(records: Map<number, Record<string, unknown>>, entity: keyof typeof MAX_IDS): Map<number, Record<string, unknown>> {
    const filtered = new Map<number, Record<string, unknown>>();
    const threshold = effectiveMaxIds[entity];
    if (threshold === undefined) return records;
    for (const [id, record] of records) {
      if (id > threshold) filtered.set(id, record);
    }
    return filtered;
  }

  const zoneOrSortFacts = aggregateScalarSummary(scalars.zoneOrSort);
  const field = <T, R>(key: ObserverKey, merge: (observations: Observation<T>[]) => R, extraKey?: string) =>
    aggregateStoredField<T, R>(store, key, merge, extraKey);
  const finishedByFacts = onlyDefinedFacts(field("finishedBy", mergeFinishedBy));
  // Quest turn-in POIs are spawns of the quest's single finisher: they join its spawn observations.
  // They are kept apart from the observed spawns, so finalize can run again on the same accumulator.
  store.clear("npcTurnInSpawns");
  store.clear("objectTurnInSpawns");
  for (const pois of store.stream<TurnInPoiValue>("turnInPoi")) {
    const turnInSpawns = turnInPoiSpawns(pois, finishedByFacts);
    store.add("npcTurnInSpawns", turnInSpawns.npcs);
    store.add("objectTurnInSpawns", turnInSpawns.objects);
  }
  const questLevelFacts = aggregateScalarSummary(scalars.questLevel);
  const questXpRows = new Map<number, { level: number; xp: number }>();
  const questXpFacts = field<QuestXpValue, number | undefined>("questXp", (observations) =>
    mergeQuestXp(observations, questLevelFacts.get(observations[0].entityId)?.value),
  );
  for (const [questId, fact] of questXpFacts) {
    if (fact.value !== undefined) questXpRows.set(questId, { level: questLevelFacts.get(questId)!.value, xp: fact.value });
  }
  const startedByFacts = onlyDefinedFacts(field("startedBy", mergeStartedBy));
  const npcReactions = field("npcFriendlyToFaction", summarizeReactions);
  const questFactions = field("questFactions", mergeQuestFactions);
  const factionsByStarter = new Map<number, Map<number, Set<Faction>>>();
  for (const [questId, fact] of questFactions) {
    for (const npcId of startedByFacts.get(questId)?.value.creatures ?? []) {
      const quests = factionsByStarter.get(npcId) ?? new Map<number, Set<Faction>>();
      quests.set(questId, fact.value);
      factionsByStarter.set(npcId, quests);
    }
  }
  const requiredRacesFacts = new Map<number, Fact<number>>();
  for (const [questId, fact] of questFactions) {
    const starters = startedByFacts.get(questId)?.value;
    const npcStarters: ReactionSummary[] =
      !starters || starters.objects.length > 0 || starters.items.length > 0
        ? []
        : starters.creatures.map((id) => npcReactions.get(id)?.value ?? summarizeReactions([]));
    const siblingFactions = new Set<Faction>();
    for (const npcId of starters?.creatures ?? []) {
      for (const [siblingId, factions] of factionsByStarter.get(npcId) ?? []) {
        if (siblingId !== questId) factions.forEach((faction) => siblingFactions.add(faction));
      }
    }
    const mask = mergeRequiredRaces(fact.value, npcStarters, siblingFactions);
    if (mask !== undefined) requiredRacesFacts.set(questId, { ...fact, value: mask });
  }
  const questRecords = emitQuestRecords({
    name: aggregateNewestScalarSummary(scalars.questName),
    questLevel: questLevelFacts,
    requiredLevel: field("requiredLevel", mergeRequiredLevel),
    zoneOrSort: zoneOrSortFacts,
    objectives: withAllObjectivesMatched(
      field("objectives", mergeQuestObjectives),
      aggregateScalarSummary(scalars.objectiveCount),
    ),
    objectivesText: aggregateScalarSummary(scalars.objectivesText),
    triggerEnd: field<QuestTriggerEndValue, ReturnType<typeof mergeQuestTriggerEnds>>("triggerEnd", (observations) =>
      mergeQuestTriggerEnds(observations, zoneOrSortFacts.get(observations[0]?.entityId)?.value),
    ),
    startedBy: startedByFacts,
    requiredRaces: requiredRacesFacts,
    finishedBy: finishedByFacts,
    reputationReward: onlyDefinedFacts(field("reputationReward", mergeReputationReward)),
  });

  // Quest givers and enders stand still, so they get a single spawn per zone.
  const questNpcIds = new Set<number>();
  const questObjectIds = new Set<number>();
  for (const record of questRecords.values()) {
    for (const field of ["startedBy", "finishedBy"] as const) {
      const relation = record[field] as { creatures: number[]; objects: number[] } | undefined;
      relation?.creatures.forEach((id) => questNpcIds.add(id));
      relation?.objects.forEach((id) => questObjectIds.add(id));
    }
  }

  const npcLevelRanges = onlyDefinedFacts(field("npcLevel", mergeNpcLevelRange));
  const npcRecords = emitNpcRecords({
    name: aggregateNewestScalarSummary(scalars.npcName),
    minLevel: mapFacts(npcLevelRanges, (range) => range.min),
    maxLevel: mapFacts(npcLevelRanges, (range) => range.max),
    spawns: field<SpawnObservationValue, ReturnType<typeof mergeNpcSpawns>>("npcSpawns", (obs) => mergeNpcSpawns(obs, questNpcIds.has(obs[0].entityId)), "npcTurnInSpawns"),
    zoneID: field("npcSpawns", mergeNpcZoneID, "npcTurnInSpawns"),
    friendlyToFaction: onlyDefinedFacts(field("npcFriendlyToFaction", mergeFriendlyToFaction)),
    rank: onlyDefinedFacts(field("npcRank", mergeNpcRank)),
  });
  const itemRecords = emitItemRecords({
    name: aggregateNewestScalarSummary(scalars.itemName),
    npcDrops: field("npcDrops", mergeNpcDrops,
    ),
    objectDrops: field("objectDrops", mergeObjectDrops,
    ),
  });
  const objectRecords = emitObjectRecords({
    name: aggregateNewestScalarSummary(scalars.objectName),
    spawns: field<SpawnObservationValue, ReturnType<typeof mergeObjectSpawns>>("objectSpawns", (obs) => mergeObjectSpawns(obs, questObjectIds.has(obs[0].entityId)), "objectTurnInSpawns"),
    zoneID: field("objectSpawns", mergeObjectZoneID, "objectTurnInSpawns"),
  });

  // Quests below MAX_IDS are already known to Questie, but may be started/finished by new npcs/objects.
  // Those relations are exported on the quest (and below, on the entity) so both sides stay consistent.
  function relationsToNewEntities(record: Record<string, unknown>): Record<string, unknown> {
    const relationOnly: Record<string, unknown> = {};
    for (const field of ["startedBy", "finishedBy"] as const) {
      const relation = record[field] as { creatures: number[]; objects: number[] } | undefined;
      if (!relation) continue;
      const creatures = relation.creatures.filter((id) => id > effectiveMaxIds.npc);
      const objects = relation.objects.filter((id) => id > effectiveMaxIds.object);
      if (creatures.length > 0 || objects.length > 0) {
        relationOnly[field] = field === "startedBy" ? { creatures, objects, items: [] } : { creatures, objects };
      }
    }
    return relationOnly;
  }

  const exportedQuestRecords = new Map<number, Record<string, unknown>>();
  for (const [id, record] of questRecords) {
    const exported = id > effectiveMaxIds.quest ? record : relationsToNewEntities(record);
    if (Object.keys(exported).length > 0) exportedQuestRecords.set(id, exported);
  }
  const questRelations = invertQuestRelations(exportedQuestRecords);

  // Relations are kept even for entities below MAX_IDS, so quests always resolve their starters/finishers.
  function withQuestRelations(
    records: Map<number, Record<string, unknown>>,
    relations: Map<number, QuestRelations>,
    entity: keyof typeof MAX_IDS,
  ): Map<number, Record<string, unknown>> {
    const merged = filterBelowMax(records, entity);
    for (const [id, relation] of relations) {
      merged.set(id, { ...merged.get(id), ...relation });
    }
    return merged;
  }

  return {
    npcFixes: writeQuestieCorrectionsLua("ForeverNpcTraces", "npcKeys", withQuestRelations(npcRecords, questRelations.npcs, "npc"), header),
    questFixes: writeQuestieCorrectionsLua("ForeverQuestTraces", "questKeys", exportedQuestRecords, header),
    itemFixes: writeQuestieCorrectionsLua("ForeverItemTraces", "itemKeys", filterBelowMax(itemRecords, "item"), header),
    objectFixes: writeQuestieCorrectionsLua("ForeverObjectTraces", "objectKeys", withQuestRelations(objectRecords, questRelations.objects, "object"), header),
    questXp: writeQuestXpLua(questXpRows, header),
  };
}
