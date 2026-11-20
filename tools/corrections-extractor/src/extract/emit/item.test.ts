import { describe, expect, it } from "vitest";
import type { Fact } from "../aggregate";
import { emitItemRecords } from "./item";

function fact<T>(entityId: number, value: T): Fact<T> {
  return { entityId, value, confidence: "high", observationCount: 1, alternativeCount: 0 };
}

describe("emitItemRecords", () => {
  it("should only set fields that were actually extracted", () => {
    const records = emitItemRecords({ name: new Map([[750, fact(750, "Tough Wolf Meat")]]), npcDrops: new Map(), objectDrops: new Map() });
    expect(records.get(750)).toEqual({ name: "Tough Wolf Meat" });
  });

  it("should include npcDrops and objectDrops when present", () => {
    const records = emitItemRecords({
      name: new Map([[12345, fact(12345, "Feather")] ]),
      npcDrops: new Map([[12345, fact(12345, [823, 824])]]),
      objectDrops: new Map([[12345, fact(12345, [2843])]]),
    });
    expect(records.get(12345)).toEqual({
      name: "Feather",
      npcDrops: [823, 824],
      objectDrops: [2843],
    });
  });

  it("should skip drops of world drop items", () => {
    const ids = [248682, 248713, 211780, 211786, 281018, 262889, 249427, 249428, 274964];
    const records = emitItemRecords({
      name: new Map(),
      npcDrops: new Map(ids.map((id) => [id, fact(id, [1])])),
      objectDrops: new Map([...ids, 12345].map((id) => [id, fact(id, [2])])),
    });
    expect([...records.keys()]).toEqual([12345]);
  });

  it("should return an empty map when no facts were observed", () => {
    const records = emitItemRecords({ name: new Map(), npcDrops: new Map(), objectDrops: new Map() });
    expect(records.size).toBe(0);
  });
});
