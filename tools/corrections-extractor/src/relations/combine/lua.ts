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
//
// Like authored corrections, each quest's opening line carries its name, here with its Wowhead page.
// Each field is preceded by the name and page of every quest it names, and ends with an
// `-- Assumption:` saying why it is believed (see ./assumptions.ts).

import { writeQuestieCorrectionsLua, type CorrectionComments } from "../../core/corrections-writer";
import { RELATION_FIELDS, SCALAR_RELATION_FIELDS, type CatalogQuest, type RelationField, type RelationSet } from "../core/types";
import { assumption } from "./assumptions";
import type { QuestView } from "./views";

export const LUA_MODULE_NAME = "ForeverQuestRelationTraces";

export interface LuaHeader {
  candidateFiles: string[];
  /** Largest inputCount among the candidate files: the episodes the trace signals read. */
  sessionCount: number;
  minScore: number;
  generatedAt: Date;
  /** File name of the review report combine publishes next to this module. */
  reviewFile: string;
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

export function wowheadQuestUrl(questId: number): string {
  return `https://wowhead.com/forever/quest=${questId}`;
}

/** Quest names and Wowhead pages, and Assumption comments from the same views the review renders. */
export function foreverComments(views: ReadonlyMap<number, QuestView>, quests: Readonly<Record<string, CatalogQuest>>): CorrectionComments {
  const nameOf = (questId: number) => quests[String(questId)]?.name;
  return {
    entity: (questId) => [nameOf(questId), wowheadQuestUrl(questId)].filter(Boolean).join(" "),
    // The review's quest label (`id "name"`), so the ids in the value and the Assumption are easy to match.
    aboveField: (questId, field) =>
      (views.get(questId)?.relations[field as RelationField] ?? []).map((target) => {
        const name = nameOf(target);
        return `${target}${name ? ` "${name}"` : ""} ${wowheadQuestUrl(target)}`;
      }),
    field: (questId, field) => {
      const view = views.get(questId);
      return view && assumption(view, field as RelationField);
    },
  };
}

export function foreverRelationsLua(records: Map<number, Record<string, unknown>>, header: LuaHeader, comments: CorrectionComments = {}): string {
  const lua = writeQuestieCorrectionsLua(
    LUA_MODULE_NAME,
    "questKeys",
    records,
    {
      sourceFileNames: header.candidateFiles,
      sessionCount: header.sessionCount,
      generatedAt: header.generatedAt,
    },
    comments,
  );
  // The writer's header counts trace files; here the inputs are candidate files.
  return lua.replace(
    /^-- Source trace files: \d+$/m,
    `-- Quest relations from \`npm run relations -- combine\` (min score ${header.minScore}); review: ${header.reviewFile}, published next to this module\n` +
      `-- Candidate files: ${header.candidateFiles.join(", ")}\n` +
      "-- Each Assumption names the strongest trace observation (independent characters), how many signals agree and the combined score",
  );
}
