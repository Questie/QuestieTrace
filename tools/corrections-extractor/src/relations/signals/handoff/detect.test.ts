import { describe, expect, it } from "vitest";
import { catalogQuest, event, gossip, makeEpisode, npc } from "../../core/fixtures";
import { EpisodeTimeline } from "../../core/timeline";
import type { CatalogQuest, QuestEpisode } from "../../core/types";
import { findHandoffs, type HandoffParams } from "./detect";

const PARAMS: HandoffParams = { popWindow: 3, listBefore: 300, listAfter: 300 };
const P = 100;
const X = 200;
const G = npc(1);

function detect(episode: QuestEpisode, quests: CatalogQuest[] = []) {
  const byId = new Map(quests.map((quest) => [quest.id, quest]));
  return findHandoffs(new EpisodeTimeline(episode), (id) => byId.get(id), PARAMS);
}

function handoffs(episode: QuestEpisode, quests: CatalogQuest[] = []) {
  return detect(episode, quests).observations;
}

/** P in the log until it is turned in at G at t = 100. */
function turnInEpisode(overrides: Partial<QuestEpisode>): QuestEpisode {
  return makeEpisode({
    questLog: [
      { t: 0, v: [P] },
      { t: 100.2, v: [] },
    ],
    ...overrides,
    questEvents: [event(99, "complete", P, G), event(100, "turnedIn", P, G), ...(overrides.questEvents ?? [])],
  });
}

describe("findHandoffs: pops", () => {
  it("takes an accept dialog at the same giver right after the turn-in as a pop", () => {
    const observations = handoffs(turnInEpisode({ questEvents: [event(100.4, "detail", X, G)] }));
    expect(observations).toMatchObject([{ kind: "pop", turnedIn: P, offered: X, confounders: [] }]);
  });

  it("borrows the giver from the completion dialog when the turn-in event has none", () => {
    const episode = makeEpisode({ questEvents: [event(99, "complete", P, G), event(100, "turnedIn", P), event(100.4, "detail", X, G)] });
    expect(handoffs(episode)).toMatchObject([{ kind: "pop", giver: G }]);
  });

  it("ignores dialogs the player picked from a fresh list, at another giver, or too late", () => {
    const fromList = turnInEpisode({ offers: [gossip(101, G, [X])], questEvents: [event(102, "detail", X, G)] });
    const otherGiver = turnInEpisode({ questEvents: [event(100.4, "detail", X, npc(2))] });
    const late = turnInEpisode({ questEvents: [event(110, "detail", X, G)] });
    expect(handoffs(fromList).filter((o) => o.kind === "pop")).toEqual([]);
    expect(handoffs(otherGiver)).toEqual([]);
    expect(handoffs(late)).toEqual([]);
  });

  it("treats a dialog after an incomplete list from the giver as picked by the player", () => {
    const incompleteList = { ...gossip(100.2, G, [X]), listComplete: false };
    const episode = turnInEpisode({ offers: [incompleteList], questEvents: [event(100.6, "detail", X, G)] });
    expect(handoffs(episode)).toEqual([]);
  });

  it("attributes a pop to the latest turn-in at the giver", () => {
    const episode = turnInEpisode({ questEvents: [event(101, "complete", 101, G), event(101.5, "turnedIn", 101, G), event(101.7, "detail", X, G)] });
    expect(handoffs(episode).map((o) => [o.kind, o.turnedIn, o.offered])).toEqual([["pop", 101, X]]);
  });

  it("does not use repeatable quests as prerequisites", () => {
    const episode = turnInEpisode({ questEvents: [event(100.4, "detail", X, G)] });
    expect(handoffs(episode, [catalogQuest(P, { specialFlags: 1 })])).toEqual([]);
  });
});

describe("findHandoffs: newly listed quests", () => {
  it("reports quests in the giver's list after the turn-in that its list before lacked", () => {
    const episode = turnInEpisode({ offers: [gossip(90, G, [300], [P]), gossip(110, G, [300, X, 201])] });
    const { observations, comparedQuests } = detect(episode);
    expect(observations.map((o) => [o.kind, o.offered, o.confounders])).toEqual([
      ["list", X, []],
      ["list", 201, []],
    ]);
    // 300 stayed available: judged (not unlocked by P), so it counts as covered.
    expect([...comparedQuests].sort()).toEqual([200, 201, 300]);
  });

  it("skips repeatable quests, which come back on their own", () => {
    const daily = { id: 201, frequency: 2 };
    const episode = turnInEpisode({ offers: [gossip(90, G, []), { ...gossip(110, G, [X]), available: [{ id: X }, daily] }] });
    expect(handoffs(episode, [catalogQuest(X, { specialFlags: 1 })])).toEqual([]);
  });

  it("skips quests that were hidden before because they were in the log", () => {
    const episode = turnInEpisode({
      questLog: [
        { t: 0, v: [P, X] },
        { t: 95, v: [P] },
        { t: 100.2, v: [] },
      ],
      offers: [gossip(90, G, []), gossip(110, G, [X])],
    });
    expect(handoffs(episode)).toEqual([]);
  });

  it("flags a level-up between the lists unless the quest was already level-eligible", () => {
    const episode = turnInEpisode({
      levels: [
        { t: 0, v: 10 },
        { t: 105, v: 11 },
      ],
      offers: [gossip(90, G, []), gossip(110, G, [X, 201])],
    });
    const observations = handoffs(episode, [catalogQuest(X, { requiredLevel: 11 }), catalogQuest(201, { requiredLevel: 8 })]);
    expect(observations.find((o) => o.offered === X)?.confounders).toEqual(["level 10 -> 11"]);
    expect(observations.find((o) => o.offered === 201)?.confounders).toEqual([]);
  });

  it("flags other completions and abandoned quests between the lists", () => {
    const episode = turnInEpisode({
      questLog: [
        { t: 0, v: [P, 50, 60] },
        { t: 100.2, v: [] },
      ],
      questEvents: [event(104, "turnedIn", 50, npc(9))],
      offers: [gossip(90, G, []), gossip(110, G, [X])],
    });
    expect(handoffs(episode)[0].confounders).toEqual(["also completed 50", "60 left the log"]);
  });

  it("flags resets and quests entering the log between the lists", () => {
    const episode = turnInEpisode({
      completed: { source: "modern", initial: [70], changes: [{ t: 105, remove: [70] }] },
      questLog: [
        { t: 0, v: [P] },
        { t: 100.2, v: [] },
        { t: 105, v: [80] },
      ],
      offers: [gossip(90, G, []), gossip(110, G, [X])],
    });
    expect(handoffs(episode)[0].confounders).toEqual(["70 reset", "80 entered the log"]);
  });
});
