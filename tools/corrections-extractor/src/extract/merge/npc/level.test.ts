import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { mergeLevelRange } from "./level";

function sighting(sessionKey: number, level: number): Observation<number> {
  return { entityId: 1, value: level, confidence: "high", provenance: { session: `s${sessionKey}`, sessionKey: `k${sessionKey}`, t: 0 } };
}

describe("mergeLevelRange", () => {
  it("should return the lowest and highest level", () => {
    expect(mergeLevelRange([sighting(1, 12), sighting(2, 12), sighting(3, 13), sighting(4, 13)])).toEqual({ min: 12, max: 13 });
  });

  it("should ignore levels seen in only one session", () => {
    expect(mergeLevelRange([sighting(1, 12), sighting(2, 12), sighting(3, 20)])).toEqual({ min: 12, max: 12 });
  });

  it("should ignore levels seen far less often than the most common level", () => {
    const observations = Array.from({ length: 30 }, (_, index) => sighting(index, 23));
    observations.push(sighting(100, 22), sighting(101, 22));
    expect(mergeLevelRange(observations)).toEqual({ min: 23, max: 23 });
  });

  it("should count one session once per level", () => {
    expect(mergeLevelRange([sighting(1, 12), sighting(1, 12)])).toBeUndefined();
  });
});
