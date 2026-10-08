import { describe, expect, it } from "vitest";
import { mergeNpcDrops, mergeObjectDrops } from "./dropsMerge";
import type { Observation } from "../../observation";

describe("mergeNpcDrops", () => {
  it("should union npc IDs across observations", () => {
    const observations: Observation<{ npcID: number }>[] = [
      { entityId: 12345, value: { npcID: 823 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { npcID: 824 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { npcID: 823 }, confidence: "high", provenance: { session: "s2", t: 10 } }, // duplicate
    ];

    expect(mergeNpcDrops(observations)).toEqual([823, 824]);
  });

  it("should return empty array for no observations", () => {
    expect(mergeNpcDrops([])).toEqual([]);
  });

  it("should sort IDs ascending", () => {
    const observations: Observation<{ npcID: number }>[] = [
      { entityId: 12345, value: { npcID: 900 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { npcID: 100 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { npcID: 500 }, confidence: "high", provenance: { session: "s1", t: 5 } },
    ];

    expect(mergeNpcDrops(observations)).toEqual([100, 500, 900]);
  });
});

describe("mergeObjectDrops", () => {
  it("should union object IDs across observations", () => {
    const observations: Observation<{ objectID: number }>[] = [
      { entityId: 12345, value: { objectID: 2843 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { objectID: 2844 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { objectID: 2843 }, confidence: "high", provenance: { session: "s2", t: 10 } }, // duplicate
    ];

    expect(mergeObjectDrops(observations)).toEqual([2843, 2844]);
  });

  it("should return empty array for no observations", () => {
    expect(mergeObjectDrops([])).toEqual([]);
  });

  it("should sort IDs ascending", () => {
    const observations: Observation<{ objectID: number }>[] = [
      { entityId: 12345, value: { objectID: 900 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { objectID: 100 }, confidence: "high", provenance: { session: "s1", t: 5 } },
      { entityId: 12345, value: { objectID: 500 }, confidence: "high", provenance: { session: "s1", t: 5 } },
    ];

    expect(mergeObjectDrops(observations)).toEqual([100, 500, 900]);
  });
});
