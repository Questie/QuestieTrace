// Turns per-pair character counts into scored candidates.
//
// - nextQuestInChain(P) = X comes only from pops. On Forever a pop is either the server offering
//   P's NextQuestInChain or the giver auto-opening its only available quest. The second kind
//   shows up as X having been offered while P was not completed (a "contradicted" pair), which is
//   why contradicted pops score low and are dropped when P has another pop target.
// - preQuestSingle(X) ∋ P comes from pops and from newly listed quests. A single character offered
//   X while P was not completed (and no same-name variant or race/class rule explains it) proves P
//   is not required, so contradicted pairs are not emitted at all.
// - preQuestGroup instead, when X's observed prerequisites behave as ALL-of (see allOfMembers).
//
// Scores are the measured precision of each evidence class against both ground-truth tiers
// (inheritedClassic and authoredForever, pooled), so a score approximates the probability that
// the edge is right. Most remaining false positives are breadcrumbs (P leads to X but is
// optional) and Forever-new quests wired into Classic chains, which the Classic tier cannot know.

import type { RelationCandidate } from "../../core/types";
import { countsNote, pairKey, pickEvidence, type PairStats } from "./tally";
import { isParallelCopy, type CompletionMoments, type VariantEdge } from "./variants";

/** Measured precision per evidence class (2026-10 data, see the handoff report). */
export const SCORES = {
  preQuestSingle: {
    /** Clean pops or lists from 3+ characters: 0.97 Classic (n 599) / 0.99 Forever (n 160). */
    threeCharacters: 0.97,
    /** Clean lists from 1-2 characters, no pop: 0.93 (n 15) / 0.96 (n 27). */
    fewLists: 0.95,
    /** A clean pop from 1-2 characters: 0.82 (n 68) / 1.0 (n 1). */
    fewPops: 0.82,
    /** Only observations with confounders: 0.67 (n 3) / 0.91 (n 11). */
    confoundedOnly: 0.8,
  },
  nextQuestInChain: {
    /** Uncontradicted, no rival pop target, 3+ characters: 0.97 (n 494) / 0.90 (n 60). */
    threePops: 0.96,
    /** Uncontradicted, no rival pop target, 1-2 characters: 0.90 (n 73) / 0.50 (n 4). */
    fewPops: 0.88,
    /** Uncontradicted, but only pops with confounders: too rare to measure. */
    confoundedOnly: 0.5,
    /** Under 20% of the characters handed X got a pop rather than a list: 0.50 (n 2) / 0.50 (n 4). */
    mostlyListed: 0.5,
    /** Uncontradicted with an uncontradicted rival pop target; scaled by its share of pops: 2/4. */
    rivalled: 0.5,
    /** Contradicted as a prerequisite, P has no other pop target, 3+ characters: 0.68 (n 72) / 0.50 (n 10). */
    contradictedThreePops: 0.66,
    /** Same with 1-2 characters: 0.24 (n 46) / 0.0 (n 2). Contradicted with other pop targets: 0/59, not emitted. */
    contradictedFewPops: 0.23,
  },
} as const;

/** Candidates scoring below this are left out: they would only add noise to the combination. */
const MIN_SCORE = 0.2;

/**
 * Observed pairs whose quest needs ALL of its observed prerequisites. With ALL-of, whichever
 * prerequisite a character finishes last triggers the hand-off, so every hand-off from one of them
 * happens with the others already completed. With ANY-of, some character gets X right after one
 * of them while another is not done. Groups need 2+ uncontradicted prerequisites that pass this
 * test both ways for every pair; anything less stays preQuestSingle. Same-name copies are never
 * tested against each other (they are flagged completed together), so a group with copies fails.
 */
export function allOfMembers(pairs: Iterable<PairStats>): Set<string> {
  const byQuest = new Map<number, PairStats[]>();
  for (const stats of pairs) {
    if (supportingCharacters(stats).size === 0 || isContradicted(stats)) continue;
    const group = byQuest.get(stats.quest);
    if (group) group.push(stats);
    else byQuest.set(stats.quest, [stats]);
  }
  const members = new Set<string>();
  for (const group of byQuest.values()) {
    if (group.length < 2) continue;
    const allOf = group.every((stats) => group.every((other) => other === stats || neededAlongside(stats, other.prerequisite)));
    if (allOf) for (const stats of group) members.add(pairKey(stats.prerequisite, stats.quest));
  }
  return members;
}

/** Every character handed X after turning in P had already completed `other`. */
function neededAlongside(stats: PairStats, other: number): boolean {
  const states = stats.companions.get(other);
  return !!states && states.done.size > 0 && states.notDone.size === 0;
}

export function prerequisiteCandidates(pairs: Iterable<PairStats>, allOf: ReadonlySet<string>): RelationCandidate[] {
  const candidates: RelationCandidate[] = [];
  for (const stats of pairs) {
    const support = supportingCharacters(stats).size;
    if (support === 0 || isContradicted(stats)) continue;
    candidates.push({
      questId: stats.quest,
      field: allOf.has(pairKey(stats.prerequisite, stats.quest)) ? "preQuestGroup" : "preQuestSingle",
      target: stats.prerequisite,
      score: prerequisiteScore(stats),
      support,
      contradict: 0,
      evidence: pickEvidence(stats),
      note: countsNote(stats),
    });
  }
  return candidates;
}

/**
 * preQuestSingle edges through same-name copies of an observed, uncontradicted edge. Edges that
 * were observed directly, or contradicted by some character, are left to the observed rules, and
 * copies never seen completing together with the original are not used (see CompletionMoments).
 * ALL-of groups are not extended: a copy of one member would have to be required too. A copy
 * inherits the observed edge's score, which measured right: 0.96 (n 217) and 1.0 (n 86) on the
 * authored Forever tier.
 */
export function variantCandidates(
  edges: Iterable<VariantEdge>,
  pairs: ReadonlyMap<string, PairStats>,
  moments: CompletionMoments,
  allOf: ReadonlySet<string>,
): RelationCandidate[] {
  const best = new Map<string, RelationCandidate>();
  for (const edge of edges) {
    if (!isParallelCopy(edge, moments)) continue;
    const key = pairKey(edge.prerequisite, edge.quest);
    const own = pairs.get(key);
    if (own && (supportingCharacters(own).size > 0 || isContradicted(own))) continue;
    const observedKey = pairKey(edge.observed.prerequisite, edge.observed.quest);
    const observed = pairs.get(observedKey);
    if (!observed || supportingCharacters(observed).size === 0 || isContradicted(observed) || allOf.has(observedKey)) continue;
    const score = prerequisiteScore(observed);
    if ((best.get(key)?.score ?? -1) >= score) continue;
    const evidence = pickEvidence(observed).slice(0, 3);
    best.set(key, {
      questId: edge.quest,
      field: "preQuestSingle",
      target: edge.prerequisite,
      score,
      // Nobody was observed taking this copy; the support belongs to the observed edge.
      support: 0,
      contradict: 0,
      evidence: [{ ref: evidence[0]?.ref ?? "", text: `same-name copy (${edge.kind}) of observed ${observed.quest} <- ${observed.prerequisite}` }, ...evidence],
      note: `variant:${edge.kind} of ${observed.quest} <- ${observed.prerequisite}; ${countsNote(observed)}`,
    });
  }
  return [...best.values()];
}

function prerequisiteScore(stats: PairStats): number {
  const clean = new Set([...stats.pop, ...stats.list]).size;
  if (clean >= 3) return SCORES.preQuestSingle.threeCharacters;
  if (stats.pop.size > 0) return SCORES.preQuestSingle.fewPops;
  if (stats.list.size > 0) return SCORES.preQuestSingle.fewLists;
  return SCORES.preQuestSingle.confoundedOnly;
}

export function nextInChainCandidates(pairs: Iterable<PairStats>): RelationCandidate[] {
  const byPrerequisite = new Map<number, PairStats[]>();
  for (const stats of pairs) {
    if (popCharacters(stats).size === 0) continue;
    const group = byPrerequisite.get(stats.prerequisite);
    if (group) group.push(stats);
    else byPrerequisite.set(stats.prerequisite, [stats]);
  }

  const candidates: RelationCandidate[] = [];
  for (const group of byPrerequisite.values()) {
    for (const stats of group) {
      const rivals = group.filter((other) => other !== stats);
      const score = nextInChainScore(stats, rivals);
      if (score < MIN_SCORE) continue;
      const support = popCharacters(stats).size;
      candidates.push({
        questId: stats.prerequisite,
        field: "nextQuestInChain",
        target: stats.quest,
        score: round(score),
        support,
        // Characters who saw a different quest pop after the same turn-in.
        contradict: new Set(rivals.flatMap((other) => [...popCharacters(other)].filter((character) => !stats.pop.has(character)))).size,
        evidence: pickEvidence(stats),
        note: countsNote(stats),
      });
    }
  }
  return candidates;
}

function nextInChainScore(stats: PairStats, rivals: PairStats[]): number {
  const scores = SCORES.nextQuestInChain;
  if (isContradicted(stats)) {
    if (rivals.length > 0) return 0;
    return stats.pop.size >= 3 ? scores.contradictedThreePops : stats.pop.size > 0 ? scores.contradictedFewPops : 0;
  }
  // A contradicted rival is the giver auto-opening its only quest, not a competing chain claim.
  const cleanRivals = rivals.filter((other) => !isContradicted(other));
  if (cleanRivals.length > 0) {
    const pops = popCharacters(stats).size;
    return scores.rivalled * (pops / (pops + cleanRivals.reduce((sum, other) => sum + popCharacters(other).size, 0)));
  }
  // The server pops P's NextQuestInChain whenever it can, so a quest usually re-listed instead is not it.
  const pops = popCharacters(stats).size;
  if (pops < 0.2 * (pops + new Set([...stats.list, ...stats.listConfounded]).size)) return scores.mostlyListed;
  return stats.pop.size >= 3 ? scores.threePops : stats.pop.size > 0 ? scores.fewPops : scores.confoundedOnly;
}

/** Some character was offered X while P was neither completed nor explained by an alternative. */
function isContradicted(stats: PairStats): boolean {
  return stats.notStarted.size + stats.inLog.size > 0;
}

function popCharacters(stats: PairStats): Set<string> {
  return new Set([...stats.pop, ...stats.popConfounded]);
}

function supportingCharacters(stats: PairStats): Set<string> {
  return new Set([...stats.pop, ...stats.popConfounded, ...stats.list, ...stats.listConfounded]);
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}
