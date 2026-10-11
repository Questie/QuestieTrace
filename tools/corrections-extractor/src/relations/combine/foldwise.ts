// Resolution inputs that are measured on the ground truth, cross-validated like the edge scores:
// an edge or quest with ground truth gets the measurement made without its own fold, everything
// else the measurement on all of it. Otherwise the held-out metrics would grade these choices on
// the same labels that made them.

import type { RelationSet } from "../core/types";
import { foldOf } from "./crossval";
import { isJudged, type Edge } from "./edges";

export interface FoldContext {
  truth: ReadonlyMap<number, RelationSet>;
  /** CV group of a quest (see crossval.ts cvGroups); an edge belongs to its `quest` side's group. */
  groupOfQuest: (questId: number) => string;
  folds: number;
}

function foldwise<T extends { quest: number }, R>(
  judged: readonly T[],
  context: FoldContext,
  measure: (items: T[]) => R,
): { all: R; without: (fold: number) => R } {
  const foldOfItem = (item: T) => foldOf(context.groupOfQuest(item.quest), context.folds);
  const all = measure([...judged]);
  const perFold = Array.from({ length: context.folds }, (_, fold) => measure(judged.filter((item) => foldOfItem(item) !== fold)));
  return { all, without: (fold) => perFold[fold] };
}

/**
 * Sources whose preQuestGroup claims are mostly right about AND (at least 5 claims, at least half
 * of them in the reference preQuestGroup). On Classic alone no source passes, so the choice rests
 * on the authored quests.
 */
export function andSourcesByQuest(
  edges: readonly Edge[],
  context: FoldContext,
): { all: ReadonlySet<string>; forQuest: (questId: number) => ReadonlySet<string> } {
  const judged = edges.filter((edge) => edge.kind === "prerequisite" && isJudged(edge, context.truth));
  const measured = foldwise(judged, context, (items) => {
    const tally = new Map<string, { right: number; total: number }>();
    for (const edge of items) {
      const isAnd = context.truth.get(edge.quest)?.preQuestGroup?.includes(edge.target) ?? false;
      for (const claim of edge.claims) {
        if (claim.field !== "preQuestGroup") continue;
        const entry = tally.get(claim.source) ?? { right: 0, total: 0 };
        entry.total++;
        if (isAnd) entry.right++;
        tally.set(claim.source, entry);
      }
    }
    return new Set([...tally].filter(([, { right, total }]) => total >= 5 && right / total >= 0.5).map(([source]) => source));
  });
  return {
    all: measured.all,
    forQuest: (questId) => (context.truth.has(questId) ? measured.without(foldOf(context.groupOfQuest(questId), context.folds)) : measured.all),
  };
}
