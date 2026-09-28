import { describe, expect, it } from "vitest";
import type { Fact } from "../aggregate";
import { emitQuestRecords } from "./quest";

function fact<T>(entityId: number, value: T): Fact<T> {
  return { entityId, value, confidence: "high", observationCount: 1, alternativeCount: 0 };
}

describe("emitQuestRecords", () => {
  it("should only set fields that were actually extracted", () => {
    const records = emitQuestRecords({
      name: new Map([[96659, fact(96659, "A Threat Within")]]),
      questLevel: new Map(),
      requiredLevel: new Map(),
      zoneOrSort: new Map(),
      objectives: new Map(),
      objectivesText: new Map(),
      triggerEnd: new Map(),
      startedBy: new Map(),
      finishedBy: new Map(),
    });
    expect(records.get(96659)).toEqual({ name: "A Threat Within" });
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitQuestRecords({
      name: new Map(),
      questLevel: new Map(),
      requiredLevel: new Map(),
      zoneOrSort: new Map(),
      objectives: new Map(),
      objectivesText: new Map(),
      triggerEnd: new Map(),
      startedBy: new Map(),
      finishedBy: new Map(),
    });
    expect(records.size).toBe(0);
  });

  it("should clamp requiredLevel to questLevel when requiredLevel exceeds questLevel", () => {
    const records = emitQuestRecords({
      name: new Map([[96659, fact(96659, "Test Quest")]]),
      questLevel: new Map([[96659, fact(96659, 30)]]),
      requiredLevel: new Map([[96659, fact(96659, 40)]]), // higher than questLevel
      zoneOrSort: new Map(),
      objectives: new Map(),
      objectivesText: new Map(),
      triggerEnd: new Map(),
      startedBy: new Map(),
      finishedBy: new Map(),
    });
    // requiredLevel should be clamped to questLevel (30)
    expect(records.get(96659)).toEqual({
      name: "Test Quest",
      questLevel: 30,
      requiredLevel: 30,
    });
  });

  it("should not clamp requiredLevel when it is <= questLevel", () => {
    const records = emitQuestRecords({
      name: new Map([[96659, fact(96659, "Test Quest")]]),
      questLevel: new Map([[96659, fact(96659, 40)]]),
      requiredLevel: new Map([[96659, fact(96659, 30)]]), // lower than questLevel
      zoneOrSort: new Map(),
      objectives: new Map(),
      objectivesText: new Map(),
      triggerEnd: new Map(),
      startedBy: new Map(),
      finishedBy: new Map(),
    });
    // requiredLevel should remain as-is (30)
    expect(records.get(96659)).toEqual({
      name: "Test Quest",
      questLevel: 40,
      requiredLevel: 30,
    });
  });

  it("should not clamp when questLevel is missing", () => {
    const records = emitQuestRecords({
      name: new Map([[96659, fact(96659, "Test Quest")]]),
      questLevel: new Map(),
      requiredLevel: new Map([[96659, fact(96659, 40)]]),
      zoneOrSort: new Map(),
      objectives: new Map(),
      objectivesText: new Map(),
      triggerEnd: new Map(),
      startedBy: new Map(),
      finishedBy: new Map(),
    });
    // requiredLevel should remain as-is when no questLevel to compare against
    expect(records.get(96659)).toEqual({
      name: "Test Quest",
      requiredLevel: 40,
    });
  });
});
