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
      spawns: new Map(),
      zoneID: new Map(),
      questStarts: new Map(),
      questEnds: new Map(),
    });
    expect(records.get(2843)).toEqual({ name: "Suspicious Chest" });
  });

  it("should include spawns and zoneID when present", () => {
    const records = emitObjectRecords({
      name: new Map([[2843, fact(2843, "Suspicious Chest")]]),
      spawns: new Map([[2843, fact(2843, { 40: [[30.01, 86.02]] })]]),
      zoneID: new Map([[2843, fact(2843, 40)]]),
      questStarts: new Map(),
      questEnds: new Map(),
    });
    expect(records.get(2843)).toEqual({
      name: "Suspicious Chest",
      spawns: { 40: [[30.01, 86.02]] },
      zoneID: 40,
    });
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitObjectRecords({ name: new Map(), spawns: new Map(), zoneID: new Map(), questStarts: new Map(), questEnds: new Map() });
    expect(records.size).toBe(0);
  });
});
