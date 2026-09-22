// Top-level orchestration: sessions -> per-entity Questie "Forever...Fixes"
// correction modules.
//
// Currently wires up the npc entity kind only (name/minLevel/maxLevel fields).
// Quest/item/object entities, and the remaining npc fields, follow the same
// shape and will be added incrementally (see observers/<entity>/ + emit/<entity>.ts).
//
// This produces CORRECTIONS layered on top of Questie's base DB (matching
// `Database/Custom/Fixes/foreverNPCFixes.lua`'s shape), not a full DB dump:
// only fields we actually have an aggregated Fact for are emitted.

import type { SessionRecord } from "../core/types";
import { aggregateField } from "./aggregate";
import { emitNpcRecords } from "./emit/npc";
import { observeMaxLevel } from "./observers/npc/maxLevel";
import { observeMinLevel } from "./observers/npc/minLevel";
import { observeName } from "./observers/npc/name";
import { writeQuestieCorrectionsLua } from "./schema/corrections-writer";

export interface ExtractOptions {
  sourceFileNames: string[];
  /** Injectable for deterministic tests; defaults to `new Date()`. */
  now?: Date;
}

export interface FactBundle {
  npcFixes: string;
}

export function extractAll(sessions: SessionRecord[], options: ExtractOptions): FactBundle {
  const header = {
    sourceFileNames: options.sourceFileNames,
    sessionCount: sessions.length,
    generatedAt: options.now ?? new Date(),
  };

  const nameObservations = sessions.flatMap(observeName);
  const minLevelObservations = sessions.flatMap(observeMinLevel);
  const maxLevelObservations = sessions.flatMap(observeMaxLevel);

  const npcRecords = emitNpcRecords({
    name: aggregateField(nameObservations),
    minLevel: aggregateField(minLevelObservations),
    maxLevel: aggregateField(maxLevelObservations),
  });

  const npcFixes = writeQuestieCorrectionsLua("ForeverTraceNpcFixes", "npcKeys", npcRecords, header);

  return { npcFixes };
}
