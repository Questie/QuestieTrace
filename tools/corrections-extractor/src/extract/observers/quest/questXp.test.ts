import { describe, expect, it } from "vitest";
import { apiSample, makeSession } from "../../../test/fixtures";
import { observeQuestXp } from "./questXp";

describe("observeQuestXp", () => {
  it("should pair the reward XP with the quest id and player level sampled at the same time", () => {
    const session = makeSession({
      GetQuestID: [apiSample(0, 0, 0), apiSample(10, 10.5, 312), apiSample(10, 10.6, 0)],
      GetRewardXP: [apiSample(0, 0, 0), apiSample(10, 10.5, 900), apiSample(10, 10.6, 0)],
      UnitLevel: { player: [apiSample(0, 0, 9)] },
    });

    expect(observeQuestXp(session)).toEqual([
      { entityId: 312, value: { xp: 900, playerLevel: 9 }, confidence: "high", provenance: { session: "test-session", t: 10 } },
    ]);
  });

  it("should skip rewards without a quest id or player level", () => {
    const noQuest = makeSession({
      GetQuestID: [apiSample(0, 0, 0)],
      GetRewardXP: [apiSample(5, 5, 900)],
      UnitLevel: { player: [apiSample(0, 0, 9)] },
    });
    const noLevel = makeSession({
      GetQuestID: [apiSample(5, 5, 312)],
      GetRewardXP: [apiSample(5, 5, 900)],
    });

    expect(observeQuestXp(noQuest)).toEqual([]);
    expect(observeQuestXp(noLevel)).toEqual([]);
  });
});
