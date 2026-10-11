import { describe, expect, it } from "vitest";
import { edgeKey, type EdgeKind } from "./edges";
import { resolveRelations, type ResolvableEdge, type ResolveOptions } from "./resolve";

function edge(kind: EdgeKind, quest: number, target: number, probability: number, claims: ResolvableEdge["claims"] = []): ResolvableEdge {
  const [a, b] = kind === "exclusive" ? [Math.min(quest, target), Math.max(quest, target)] : [quest, target];
  return { key: edgeKey(kind, a, b), kind, quest: a, target: b, probability, claims };
}

const andClaim = [{ source: "handoff/preQuestGroup", field: "preQuestGroup" as const }];

const OPTIONS: ResolveOptions = {
  minScore: 0.9,
  knownQuest: (questId) => questId !== 404,
  cliqueMinScore: 0.5,
  // Quests 1-99 are distinct; 100+ share a name with quest - 100 (same-name copies).
  familyOf: (questId) => `family-${questId % 100}`,
  andSourcesFor: () => new Set(["handoff/preQuestGroup"]),
};

function resolve(edges: ResolvableEdge[]) {
  return resolveRelations(edges, OPTIONS);
}

describe("resolveRelations", () => {
  it("turns a breadcrumb's chain into breadcrumb fields and drops the prerequisite and exclusivity it explains", () => {
    const breadcrumb = edge("breadcrumb", 1, 2, 0.95);
    const result = resolve([breadcrumb, edge("prerequisite", 2, 1, 0.99), edge("exclusive", 1, 2, 0.95)]);

    expect(result.relations.get(1)).toEqual({ breadcrumbForQuestId: [2], nextQuestInChain: [2] });
    expect(result.relations.get(2)).toEqual({ breadcrumbs: [1] });
    expect(result.decisions.get(edgeKey("prerequisite", 2, 1))).toEqual({ outcome: "breadcrumb", because: breadcrumb.key });
    expect(result.decisions.get(edgeKey("exclusive", 1, 2))).toEqual({ outcome: "breadcrumb", because: breadcrumb.key });
  });

  it("drops prerequisites between same-name copies of a breadcrumb and of its target", () => {
    // 101 and 102 are copies of 1 and 2; where copies complete together, 102 looks like it needs 101.
    const breadcrumb = edge("breadcrumb", 1, 2, 0.95);
    const result = resolve([breadcrumb, edge("prerequisite", 102, 101, 0.99), edge("prerequisite", 102, 7, 0.99)]);

    expect(result.decisions.get(edgeKey("prerequisite", 102, 101))).toEqual({ outcome: "breadcrumb", because: breadcrumb.key });
    expect(result.relations.get(102)).toEqual({ preQuestSingle: [7] });
  });

  it("settles a breadcrumb against a stronger reverse prerequisite by score", () => {
    // 1 needs 2 contradicts "1 is a breadcrumb for 2": 1 would be hidden before it could be taken.
    const result = resolve([edge("breadcrumb", 1, 2, 0.91), edge("prerequisite", 1, 2, 0.99)]);

    expect(result.relations.get(1)).toEqual({ preQuestSingle: [2] });
    expect(result.decisions.get(edgeKey("breadcrumb", 1, 2))?.outcome).toBe("cycle");
  });

  it("lets a stronger rival chain on the breadcrumb quest win over the breadcrumb", () => {
    const result = resolve([edge("breadcrumb", 1, 2, 0.91), edge("next", 1, 3, 0.99)]);

    expect(result.relations.get(1)).toEqual({ nextQuestInChain: [3] });
    expect(result.decisions.get(edgeKey("breadcrumb", 1, 2))?.outcome).toBe("runner-up");
  });

  it("writes AND only when every distinct prerequisite has AND evidence", () => {
    const both = resolve([edge("prerequisite", 10, 1, 0.95, andClaim), edge("prerequisite", 10, 2, 0.95, andClaim)]);
    const one = resolve([edge("prerequisite", 10, 1, 0.95, andClaim), edge("prerequisite", 10, 2, 0.95)]);

    expect(both.relations.get(10)).toEqual({ preQuestGroup: [1, 2] });
    expect(one.relations.get(10)).toEqual({ preQuestSingle: [1, 2] });
  });

  it("keeps same-name copies of one prerequisite as an OR list, even with AND evidence", () => {
    const copies = resolve([edge("prerequisite", 10, 1, 0.95, andClaim), edge("prerequisite", 10, 101, 0.95, andClaim)]);
    const copiesAndOther = resolve([
      edge("prerequisite", 10, 1, 0.95, andClaim),
      edge("prerequisite", 10, 101, 0.95, andClaim),
      edge("prerequisite", 10, 2, 0.95, andClaim),
    ]);

    expect(copies.relations.get(10)).toEqual({ preQuestSingle: [1, 101] });
    expect(copiesAndOther.relations.get(10)).toEqual({ preQuestSingle: [1, 2, 101] });
  });

  it("keeps one target per scalar field and records the runner-up", () => {
    const winner = edge("next", 1, 3, 0.97);
    const result = resolve([edge("next", 1, 2, 0.95), winner]);

    expect(result.relations.get(1)).toEqual({ nextQuestInChain: [3] });
    expect(result.decisions.get(edgeKey("next", 1, 2))).toEqual({ outcome: "runner-up", because: winner.key });
  });

  it("writes exclusiveTo on both sides and completes a group only with the missing pair's own evidence", () => {
    const result = resolve([edge("exclusive", 1, 2, 0.95), edge("exclusive", 2, 3, 0.95), edge("exclusive", 1, 3, 0.6), edge("exclusive", 3, 4, 0.6)]);

    expect(result.relations.get(1)).toEqual({ exclusiveTo: [2, 3] });
    expect(result.relations.get(2)).toEqual({ exclusiveTo: [1, 3] });
    expect(result.relations.get(3)).toEqual({ exclusiveTo: [1, 2] });
    expect(result.relations.has(4)).toBe(false);
    expect(result.decisions.get(edgeKey("exclusive", 1, 3))?.outcome).toBe("clique");
  });

  it("drops prerequisites implied by an accepted chain, unless the direct one is stronger", () => {
    const viaTwo = edge("prerequisite", 3, 2, 0.99);
    const reduced = resolve([viaTwo, edge("prerequisite", 2, 1, 0.99), edge("prerequisite", 3, 1, 0.95)]);
    const kept = resolve([edge("prerequisite", 3, 2, 0.92), edge("prerequisite", 2, 1, 0.99), edge("prerequisite", 3, 1, 0.99)]);

    expect(reduced.relations.get(3)).toEqual({ preQuestSingle: [2] });
    expect(reduced.decisions.get(edgeKey("prerequisite", 3, 1))).toEqual({ outcome: "transitive", because: viaTwo.key });
    expect(kept.relations.get(3)).toEqual({ preQuestSingle: [1, 2] });
  });

  it("breaks ordering cycles at their weakest edge, across prerequisites and chains", () => {
    // 9 -> 1 leads into the cycle 1 -> 2 -> 3 -> 1 and is the weakest edge, but is not part of the cycle.
    const leadIn = edge("prerequisite", 1, 9, 0.91);
    const result = resolve([leadIn, edge("prerequisite", 2, 1, 0.99), edge("prerequisite", 3, 2, 0.98), edge("next", 3, 1, 0.92)]);

    expect(result.decisions.get(edgeKey("next", 3, 1))?.outcome).toBe("cycle");
    expect(result.decisions.get(leadIn.key)?.outcome).toBe("accepted");
    expect(result.relations.get(3)).toEqual({ preQuestSingle: [2] });
    expect(result.relations.get(2)).toEqual({ preQuestSingle: [1] });
  });

  it("never makes a quest both prerequisite of and exclusive with the same quest", () => {
    const prerequisite = edge("prerequisite", 2, 1, 0.99);
    const result = resolve([prerequisite, edge("exclusive", 1, 2, 0.95)]);

    expect(result.relations.get(2)).toEqual({ preQuestSingle: [1] });
    expect(result.relations.has(1)).toBe(false);
    expect(result.decisions.get(edgeKey("exclusive", 1, 2))).toEqual({ outcome: "conflict", because: prerequisite.key });
  });

  it("never makes a quest exclusive with one it follows through a chain", () => {
    const throughChain = resolve([edge("prerequisite", 2, 1, 0.99), edge("prerequisite", 3, 2, 0.99), edge("exclusive", 1, 3, 0.95)]);
    const throughNext = resolve([edge("next", 1, 2, 0.97), edge("exclusive", 1, 2, 0.95)]);

    expect(throughChain.decisions.get(edgeKey("exclusive", 1, 3))?.outcome).toBe("conflict");
    expect(throughNext.decisions.get(edgeKey("exclusive", 1, 2))?.outcome).toBe("conflict");
    expect(throughNext.relations.get(1)).toEqual({ nextQuestInChain: [2] });
  });

  it("completes exclusive groups only from pairs that survived the conflict rule", () => {
    // 2 needs 1, so the pair 1-2 goes, and with it the only link putting 1 and 3 in one group.
    const result = resolve([edge("exclusive", 1, 2, 0.95), edge("exclusive", 2, 3, 0.95), edge("exclusive", 1, 3, 0.6), edge("prerequisite", 2, 1, 0.99)]);

    expect(result.decisions.get(edgeKey("exclusive", 1, 3))?.outcome).toBe("below-threshold");
    expect(result.relations.get(2)).toEqual({ preQuestSingle: [1], exclusiveTo: [3] });
    expect(result.relations.has(1)).toBe(false);
  });

  it("never writes a relation to a quest the catalog does not know", () => {
    const result = resolve([edge("prerequisite", 2, 404, 0.99)]);

    expect(result.relations.size).toBe(0);
    expect(result.decisions.get(edgeKey("prerequisite", 2, 404))?.outcome).toBe("unknown-quest");
  });
});
