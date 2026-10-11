import { describe, expect, it } from "vitest";
import type { CandidateFile, GroundTruth, RelationCandidate, RelationSet } from "../core/types";
import { collectEdges, domainOf, edgeKey, questDomains, truthHas } from "./edges";

function file(signal: string, candidates: Array<Pick<RelationCandidate, "questId" | "field" | "target" | "score">>): CandidateFile {
  return {
    signal,
    generatedAt: "",
    inputCount: 0,
    params: {},
    coveredQuestIds: [],
    candidates: candidates.map((candidate) => ({ ...candidate, support: 1, contradict: 0, evidence: [] })),
  };
}

describe("collectEdges", () => {
  it("folds both sides of one relationship into one edge, one claim per source", () => {
    const edges = collectEdges([
      file("breadcrumbs", [
        { questId: 2, field: "breadcrumbs", target: 1, score: 0.9 },
        { questId: 1, field: "breadcrumbForQuestId", target: 2, score: 0.8 },
      ]),
      file("cooccurrence", [
        { questId: 5, field: "exclusiveTo", target: 3, score: 0.7 },
        { questId: 3, field: "exclusiveTo", target: 5, score: 0.95 },
        { questId: 10, field: "preQuestGroup", target: 4, score: 0.5 },
      ]),
      file("offer-set", [{ questId: 10, field: "preQuestSingle", target: 4, score: 0.97 }]),
    ]);

    expect([...edges.keys()].sort()).toEqual([edgeKey("breadcrumb", 1, 2), edgeKey("exclusive", 3, 5), edgeKey("prerequisite", 10, 4)].sort());
    expect(edges.get(edgeKey("breadcrumb", 1, 2))!.claims.map((claim) => [claim.source, claim.score])).toEqual([["breadcrumbs/breadcrumbForQuestId", 0.9]]);
    expect(edges.get(edgeKey("exclusive", 3, 5))!.claims.map((claim) => claim.score)).toEqual([0.95]);
    expect(edges.get(edgeKey("prerequisite", 10, 4))!.claims.map((claim) => claim.source)).toEqual(["cooccurrence/preQuestGroup", "offer-set/preQuestSingle"]);
  });
});

describe("questDomains", () => {
  it("goes by ground-truth tier before quest id", () => {
    const truth: GroundTruth = {
      generatedAt: "",
      sources: [],
      // 65593 has a Forever-range id but is filed as inherited Classic data.
      tiers: { authoredForever: { "92000": {} }, inheritedClassic: { "500": {}, "65593": {} } },
    };
    const domains = questDomains(truth);

    expect([92000, 92001, 65593, 500].map((questId) => [domains.referenceOf(questId), domains.isForeverNew(questId)])).toEqual([
      ["authored", true],
      ["none", true],
      ["inherited", false],
      ["inherited", false],
    ]);
    expect(domainOf({ kind: "exclusive", quest: 500, target: 92001 }, domains)).toBe("forever");
    expect(domainOf({ kind: "prerequisite", quest: 65593, target: 92001 }, domains)).toBe("classic");
  });
});

describe("truthHas", () => {
  it("accepts a relationship recorded on either side of the reference data", () => {
    const truth = new Map<number, RelationSet>([
      [2, { breadcrumbs: [1] }],
      [5, { exclusiveTo: [3] }],
      [10, { preQuestGroup: [4] }],
    ]);

    expect(truthHas({ kind: "breadcrumb", quest: 1, target: 2 }, truth)).toBe(true);
    expect(truthHas({ kind: "exclusive", quest: 3, target: 5 }, truth)).toBe(true);
    expect(truthHas({ kind: "prerequisite", quest: 10, target: 4 }, truth)).toBe(true);
    expect(truthHas({ kind: "next", quest: 10, target: 4 }, truth)).toBe(false);
  });
});
