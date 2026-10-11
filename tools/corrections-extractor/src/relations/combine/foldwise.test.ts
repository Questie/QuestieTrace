import { describe, expect, it } from "vitest";
import type { RelationSet } from "../core/types";
import { edgeKey, type Claim, type Edge, type EdgeKind } from "./edges";
import { andSourcesByQuest, type FoldContext } from "./foldwise";

function edge(kind: EdgeKind, quest: number, target: number, sources: string[]): Edge {
  const claims: Claim[] = sources.map((source) => ({
    source,
    signal: source.split("/")[0],
    field: source.endsWith("preQuestGroup") ? "preQuestGroup" : "preQuestSingle",
    score: 0.9,
    support: 1,
    contradict: 0,
    evidence: [],
  }));
  return { key: edgeKey(kind, quest, target), kind, quest, target, claims };
}

function context(truth: Map<number, RelationSet>): FoldContext {
  return { truth, groupOfQuest: (questId) => `group-${questId}`, folds: 5 };
}

describe("andSourcesByQuest", () => {
  it("never lets a judged quest's own labels make a source trusted for that quest", () => {
    // Only quest 92001's references show "x" voting AND correctly.
    const edges = [1, 2, 3, 4, 5].map((target) => edge("prerequisite", 92001, target, ["x/preQuestGroup"]));
    const truth = new Map<number, RelationSet>([[92001, { preQuestGroup: [1, 2, 3, 4, 5] }]]);

    const sources = andSourcesByQuest(edges, context(truth));

    expect(sources.all.has("x/preQuestGroup")).toBe(true);
    expect(sources.forQuest(92999).has("x/preQuestGroup")).toBe(true);
    expect(sources.forQuest(92001).has("x/preQuestGroup")).toBe(false);
  });
});
