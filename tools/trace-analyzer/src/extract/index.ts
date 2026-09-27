// Top-level orchestration: sessions -> per-entity Questie "forever*Traces"
// correction modules.
//
// Wires up all four entity kinds (npc/quest/item/object), each currently only
// their `name` field (plus npc's minLevel/maxLevel stubs). More fields follow
// the same shape and will be added incrementally (see observers/<entity>/ +
// emit/<entity>.ts).
//
// This produces CORRECTIONS layered on top of Questie's base DB (matching
// `Database/Custom/Fixes/foreverNPCFixes.lua`'s shape), not a full DB dump:
// only fields we actually have an aggregated Fact for are emitted.
//
// Only sessions from WoW Forever builds (interfaceVersion starting with "16")
// are processed. See PlayerIdentity.lua for how interfaceVersion is captured:

import type { SessionRecord } from "../core/types";
import { aggregateField } from "./aggregate";
import { emitItemRecords } from "./emit/item";
import { emitNpcRecords } from "./emit/npc";
import { emitObjectRecords } from "./emit/object";
import { emitQuestRecords } from "./emit/quest";
import { observeName as observeItemName } from "./observers/item/name";
import { observeNpcDrops } from "./observers/item/npcDrops";
import { observeObjectDrops } from "./observers/item/objectDrops";
import { mergeNpcDrops, mergeObjectDrops } from "./observers/item/dropsMerge";
import {
  observeMaxLevel,
  observeMinLevel,
  observeSpawns as observeNpcSpawns,
  observeZoneID as observeNpcZoneID,
  mergeSpawns as mergeNpcSpawns,
  mergeZoneID as mergeNpcZoneID,
} from './observers/npc';
import { observeName as observeNpcName } from "./observers/npc/name";
import { observeQuestStarts as observeNpcQuestStarts, mergeQuestStarts as mergeNpcQuestStarts } from "./observers/npc/questStarts";
import { observeQuestEnds as observeNpcQuestEnds, mergeQuestEnds as mergeNpcQuestEnds } from "./observers/npc/questEnds";
import { observeName as observeObjectName } from "./observers/object/name";
import {
  observeSpawns as observeObjectSpawns,
  observeZoneID as observeObjectZoneID,
  mergeSpawns as mergeObjectSpawns,
  mergeZoneID as mergeObjectZoneID,
} from "./observers/object";
import { observeQuestStarts as observeObjectQuestStarts, mergeQuestStarts as mergeObjectQuestStarts } from "./observers/object/questStarts";
import { observeQuestEnds as observeObjectQuestEnds, mergeQuestEnds as mergeObjectQuestEnds } from "./observers/object/questEnds";
import { observeName as observeQuestName, observeStartedBy, mergeStartedBy, observeFinishedBy, mergeFinishedBy } from "./observers/quest";
import { observeQuestLevel } from './observers/quest';
import { observeRequiredLevel } from './observers/quest';
import { observeZoneOrSort } from './observers/quest';
import { observeObjectives, mergeQuestObjectives, observeTriggerEnd, mergeQuestTriggerEnds } from './observers/quest';
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";

const MAX_IDS = {
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

export function extractAll(sessions: SessionRecord[], options: ExtractOptions): FactBundle {
  const foreverSessions = sessions.filter((session) => {
    const version = getInterfaceVersion(session);
    return isForeverBuild(version);
  });

  const header = {
    sourceFileNames: options.sourceFileNames,
    sessionCount: foreverSessions.length,
    generatedAt: options.now ?? new Date(),
  };

  const effectiveMaxIds = { ...MAX_IDS, ...options.maxIds };

  const sessionsToProcess = foreverSessions;

  function filterBelowMax(records: Map<number, Record<string, unknown>>, entity: keyof typeof MAX_IDS): Map<number, Record<string, unknown>> {
    const filtered = new Map<number, Record<string, unknown>>();
    const threshold = effectiveMaxIds[entity];
    if (threshold === undefined) return records;
    for (const [id, record] of records) {
      if (id > threshold) filtered.set(id, record);
    }
    return filtered;
  }

  const npcRecords = emitNpcRecords({
    name: aggregateField(sessionsToProcess.flatMap(observeNpcName)),
    minLevel: aggregateField(sessionsToProcess.flatMap(observeMinLevel)),
    maxLevel: aggregateField(sessionsToProcess.flatMap(observeMaxLevel)),
    spawns: aggregateField(
      sessionsToProcess.flatMap(observeNpcSpawns),
      mergeNpcSpawns,
    ),
    zoneID: aggregateField(
      sessionsToProcess.flatMap(observeNpcZoneID),
      mergeNpcZoneID,
    ),
    questStarts: aggregateField(
      sessionsToProcess.flatMap(observeNpcQuestStarts),
      mergeNpcQuestStarts,
    ),
    questEnds: aggregateField(
      sessionsToProcess.flatMap(observeNpcQuestEnds),
      mergeNpcQuestEnds,
    ),
  });
  const zoneOrSortFacts = aggregateField(sessionsToProcess.flatMap(observeZoneOrSort));
  const questRecords = emitQuestRecords({
    name: aggregateField(sessionsToProcess.flatMap(observeQuestName)),
    questLevel: aggregateField(sessionsToProcess.flatMap(observeQuestLevel)),
    requiredLevel: aggregateField(sessionsToProcess.flatMap(observeRequiredLevel)),
    zoneOrSort: zoneOrSortFacts,
    objectives: aggregateField(
      sessionsToProcess.flatMap(observeObjectives),
      mergeQuestObjectives,
    ),
    triggerEnd: aggregateField(
      sessionsToProcess.flatMap(observeTriggerEnd),
      (observations) => mergeQuestTriggerEnds(observations, zoneOrSortFacts.get(observations[0]?.entityId)?.value),
    ),
    startedBy: aggregateField(
      sessionsToProcess.flatMap(observeStartedBy),
      mergeStartedBy,
    ),
    finishedBy: aggregateField(
      sessionsToProcess.flatMap(observeFinishedBy),
      mergeFinishedBy,
    ),
  });
  const itemRecords = emitItemRecords({
    name: aggregateField(sessionsToProcess.flatMap(observeItemName)),
    npcDrops: aggregateField(
      sessionsToProcess.flatMap(observeNpcDrops),
      mergeNpcDrops,
    ),
    objectDrops: aggregateField(
      sessionsToProcess.flatMap(observeObjectDrops),
      mergeObjectDrops,
    ),
  });
  const objectRecords = emitObjectRecords({
    name: aggregateField(sessionsToProcess.flatMap(observeObjectName)),
    spawns: aggregateField(
      sessionsToProcess.flatMap(observeObjectSpawns),
      mergeObjectSpawns,
    ),
    zoneID: aggregateField(
      sessionsToProcess.flatMap(observeObjectZoneID),
      mergeObjectZoneID,
    ),
    questStarts: aggregateField(
      sessionsToProcess.flatMap(observeObjectQuestStarts),
      mergeObjectQuestStarts,
    ),
    questEnds: aggregateField(
      sessionsToProcess.flatMap(observeObjectQuestEnds),
      mergeObjectQuestEnds,
    ),
  });

  return {
    npcFixes: writeQuestieCorrectionsLua("ForeverNpcTraces", "npcKeys", filterBelowMax(npcRecords, "npc"), header),
    questFixes: writeQuestieCorrectionsLua("ForeverQuestTraces", "questKeys", filterBelowMax(questRecords, "quest"), header),
    itemFixes: writeQuestieCorrectionsLua("ForeverItemTraces", "itemKeys", filterBelowMax(itemRecords, "item"), header),
    objectFixes: writeQuestieCorrectionsLua("ForeverObjectTraces", "objectKeys", filterBelowMax(objectRecords, "object"), header),
  };
}
