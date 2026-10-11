// `npm run relations -- signal:offer-set [--dump <path>] [--param name=value ...]`: completed
// sets at offer time -> prerequisites (contract: src/relations/README.md).
//
// Pipeline: observe.ts (episodes -> one view per quest and character), aggregate.ts (views ->
// per-candidate counts), infer.ts (counts -> edges), score.ts (edge features -> calibrated score).
// --dump writes every edge with its features as JSON lines, for calibration; --param overrides a
// numeric tunable below for experiments.

import { writeFileSync } from "fs";
import { resolve } from "path";
import { loadCatalog, loadEpisodes, writeCandidates, writeJsonAtomic } from "../../core/io";
import { paths } from "../../core/paths";
import type { CandidateFile, QuestEpisode, RelationCandidate } from "../../core/types";
import { aggregateQuest, finisherIndex, type AggregateParams, type QuestAssessment } from "./aggregate";
import { inferQuest, toCandidate, type InferParams, type InferredEdge } from "./infer";
import { candidateFilter, emptyObserveStats, groupByCharacter, observeCharacter, type CharacterView, type ObserveOptions } from "./observe";
import { scoreEdge } from "./score";
import { VariantIndex } from "./variants";

const PARAMS = {
  // A turn-in counts at its event time, but co-completed copies only arrive with the completed-quest
  // delta 0.1-0.4s later, sometimes after the next offer was recorded.
  completionToleranceSeconds: 1,
  freshSeconds: 900,
  immediateSeconds: 30,
  latestTieSeconds: 2,
  maxContradictions: 0,
  minCharacters: 2,
  minPendingRate: 0.5,
  minSecondarySupport: 5,
} satisfies Required<ObserveOptions> & AggregateParams & InferParams;

function parseParams(args: string[]): typeof PARAMS {
  const params = { ...PARAMS };
  args.forEach((arg, index) => {
    if (arg !== "--param") return;
    const [name, value] = (args[index + 1] ?? "").split("=");
    if (!(name in params) || !Number.isFinite(Number(value))) throw new Error(`bad --param ${args[index + 1]}`);
    params[name as keyof typeof PARAMS] = Number(value);
  });
  return params;
}

/** Givers each quest was seen turned in at, across all characters. */
function observedTurnIns(characters: Iterable<QuestEpisode[]>): Map<number, Set<string>> {
  const turnIns = new Map<number, Set<string>>();
  for (const episodes of characters) {
    for (const episode of episodes) {
      for (const event of episode.questEvents) {
        if (event.kind !== "turnedIn" || !event.giver || event.giver.kind === "item") continue;
        let set = turnIns.get(event.questId);
        if (!set) turnIns.set(event.questId, (set = new Set()));
        set.add(`${event.giver.kind}:${event.giver.id}`);
      }
    }
  }
  return turnIns;
}

export async function run(args: string[]): Promise<void> {
  const dumpIndex = args.indexOf("--dump");
  const dumpPath = dumpIndex >= 0 ? args[dumpIndex + 1] : undefined;
  const params = parseParams(args);

  const catalog = loadCatalog();
  const stats = emptyObserveStats();
  const characters = groupByCharacter(await loadEpisodes(), stats);

  const viewsByQuest = new Map<number, CharacterView[]>();
  const isCandidate = candidateFilter(catalog);
  for (const episodes of characters.values()) {
    for (const view of observeCharacter(episodes, catalog, params, isCandidate)) {
      stats.views++;
      const list = viewsByQuest.get(view.questId);
      if (list) list.push(view);
      else viewsByQuest.set(view.questId, [view]);
    }
  }

  const variants = new VariantIndex(catalog);
  const finishersOf = finisherIndex(catalog, observedTurnIns(characters.values()));
  const assessments = new Map<number, QuestAssessment>();
  for (const [questId, views] of viewsByQuest) {
    assessments.set(questId, aggregateQuest(questId, views, catalog, params, finishersOf, variants));
  }

  const edges: InferredEdge[] = [];
  const covered: number[] = [];
  for (const assessment of assessments.values()) {
    if (assessment.characters < params.minCharacters) continue;
    covered.push(assessment.questId);
    edges.push(...inferQuest(assessment, assessments, variants, params));
  }
  covered.sort((a, b) => a - b);

  const candidates: RelationCandidate[] = edges
    .map((edge) => toCandidate(edge, scoreEdge(edge.features)))
    .sort((a, b) => a.questId - b.questId || b.score - a.score || a.target - b.target);

  const file: CandidateFile = {
    signal: "offer-set",
    generatedAt: new Date().toISOString(),
    inputCount: stats.usableEpisodes,
    params: { ...params, support: "independent characters", score: "calibrated lookup, see signals/offer-set/score.ts" },
    coveredQuestIds: covered,
    candidates,
  };
  const path = writeCandidates(file);
  writeJsonAtomic(resolve(paths.reportsDir, "offer-set-summary.json"), {
    generatedAt: file.generatedAt,
    observe: stats,
    questsSeen: viewsByQuest.size,
    covered: covered.length,
    candidates: candidates.length,
  });
  if (dumpPath) {
    writeFileSync(dumpPath, edges.map((edge) => JSON.stringify({ questId: edge.questId, target: edge.target, ...edge.features })).join("\n") + "\n");
  }
  console.log(`offer-set: ${stats.characters} characters, ${viewsByQuest.size} quests seen, ${covered.length} covered, ${candidates.length} candidates -> ${path}`);
}
