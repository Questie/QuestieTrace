// Everything combine reads: every candidates/*.json (whatever signals exist), the catalog for
// names and givers, and the ground truth for calibration and metrics.
//
// Wowhead files are split off here into a context-only channel. Questie's own relation data was
// scraped from Wowhead Series rows, so Wowhead agreeing with the ground truth is circular and its
// measured precision means nothing. Its claims are shown to reviewers next to the trace evidence,
// and nothing else: no model column, no coverage or silence, no say in any resolution rule.

import { readdirSync } from "fs";
import { basename } from "path";
import { loadCandidates, loadCatalog, loadGroundTruth } from "../core/io";
import { paths } from "../core/paths";
import type { CandidateFile, GroundTruth, QuestCatalog, RelationSet } from "../core/types";
import { collectEdges, questDomains, truthLookup, type Edge, type QuestDomains } from "./edges";
import type { Coverage } from "./features";

export interface CombineInputs {
  /** Candidate files the model and resolution use. */
  files: CandidateFile[];
  /** Context-only files (Wowhead), never used for scoring or decisions. */
  contextFiles: CandidateFile[];
  /** Canonical edges claimed by the context-only files, keyed like the model's edges. */
  context: Map<string, Edge>;
  catalog: QuestCatalog;
  groundTruth: GroundTruth;
  truth: Map<number, RelationSet>;
  domains: QuestDomains;
  coverage: Coverage;
  /** Same-name copies share a family (faction/race/profession variants of one quest). */
  familyOf: (questId: number) => string;
}

export function isContextOnly(signal: string): boolean {
  return signal === "wowhead" || signal.startsWith("wowhead-");
}

export function loadInputs(): CombineInputs {
  const signals = readdirSync(paths.candidatesDir)
    .filter((name) => name.endsWith(".json"))
    .map((name) => basename(name, ".json"))
    .sort();
  if (signals.length === 0) throw new Error(`no candidate files in ${paths.candidatesDir}; run the signals first`);
  return prepareInputs(signals.map(loadCandidates), loadCatalog(), loadGroundTruth());
}

export function prepareInputs(allFiles: readonly CandidateFile[], catalog: QuestCatalog, groundTruth: GroundTruth): CombineInputs {
  const files = allFiles.filter((file) => !isContextOnly(file.signal));
  const contextFiles = allFiles.filter((file) => isContextOnly(file.signal));
  if (files.length === 0) throw new Error("no trace signal candidate files; Wowhead alone is context only");
  return {
    files,
    contextFiles,
    context: collectEdges(contextFiles),
    catalog,
    groundTruth,
    truth: truthLookup(groundTruth),
    domains: questDomains(groundTruth),
    coverage: new Map(files.map((file) => [file.signal, new Set(file.coveredQuestIds)])),
    familyOf: familyLookup(catalog),
  };
}

export function familyLookup(catalog: QuestCatalog): (questId: number) => string {
  return (questId) => {
    const name = catalog.quests[String(questId)]?.name;
    return name ? `name:${name.trim().toLowerCase()}` : `id:${questId}`;
  };
}
