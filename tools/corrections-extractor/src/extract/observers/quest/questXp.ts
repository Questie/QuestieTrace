// QuestXP.db[questId] = {questLevel, xp} - Questie support data, not a questKeys field.
//
// Source: GetRewardXP, sampled in the quest dialog together with GetQuestID. The client
// reports the XP already scaled to the player's level, while the database stores the base
// value. The two are equal while the quest is at most XP_FULL_RANGE levels below the
// player (the multiplier in Questie's getAdjustedXP is capped at 1 from there), so only
// samples in that range are merged. Each sample is kept with the player's level, which is
// compared with the quest level at merge time because the quest level is a finalized fact.
// The client also rounds the XP it shows, so a sample is matched with the Forever QuestXP
// table (resources/quest-xp.ts) and the table's raw value is exported.

import { getStream } from "../../../core/emulator";
import type { FunctionStreamEntry, SessionRecord } from "../../../core/types";
import { questXpDifficulty, questXpFor } from "../../resources/quest-xp";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export interface QuestXpValue {
  xp: number;
  playerLevel: number;
}

/** A quest more than this many levels below the player is scaled down. */
const XP_FULL_RANGE = 5;

/** Value of the latest entry recorded at or before `tp`, which orders samples taken at the same `t`. */
function valueAtTp(stream: FunctionStreamEntry[], tp: number): unknown {
  let lo = 0;
  let hi = stream.length - 1;
  let result = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    if (stream[mid].tp <= tp) {
      result = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return result === -1 ? undefined : stream[result].v;
}

export const observeQuestXp: FieldObserver<QuestXpValue> = (session: SessionRecord) => {
  const observations: Observation<QuestXpValue>[] = [];
  const questIds = getStream(session, "GetQuestID") ?? [];
  const levels = getStream(session, "UnitLevel", "player") ?? [];

  for (const entry of getStream(session, "GetRewardXP") ?? []) {
    if (typeof entry.v !== "number" || !(entry.v > 0)) continue;
    const questId = valueAtTp(questIds, entry.tp);
    const playerLevel = valueAtTp(levels, entry.tp);
    if (typeof questId !== "number" || questId <= 0 || typeof playerLevel !== "number") continue;

    observations.push({
      entityId: questId,
      value: { xp: entry.v, playerLevel },
      confidence: "high",
      provenance: { session: sessionLabel(session), t: entry.t },
    });
  }
  return observations;
};

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
