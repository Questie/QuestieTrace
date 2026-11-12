import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import type { QuestXpValue } from "../../observers/quest/questXp";
import { mergeQuestXp } from "./questXp";

function observation(xp: number, playerLevel: number): Observation<QuestXpValue> {
  return { entityId: 1, value: { xp, playerLevel }, confidence: "high", provenance: { session: "s", t: 0 } };
}

describe("mergeQuestXp", () => {
  it("should export the raw table XP for a sample the client rounded", () => {
    expect(mergeQuestXp([observation(900, 12)], 12)).toBe(910);
    expect(mergeQuestXp([observation(525, 8)], 8)).toBe(530);
  });

  it("should ignore samples that match no difficulty of the quest level", () => {
    expect(mergeQuestXp([observation(1000, 12), observation(900, 12)], 12)).toBe(910);
    expect(mergeQuestXp([observation(1000, 12)], 12)).toBeUndefined();
  });

  it("should ignore samples of a quest more than five levels below the player", () => {
    expect(mergeQuestXp([observation(900, 17)], 12)).toBe(910);
    expect(mergeQuestXp([observation(900, 18)], 12)).toBeUndefined();
  });

  it("should return the difficulty most samples agree on", () => {
    expect(mergeQuestXp([observation(900, 12), observation(230, 12), observation(900, 12)], 12)).toBe(910);
  });

  it("should return nothing when the most observed difficulties tie", () => {
    expect(mergeQuestXp([observation(900, 12), observation(230, 12)], 12)).toBeUndefined();
  });

  it("should return nothing without a quest level or a table row for it", () => {
    expect(mergeQuestXp([observation(900, 12)], undefined)).toBeUndefined();
    expect(mergeQuestXp([observation(900, 12)], -1)).toBeUndefined();
  });
});
