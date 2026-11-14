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
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export interface QuestXpValue {
  xp: number;
  playerLevel: number;
}

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
