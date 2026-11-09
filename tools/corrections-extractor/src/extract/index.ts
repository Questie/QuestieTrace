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
import { aggregateNewestScalarSummary, highestConfidence, aggregateScalarSummary, addToScalarSummary, type Fact, type ScalarSummary } from "./aggregate";
import type { QuestObjectivesValue } from "./observers/quest/objectives";
import type { Observation } from "./observation";
import { ObservationStore } from "./observation-store";
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
  mergeSpawns as mergeNpcSpawns,
  mergeZoneID as mergeNpcZoneID,
  observeFriendlyToFaction,
  mergeFriendlyToFaction,
  observeRank as observeNpcRank,
  mergeRank as mergeNpcRank,
} from './observers/npc';
import { observeName as observeNpcName } from "./observers/npc/name";
import { invertQuestRelations, type QuestRelations } from "./questRelations";
import { sessionServerTime } from "./probes";
import { observeName as observeObjectName } from "./observers/object/name";
import {
  observeSpawns as observeObjectSpawns,
  mergeSpawns as mergeObjectSpawns,
  mergeZoneID as mergeObjectZoneID,
} from "./observers/object";
import { observeName as observeQuestName, observeStartedBy, mergeStartedBy, observeFinishedBy, mergeFinishedBy, observeTurnInPoi, turnInPoiSpawns } from "./observers/quest";
import type { TurnInPoiValue } from "./observers/quest/turnInPoi";
import type { SpawnObservationValue } from "./observers/npc/spawns";
import { observeQuestLevel } from './observers/quest';
import { observeRequiredLevel } from './observers/quest';
import { observeZoneOrSort } from './observers/quest';
import { observeObjectives, mergeQuestObjectives, observeObjectiveCount, observeTriggerEnd, mergeQuestTriggerEnds, type QuestTriggerEndValue } from "./observers/quest";
import { observeObjectivesText } from './observers/quest';
import { observeReputationReward, mergeReputationReward } from './observers/quest';
import { observeQuestXp, mergeQuestXp, type QuestXpValue } from "./observers/quest/questXp";
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";
import { writeQuestXpLua } from "./schema/quest-xp-writer";
import {mergeRequiredLevel} from './observers/quest/requiredLevel';
import { mergeQuestFactions, mergeRequiredRaces, observeQuestFactions, summarizeReactions, type ReactionSummary } from "./observers/quest/requiredRaces";
import type { Faction } from "./observers/npc/friendlyToFaction";

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
  questXp: observeQuestXp,
  questFactions: observeQuestFactions,
  turnInPoi: observeTurnInPoi,
  // npcZoneID and objectZoneID are merged from the spawn observations: their observers
  // are the spawn observers.
  npcSpawns: observeNpcSpawns,
  npcLevel: observeNpcLevel,
  npcFriendlyToFaction: observeFriendlyToFaction,
  npcRank: observeNpcRank,
  npcDrops: observeNpcDrops,
  objectDrops: observeObjectDrops,
  objectSpawns: observeObjectSpawns,
};

/**
 * The entity each npc, item and object field is about. Only entities above MAX_IDS are
 * exported, so their observations at or below it are dropped while folding.
 * npcFriendlyToFaction is kept for every NPC: Classic NPCs start Forever quests, and
 * their reactions decide those quests' requiredRaces.
 */
const OBSERVER_ENTITIES: Partial<Record<keyof typeof OBSERVERS, keyof typeof MAX_IDS>> = {
  questXp: "quest",
  npcSpawns: "npc",
  npcLevel: "npc",
  npcRank: "npc",
  npcDrops: "item",
  objectDrops: "item",
  objectSpawns: "object",
};

type ScalarObserverKey = keyof typeof SCALAR_OBSERVERS;
type ObserverKey = keyof typeof OBSERVERS;
type ObservedValue<F extends (session: SessionRecord) => unknown> =
  ReturnType<F> extends Observation<infer V>[] ? V : never;

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

/** One session's observations, ready to be added to an accumulator (see observeSession). */
export interface StagedSession {
  /** Stable key of the session (core/trace-data/session-key.ts); a per-run "#<n>" is used when unknown. */
  sessionKey?: string;
  contributorId?: string;
  sessionTime: number | null;
  scalars: [ScalarObserverKey, Observation<unknown>[]][];
  observations: [ObserverKey, Observation<unknown>[]][];
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

/**
 * Runs every observer over one session. Returns undefined for sessions that are not
 * observed: only sessions from WoW Forever builds are. Sessions without GetBuildInfo are
 * INCLUDED (they might be early Forever beta traces with valuable data) - any Classic IDs
 * they contain are filtered out by filterBelowMax in finalize.
 *
 * npc, item and object observations at or below `maxIds` are dropped right away: they are
 * never exported.
 */
export function observeSession(session: SessionRecord, maxIds: Partial<typeof MAX_IDS> = {}): StagedSession | undefined {
  if (!isForeverSession(session)) return undefined;
  const limits = { ...MAX_IDS, ...maxIds };
  return {
    sessionTime: sessionServerTime(session),
    scalars: (Object.keys(SCALAR_OBSERVERS) as ScalarObserverKey[]).map((key) => [key, SCALAR_OBSERVERS[key](session) as Observation<unknown>[]]),
    observations: (Object.keys(OBSERVERS) as ObserverKey[]).map((key) => {
      const entity = OBSERVER_ENTITIES[key];
      const maxId = entity === undefined ? -Infinity : limits[entity];
      return [key, (OBSERVERS[key](session) as Observation<unknown>[]).filter((observation) => observation.entityId > maxId)];
    }),
  };
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
  const finishedByFacts = field("finishedBy", mergeFinishedBy);
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
  const startedByFacts = field("startedBy", mergeStartedBy);
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
