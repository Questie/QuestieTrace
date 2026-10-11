import { describe, expect, it } from "vitest";
import { classAllowed, isStaticallyEligible, raceAllowed } from "./eligibility";
import { catalogQuest } from "./fixtures";

describe("raceAllowed", () => {
  it("maps Skyborne race ids to bits 32/33 without truncation", () => {
    // 4294967373 = 2^32 + 77: Skyborne Alliance (95) plus Human, Dwarf, Night Elf and Gnome.
    expect(raceAllowed(4294967373, 95)).toBe(true);
    expect(raceAllowed(4294967373, 96)).toBe(false);
    expect(raceAllowed(4294967373, 1)).toBe(true);
    expect(raceAllowed(4294967373, 2)).toBe(false);
  });

  it("treats a missing or zero mask as everyone", () => {
    expect(raceAllowed(undefined, 5)).toBe(true);
    expect(raceAllowed(0, 95)).toBe(true);
  });
});

describe("classAllowed", () => {
  it("uses 2^(classId - 1) bits", () => {
    expect(classAllowed(1024, 11)).toBe(true); // Druid
    expect(classAllowed(1024, 1)).toBe(false);
  });
});

describe("isStaticallyEligible", () => {
  const quest = catalogQuest(1, { requiredLevel: 10, requiredClasses: 2 ** (4 - 1) });

  it("rejects the wrong class and too-low levels, honouring slack", () => {
    expect(isStaticallyEligible(quest, { classId: 4 }, 10)).toBe(true);
    expect(isStaticallyEligible(quest, { classId: 1 }, 10)).toBe(false);
    expect(isStaticallyEligible(quest, { classId: 4 }, 9)).toBe(false);
    expect(isStaticallyEligible(quest, { classId: 4 }, 11, { levelSlack: -2 })).toBe(false);
  });
});
