import { describe, expect, it } from "vitest";
import { mergeRequiredLevel } from "./requiredLevel";

describe("mergeRequiredLevel", () => {
  it("should return minimum value across observations from multiple sessions", () => {
    const observations = [
      { entityId: 96659, value: 30, confidence: "medium" as const, provenance: { session: "s1", t: 3 } },
      { entityId: 96659, value: 25, confidence: "medium" as const, provenance: { session: "s2", t: 5 } },
      { entityId: 96659, value: 28, confidence: "medium" as const, provenance: { session: "s3", t: 7 } },
    ];

    expect(mergeRequiredLevel(observations)).toBe(25);
  });

  it("should handle single observation", () => {
    const observations = [
      { entityId: 96659, value: 42, confidence: "high" as const, provenance: { session: "s1", t: 3 } },
    ];

    expect(mergeRequiredLevel(observations)).toBe(42);
  });

  it("should handle observations with different confidences", () => {
    const observations = [
      { entityId: 96659, value: 35, confidence: "high" as const, provenance: { session: "s1", t: 3 } },
      { entityId: 96659, value: 25, confidence: "medium" as const, provenance: { session: "s2", t: 5 } },
      { entityId: 96659, value: 30, confidence: "low" as const, provenance: { session: "s3", t: 7 } },
    ];

    // Minimum regardless of confidence
    expect(mergeRequiredLevel(observations)).toBe(25);
  });
});
