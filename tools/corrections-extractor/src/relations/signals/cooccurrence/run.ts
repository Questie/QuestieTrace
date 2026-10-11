// `npm run relations -- signal:cooccurrence`: completion histories across characters ->
// prerequisite (implication) and exclusiveTo (exclusion) candidates.
//
// Every session records the character's full completed set at capture start plus every change,
// so this signal reaches quests no giver interaction was recorded for. Pipeline: history.ts
// (sessions -> one ordered history per character), population.ts (dense indexes),
// prerequisites.ts, coflag.ts and exclusion.ts (inference), calibration.ts (features -> score).

import { resolve } from "path";
import { isForeverInterface, isForeverQuestId } from "../../core/eligibility";
import { loadCatalog, loadEpisodes, writeCandidates, writeJsonAtomic } from "../../core/io";
import { paths } from "../../core/paths";
import type { CandidateFile, QuestCatalog, QuestEpisode, RelationCandidate } from "../../core/types";
import { coFlagScore, exclusionScore, prerequisiteScore } from "./calibration";
import { inferCoFlags, type CoFlagEdge, type CoFlagParams } from "./coflag";
import { inferExclusions, type ExclusionEdge, type ExclusionParams } from "./exclusion";
import { buildHistories, unstableQuests, type HistoryOptions } from "./history";
import { Population } from "./population";
import { inferPrerequisites, type PrerequisiteEdge, type PrerequisiteParams } from "./prerequisites";

export const PARAMS = {
  history: { maxInitialLoss: 0.1 } satisfies Omit<HistoryOptions, "ignore">,
  prerequisites: {
    minTakers: 3,
    minOrdered: 1,
    maxNullProbability: 0.05,
    orderToleranceSeconds: 10,
    handoffSeconds: 300,
  } satisfies PrerequisiteParams,
  coflag: { windowSeconds: 3, minObservations: 2 } satisfies CoFlagParams,
  exclusion: {
    minCompleters: 3,
    minExpected: 6,
    minGroupExpected: 3,
    minGroupMemberCompleters: 2,
  } satisfies ExclusionParams,
};

export async function run(_args: string[]): Promise<void> {
  const allEpisodes = await loadEpisodes();
  const episodes = allEpisodes.filter((episode) => isForeverInterface(episode.interfaceVersion));
  const catalog = loadCatalog();
  const { file, report } = analyze(episodes, catalog);

  const path = writeCandidates(file);
  const reportPath = resolve(paths.reportsDir, "cooccurrence-summary.json");
  writeJsonAtomic(reportPath, { ...report, episodesRead: allEpisodes.length });
  console.log(`${file.candidates.length} candidates, ${file.coveredQuestIds.length} covered -> ${path}`);
  console.log(JSON.stringify(report, null, 2));
}

/** The whole signal without I/O, so tuning scripts can run it with other params. */
export function analyze(episodes: readonly QuestEpisode[], catalog: QuestCatalog, params = PARAMS) {
  const ignore = unstableQuests(episodes, catalog);
  const { histories, report: historyReport } = buildHistories(episodes, { ...params.history, ignore });
  const population = new Population(histories, catalog, ignore);
  const coflags = inferCoFlags(population, params.coflag);
  const coFlagged = new Set(coflags.edges.map((edge) => `${edge.a}:${edge.b}`));
  const prerequisites = inferPrerequisites(population, params.prerequisites, (a, b) => coFlagged.has(a < b ? `${a}:${b}` : `${b}:${a}`));
  const exclusions = inferExclusions(population, params.exclusion);

  const coFlagCovered = new Set(coflags.covered);
  const candidates = [
    ...prerequisites.edges.map(prerequisiteCandidate),
    ...coflags.edges.flatMap(coFlagCandidates),
    ...exclusions.edges.flatMap(exclusionCandidates),
  ].sort((a, b) => a.questId - b.questId || a.field.localeCompare(b.field) || a.target - b.target);

  const file: CandidateFile = {
    signal: "cooccurrence",
    generatedAt: new Date().toISOString(),
    inputCount: episodes.length,
    params: {
      ...params,
      support: "prerequisites: takers of the quest who could take the target; exclusiveTo: completers of either quest who could take the other",
      contradict: "always 0 for prerequisites and never-together exclusions (one contradiction drops the edge); co-flags: 0 as well",
    },
    // Only quests judged for both fields, so neither field's recall counts quests it never saw.
    coveredQuestIds: prerequisites.covered.filter((questId) => coFlagCovered.has(questId)),
    candidates,
  };
  const report = {
    history: historyReport,
    ignoredUnstableQuests: ignore.size,
    coveredBoth: file.coveredQuestIds.length,
    prerequisites: {
      covered: prerequisites.covered.length,
      edges: prerequisites.edges.length,
      pruned: prerequisites.pruned,
      uncoveredVariants: prerequisites.uncoveredVariants,
    },
    coflag: { covered: coflags.covered.length, observedPairs: coflags.observedPairs, contradicted: coflags.contradicted, pairs: coflags.edges.length },
    exclusion: { testedPairs: exclusions.testedPairs, conflicted: exclusions.conflicted, pairs: exclusions.edges.length },
    foreverNew: {
      prerequisiteEdges: prerequisites.edges.filter((edge) => isForeverQuestId(edge.questId)).length,
      coflagPairs: coflags.edges.filter((edge) => isForeverQuestId(edge.a) || isForeverQuestId(edge.b)).length,
      exclusionPairs: exclusions.edges.filter((edge) => isForeverQuestId(edge.a) || isForeverQuestId(edge.b)).length,
    },
  };
  return { file, report, prerequisites, coflags, exclusions };
}

function prerequisiteCandidate(edge: PrerequisiteEdge): RelationCandidate {
  return {
    questId: edge.questId,
    field: edge.field,
    target: edge.prereq,
    score: prerequisiteScore(edge),
    support: edge.takers,
    contradict: 0,
    evidence: edge.witnesses,
    note:
      `takers=${edge.takers}/${edge.allTakers} ordered=${edge.ordered} unordered=${edge.unordered} ` +
      `baseline=${edge.baseline.completed}/${edge.baseline.of} null=${edge.nullProbability.toExponential(1)} ` +
      `shape=${edge.shape} siblings=${edge.siblings} rank=${edge.rank} handoffs=${edge.handoffs}/${edge.gaps.length} levelUps=${edge.levelUps}`,
  };
}

function exclusionCandidates(edge: ExclusionEdge): RelationCandidate[] {
  const note =
    `completers=${edge.completersA}+${edge.completersB} expected=${edge.expected.toFixed(1)} kind=${edge.kind}` +
    (edge.kind === "group" ? ` groupSize=${edge.groupSize} groupExpected=${edge.groupExpected.toFixed(1)}` : "") +
    ` switched=${edge.switched}`;
  return bothDirections(edge.a, edge.b, {
    field: "exclusiveTo",
    score: exclusionScore(edge),
    support: edge.completersA + edge.completersB,
    contradict: 0,
    evidence: edge.evidence,
    note,
  });
}

function coFlagCandidates(edge: CoFlagEdge): RelationCandidate[] {
  return bothDirections(edge.a, edge.b, {
    field: "exclusiveTo",
    score: coFlagScore(edge),
    support: edge.observations + edge.unordered,
    contradict: edge.contradict,
    evidence: edge.evidence,
    note: `kind=coflag observations=${edge.observations} turnedIn=${edge.turnedInA}+${edge.turnedInB} unordered=${edge.unordered}`,
  });
}

/** Questie's exclusiveTo is one-directional, so every exclusion is emitted on both quests. */
function bothDirections(a: number, b: number, base: Omit<RelationCandidate, "questId" | "target">): RelationCandidate[] {
  return [
    { ...base, questId: a, target: b },
    { ...base, questId: b, target: a },
  ];
}
