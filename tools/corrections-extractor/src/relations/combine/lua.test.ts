import { describe, expect, it } from "vitest";
import { catalogQuest } from "../core/fixtures";
import type { GroundTruth } from "../core/types";
import { questDomains } from "./edges";
import { foreverComments, foreverRecords, foreverRelationsLua } from "./lua";
import type { QuestView } from "./views";

describe("foreverRecords", () => {
  it("writes Forever-new quests only, and leaves authored prerequisites alone", () => {
    // 65593 has a Forever-range id but is inherited Classic data, so it is review-only.
    const relations = new Map([
      [92000, { preQuestSingle: [1], nextQuestInChain: [2] }],
      [92001, { preQuestSingle: [3], exclusiveTo: [92002] }],
      [500, { preQuestSingle: [4] }],
      [65593, { nextQuestInChain: [65597] }],
    ]);
    // Questie ignores preQuestGroup once preQuestSingle is set, so writing one here would override the authored AND list.
    const authored = { "92000": { preQuestGroup: [1, 5] } };
    const truth: GroundTruth = { generatedAt: "", sources: [], tiers: { authoredForever: authored, inheritedClassic: { "500": {}, "65593": {} } } };

    const records = foreverRecords(relations, authored, questDomains(truth).isForeverNew);

    expect(records.get(92000)).toEqual({ nextQuestInChain: 2 });
    expect(records.get(92001)).toEqual({ preQuestSingle: [3], exclusiveTo: [92002] });
    expect(records.has(500)).toBe(false);
    expect(records.has(65593)).toBe(false);
  });
});

describe("foreverRelationsLua", () => {
  it("replaces the writer's trace-file count with the review file and candidate files", () => {
    const header = { candidateFiles: ["handoff", "offer-set"], sessionCount: 12, minScore: 0.9, generatedAt: new Date(0), reviewFile: "relations-review.md" };

    const lua = foreverRelationsLua(new Map([[92000, { nextQuestInChain: 92001 }]]), header);

    expect(lua).toContain("(min score 0.9); review: relations-review.md, published next to this module");
    expect(lua).toContain("-- Candidate files: handoff, offer-set");
    expect(lua).not.toContain("Source trace files");
  });

  it("names and links the quest and every quest a field names, and ends the field with its assumption", () => {
    const header = { candidateFiles: ["handoff"], sessionCount: 12, minScore: 0.9, generatedAt: new Date(0), reviewFile: "relations-review.md" };
    const handoff = { source: "handoff/nextQuestInChain", signal: "handoff", field: "nextQuestInChain" as const, score: 0.96, support: 31, contradict: 0, evidence: [] };
    const view: QuestView = {
      questId: 92000,
      name: "Book Inventory",
      domain: "forever",
      reference: "none",
      relations: { nextQuestInChain: [92001] },
      edges: [{ key: "k", kind: "next", field: "nextQuestInChain", target: 92001, probability: 0.97, heldOut: false, outcome: "accepted", claims: [handoff], context: [], contributions: [] }],
      referenceOnly: [],
      hints: [],
      notes: [],
    };

    const quests = { "92000": catalogQuest(92000, { name: "Book Inventory" }), "92001": catalogQuest(92001, { name: "Rascally Rodents" }) };

    const lua = foreverRelationsLua(new Map([[92000, { nextQuestInChain: 92001 }]]), header, foreverComments(new Map([[92000, view]]), quests));

    expect(lua).toContain(
      "        [92000] = { -- Book Inventory https://wowhead.com/forever/quest=92000\n" +
        '            -- 92001 "Rascally Rodents" https://wowhead.com/forever/quest=92001\n' +
        "            [questKeys.nextQuestInChain] = 92001, -- Assumption: turning it in led straight to 92001 (31 characters), score 0.970\n",
    );
  });
});
