import { describe, expect, it } from "vitest";
import type { RelationSet } from "../core/types";
import { crossValidationSplits, cvGroups, scoreEdges } from "./crossval";
import { edgeKey, type Edge } from "./edges";

/** A prerequisite edge claimed by one signal; `score` varies so the model has something to learn. */
function claimed(quest: number, target: number, score: number): Edge {
  return {
    key: edgeKey("prerequisite", quest, target),
    kind: "prerequisite",
    quest,
    target,
    claims: [{ source: "handoff/preQuestSingle", signal: "handoff", field: "preQuestSingle", score, support: 3, contradict: 0, evidence: [] }],
  };
}

describe("crossValidationSplits", () => {
  it("puts every family in exactly one test fold and never in its own training set", () => {
    const rows = Array.from({ length: 200 }, (_, i) => ({ id: i, family: `family-${i % 37}` }));
    const splits = crossValidationSplits(rows, (row) => row.family, 5);

    const tested = splits.flatMap((split) => split.test.map((row) => row.id)).sort((a, b) => a - b);
    expect(tested).toEqual(rows.map((row) => row.id));
    for (const { train, test } of splits) {
      const trainFamilies = new Set(train.map((row) => row.family));
      expect(test.some((row) => trainFamilies.has(row.family))).toBe(false);
    }
  });
});

describe("cvGroups", () => {
  it("holds out both quests of an exclusive pair or breadcrumb together, with their copies, but does not chain prerequisites", () => {
    // 1 and 101 are same-name copies; every other quest is its own family.
    const familyOf = (questId: number) => `family-${questId % 100}`;
    const groupOf = cvGroups(
      [
        { kind: "exclusive", quest: 1, target: 2 },
        { kind: "breadcrumb", quest: 3, target: 4 },
        { kind: "prerequisite", quest: 6, target: 5 },
        { kind: "next", quest: 7, target: 8 },
      ],
      familyOf,
    );

    expect(groupOf(2)).toBe(groupOf(1));
    expect(groupOf(2)).toBe(groupOf(101));
    expect(groupOf(4)).toBe(groupOf(3));
    expect(groupOf(6)).not.toBe(groupOf(5));
    expect(groupOf(8)).not.toBe(groupOf(7));
    expect(groupOf(3)).not.toBe(groupOf(1));
  });
});

describe("scoreEdges", () => {
  it("scores a judged edge without ever seeing its own label", () => {
    const edges = Array.from({ length: 60 }, (_, i) => claimed(1000 + i, 1, i % 2 === 0 ? 0.95 : 0.4));
    const truth = new Map<number, RelationSet>(edges.map((edge, i) => [edge.quest, i % 2 === 0 ? { preQuestSingle: [1] } : {}]));
    const familyOf = (questId: number) => `family-${questId}`;
    const probe = edges[0];
    const scoreOfProbe = (labels: Map<number, RelationSet>) => scoreEdges(edges, labels, new Map(), familyOf).scored.find((edge) => edge.key === probe.key)!;

    const flipped = new Map(truth);
    flipped.set(probe.quest, {});

    expect(scoreOfProbe(truth).heldOut).toBe(true);
    expect(scoreOfProbe(flipped).probability).toBe(scoreOfProbe(truth).probability);
  });

  it("never accepts a kind with too little ground truth to calibrate", () => {
    const parentQuest = (quest: number): Edge => ({ ...claimed(quest, 1, 0.99), key: edgeKey("parentQuest", quest, 1), kind: "parentQuest" });
    // Three judged edges, all true: a fitted intercept alone would accept every new edge of the kind.
    const edges = [parentQuest(5), parentQuest(6), parentQuest(7), parentQuest(8)];
    const truth = new Map<number, RelationSet>([5, 6, 7].map((quest) => [quest, { parentQuest: [1] }]));
    const unjudged = scoreEdges(edges, truth, new Map(), String).scored.find((edge) => edge.quest === 8)!;

    expect(unjudged.heldOut).toBe(false);
    expect(unjudged.probability).toBe(0);
  });
});
