import { describe, expect, it } from "vitest";
import { catalogQuest, event, gossip, makeEpisode, npc } from "../../core/fixtures";
import type { QuestCatalog } from "../../core/types";
import { emptyObserveStats, groupByCharacter, observeCharacter } from "./observe";

const UNIVERSAL = 1;
const PREREQ = 10;
const QUEST = 20;
const DAILY = 30;
const GIVER = npc(100);

const catalog: QuestCatalog = {
  generatedAt: "",
  sources: [],
  quests: {
    [UNIVERSAL]: catalogQuest(UNIVERSAL),
    [PREREQ]: catalogQuest(PREREQ, { finishers: { npcs: [100], objects: [] } }),
    [QUEST]: catalogQuest(QUEST, { starters: { npcs: [100], objects: [], items: [] } }),
    [DAILY]: catalogQuest(DAILY, { specialFlags: 1 }),
  },
};

describe("observeCharacter", () => {
  it("keeps catalog, non-repeatable quests completed at the first sighting, with freshness and the giver window", () => {
    const episode = makeEpisode({
      completed: { source: "modern", initial: [UNIVERSAL, DAILY, 99999], changes: [{ t: 101, add: [PREREQ] }] },
      offers: [gossip(10, GIVER, [PREREQ]), gossip(100.5, GIVER, [QUEST])],
      // The turn-in event lands before the delayed completed-quest delta and before the next list.
      questEvents: [event(100, "turnedIn", PREREQ, GIVER)],
    });
    const view = observeCharacter([episode], catalog).find((v) => v.questId === QUEST)!;
    expect(view.survivors).toEqual([UNIVERSAL, PREREQ]);
    expect(view.fresh).toEqual(new Map([[PREREQ, 0.5]]));
    expect(view.arrival).toEqual({ level: 10, pending: [PREREQ] });
  });

  it("drops a candidate the character lacked at any sighting, across sessions in sessionOrder", () => {
    // In-progress exports have no session name; sessionOrder is the only ordering.
    const early = makeEpisode({
      characterKey: "c",
      sessionOrder: 0,
      completed: { source: "modern", initial: [UNIVERSAL], changes: [] },
      offers: [gossip(5, GIVER, [QUEST])],
    });
    const late = makeEpisode({
      characterKey: "c",
      sessionOrder: 1,
      completed: { source: "modern", initial: [UNIVERSAL, PREREQ], changes: [] },
      offers: [gossip(5, GIVER, [QUEST])],
    });
    const characters = groupByCharacter([late, early], emptyObserveStats());
    const view = observeCharacter(characters.get("c")!, catalog).find((v) => v.questId === QUEST)!;
    expect(view.survivors).toEqual([UNIVERSAL]);
    expect(view.episodeKey).toBe(early.key);
    expect(view.sightings).toBe(2);
  });

  it("treats a quest already in the log at capture start as taken with the initial completed set", () => {
    const episode = makeEpisode({
      completed: { source: "modern", initial: [UNIVERSAL], changes: [{ t: 50, add: [PREREQ] }] },
      questLog: [{ t: 0, v: [QUEST] }],
    });
    const view = observeCharacter([episode], catalog).find((v) => v.questId === QUEST)!;
    expect(view.kind).toBe("log");
    expect(view.survivors).toEqual([UNIVERSAL]);
  });
});

describe("groupByCharacter", () => {
  it("skips non-Forever sessions, sessions without history, and empty histories past level 2", () => {
    const stats = emptyObserveStats();
    const characters = groupByCharacter(
      [
        makeEpisode({ characterKey: "ok", completed: { source: "modern", initial: [UNIVERSAL], changes: [] } }),
        makeEpisode({ characterKey: "fresh-character", levels: [{ t: 0, v: 1 }] }),
        makeEpisode({ characterKey: "classic", interfaceVersion: 11507 }),
        makeEpisode({ characterKey: "no-history", completed: undefined }),
        makeEpisode({ characterKey: "unloaded", levels: [{ t: 0, v: 30 }] }),
      ],
      stats,
    );
    expect([...characters.keys()]).toEqual(["ok", "fresh-character"]);
    expect(stats).toMatchObject({ skippedNotForever: 1, skippedNoCompleted: 1, skippedEmptyHistory: 1 });
  });
});
