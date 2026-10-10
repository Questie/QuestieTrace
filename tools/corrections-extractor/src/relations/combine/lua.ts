// The Questie corrections module for Forever-new quests (ForeverQuestRelationTraces), written with
// the extractor's shared corrections writer. Classic quests never go in: Forever inherits their
// relations from Classic data, and disagreements are for a person to review first. That includes
// quests with Forever-range ids that the ground truth files as inherited Classic data.
//
// Quests with authored relations are included anyway. Authored corrections load after traces and
// override per field, so this only fills fields the authors left empty. Prerequisites are one slot,
// not two fields: Questie ignores preQuestGroup whenever preQuestSingle is set, so a generated
// preQuestSingle would silently override an authored preQuestGroup. Quests with authored
// prerequisites of either kind therefore get none from here.

import { writeQuestieCorrectionsLua } from "../../extract/schema/corrections-writer";
import { RELATION_FIELDS, SCALAR_RELATION_FIELDS, type RelationSet } from "../core/types";

export const LUA_MODULE_NAME = "ForeverQuestRelationTraces";

export interface LuaHeader {
  candidateFiles: string[];
  /** Largest inputCount among the candidate files: the episodes the trace signals read. */
  sessionCount: number;
  minScore: number;
  generatedAt: Date;
}

/** Questie stores scalar fields as a bare id and list fields as a table; fields keep RELATION_FIELDS order. */
export function foreverRecords(
  relations: ReadonlyMap<number, RelationSet>,
  authored: Readonly<Record<string, RelationSet>>,
  isForeverNew: (questId: number) => boolean,
): Map<number, Record<string, unknown>> {
  const records = new Map<number, Record<string, unknown>>();
  for (const [questId, set] of relations) {
    if (!isForeverNew(questId)) continue;
    const authoredSet = authored[String(questId)];
    const authoredPrerequisites = authoredSet?.preQuestSingle !== undefined || authoredSet?.preQuestGroup !== undefined;
    const record: Record<string, unknown> = {};
    for (const field of RELATION_FIELDS) {
      const values = set[field];
      if (!values || values.length === 0) continue;
      if (authoredPrerequisites && (field === "preQuestSingle" || field === "preQuestGroup")) continue;
      record[field] = SCALAR_RELATION_FIELDS.has(field) ? values[0] : values;
    }
    if (Object.keys(record).length > 0) records.set(questId, record);
  }
  return records;
}

export function foreverRelationsLua(records: Map<number, Record<string, unknown>>, header: LuaHeader): string {
  const lua = writeQuestieCorrectionsLua(LUA_MODULE_NAME, "questKeys", records, {
    sourceFileNames: header.candidateFiles,
    sessionCount: header.sessionCount,
    generatedAt: header.generatedAt,
  });
  // The writer's header counts trace files; here the inputs are candidate files.
  return lua.replace(
    /^-- Source trace files: \d+$/m,
    `-- Quest relations from \`npm run relations -- combine\` (min score ${header.minScore}); review: .relations/reports/combine-review.md\n` +
      `-- Candidate files: ${header.candidateFiles.join(", ")}`,
  );
}
