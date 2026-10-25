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
import { aggregateField, aggregateNewestScalarSummary, aggregateScalarSummary, addToScalarSummary, type Fact, type ScalarSummary } from "./aggregate";
import type { QuestObjectivesValue } from "./observers/quest/objectives";
import type { Observation } from "./observation";
import { emitItemRecords } from "./emit/item";
import { emitNpcRecords } from "./emit/npc";
import { emitObjectRecords } from "./emit/object";
import { emitQuestRecords } from "./emit/quest";
import { observeName as observeItemName } from "./observers/item/name";
import { observeNpcDrops } from "./observers/item/npcDrops";
import { observeObjectDrops } from "./observers/item/objectDrops";
import { mergeNpcDrops, mergeObjectDrops } from "./observers/item/dropsMerge";
import {
  observeLevel as observeNpcLevel,
  mergeLevelRange as mergeNpcLevelRange,
  observeSpawns as observeNpcSpawns,
  observeZoneID as observeNpcZoneID,
  mergeSpawns as mergeNpcSpawns,
  mergeZoneID as mergeNpcZoneID,
  observeFriendlyToFaction,
  mergeFriendlyToFaction,
} from './observers/npc';
import { observeName as observeNpcName } from "./observers/npc/name";
import { invertQuestRelations, type QuestRelations } from "./questRelations";
import { sessionServerTime } from "./probes";
import { observeName as observeObjectName } from "./observers/object/name";
import {
  observeSpawns as observeObjectSpawns,
  observeZoneID as observeObjectZoneID,
  mergeSpawns as mergeObjectSpawns,
  mergeZoneID as mergeObjectZoneID,
} from "./observers/object";
import { observeName as observeQuestName, observeStartedBy, mergeStartedBy, observeFinishedBy, mergeFinishedBy, observeTurnInPoi, turnInPoiSpawns } from "./observers/quest";
import { observeQuestLevel } from './observers/quest';
import { observeRequiredLevel } from './observers/quest';
import { observeZoneOrSort } from './observers/quest';
import { observeObjectives, mergeQuestObjectives, observeObjectiveCount, observeTriggerEnd, mergeQuestTriggerEnds } from './observers/quest';
import { observeObjectivesText } from './observers/quest';
import { observeReputationReward, mergeReputationReward } from './observers/quest';
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";
import {mergeRequiredLevel} from './observers/quest/requiredLevel';

export const MAX_IDS = {
    npc: 18199,
    quest: 9665,
    item: 25818,
    object: 300142,
};

export interface ExtractOptions {
  sourceFileNames: string[];
  /** Injectable for deterministic tests; defaults to `new Date()`. */
  now?: Date;
  /** Maximum known IDs per entity from classic data. Entities at or below
   * these thresholds are skipped -- only "new Forever IDs" are exported.
   * Injectable for tests. */
  maxIds?: Partial<typeof MAX_IDS>;
}

export interface FactBundle {
  npcFixes: string;
  questFixes: string;
  itemFixes: string;
  objectFixes: string;
}

/**
 * Extracts the interfaceVersion from a session's GetBuildInfo capture.
 * Returns undefined if the session has no GetBuildInfo data.
 *
 * The function stream structure is always: { player: [ { t: 0, tp: 0, v: 16001 } ] }
 * as captured by PlayerIdentity.lua.
 */
function getInterfaceVersion(session: SessionRecord): number | undefined {
  const buildInfo = session.functions["GetBuildInfo"];
  if (!buildInfo || typeof buildInfo !== "object" || Array.isArray(buildInfo)) {
    return undefined;
  }
  const playerStream = buildInfo["player"];
  if (!Array.isArray(playerStream) || playerStream.length === 0) return undefined;
  const firstEntry = playerStream[0];
  if (!firstEntry || typeof firstEntry !== "object") return undefined;
  return typeof firstEntry.v === "number" ? firstEntry.v : undefined;
}

/**
 * Checks if an interfaceVersion is from a WoW Forever build.
 * Forever builds have interfaceVersion starting with "16" (e.g. 16001, 16002, etc.)
 * TBC had 10xxx, WotLK had 11xxx, Cata had 12xxx, etc.
 */
function isForeverBuild(interfaceVersion: number | undefined): boolean {
  if (interfaceVersion === undefined) return false;
  return String(interfaceVersion).startsWith("16");
}

/** Sessions without GetBuildInfo count as Forever (see foldSessions). */
export function isForeverSession(session: SessionRecord): boolean {
  const version = getInterfaceVersion(session);
  return version === undefined || isForeverBuild(version);
}

/** Fields using the default scalar merge: only a per-value summary is kept. */
const SCALAR_OBSERVERS = {
  zoneOrSort: observeZoneOrSort,
  questName: observeQuestName,
  questLevel: observeQuestLevel,
  objectivesText: observeObjectivesText,
  objectiveCount: observeObjectiveCount,
  npcName: observeNpcName,
  itemName: observeItemName,
  objectName: observeObjectName,
};

/** Fields with a custom merge, which needs every raw observation. */
const OBSERVERS = {
  requiredLevel: observeRequiredLevel,
  objectives: observeObjectives,
  triggerEnd: observeTriggerEnd,
  startedBy: observeStartedBy,
  finishedBy: observeFinishedBy,
  reputationReward: observeReputationReward,
  turnInPoi: observeTurnInPoi,
  npcSpawns: observeNpcSpawns,
  npcZoneID: observeNpcZoneID,
  npcLevel: observeNpcLevel,
  npcFriendlyToFaction: observeFriendlyToFaction,
  npcDrops: observeNpcDrops,
  objectDrops: observeObjectDrops,
  objectSpawns: observeObjectSpawns,
  objectZoneID: observeObjectZoneID,
};

type ScalarObserverKey = keyof typeof SCALAR_OBSERVERS;
type ObserverKey = keyof typeof OBSERVERS;
type ObservedValue<F extends (session: SessionRecord) => unknown> =
  ReturnType<F> extends Observation<infer V>[] ? V : never;

/**
 * Observations collected so far, per field. Holding only observations (not the
 * sessions they came from) lets callers fold in one trace file at a time.
 */
export interface ExtractAccumulator {
  sessionCount: number;
  scalars: { [K in ScalarObserverKey]: ScalarSummary<ObservedValue<(typeof SCALAR_OBSERVERS)[K]>> };
  observations: { [K in ObserverKey]: ReturnType<(typeof OBSERVERS)[K]> };
}

export function createAccumulator(): ExtractAccumulator {
  const scalars = {} as Record<ScalarObserverKey, ScalarSummary<unknown>>;
  for (const key of Object.keys(SCALAR_OBSERVERS) as ScalarObserverKey[]) {
    scalars[key] = new Map();
  }
  const observations = {} as Record<ObserverKey, unknown[]>;
  for (const key of Object.keys(OBSERVERS) as ObserverKey[]) {
    observations[key] = [];
  }
  return {
    sessionCount: 0,
    scalars: scalars as ExtractAccumulator["scalars"],
    observations: observations as ExtractAccumulator["observations"],
  };
}

/**
 * Runs every observer over `sessions` and adds the results to `acc`.
 *
 * Only sessions from WoW Forever builds are observed. Sessions without
 * GetBuildInfo are INCLUDED (they might be early Forever beta traces with
 * valuable data) - any Classic IDs they contain are filtered out by
 * filterBelowMax in finalize.
 *
 * All observers run before anything is added, so a throwing observer leaves
 * `acc` untouched.
 */
export function foldSessions(acc: ExtractAccumulator, sessions: SessionRecord[]): void {
  const staged = sessions.filter(isForeverSession).map((session) => ({
    sessionTime: sessionServerTime(session),
    scalars: (Object.keys(SCALAR_OBSERVERS) as ScalarObserverKey[]).map((key) => [key, SCALAR_OBSERVERS[key](session)] as const),
    observations: (Object.keys(OBSERVERS) as ObserverKey[]).map((key) => [key, OBSERVERS[key](session)] as const),
  }));

  for (const session of staged) {
    acc.sessionCount++;
    const sessionKey = acc.sessionCount;
    for (const [key, observations] of session.scalars) {
      for (const observation of observations) {
        observation.provenance.sessionKey = sessionKey;
        observation.provenance.sessionTime = session.sessionTime;
      }
      addToScalarSummary(acc.scalars[key] as ScalarSummary<unknown>, observations as Observation<unknown>[]);
    }
    for (const [key, observations] of session.observations) {
      const target = acc.observations[key] as unknown[];
      for (const observation of observations) {
        observation.provenance.sessionKey = sessionKey;
        observation.provenance.sessionTime = session.sessionTime;
        target.push(observation);
      }
    }
  }
}

export function extractAll(sessions: SessionRecord[], options: ExtractOptions): FactBundle {
  const acc = createAccumulator();
  foldSessions(acc, sessions);
  return finalize(acc, options);
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

export function finalize(acc: ExtractAccumulator, options: ExtractOptions): FactBundle {
  const { scalars, observations } = acc;

  const header = {
    sourceFileNames: options.sourceFileNames,
    sessionCount: acc.sessionCount,
    generatedAt: options.now ?? new Date(),
  };

  const effectiveMaxIds = { ...MAX_IDS, ...options.maxIds };
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
  const finishedByFacts = aggregateField(observations.finishedBy, mergeFinishedBy);
  const turnInSpawns = turnInPoiSpawns(observations.turnInPoi, finishedByFacts);
  const questRecords = emitQuestRecords({
    name: aggregateNewestScalarSummary(scalars.questName),
    questLevel: aggregateScalarSummary(scalars.questLevel),
    requiredLevel: aggregateField(observations.requiredLevel, mergeRequiredLevel),
    zoneOrSort: zoneOrSortFacts,
    objectives: withAllObjectivesMatched(
      aggregateField(observations.objectives, mergeQuestObjectives),
      aggregateScalarSummary(scalars.objectiveCount),
    ),
    objectivesText: aggregateScalarSummary(scalars.objectivesText),
    triggerEnd: aggregateField(
      observations.triggerEnd,
      (observations) => mergeQuestTriggerEnds(observations, zoneOrSortFacts.get(observations[0]?.entityId)?.value),
    ),
    startedBy: aggregateField(
      observations.startedBy,
      mergeStartedBy,
    ),
    finishedBy: finishedByFacts,
    reputationReward: onlyDefinedFacts(aggregateField(observations.reputationReward, mergeReputationReward)),
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

  const npcLevelRanges = onlyDefinedFacts(aggregateField(observations.npcLevel, mergeNpcLevelRange));
  const npcRecords = emitNpcRecords({
    name: aggregateNewestScalarSummary(scalars.npcName),
    minLevel: mapFacts(npcLevelRanges, (range) => range.min),
    maxLevel: mapFacts(npcLevelRanges, (range) => range.max),
    spawns: aggregateField(
      [...observations.npcSpawns, ...turnInSpawns.npcs],
      (obs) => mergeNpcSpawns(obs, questNpcIds.has(obs[0].entityId)),
    ),
    zoneID: aggregateField(
      [...observations.npcZoneID, ...turnInSpawns.npcs],
      mergeNpcZoneID,
    ),
    friendlyToFaction: onlyDefinedFacts(aggregateField(observations.npcFriendlyToFaction, mergeFriendlyToFaction)),
  });
  const itemRecords = emitItemRecords({
    name: aggregateNewestScalarSummary(scalars.itemName),
    npcDrops: aggregateField(
      observations.npcDrops,
      mergeNpcDrops,
    ),
    objectDrops: aggregateField(
      observations.objectDrops,
      mergeObjectDrops,
    ),
  });
  const objectRecords = emitObjectRecords({
    name: aggregateNewestScalarSummary(scalars.objectName),
    spawns: aggregateField(
      [...observations.objectSpawns, ...turnInSpawns.objects],
      (obs) => mergeObjectSpawns(obs, questObjectIds.has(obs[0].entityId)),
    ),
    zoneID: aggregateField(
      [...observations.objectZoneID, ...turnInSpawns.objects],
      mergeObjectZoneID,
    ),
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
  };
}
