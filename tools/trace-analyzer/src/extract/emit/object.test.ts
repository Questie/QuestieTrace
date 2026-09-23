import { describe, expect, it } from "vitest";
import type { Fact } from "../aggregate";
import { emitObjectRecords } from "./object";

function fact<T>(entityId: number, value: T): Fact<T> {
  return { entityId, value, confidence: "high", observationCount: 1, alternativeCount: 0 };
}

describe("emitObjectRecords", () => {
  it("should only set fields that were actually extracted", () => {
    const records = emitObjectRecords({
      name: new Map([[2843, fact(2843, "Suspicious Chest")]]),
      questStarts: new Map(),
      questEnds: new Map(),
    });
    expect(records.get(2843)).toEqual({ name: "Suspicious Chest" });
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitObjectRecords({ name: new Map(), questStarts: new Map(), questEnds: new Map() });
    expect(records.size).toBe(0);
  });
});
