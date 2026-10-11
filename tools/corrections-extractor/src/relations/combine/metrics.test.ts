import { describe, expect, it } from "vitest";
import type { CandidateFile, GroundTruth, RelationSet } from "../core/types";
import { heldOutMetrics } from "./metrics";

const truth: GroundTruth = {
  generatedAt: "",
  sources: [],
  tiers: { authoredForever: { "92001": { preQuestSingle: [10] }, "92002": { preQuestSingle: [20] }, "92003": { preQuestSingle: [30] } }, inheritedClassic: {} },
};

function signal(name: string, claims: Array<[number, number, number]>): CandidateFile {
  return {
    signal: name,
    generatedAt: "",
    inputCount: 0,
    params: {},
    coveredQuestIds: [92001, 92002, 92003],
    candidates: claims.map(([questId, target, score]) => ({ questId, field: "preQuestSingle", target, score, support: 1, contradict: 0, evidence: [] })),
  };
}

function bestSingleFor(files: CandidateFile[]) {
  const combined = new Map<number, RelationSet>([
    [92001, { preQuestSingle: [10] }],
    [92002, { preQuestSingle: [20] }],
    [92003, { preQuestSingle: [30] }],
  ]);
  const [authored] = heldOutMetrics(combined, combined, files, truth);
  return authored.rows.find((row) => row.field === "preQuestSingle")!.bestSingle;
}

describe("heldOutMetrics", () => {
  it("compares against the single signal with the most recall at no less precision, at that signal's own threshold", () => {
    const partial = signal("partial", [
      [92001, 10, 0.9],
      [92002, 20, 0.9],
    ]);
    // Complete, but only precise once its 0.3 claim is cut off at 0.5.
    const complete = signal("complete", [
      [92001, 10, 0.9],
      [92002, 20, 0.9],
      [92003, 30, 0.9],
      [92003, 99, 0.3],
    ]);

    expect(bestSingleFor([partial, complete])).toMatchObject({ signal: "complete", minScore: 0.5, score: { precision: 1, recall: 1 } });
  });

  it("falls back to the most precise single signal when none matches the combined precision", () => {
    const precise = signal("precise", [
      [92001, 10, 0.9],
      [92002, 20, 0.9],
      [92001, 97, 0.9],
    ]);
    const broad = signal("broad", [
      [92001, 10, 0.9],
      [92002, 20, 0.9],
      [92003, 30, 0.9],
      [92001, 97, 0.9],
      [92002, 98, 0.9],
      [92003, 99, 0.9],
    ]);

    expect(bestSingleFor([precise, broad])?.signal).toBe("precise");
  });
});
