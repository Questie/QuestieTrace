import { describe, expect, it } from "vitest";
import type { Observation } from "./observation";
import {
  addToScalarSummary,
  aggregateField,
  aggregateNewestScalarSummary,
  aggregateScalarSummary,
  type ScalarSummary,
} from "./aggregate";

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

describe("scalar summary", () => {
  it("should produce the same Facts when observations are added in batches as aggregateField does at once", () => {
    const batchA = [obs(1, "common", "medium", 1), obs(2, { x: 1 }, "low", 1), obs(1, "rare", "medium", 2)];
    const batchB = [obs(1, "common", "high", 3), obs(2, { x: 1 }, "low", 4), obs(1, "rare", "medium", 5)];
    const summary: ScalarSummary<unknown> = new Map();

    addToScalarSummary(summary, batchA);
    addToScalarSummary(summary, batchB);

    expect(aggregateScalarSummary(summary)).toEqual(aggregateField<unknown>([...batchA, ...batchB]));
  });

  it("should keep one entry per distinct value instead of every observation", () => {
    const summary: ScalarSummary<string> = new Map();

    addToScalarSummary(summary, [obs(1, "a", "high", 0), obs(1, "a", "high", 1), obs(1, "b", "low", 2)]);

    expect(summary.get(1)?.size).toBe(2);
    expect(aggregateScalarSummary(summary).get(1)).toMatchObject({ value: "a", observationCount: 3, alternativeCount: 1 });
  });
});

function sessionObs<T>(entityId: number, value: T, sessionKey: number, sessionTime: number | null): Observation<T> {
  return { entityId, value, confidence: "high", provenance: { session: `s${sessionKey}`, sessionKey, sessionTime, t: 0 } };
}

describe("aggregateNewestScalarSummary", () => {
  it("should pick the value of the newest session over a more frequent older value", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [1, 2, 3].map((key) => sessionObs(1, "Old Name", key, key * 100)));
    addToScalarSummary(summary, [4, 5].map((key) => sessionObs(1, "New Name", key, key * 100)));

    expect(aggregateNewestScalarSummary(summary).get(1)).toMatchObject({ value: "New Name", observationCount: 5, alternativeCount: 1 });
  });

  it("should not let a newest value seen in only one session win", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [1, 2].map((key) => sessionObs(1, "Old Name", key, key * 100)));
    addToScalarSummary(summary, [sessionObs(1, "New Name", 3, 300)]);

    expect(aggregateNewestScalarSummary(summary).get(1)?.value).toBe("Old Name");
  });

  it("should count repeated observations within one session as one session", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [1, 2].map((key) => sessionObs(1, "Old Name", key, key * 100)));
    addToScalarSummary(summary, [sessionObs(1, "New Name", 3, 300), sessionObs(1, "New Name", 3, 300)]);

    expect(aggregateNewestScalarSummary(summary).get(1)?.value).toBe("Old Name");
  });

  it("should fall back to the default merge when no value has a server time", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [1, 2, 3].map((key) => sessionObs(1, "Frequent", key, null)));
    addToScalarSummary(summary, [4, 5].map((key) => sessionObs(1, "Rare", key, null)));

    expect(aggregateNewestScalarSummary(summary).get(1)?.value).toBe("Frequent");
  });

  it("should pick the only value even when it was seen in a single session", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [sessionObs(1, "Only Name", 1, 100)]);

    expect(aggregateNewestScalarSummary(summary).get(1)?.value).toBe("Only Name");
  });

  it("should pick the newest value regardless of the order sessions were added in", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [4, 5].map((key) => sessionObs(1, "New Name", key, key * 100)));
    addToScalarSummary(summary, [1, 2, 3].map((key) => sessionObs(1, "Old Name", key, key * 100)));

    expect(aggregateNewestScalarSummary(summary).get(1)?.value).toBe("New Name");
  });

  it("should only count sessions with a server time towards the newest value", () => {
    const summary: ScalarSummary<string> = new Map();
    addToScalarSummary(summary, [1, 2].map((key) => sessionObs(1, "Old Name", key, key * 100)));
    addToScalarSummary(summary, [sessionObs(1, "New Name", 3, null), sessionObs(1, "New Name", 4, 400)]);

    expect(aggregateNewestScalarSummary(summary).get(1)?.value).toBe("Old Name");
  });
});
