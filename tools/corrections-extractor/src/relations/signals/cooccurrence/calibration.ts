// Edge features -> score: the share of edges with those features that the ground truth confirms.
// Measured 2026-10-10 on 19,200 Forever episodes / 1,827 characters by checking each bin's
// candidates against groundtruth.json outside the signal (which never reads it). Both tiers are
// pooled, leaning on inheritedClassic where they disagree: authoredForever edges come in large
// co-flagged families, so its bins hold few independent decisions. Ground truth misses some real
// Forever edges, so these are lower bounds. Re-measure after changing the inference.

import type { CoFlagEdge } from "./coflag";
import type { ExclusionEdge } from "./exclusion";
import type { PrerequisiteEdge } from "./prerequisites";

/**
 * Hand-offs (taking X within minutes of completing P) are what separates a direct prerequisite
 * from a zone-mate every taker happened to finish earlier; the rank says whether P belongs to the
 * step taken soonest after completion.
 *
 * | rank 1, full                    | AF          | IC          | score |
 * | 5+ hand-offs                    | 454/459 .99 | 533/549 .97 | .97   |
 * | 2-4, at most 2 steps            | 190/190 1.0 |  78/84  .93 | .93   |
 * | 2-4, 3+ steps                   |   -         |  21/24  .88 | .8    |
 * | 1                               |  49/86  .57 |  26/63  .41 | .45   |
 * | 0, some same-session order      |   7/9   .78 |   9/33  .27 | .3    |
 * | 0, never in one session         |   0/26      |   1/122 .01 | .01   |
 * | rank 2+: 5+ / 1-4 / 0 hand-offs | 12/20, 0/15, 0/201 | 6/13, 3/58, 1/1196 | .5 / .04 / .01 |
 * | variants: 5+ / fewer            | 17/24, 0/1  | 2/3, 0/26   | .7 / .01 |
 *
 * Hand-offs explained by the prerequisite's turn-in levelling the character up to X's required
 * level are not counted (see Implication.gaps).
 */
export function prerequisiteScore(edge: PrerequisiteEdge): number {
  const { handoffs } = edge;
  if (edge.shape === "variant") return handoffs >= 5 ? 0.7 : 0.01;
  if (edge.rank > 1) return handoffs >= 5 ? 0.5 : handoffs >= 1 ? 0.04 : 0.01;
  if (handoffs >= 5) return 0.97;
  if (handoffs >= 2) return edge.siblings <= 2 ? 0.93 : 0.8;
  if (handoffs === 1) return 0.45;
  return edge.gaps.length > 0 ? 0.3 : 0.01;
}

/**
 * | observations | AF          | IC          | score |
 * | 3+           | 608/612 .99 | 100/106 .94 | .97   |
 * | 2            |   -         | 110/126 .87 | .87   |
 * Most misses are same-name Forever copies the server co-flags but ground truth leaves unlinked.
 */
export function coFlagScore(edge: CoFlagEdge): number {
  return edge.observations >= 3 ? 0.97 : 0.87;
}

/**
 * "Never together" has no confirmed edge at the emitted thresholds. At looser ones (expected
 * co-completions >= 2 for pairs, any for structural cliques) it measured 1/255 for pairs and 1/11
 * for cliques, so whatever clears the thresholds keeps a low score until a human confirms it.
 */
export function exclusionScore(_edge: ExclusionEdge): number {
  return 0.1;
}
