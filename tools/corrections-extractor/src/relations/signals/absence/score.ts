// Turns an edge's evidence into a calibrated score: the measured chance that such an edge is in
// Questie's data.
//
// score = ceiling(group, tie) × (1 - e^(-4 × evidence)), where evidence is the sum over characters
// of their best blame share (attribute.ts). Precision stops improving once about one character's
// worth of clean evidence exists, hence the quick saturation. Ceilings are the precision of
// candidates with evidence >= 0.5, judged against both ground-truth tiers pooled (2026-10-11,
// 18.9k Forever sessions, episodes with sessionOrder), smoothed with a Beta(1, 3) prior so a
// bucket with few edges stays modest. TP/judged per bucket:
//
//   group                   strong tie          weak tie        no tie
//   prerequisite            394/417 -> 0.94     3/8 -> 0.33     0/1 -> 0.20
//   pendingPrerequisite     Classic 72/83 -> 0.84, Forever 27/63 -> 0.42; weak 2/10 -> 0.21
//   anyPrerequisite         4/4 -> 0.63         (no data)       (no data)
//   chain                   32/32 -> 0.92       (no data)       (no data)
//   breadcrumbs             11/21 -> 0.48       (no data)       (no data)
//   exclusiveTo             3/3 -> 0.57         0/32 -> 0.03    (no data)
//   disabledByQuest         0 of 9 edges, any tie -> 0.05
//   availableUntilCompleted 1/2 -> 0.33         0/2 -> 0.17     (no data)
//
// prerequisite: some character was blocked while never having taken P. pendingPrerequisite: P was
// in the log at every block, which is exactly what a breadcrumb nearly everyone does looks like;
// Forever uses those far more than Classic, hence the split by population. "Strong" ties are
// same-name variants and observed hand-offs, "weak" ones a shared giver, a shared list or a
// finisher that starts the other quest (locality.ts). Re-measure whenever the signal changes:
// `npm run relations -- score absence` at several --min-score values shows whether score bands
// still match measured precision.

import type { LocalReason } from "./locality";

export type ScoreGroup =
  | "prerequisite"
  | "pendingPrerequisite"
  | "anyPrerequisite"
  | "chain"
  | "breadcrumbs"
  | "exclusiveTo"
  | "disabledByQuest"
  | "availableUntilCompleted";

type Tie = "strong" | "weak" | "none";

const NO_DATA_CEILING = 0.1;

const CEILING: Record<ScoreGroup, Partial<Record<Tie, number>>> = {
  prerequisite: { strong: 0.94, weak: 0.33, none: 0.2 },
  pendingPrerequisite: { strong: 0.84, weak: 0.21 },
  anyPrerequisite: { strong: 0.63 },
  chain: { strong: 0.92 },
  breadcrumbs: { strong: 0.48 },
  exclusiveTo: { strong: 0.57, weak: 0.03 },
  disabledByQuest: { strong: 0.05, weak: 0.05, none: 0.05 },
  availableUntilCompleted: { strong: 0.33, weak: 0.17 },
};

const FOREVER_PENDING_PREREQUISITE_CEILING = 0.42;

function tieOf(reason: LocalReason | undefined): Tie {
  if (reason === undefined) return "none";
  return reason === "sameName" || reason === "handoff" ? "strong" : "weak";
}

/** `shares`: per character, the best blame share for this edge. */
export function scoreEdge(group: ScoreGroup, shares: number[], reason: LocalReason | undefined, foreverQuest: boolean): number {
  const tie = tieOf(reason);
  const forever = group === "pendingPrerequisite" && foreverQuest && tie === "strong";
  const ceiling = forever ? FOREVER_PENDING_PREREQUISITE_CEILING : (CEILING[group][tie] ?? NO_DATA_CEILING);
  const evidence = shares.reduce((sum, share) => sum + share, 0);
  return ceiling * (1 - Math.exp(-4 * evidence));
}
