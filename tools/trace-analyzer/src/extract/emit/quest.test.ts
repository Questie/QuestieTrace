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
      startedBy: new Map(),
    });
    expect(records.get(96659)).toEqual({ name: "A Threat Within" });
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitQuestRecords({ name: new Map(), questLevel: new Map(), requiredLevel: new Map(), startedBy: new Map() });
    expect(records.size).toBe(0);
  });
});
