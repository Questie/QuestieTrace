import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { mergeRank } from "./rank";

function vote(sessionKey: number, rank: number): Observation<number> {
  return { entityId: 1, value: rank, confidence: "high", provenance: { session: `s${sessionKey}`, sessionKey: `k${sessionKey}`, t: 0 } };
}

describe("mergeRank", () => {
  it("should return the rank seen in the most sessions", () => {
    expect(mergeRank([vote(1, 1), vote(2, 1), vote(3, 4)])).toBe(1);
  });

  it("should count several observations in one session once", () => {
    expect(mergeRank([vote(1, 4), vote(1, 4), vote(1, 4), vote(2, 1), vote(3, 1)])).toBe(1);
  });

  it("should return undefined when the most common ranks tie", () => {
    expect(mergeRank([vote(1, 1), vote(2, 1), vote(3, 4), vote(4, 4)])).toBeUndefined();
  });

  it("should return undefined when the rank was seen in only one session", () => {
    expect(mergeRank([vote(1, 1), vote(1, 1)])).toBeUndefined();
  });

  it("should return undefined for rank 0", () => {
    expect(mergeRank([vote(1, 0), vote(2, 0)])).toBeUndefined();
  });
});
