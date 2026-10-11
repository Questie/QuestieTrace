import { describe, expect, it } from "vitest";
import { claimsFromPage, type Claim } from "./interpret";
import type { QuestPageFacts, QuestRecord } from "./parse";

function page(questId: number, facts: Partial<QuestPageFacts>, quests: Record<number, QuestRecord> = {}): QuestPageFacts {
  return {
    questId,
    environment: "forever",
    series: [],
    panels: {},
    quests: new Map(Object.entries(quests).map(([id, record]) => [Number(id), record])),
    ...facts,
  };
}

/** Series rows from quest id lists; the page's own quest is marked current. */
function series(questId: number, ...rows: number[][]): QuestPageFacts["series"] {
  return rows.map((ids, i) => ({ index: i + 1, members: ids.map((id) => ({ questId: id, current: id === questId })) }));
}

function edges(claims: Claim[]): string[] {
  return claims.map((claim) => `${claim.questId}.${claim.field}=${claim.target} (${claim.shape})`).sort();
}

describe("claimsFromPage: Series", () => {
  it("turns a linear chain into prerequisites and next-in-chain links between adjacent rows only", () => {
    const facts = page(2, { series: series(2, [1], [2], [3]) });
    expect(edges(claimsFromPage(facts))).toEqual([
      "1.nextQuestInChain=2 (single)",
      "2.nextQuestInChain=3 (single)",
      "2.preQuestSingle=1 (single)",
      "3.preQuestSingle=2 (single)",
    ]);
  });

  it("does not link across an empty row", () => {
    expect(claimsFromPage(page(1, { series: series(1, [1], [], [3]) }))).toEqual([]);
  });

  it("drops pairs no character can do both of: opposite factions, disjoint race or class masks", () => {
    // Wowhead's Taming the Beast row merges Night Elf quest 6063 with Forever-race quest 94013.
    const quests: Record<number, QuestRecord> = {
      6071: { name: "The Hunter's Path", races: 8n, classes: 4 },
      94007: { name: "Taming the Beast", races: 12884901888n, classes: 4 },
      6063: { name: "Taming the Beast", races: 8n, classes: 4 },
      94013: { name: "Taming the Beast", races: 12884901888n, classes: 4 },
      500: { side: 1 },
      501: { side: 2 },
    };
    expect(edges(claimsFromPage(page(6063, { series: series(6063, [6071, 94007], [6063, 94013]) }, quests)))).toEqual([
      "6063.preQuestSingle=6071 (single)",
      "6071.nextQuestInChain=6063 (single)",
      "94007.nextQuestInChain=94013 (single)",
      "94013.preQuestSingle=94007 (single)",
    ]);
    expect(claimsFromPage(page(500, { series: series(500, [500], [501]) }, quests))).toEqual([]);
  });

  it("calls several same-name quests variants and several different quests a group, both as alternatives", () => {
    const quests: Record<number, QuestRecord> = { 10: { name: "Call of Fire" }, 11: { name: "Call of Fire" }, 20: { name: "A" }, 21: { name: "B" } };
    expect(edges(claimsFromPage(page(12, { series: series(12, [10, 11], [12]) }, quests)).filter((c) => c.field === "preQuestSingle"))).toEqual([
      "12.preQuestSingle=10 (variants)",
      "12.preQuestSingle=11 (variants)",
    ]);
    expect(edges(claimsFromPage(page(22, { series: series(22, [20, 21], [22]) }, quests)).filter((c) => c.field === "preQuestSingle"))).toEqual([
      "22.preQuestSingle=20 (group)",
      "22.preQuestSingle=21 (group)",
    ]);
  });

  it("does not claim nextQuestInChain when a quest leads to several next quests", () => {
    const claims = claimsFromPage(page(1, { series: series(1, [1], [2, 3]) }));
    expect(claims.filter((claim) => claim.field === "nextQuestInChain")).toEqual([]);
  });

  it("marks first-row quests without objectives as lead-ins and adds breadcrumb claims for them", () => {
    // 239 Westbrook Garrison Needs Help! is a breadcrumb into 11 Riverpaw Gnoll Bounty.
    const quests: Record<number, QuestRecord> = { 239: { hasObjectives: false }, 11: { hasObjectives: true }, 12: { hasObjectives: false } };
    expect(edges(claimsFromPage(page(11, { series: series(11, [239], [11], [12]) }, quests)))).toEqual([
      "11.breadcrumbs=239 (lead-in)",
      "11.nextQuestInChain=12 (single)",
      "11.preQuestSingle=239 (lead-in)",
      "12.preQuestSingle=11 (single)",
      "239.breadcrumbForQuestId=11 (lead-in)",
      "239.nextQuestInChain=11 (lead-in)",
    ]);
  });

  it("does not call a first row a lead-in when objectives are unknown or present", () => {
    const quests: Record<number, QuestRecord> = { 1: { hasObjectives: true } };
    expect(edges(claimsFromPage(page(2, { series: series(2, [1], [2]) }, quests)))).toEqual(["1.nextQuestInChain=2 (single)", "2.preQuestSingle=1 (single)"]);
    expect(edges(claimsFromPage(page(2, { series: series(2, [3], [2]) })))).toEqual(["2.preQuestSingle=3 (single)", "3.nextQuestInChain=2 (single)"]);
  });
});

describe("claimsFromPage: panels", () => {
  it("puts Requires and Requires In Progress on the page's quest, Unlocks and Disables on the listed quests", () => {
    const facts = page(100, { environment: "era", panels: { Requires: [90], "Requires In Progress": [91], Unlocks: [110], Disables: [120] } });
    expect(edges(claimsFromPage(facts))).toEqual([
      "100.parentQuest=91 (single)",
      "100.preQuestSingle=90 (single)",
      "110.preQuestSingle=100 (single)",
      "120.exclusiveTo=100 (single)",
    ]);
  });

  it("drops the page's own quest from self-including lists and marks them", () => {
    const facts = page(7886, { environment: "era", panels: { "Requires Any": [7886, 7887, 7788] } });
    expect(edges(claimsFromPage(facts))).toEqual(["7886.preQuestSingle=7788 (with-self)", "7886.preQuestSingle=7887 (with-self)"]);
  });

  it("maps a Requires list of different quests to preQuestGroup, also when it lists the page's own quest", () => {
    const facts = page(5, { environment: "era", panels: { Requires: [1, 2] } }, { 1: { name: "A" }, 2: { name: "B" } });
    expect(edges(claimsFromPage(facts))).toEqual(["5.preQuestGroup=1 (group)", "5.preQuestGroup=2 (group)"]);
    const withSelf = page(5, { environment: "era", panels: { Requires: [5, 1, 2] } }, { 1: { name: "A" }, 2: { name: "B" }, 5: { name: "C" } });
    expect(edges(claimsFromPage(withSelf))).toEqual(["5.preQuestGroup=1 (with-self)", "5.preQuestGroup=2 (with-self)"]);
  });
});

describe("claimsFromPage: Storyline", () => {
  it("treats runs of same-name quests as one step and links every variant to its neighbours", () => {
    const quests: Record<number, QuestRecord> = {
      92749: { name: "A Dynamite Plan" },
      92750: { name: "Detonation at a Distance" },
      92751: { name: "Detonation at a Distance" },
      92752: { name: "Explosive Consultation" },
    };
    const facts = page(92749, { storyline: { id: 6026, name: "Toxic Soil", questIds: [92749, 92750, 92751, 92752] } }, quests);
    expect(edges(claimsFromPage(facts))).toEqual([
      "92750.preQuestSingle=92749 (single)",
      "92751.preQuestSingle=92749 (single)",
      "92752.preQuestSingle=92750 (variants)",
      "92752.preQuestSingle=92751 (variants)",
    ]);
  });
});
