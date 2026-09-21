import { describe, expect, it } from "vitest";
import type { Observation } from "./observation";
import { aggregateField } from "./aggregate";

function obs<T>(entityId: number, value: T, confidence: Observation<T>["confidence"], t: number): Observation<T> {
  return { entityId, value, confidence, provenance: { session: "s", t } };
}

describe("aggregateField", () => {
  it("should return one Fact per distinct entityId", () => {
    const facts = aggregateField([obs(1, "a", "high", 0), obs(2, "b", "high", 0)]);
    expect([...facts.keys()].sort()).toEqual([1, 2]);
  });

  it("should pick the highest-confidence value when observations disagree", () => {
    const facts = aggregateField([obs(1, "low-value", "low", 0), obs(1, "high-value", "high", 1)]);
    expect(facts.get(1)).toMatchObject({ value: "high-value", confidence: "high", alternativeCount: 1 });
  });

  it("should break ties by most-frequently-observed value at equal confidence", () => {
    const facts = aggregateField([
      obs(1, "rare", "high", 0),
      obs(1, "common", "high", 1),
      obs(1, "common", "high", 2),
    ]);
    expect(facts.get(1)).toMatchObject({ value: "common", confidence: "high", observationCount: 3, alternativeCount: 1 });
  });

  it("should break remaining ties by most-recent observation", () => {
    const facts = aggregateField([obs(1, "older", "high", 0), obs(1, "newer", "high", 10)]);
    expect(facts.get(1)?.value).toBe("newer");
  });

  it("should report observationCount and confidence even with a single observation", () => {
    const facts = aggregateField([obs(1, "only", "medium", 5)]);
    expect(facts.get(1)).toMatchObject({ value: "only", confidence: "medium", observationCount: 1, alternativeCount: 0 });
  });

  it("should use a custom merge function when provided, bypassing scalar tie-break", () => {
    const facts = aggregateField(
      [obs(1, 10, "low", 0), obs(1, 20, "low", 1)],
      (observationsForEntity) => observationsForEntity.reduce((sum, o) => sum + o.value, 0),
    );
    expect(facts.get(1)).toMatchObject({ value: 30, confidence: "low", observationCount: 2, alternativeCount: 0 });
  });
});
