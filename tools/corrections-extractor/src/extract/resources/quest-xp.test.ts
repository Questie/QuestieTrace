import { describe, expect, it } from "vitest";
import { questXpDifficulty, questXpFor, roundQuestXp } from "./quest-xp";

describe("roundQuestXp", () => {
  it("should round like the client for each XP range", () => {
    expect(roundQuestXp(530)).toBe(525);
    expect(roundQuestXp(910)).toBe(900);
    expect(roundQuestXp(2450)).toBe(2450);
    expect(roundQuestXp(93)).toBe(95);
    expect(roundQuestXp(103)).toBe(100);
  });
});

describe("questXpDifficulty", () => {
  it("should find the difficulty of an exact value", () => {
    expect(questXpDifficulty(30, 2450)).toBe(5);
  });

  it("should find the difficulty of a value the client rounded", () => {
    expect(questXpDifficulty(12, 900)).toBe(5);
    expect(questXpDifficulty(8, 525)).toBe(4);
  });

  it("should return undefined for a value that is not in the level's row", () => {
    expect(questXpDifficulty(30, 2451)).toBeUndefined();
    expect(questXpDifficulty(30, 0)).toBeUndefined();
  });

  it("should return undefined for a level without a row", () => {
    expect(questXpDifficulty(0, 100)).toBeUndefined();
    expect(questXpDifficulty(101, 100)).toBeUndefined();
  });

  it("should never match two difficulties of one level", () => {
    for (let level = 1; level <= 100; level++) {
      for (let difficulty = 1; difficulty <= 7; difficulty++) {
        const raw = questXpFor(level, difficulty)!;
        expect(questXpDifficulty(level, roundQuestXp(raw))).toBe(difficulty);
      }
    }
  });
});

describe("questXpFor", () => {
  it("should return the raw XP, not the rounded one", () => {
    expect(questXpFor(12, 5)).toBe(910);
    expect(questXpFor(8, 4)).toBe(530);
  });

  it("should return undefined for the empty difficulty columns and unknown levels", () => {
    expect(questXpFor(30, 0)).toBeUndefined();
    expect(questXpFor(30, 8)).toBeUndefined();
    expect(questXpFor(500, 1)).toBeUndefined();
  });
});
