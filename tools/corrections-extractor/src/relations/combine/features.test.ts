import { describe, expect, it } from "vitest";
import { edgeKey, type Claim, type Edge, type EdgeKind } from "./edges";
import { buildFeatureSpace } from "./features";

function claim(signal: string, score: number, support = 4, contradict = 0): Claim {
  return { source: `${signal}/preQuestSingle`, signal, field: "preQuestSingle", score, support, contradict, evidence: [] };
}

function edge(kind: EdgeKind, quest: number, target: number, claims: Claim[]): Edge {
  return { key: edgeKey(kind, quest, target), kind, quest, target, claims };
}

describe("buildFeatureSpace", () => {
  it("adds score and contradiction columns only where a source varies, and marks covering signals that stayed silent", () => {
    const edges = [
      edge("prerequisite", 1, 5, [claim("a", 0.9, 4, 1)]),
      edge("prerequisite", 2, 6, [claim("a", 0.6), claim("b", 0.8)]),
      edge("prerequisite", 3, 7, [claim("b", 0.8)]),
    ];
    // a covers quest 1 only; b covers 1 and 2. Nobody covers 3.
    const coverage = new Map([
      ["a", new Set([1])],
      ["b", new Set([1, 2])],
    ]);
    const space = buildFeatureSpace("prerequisite", edges, coverage);
    const row = (index: number) => Object.fromEntries(space.columns.map((column, j) => [column.name, space.vector(edges[index])[j]]));

    expect(space.columns.map((column) => column.name)).toEqual([
      "a/preQuestSingle claimed",
      "a/preQuestSingle score",
      "a/preQuestSingle contradict",
      "b/preQuestSingle claimed",
      "a silent",
      "b silent",
    ]);
    expect(row(0)).toMatchObject({ "a/preQuestSingle contradict": 0.2, "a silent": 0, "b silent": 1 });
    expect(row(2)).toMatchObject({ "a silent": 0, "b silent": 0, "b/preQuestSingle claimed": 1 });
    expect(buildFeatureSpace("prerequisite", edges, coverage, { scores: false }).columns.map((column) => column.name)).not.toContain("a/preQuestSingle score");
  });

  it("counts a two-sided edge as covered only when the signal covered both quests", () => {
    const pairs = [edge("exclusive", 1, 2, [claim("a", 0.9)]), edge("exclusive", 1, 3, [claim("a", 0.9)]), edge("exclusive", 1, 4, [claim("b", 0.9)])];
    const coverage = new Map([
      ["a", new Set([1, 2, 3])],
      ["b", new Set([1, 2])],
    ]);
    const space = buildFeatureSpace("exclusive", pairs, coverage);
    const silent = (index: number, signal: string) => space.vector(pairs[index])[space.columns.findIndex((column) => column.name === `${signal} silent`)];

    expect(silent(0, "b")).toBe(1);
    expect(silent(1, "b")).toBe(0);
    expect(silent(2, "a")).toBe(0);
  });
});
