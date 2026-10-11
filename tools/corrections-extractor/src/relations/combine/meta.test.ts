import { describe, expect, it } from "vitest";
import { makeEpisode } from "../core/fixtures";
import type { QuestEpisode } from "../core/types";
import { authoredStatusCounts, countEpisodes, luaRelationCount } from "./meta";
import type { EdgeView, QuestView } from "./views";

function edge(outcome: EdgeView["outcome"], status?: EdgeView["status"]): EdgeView {
  return { key: "k", kind: "prerequisite", field: "preQuestSingle", target: 1, probability: 0.95, heldOut: true, outcome, status, claims: [], context: [], contributions: [] };
}

function view(questId: number, domain: QuestView["domain"], reference: QuestView["reference"], edges: EdgeView[]): QuestView {
  return { questId, domain, reference, relations: {}, edges, referenceOnly: [], hints: [], notes: [] };
}

async function* stream(episodes: QuestEpisode[]): AsyncGenerator<QuestEpisode> {
  yield* episodes;
}

describe("relations meta", () => {
  it("counts the episodes the signals read, and each character once", async () => {
    const episodes = [
      makeEpisode({ characterKey: "a", interfaceVersion: 16001 }),
      makeEpisode({ characterKey: "a", interfaceVersion: 16002 }),
      makeEpisode({ characterKey: "b", interfaceVersion: undefined }),
      // Old Classic trace: the signals skip it, so the summary must too.
      makeEpisode({ characterKey: "c", interfaceVersion: 11507 }),
    ];

    expect(await countEpisodes(stream(episodes))).toEqual({ episodes: 3, characters: 2 });
  });

  it("counts every id of a list field and one per scalar field", () => {
    const records = new Map<number, Record<string, unknown>>([
      [92000, { preQuestSingle: [1], exclusiveTo: [92001, 92002] }],
      [92001, { nextQuestInChain: 92003 }],
    ]);

    expect(luaRelationCount(records)).toBe(4);
  });

  it("counts written relations on authored Forever-new quests only", () => {
    const views = [
      view(92000, "forever", "authored", [edge("accepted", "agree"), edge("derived", "new"), edge("clique", "conflict"), edge("below-threshold")]),
      view(92001, "forever", "none", [edge("accepted", "new")]),
      view(500, "classic", "inherited", [edge("accepted", "conflict")]),
    ];

    expect(authoredStatusCounts(views)).toEqual({ agree: 1, new: 1, conflict: 1 });
  });
});
