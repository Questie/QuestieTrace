// Edge features -> score: the measured precision of the edge's bucket, Laplace-smoothed
// ((tp + 1) / (n + 2)) so small buckets stay below certainty. Buckets are pooled across the two
// truth tiers where they agree, and split by tier where they do not: Forever's authored data lists
// co-completed copies, and inherited Classic data predates the prerequisites Forever added to old
// quests. Counts below are tp/n from the 2026-10-11 run (1,843 characters); re-measure with
// `--dump` when the rules or the data change much.

import type { EdgeFeatures } from "./infer";

/** Supporting characters from which an edge counts as well sampled. */
const MANY_CHARACTERS = 10;

export function scoreEdge(f: EdgeFeatures): number {
  const levelUnlocked = f.levelUnlockedShare > 0;
  const many = f.support >= MANY_CHARACTERS;

  if (f.field === "preQuestGroup") {
    // Unconfirmed (play order only): 3/13 pooled, Classic 0/10 and Forever 3/3 from one quest.
    if (!f.allOfConfirmed) return 0.27;
    // Confirmed: Forever 15/15; Classic 5/9, the misses being Forever quests newly required by old ones.
    return f.forever ? 0.94 : 0.55;
  }

  // Faction-split OR members: 16/16 (Forever only).
  if (f.shape === "or") return 0.94;

  // Rare since copies cover most races: 1 emitted, unjudged.
  if (f.shape === "variant") return 0.5;

  // Copies of Forever quests' prerequisites: 404/412 (0/2 when levelling explains the offer).
  // Questie never lists Forever's copies on Classic quests: 2/21.
  if (f.shape === "copy" && !f.forever) return 0.13;
  if (f.shape === "copy" && f.tier === "primary") return levelUnlocked ? 0.25 : 0.98;

  // Secondary, own or copy: 30/38 from 10+ characters, 3/10 from 5-9.
  if (f.tier === "secondary") return many ? 0.78 : 0.33;

  // Primary: 10+ characters 574/581 (51/53 with a level-up just before the first offer),
  // 2-9 characters 161/167 (20/22).
  if (many) return levelUnlocked ? 0.95 : 0.98;
  return levelUnlocked ? 0.88 : 0.96;
}
