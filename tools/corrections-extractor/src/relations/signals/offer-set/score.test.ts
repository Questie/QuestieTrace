import { describe, expect, it } from "vitest";
import type { EdgeFeatures } from "./infer";
import { scoreEdge } from "./score";

const primary: EdgeFeatures = {
  field: "preQuestSingle",
  forever: true,
  tier: "primary",
  shape: "direct",
  characters: 40,
  support: 40,
  contradict: 0,
  link: true,
  pending: 20,
  background: 0,
  fresh: 30,
  immediate: 30,
  latest: 30,
  levelUnlockedShare: 0,
  remaining: 1,
  allOfConfirmed: false,
};

describe("scoreEdge", () => {
  it("scores edges from few characters below the same evidence from many", () => {
    expect(scoreEdge({ ...primary, characters: 2, support: 2 })).toBeLessThan(scoreEdge(primary));
    const secondary = { ...primary, tier: "secondary" as const, link: false };
    expect(scoreEdge({ ...secondary, characters: 6, support: 6 })).toBeLessThan(scoreEdge(secondary));
  });

  it("scores a group backed only by play order far below a confirmed one", () => {
    const group = { ...primary, field: "preQuestGroup" as const, remaining: 2 };
    expect(scoreEdge(group)).toBeLessThan(0.5);
    expect(scoreEdge({ ...group, allOfConfirmed: true })).toBeGreaterThan(0.9);
  });
});
