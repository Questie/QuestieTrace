// QuestXP.db[questId] = {questLevel, xp}. Observed in observers/quest/questXp.ts.

import { questXpDifficulty, questXpFor } from "../../resources/quest-xp";
import type { Observation } from "../../observation";
import type { QuestXpValue } from "../../observers/quest/questXp";

/** A quest more than this many levels below the player is scaled down. */
const XP_FULL_RANGE = 5;

/**
 * The raw XP of the difficulty most samples agree on. Only samples that are not scaled down
 * and match the rounded XP of one difficulty of the quest level in the Forever QuestXP table
 * count; buffed or mis-attributed samples match nothing. Without a quest level in the
 * table, or when the most observed difficulties tie, no value is exported.
 */
export function mergeQuestXp(observations: Observation<QuestXpValue>[], questLevel: number | undefined): number | undefined {
  if (questLevel === undefined) return undefined;

  const counts = new Map<number, number>();
  for (const { value } of observations) {
    if (questLevel - value.playerLevel < -XP_FULL_RANGE) continue;
    const difficulty = questXpDifficulty(questLevel, value.xp);
    if (difficulty !== undefined) counts.set(difficulty, (counts.get(difficulty) ?? 0) + 1);
  }

  let best: number | undefined;
  let bestCount = 0;
  let tied = false;
  for (const [difficulty, count] of counts) {
    if (count > bestCount) {
      best = difficulty;
      bestCount = count;
      tied = false;
    } else if (count === bestCount) {
      tied = true;
    }
  }
  return best === undefined || tied ? undefined : questXpFor(questLevel, best);
}
