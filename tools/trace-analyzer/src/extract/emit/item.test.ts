import { describe, expect, it } from "vitest";
import type { Fact } from "../aggregate";
import { emitItemRecords } from "./item";

function fact<T>(entityId: number, value: T): Fact<T> {
  return { entityId, value, confidence: "high", observationCount: 1, alternativeCount: 0 };
}

describe("emitItemRecords", () => {
  it("should only set fields that were actually extracted", () => {
    const records = emitItemRecords({ name: new Map([[750, fact(750, "Tough Wolf Meat")]]) });
    expect(records.get(750)).toEqual({ name: "Tough Wolf Meat" });
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitItemRecords({ name: new Map() });
    expect(records.size).toBe(0);
  });
});
