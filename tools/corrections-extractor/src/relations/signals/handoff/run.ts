// `npm run relations -- signal:handoff`: turn-in -> immediate next offer -> candidates/handoff.json
//
// Pass 1 finds hand-offs in every Forever episode (./detect.ts) and tallies them per character
// (./tally.ts), then derives edges through same-name copies (./variants.ts). Pass 2 re-reads the
// episodes for what needs the full list of claimed edges: counter-evidence (./counter.ts), which
// other prerequisites were done at each hand-off (ALL-of vs ANY-of), and when copies were flagged
// completed (parallel copies vs chain steps). Candidates and scores come from ./candidates.ts.
// Also writes reports/handoff-timing.json (turn-in -> next dialog/list delays, used to pick the
// windows below) and reports/handoff-pairs.json (every pair's counts, for review and calibration).

import { resolve } from "path";
import { isForeverInterface, isForeverQuestId } from "../../core/eligibility";
import { loadCatalog, readEpisodes, writeCandidates, writeJsonAtomic } from "../../core/io";
import { paths } from "../../core/paths";
import { EpisodeTimeline } from "../../core/timeline";
import type { CandidateFile, CatalogQuest } from "../../core/types";
import { allOfMembers, nextInChainCandidates, prerequisiteCandidates, SCORES, variantCandidates } from "./candidates";
import { findCounterEvidence, type ClaimedPrerequisites } from "./counter";
import { findHandoffs, type HandoffParams, type QuestLookup } from "./detect";
import { HandoffTally, type PairStats } from "./tally";
import { CompletionMoments, questsInVariantEdges, sameNameVariants, variantEdges } from "./variants";

// Windows from reports/handoff-timing.json (2026-10): 18.9k of 20.3k accept dialogs at the
// turn-in giver came 0.1-0.5s after the turn-in; the giver's list before a turn-in is a median 3s
// (p90 12s) old. The list windows are generous because confounders in between are recorded.
export const PARAMS: HandoffParams = {
  popWindow: 3,
  listBefore: 300,
  listAfter: 300,
};

export async function run(_args: string[]): Promise<void> {
  const catalog = loadCatalog();
  const lookup: QuestLookup = (questId) => catalog.quests[String(questId)];
  const tally = new HandoffTally();
  const timing = new TimingHistogram();
  const covered = new Set<number>();
  let inputCount = 0;
  let skippedInterface = 0;

  for await (const episode of readEpisodes()) {
    if (!isForeverInterface(episode.interfaceVersion)) {
      skippedInterface++;
      continue;
    }
    inputCount++;
    const { turnIns, observations, comparedQuests } = findHandoffs(new EpisodeTimeline(episode), lookup, PARAMS);
    for (const questId of comparedQuests) covered.add(questId);
    for (const turnIn of turnIns) {
      timing.add(turnIn);
      if (turnIn.giver && (turnIn.nextListDelay !== undefined || turnIn.nextDetailDelay !== undefined)) covered.add(turnIn.questId);
    }
    for (const observation of observations) {
      tally.addHandoff(episode, observation);
      covered.add(observation.offered);
    }
  }

  const variants = sameNameVariants(Object.values(catalog.quests) as CatalogQuest[]);
  const observedPairs = [...tally.pairs.values()];
  // Variant edges are tracked like observed ones so pass 2 can find counter-evidence for them too.
  const variantPairs = variantEdges(observedPairs, variants, lookup);
  for (const edge of variantPairs) tally.statsFor(edge.prerequisite, edge.quest);
  const observedClaims = claimedPrerequisites(observedPairs);
  const claimed = claimedPrerequisites(tally.pairs.values());
  const moments = new CompletionMoments(questsInVariantEdges(variantPairs));
  const alternativesOf = (prerequisite: number) => variants.get(prerequisite) ?? [];
  for await (const episode of readEpisodes()) {
    if (!isForeverInterface(episode.interfaceVersion)) continue;
    const timeline = new EpisodeTimeline(episode);
    for (const counter of findCounterEvidence(timeline, claimed, alternativesOf, lookup)) tally.addCounter(episode, counter);
    // Which of X's other observed prerequisites were done at each hand-off (ALL-of vs ANY-of).
    for (const observation of findHandoffs(timeline, lookup, PARAMS).observations) {
      const copies = alternativesOf(observation.turnedIn);
      const others = [...(observedClaims.get(observation.offered) ?? [])].filter((other) => other !== observation.turnedIn && !copies.includes(other));
      tally.addCompanionStates(episode, observation, others.map((other) => [other, timeline.isCompletedAt(other, observation.turnInAt)]));
    }
    moments.add(episode);
  }

  const allOf = allOfMembers(tally.pairs.values());
  const candidates = [
    ...prerequisiteCandidates(tally.pairs.values(), allOf),
    ...variantCandidates(variantPairs, tally.pairs, moments, allOf),
    ...nextInChainCandidates(tally.pairs.values()),
  ];
  candidates.sort((a, b) => a.questId - b.questId || a.field.localeCompare(b.field) || a.target - b.target);
  // Copies get candidates without being observed; covering them keeps recall honest.
  for (const candidate of candidates) covered.add(candidate.questId);
  const file: CandidateFile = {
    signal: "handoff",
    generatedAt: new Date().toISOString(),
    inputCount,
    params: {
      ...PARAMS,
      scores: SCORES,
      episodes: "Forever interface versions (16xxx) and versionless episodes",
      support: "characters with a pop or a newly listed offer after turning in the prerequisite (0 for same-name copies; see note)",
      contradict:
        "prerequisites: always 0, contradicted pairs are not emitted; nextQuestInChain: characters who saw a different quest pop after the same turn-in",
    },
    coveredQuestIds: [...covered].sort((a, b) => a - b),
    candidates,
  };
  const path = writeCandidates(file);

  writeJsonAtomic(resolve(paths.reportsDir, "handoff-timing.json"), { generatedAt: file.generatedAt, params: PARAMS, ...timing.toJson() });
  writeJsonAtomic(resolve(paths.reportsDir, "handoff-pairs.json"), { generatedAt: file.generatedAt, pairs: observedPairs.map(pairSummary) });
  const byField = countBy(candidates, (candidate) => `${candidate.field}${candidate.support === 0 ? " (copy)" : ""}`);
  const foreverQuests = new Set(candidates.filter((candidate) => isForeverQuestId(candidate.questId)).map((candidate) => candidate.questId));
  console.log(
    `handoff: ${inputCount} episodes (${skippedInterface} non-Forever skipped), ${observedPairs.length} observed pairs, ` +
      `${candidates.length} candidates on ${foreverQuests.size} Forever-new quests, ${covered.size} covered -> ${path}\n  ${JSON.stringify(byField)}`,
  );
}

function claimedPrerequisites(pairs: Iterable<PairStats>): ClaimedPrerequisites {
  const claimed = new Map<number, Set<number>>();
  for (const stats of pairs) {
    const set = claimed.get(stats.quest);
    if (set) set.add(stats.prerequisite);
    else claimed.set(stats.quest, new Set([stats.prerequisite]));
  }
  return claimed;
}

function countBy<T>(items: T[], key: (item: T) => string): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const item of items) counts[key(item)] = (counts[key(item)] ?? 0) + 1;
  return counts;
}

function pairSummary(stats: PairStats) {
  return {
    prerequisite: stats.prerequisite,
    quest: stats.quest,
    pop: stats.pop.size,
    popConfounded: stats.popConfounded.size,
    list: stats.list.size,
    listConfounded: stats.listConfounded.size,
    notStarted: stats.notStarted.size,
    inLog: stats.inLog.size,
    alternative: stats.alternative.size,
  };
}

/** Delay distributions around turn-ins at a known giver, to choose the windows from data. */
class TimingHistogram {
  private static readonly EDGES = [0.1, 0.25, 0.5, 1, 1.5, 2, 3, 5, 10, 20, 30, 60, 120, 300, 600];
  private readonly series = { nextDetail: [] as number[], previousList: [] as number[], nextList: [] as number[] };
  private turnIns = 0;
  private withGiver = 0;

  add(turnIn: { giver?: unknown; nextDetailDelay?: number; previousListDelay?: number; nextListDelay?: number }): void {
    this.turnIns++;
    if (!turnIn.giver) return;
    this.withGiver++;
    if (turnIn.nextDetailDelay !== undefined) this.series.nextDetail.push(turnIn.nextDetailDelay);
    if (turnIn.previousListDelay !== undefined) this.series.previousList.push(turnIn.previousListDelay);
    if (turnIn.nextListDelay !== undefined) this.series.nextList.push(turnIn.nextListDelay);
  }

  toJson() {
    const describe = (values: number[]) => {
      const sorted = [...values].sort((a, b) => a - b);
      const quantile = (q: number) => (sorted.length === 0 ? undefined : Number(sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))].toFixed(3)));
      const bins: Record<string, number> = {};
      let lower = 0;
      for (const edge of [...TimingHistogram.EDGES, Infinity]) {
        bins[`<${edge}`] = sorted.filter((value) => value >= lower && value < edge).length;
        lower = edge;
      }
      return { count: sorted.length, p10: quantile(0.1), p50: quantile(0.5), p90: quantile(0.9), p99: quantile(0.99), bins };
    };
    return {
      turnIns: this.turnIns,
      turnInsWithGiver: this.withGiver,
      nextDetailAtSameGiver: describe(this.series.nextDetail),
      previousListAtSameGiver: describe(this.series.previousList),
      nextListAtSameGiver: describe(this.series.nextList),
    };
  }
}

