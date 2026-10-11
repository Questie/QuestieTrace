// `npm run relations -- signal:absence`: complete offer lists that lack a quest the giver starts
// -> what blocked it (exclusiveTo, nextQuestInChain, breadcrumbs, disabledByQuest,
// availableUntilCompleted, missing prerequisites). Pipeline: offers.ts (when quests WERE
// available), blocked.ts (when they were not, and the suspects), classify.ts (suspect -> field),
// score.ts (evidence -> score).

import { resolve } from "path";
import { isForeverQuestId } from "../../core/eligibility";
import { loadCatalog, readEpisodes, writeCandidates, writeJsonAtomic } from "../../core/io";
import { paths } from "../../core/paths";
import type { CandidateFile } from "../../core/types";
import { analyzeAbsence } from "./analyze";
import { DEFAULT_PARAMS } from "./params";

export async function run(_args: string[]): Promise<void> {
  const catalog = loadCatalog();
  const params = DEFAULT_PARAMS;
  const result = await analyzeAbsence(() => readEpisodes(), catalog, params);
  const { candidates, coveredQuestIds, index, blocked } = result;

  const file: CandidateFile = {
    signal: "absence",
    generatedAt: new Date().toISOString(),
    inputCount: index.episodes,
    params: { ...params },
    coveredQuestIds,
    candidates,
  };
  const path = writeCandidates(file);

  const likely = candidates.filter((candidate) => candidate.score >= 0.5);
  const report = {
    generatedAt: file.generatedAt,
    episodes: index.episodes,
    questsOffered: index.quests.size,
    givers: index.questsByGiver.size,
    blockedMoments: blocked.blockedMoments,
    skippedMoments: Object.fromEntries([...blocked.skipped].sort((a, b) => b[1] - a[1])),
    coveredQuests: coveredQuestIds.length,
    coveredForeverQuests: coveredQuestIds.filter(isForeverQuestId).length,
    candidates: candidates.length,
    byField: countBy(candidates, (candidate) => candidate.field),
    /** Score >= 0.5 is where measured precision is about 0.75 or better (see score.ts). */
    likely: {
      all: countBy(likely, (candidate) => candidate.field),
      foreverQuests: countBy(likely.filter((candidate) => isForeverQuestId(candidate.questId)), (candidate) => candidate.field),
    },
    byScoreBand: countBy(candidates, (candidate) => (Math.min(0.9, Math.floor(candidate.score * 10) / 10)).toFixed(1)),
  };
  writeJsonAtomic(resolve(paths.reportsDir, "absence-summary.json"), report);
  console.log(JSON.stringify(report, null, 2));
  console.log(`-> ${path}`);
}

function countBy<T>(items: T[], keyOf: (item: T) => string): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const item of items) counts[keyOf(item)] = (counts[keyOf(item)] ?? 0) + 1;
  return counts;
}
