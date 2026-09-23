import { describe, expect, it } from "vitest";
import type { Fact } from "../aggregate";
import { emitNpcRecords } from "./npc";

function fact<T>(entityId: number, value: T): Fact<T> {
  return { entityId, value, confidence: "high", observationCount: 1, alternativeCount: 0 };
}

describe("emitNpcRecords", () => {
  it("should only set fields that were actually extracted", () => {
    const records = emitNpcRecords({
      name: new Map([[823, fact(823, "Deputy Willem")]]),
      minLevel: new Map([[823, fact(823, 18)]]),
      maxLevel: new Map(),
      questStarts: new Map(),
      questEnds: new Map(),
    });
    expect(records.get(823)).toEqual({ name: "Deputy Willem", minLevel: 18 });
  });

  it("should union entity ids across all fields, including ids only seen in one field", () => {
    const records = emitNpcRecords({
      name: new Map([[1, fact(1, "Only Named")]]),
      minLevel: new Map([[2, fact(2, 5)]]),
      maxLevel: new Map(),
      questStarts: new Map(),
      questEnds: new Map(),
    });
    expect([...records.keys()].sort()).toEqual([1, 2]);
    expect(records.get(1)).toEqual({ name: "Only Named" });
    expect(records.get(2)).toEqual({ minLevel: 5 });
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitNpcRecords({
      name: new Map(),
      minLevel: new Map(),
      maxLevel: new Map(),
      questStarts: new Map(),
      questEnds: new Map(),
    });
    expect(records.size).toBe(0);
  });
});
