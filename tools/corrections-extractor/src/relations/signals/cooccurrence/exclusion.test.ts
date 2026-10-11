import { describe, expect, it } from "vitest";
import { catalogQuest } from "../../core/fixtures";
import type { CatalogQuest, QuestEpisode } from "../../core/types";
import { inferExclusions } from "./exclusion";
import { catalogOf, populationOf, session } from "./fixtures";

// Twelve characters of one race and class, half completing 51 and half 52: 3 expected together
// (2.8 once a thirteenth character joins), against a bar of 2.5.
const params = { minCompleters: 2, minExpected: 2.5, minGroupExpected: 3, minGroupMemberCompleters: 2 };
const split = (raceOfB = 1): QuestEpisode[] => [
  ...Array.from({ length: 6 }, (_, i) => session(`a${i}`, { initial: [8], turnIns: [[100, 51]] })),
  ...Array.from({ length: 6 }, (_, i) => session(`b${i}`, { race: raceOfB, initial: [8], turnIns: [[100, 52]] })),
];
const pairs = (episodes: QuestEpisode[], quests: Array<number | CatalogQuest> = [8, 51, 52]) =>
  inferExclusions(populationOf(episodes, catalogOf(quests)), params).edges.map(({ a, b }) => [a, b]);

describe("inferExclusions", () => {
  it("flags two quests eligible characters complete often but never together", () => {
    expect(pairs(split())).toEqual([[51, 52]]);
  });

  it("does not compare quests behind different hidden gates", () => {
    // Different profession skills explain the split on their own.
    expect(pairs(split(), [8, 51, catalogQuest(52, { requiredSkill: [171, 1] })])).toEqual([]);
  });

  it("expects nothing in common between races that each did their own quest", () => {
    expect(pairs(split(3))).toEqual([]);
  });

  it("drops the pair once a character held both at the same time", () => {
    expect(pairs([...split(), session("both", { initial: [8], accepts: [[10, 51], [20, 52]] })])).toEqual([]);
  });
});
