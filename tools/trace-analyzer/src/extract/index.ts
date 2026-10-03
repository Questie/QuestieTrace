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
import { observeObjectivesText } from './observers/quest';
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";
import {mergeRequiredLevel} from './observers/quest/requiredLevel';

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
  // Filter out sessions from known non-Forever builds.
  // Sessions without GetBuildInfo are INCLUDED (they might be early Forever beta
  // traces with valuable data) - any Classic IDs they contain will be filtered out
  // by filterBelowMax below.
  const foreverSessions = sessions.filter((session) => {
    const version = getInterfaceVersion(session);
    // If we have interfaceVersion, only include Forever builds (16xxx)
    if (version !== undefined) {
      return isForeverBuild(version);
    }
    // No GetBuildInfo - include the session (might have Forever IDs)
    return true;
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

  const zoneOrSortFacts = aggregateField(sessionsToProcess.flatMap(observeZoneOrSort));
  const questRecords = emitQuestRecords({
    name: aggregateField(sessionsToProcess.flatMap(observeQuestName)),
    questLevel: aggregateField(sessionsToProcess.flatMap(observeQuestLevel)),
    requiredLevel: aggregateField(sessionsToProcess.flatMap(observeRequiredLevel), mergeRequiredLevel),
    zoneOrSort: zoneOrSortFacts,
    objectives: aggregateField(
      sessionsToProcess.flatMap(observeObjectives),
      mergeQuestObjectives,
    ),
    objectivesText: aggregateField(sessionsToProcess.flatMap(observeObjectivesText)),
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

  // Collect NPC and object IDs referenced in quest startedBy/finishedBy
  // to protect them from MAX_IDS filtering
  const referencedNpcIds = new Set<number>();
  const referencedObjectIds = new Set<number>();

  for (const record of questRecords.values()) {
    const startedBy = record.startedBy as { creatures: number[]; objects: number[]; items: number[] } | undefined;
    const finishedBy = record.finishedBy as { creatures: number[]; objects: number[] } | undefined;

    if (startedBy) {
      for (const id of startedBy.creatures) referencedNpcIds.add(id);
      for (const id of startedBy.objects) referencedObjectIds.add(id);
    }
    if (finishedBy) {
      for (const id of finishedBy.creatures) referencedNpcIds.add(id);
      for (const id of finishedBy.objects) referencedObjectIds.add(id);
    }
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

  // Filter with protection for referenced NPCs/objects
  // For protected IDs (below threshold but referenced in quests), only keep questStarts/questEnds
  // Also filter quest IDs in questStarts/questEnds by MAX_IDS.quest threshold
  function filterBelowMaxWithProtection(
    records: Map<number, Record<string, unknown>>,
    entity: keyof typeof MAX_IDS,
    protectedIds: Set<number>
  ): Map<number, Record<string, unknown>> {
    const filtered = new Map<number, Record<string, unknown>>();
    const threshold = effectiveMaxIds[entity];
    const questThreshold = effectiveMaxIds.quest;
    if (threshold === undefined) return records;
    for (const [id, record] of records) {
      if (id > threshold) {
        // Above threshold: keep all fields
        filtered.set(id, record);
      } else if (protectedIds.has(id)) {
        // Below threshold but referenced: only keep questStarts/questEnds with quest IDs above questThreshold
        const protectedRecord: Record<string, unknown> = {};
        let hasValidQuests = false;
        if (record.questStarts !== undefined) {
          const questStarts = (record.questStarts as number[]).filter((qid) => qid > questThreshold);
          if (questStarts.length > 0) {
            protectedRecord.questStarts = questStarts;
            hasValidQuests = true;
          }
        }
        if (record.questEnds !== undefined) {
          const questEnds = (record.questEnds as number[]).filter((qid) => qid > questThreshold);
          if (questEnds.length > 0) {
            protectedRecord.questEnds = questEnds;
            hasValidQuests = true;
          }
        }
        if (hasValidQuests) {
          filtered.set(id, protectedRecord);
        }
      }
    }
    return filtered;
  }

  return {
    npcFixes: writeQuestieCorrectionsLua("ForeverNpcTraces", "npcKeys", filterBelowMaxWithProtection(npcRecords, "npc", referencedNpcIds), header),
    questFixes: writeQuestieCorrectionsLua("ForeverQuestTraces", "questKeys", filterBelowMax(questRecords, "quest"), header),
    itemFixes: writeQuestieCorrectionsLua("ForeverItemTraces", "itemKeys", filterBelowMax(itemRecords, "item"), header),
    objectFixes: writeQuestieCorrectionsLua("ForeverObjectTraces", "objectKeys", filterBelowMaxWithProtection(objectRecords, "object", referencedObjectIds), header),
  };
}
