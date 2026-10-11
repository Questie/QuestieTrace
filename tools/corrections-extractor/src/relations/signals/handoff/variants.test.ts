import { describe, expect, it } from "vitest";
import { catalogQuest, makeEpisode } from "../../core/fixtures";
import type { CatalogQuest } from "../../core/types";
import { CompletionMoments, isParallelCopy, sameNameVariants, variantEdges } from "./variants";

const ALLIANCE = 1 + 4 + 8 + 64;
const HORDE = 2 + 16 + 32 + 128;

function edgesFor(quests: CatalogQuest[], observed: { prerequisite: number; quest: number }) {
  const byId = new Map(quests.map((quest) => [quest.id, quest]));
  return variantEdges([observed], sameNameVariants(quests), (id) => byId.get(id)).map((edge) => [edge.kind, edge.prerequisite, edge.quest]);
}

describe("variantEdges", () => {
  it("extends an observed edge to copies a character could take with the other side", () => {
    const quests = [
      catalogQuest(10001, { name: "Intro", requiredRaces: ALLIANCE }),
      catalogQuest(10002, { name: "Intro", requiredRaces: HORDE }),
      catalogQuest(10003, { name: "Intro" }),
      catalogQuest(10010, { name: "Next", requiredRaces: ALLIANCE }),
    ];
    // The Horde copy can never precede the Alliance-only quest.
    expect(edgesFor(quests, { prerequisite: 10001, quest: 10010 })).toEqual([["prerequisite", 10003, 10010]]);
  });

  it("does not treat Classic namesakes of Forever quests, or steps of a same-name chain, as copies", () => {
    const quests = [catalogQuest(10001, { name: "Intro" }), catalogQuest(500, { name: "Intro" }), catalogQuest(10010, { name: "Next" })];
    expect(edgesFor(quests, { prerequisite: 10001, quest: 10010 })).toEqual([]);
    const chain = [catalogQuest(10001, { name: "Step" }), catalogQuest(10002, { name: "Step" }), catalogQuest(10003, { name: "Step" })];
    expect(edgesFor(chain, { prerequisite: 10001, quest: 10002 })).toEqual([]);
  });
});

describe("CompletionMoments", () => {
  const edge = { prerequisite: 2, quest: 9, kind: "prerequisite" as const, observed: { prerequisite: 1, quest: 9 } };
  const flagged = (...changes: Array<{ t: number; add: number[] }>) =>
    makeEpisode({ completed: { source: "modern", initial: [], changes } });

  it("accepts copies that some character had flagged in one delta and none separately", () => {
    const moments = new CompletionMoments(new Set([1, 2]));
    moments.add(flagged({ t: 10, add: [1, 2] }));
    expect(isParallelCopy(edge, moments)).toBe(true);
    moments.add(flagged({ t: 10, add: [1] }, { t: 50, add: [2] }));
    expect(isParallelCopy(edge, moments)).toBe(false);
  });

  it("ignores bulk catch-up deltas, which flag whole chains at once", () => {
    const moments = new CompletionMoments(new Set([1, 2]));
    moments.add(flagged({ t: 10, add: [1, 2, ...Array.from({ length: 30 }, (_, i) => 100 + i)] }));
    expect(isParallelCopy(edge, moments)).toBe(false);
  });
});
