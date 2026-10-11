import { describe, expect, it } from "vitest";
import { scoreCandidates } from "./score";
import type { CandidateFile, GroundTruth, RelationCandidate } from "./types";

function candidate(questId: number, field: RelationCandidate["field"], target: number, score = 1): RelationCandidate {
  return { questId, field, target, score, support: 1, contradict: 0, evidence: [] };
}

const truth: GroundTruth = {
  generatedAt: "",
  sources: [],
  tiers: {
    authoredForever: {
      "100": { preQuestSingle: [10, 11] },
      "200": { preQuestGroup: [20, 21], exclusiveTo: [300] },
    },
    inheritedClassic: {},
  },
};

function file(candidates: RelationCandidate[], coveredQuestIds: number[]): CandidateFile {
  return { signal: "test", generatedAt: "", inputCount: 0, params: {}, coveredQuestIds, candidates };
}

describe("scoreCandidates", () => {
  it("counts hits, misses against known quests, and missed truth edges on covered quests only", () => {
    const report = scoreCandidates(
      file(
        [
          candidate(100, "preQuestSingle", 10), // hit
          candidate(100, "preQuestSingle", 99), // wrong target
          candidate(555, "preQuestSingle", 1), // quest unknown to the tier: not judged
        ],
        [100],
      ),
      truth,
    );
    const single = report.tiers[0].fields.find((score) => score.field === "preQuestSingle")!;
    expect(report.tiers[0].judgedCandidates).toBe(2);
    expect(single).toMatchObject({ emitted: true, truePositives: 1, falsePositives: 1, falseNegatives: 1 });
  });

  it("credits prerequisites found under the wrong OR/AND field via prerequisiteAny", () => {
    const report = scoreCandidates(file([candidate(200, "preQuestSingle", 20)], [200]), truth);
    const fields = report.tiers[0].fields;
    expect(fields.find((score) => score.field === "preQuestSingle")).toMatchObject({ truePositives: 0, falsePositives: 1 });
    expect(fields.find((score) => score.field === "prerequisiteAny")).toMatchObject({ truePositives: 1, falsePositives: 0, falseNegatives: 1 });
  });

  it("does not let hits on uncovered quests inflate recall", () => {
    // 100 is covered but its truth edges are unclaimed; the hit on 200 is outside coverage.
    const report = scoreCandidates(file([candidate(200, "preQuestGroup", 20)], [100]), truth);
    const group = report.tiers[0].fields.find((score) => score.field === "preQuestGroup")!;
    expect(group).toMatchObject({ truePositives: 1, falsePositives: 0, recall: 0 });
  });

  it("ignores candidates below the minimum score", () => {
    const report = scoreCandidates(file([candidate(100, "preQuestSingle", 10, 0.2)], [100]), truth, { minScore: 0.5 });
    expect(report.tiers[0].judgedCandidates).toBe(0);
  });
});
