import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { apiSample, makeSession } from "../../../test/fixtures";
import { mergeQuestXp, observeQuestXp, type QuestXpValue } from "./questXp";

function observation(xp: number, playerLevel: number): Observation<QuestXpValue> {
  return { entityId: 1, value: { xp, playerLevel }, confidence: "high", provenance: { session: "s", t: 0 } };
}

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

describe("mergeQuestXp", () => {
  it("should ignore samples of a quest more than five levels below the player", () => {
    expect(mergeQuestXp([observation(300, 20), observation(900, 14)], 14)).toBe(900);
    expect(mergeQuestXp([observation(300, 20)], 14)).toBeUndefined();
  });

  it("should return the most observed XP", () => {
    expect(mergeQuestXp([observation(900, 10), observation(950, 10), observation(900, 10)], 10)).toBe(900);
  });

  it("should return nothing when the most observed values tie", () => {
    expect(mergeQuestXp([observation(900, 10), observation(950, 10)], 10)).toBeUndefined();
  });

  it("should return nothing without a quest level", () => {
    expect(mergeQuestXp([observation(900, 10)], undefined)).toBeUndefined();
  });
});
