import { describe, expect, it } from "vitest";
import { aggregateClaims, type PageClaim, type SeriesPositions } from "./aggregate";
import { baseScore } from "./scores";

function seriesClaim(pageId: number, overrides: Partial<PageClaim> = {}): PageClaim {
  return {
    surface: "series",
    questId: 2,
    field: "preQuestSingle",
    target: 1,
    shape: "single",
    pageId,
    adjacency: [1, 2],
    text: "Series row 1 [1] -> row 2 [2]",
    provenance: "forever-new-edge",
    environment: "forever",
    ...overrides,
  };
}

function positions(...entries: Array<[number, number[]]>): SeriesPositions {
  return new Map(entries.map(([pageId, order]) => [pageId, new Map(order.map((questId, row) => [questId, row]))]));
}

describe("aggregateClaims", () => {
  it("merges pages showing one edge, counts pages that order the quests differently, and dilutes the score", () => {
    const claims = [seriesClaim(1), seriesClaim(2)];
    // Pages 1 and 2 show 1 -> 2; page 3 shows 1 -> 9 -> 2; page 4 does not show quest 2 at all.
    const candidates = aggregateClaims(claims, positions([1, [1, 2]], [2, [1, 2]], [3, [1, 9, 2]], [4, [1, 9]])).get("series")!;
    const full = baseScore("series", "preQuestSingle", "single", "forever-new-edge");
    expect(candidates).toHaveLength(1);
    expect(candidates[0]).toMatchObject({ questId: 2, field: "preQuestSingle", target: 1, support: 2, contradict: 1 });
    expect(candidates[0].score).toBeCloseTo(Math.round(((full * 2) / 3) * 100) / 100);
  });

  it("scores by the most trusted claim and lists Forever pages first in the evidence", () => {
    const claims = [
      seriesClaim(5, { environment: "era", provenance: "era-page", shape: "variants" }),
      seriesClaim(9, { provenance: "forever-era-edge" }),
    ];
    const [candidate] = aggregateClaims(claims, positions([5, [1, 2]], [9, [1, 2]])).get("series")!;
    expect(candidate.score).toBe(baseScore("series", "preQuestSingle", "single", "forever-era-edge"));
    expect(candidate.note).toBe("surface=series field=preQuestSingle shape=single provenance=forever-era-edge");
    expect(candidate.evidence.map((evidence) => evidence.ref)).toEqual(["wowhead:quest/9", "wowhead:quest/5"]);
  });
});
