// Top-level orchestration: sessions -> per-entity Questie "Forever...Fixes"
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

import type { SessionRecord } from "../core/types";
import { aggregateField } from "./aggregate";
import { emitItemRecords } from "./emit/item";
import { emitNpcRecords } from "./emit/npc";
import { emitObjectRecords } from "./emit/object";
import { emitQuestRecords } from "./emit/quest";
import { observeName as observeItemName } from "./observers/item/name";
import {
  observeMaxLevel,
  observeMinLevel,
  observeSpawns,
  observeZoneID,
  mergeSpawns,
  mergeZoneID,
} from './observers/npc';
import { observeName as observeNpcName } from "./observers/npc/name";
import { observeQuestStarts as observeNpcQuestStarts, mergeQuestStarts as mergeNpcQuestStarts } from "./observers/npc/questStarts";
import { observeQuestEnds as observeNpcQuestEnds, mergeQuestEnds as mergeNpcQuestEnds } from "./observers/npc/questEnds";
import { observeName as observeObjectName } from "./observers/object/name";
import { observeQuestStarts as observeObjectQuestStarts, mergeQuestStarts as mergeObjectQuestStarts } from "./observers/object/questStarts";
import { observeQuestEnds as observeObjectQuestEnds, mergeQuestEnds as mergeObjectQuestEnds } from "./observers/object/questEnds";
import { observeName as observeQuestName, observeStartedBy, mergeStartedBy, observeFinishedBy, mergeFinishedBy } from "./observers/quest";
import { observeQuestLevel } from './observers/quest';
import { observeRequiredLevel } from './observers/quest';
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";

export interface ExtractOptions {
  sourceFileNames: string[];
  /** Injectable for deterministic tests; defaults to `new Date()`. */
  now?: Date;
}

export interface FactBundle {
  npcFixes: string;
  questFixes: string;
  itemFixes: string;
  objectFixes: string;
}

export function extractAll(sessions: SessionRecord[], options: ExtractOptions): FactBundle {
  const header = {
    sourceFileNames: options.sourceFileNames,
    sessionCount: sessions.length,
    generatedAt: options.now ?? new Date(),
  };

  const npcRecords = emitNpcRecords({
    name: aggregateField(sessions.flatMap(observeNpcName)),
    minLevel: aggregateField(sessions.flatMap(observeMinLevel)),
    maxLevel: aggregateField(sessions.flatMap(observeMaxLevel)),
    spawns: aggregateField(
      sessions.flatMap(observeSpawns),
      mergeSpawns,
    ),
    zoneID: aggregateField(
      sessions.flatMap(observeZoneID),
      mergeZoneID,
    ),
    questStarts: aggregateField(
      sessions.flatMap(observeNpcQuestStarts),
      mergeNpcQuestStarts,
    ),
    questEnds: aggregateField(
      sessions.flatMap(observeNpcQuestEnds),
      mergeNpcQuestEnds,
    ),
  });
  const questRecords = emitQuestRecords({
    name: aggregateField(sessions.flatMap(observeQuestName)),
    questLevel: aggregateField(sessions.flatMap(observeQuestLevel)),
    requiredLevel: aggregateField(sessions.flatMap(observeRequiredLevel)),
    startedBy: aggregateField(
      sessions.flatMap(observeStartedBy),
      mergeStartedBy,
    ),
    finishedBy: aggregateField(
      sessions.flatMap(observeFinishedBy),
      mergeFinishedBy,
    ),
  });
  const itemRecords = emitItemRecords({
    name: aggregateField(sessions.flatMap(observeItemName)),
  });
  const objectRecords = emitObjectRecords({
    name: aggregateField(sessions.flatMap(observeObjectName)),
    questStarts: aggregateField(
      sessions.flatMap(observeObjectQuestStarts),
      mergeObjectQuestStarts,
    ),
    questEnds: aggregateField(
      sessions.flatMap(observeObjectQuestEnds),
      mergeObjectQuestEnds,
    ),
  });

  return {
    npcFixes: writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", npcRecords, header),
    questFixes: writeQuestieCorrectionsLua("ForeverTraceQuestFixes", "questKeys", questRecords, header),
    itemFixes: writeQuestieCorrectionsLua("ForeverTraceItemFixes", "itemKeys", itemRecords, header),
    objectFixes: writeQuestieCorrectionsLua("ForeverTraceObjectFixes", "objectKeys", objectRecords, header),
  };
}
