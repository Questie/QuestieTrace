import { describe, expect, it } from "vitest";
import { catalogQuest, event, gossip, makeEpisode, npc } from "../../core/fixtures";
import { EpisodeTimeline } from "../../core/timeline";
import type { CatalogQuest, QuestEpisode } from "../../core/types";
import { findCounterEvidence } from "./counter";

const P = 100;
const X = 200;
const claimed = new Map([[X, new Set([P])]]);

function counters(episode: QuestEpisode, options: { quests?: CatalogQuest[]; alternatives?: number[] } = {}) {
  const byId = new Map((options.quests ?? []).map((quest) => [quest.id, quest]));
  return findCounterEvidence(new EpisodeTimeline(episode), claimed, () => options.alternatives ?? [], (id) => byId.get(id));
}

describe("findCounterEvidence", () => {
  it("splits offers without P by whether P was not started or still in the log", () => {
    const notStarted = makeEpisode({ offers: [gossip(10, npc(1), [X])] });
    const inLog = makeEpisode({ questLog: [{ t: 0, v: [P] }], questEvents: [event(10, "detail", X, npc(1))] });
    expect(counters(notStarted).map((o) => o.kind)).toEqual(["notStarted"]);
    expect(counters(inLog).map((o) => o.kind)).toEqual(["inLog"]);
  });

  it("is silent once P is completed, counting a turn-in just before the offer", () => {
    const completed = makeEpisode({ completed: { source: "modern", initial: [P], changes: [] }, offers: [gossip(10, npc(1), [X])] });
    const justTurnedIn = makeEpisode({
      questLog: [{ t: 0, v: [P] }],
      questEvents: [event(10, "turnedIn", P, npc(1)), event(10.2, "detail", X, npc(1))],
    });
    expect(counters(completed)).toEqual([]);
    expect(counters(justTurnedIn)).toEqual([]);
  });

  it("tolerates a completion delta landing just after the offer, but not a turn-in event", () => {
    const lateDelta = makeEpisode({ completed: { source: "modern", initial: [], changes: [{ t: 10.6, add: [P] }] }, offers: [gossip(10, npc(1), [X])] });
    const beforeTurnIn = makeEpisode({
      questLog: [{ t: 0, v: [P] }],
      offers: [gossip(10, npc(1), [X])],
      questEvents: [event(10.5, "turnedIn", P, npc(1))],
    });
    expect(counters(lateDelta)).toEqual([]);
    expect(counters(beforeTurnIn).map((o) => o.kind)).toEqual(["inLog"]);
  });

  it("explains offers by a completed alternative or by P being closed to the character's race", () => {
    const viaVariant = makeEpisode({ completed: { source: "modern", initial: [101], changes: [] }, offers: [gossip(10, npc(1), [X])] });
    const otherRace = makeEpisode({ player: { raceId: 2 }, offers: [gossip(10, npc(1), [X])] });
    expect(counters(viaVariant, { alternatives: [101] })).toMatchObject([{ kind: "alternative", because: "101 completed" }]);
    expect(counters(otherRace, { quests: [catalogQuest(P, { requiredRaces: 1 })] })).toMatchObject([{ kind: "alternative" }]);
  });

  it("says nothing without a completed-quest history", () => {
    expect(counters(makeEpisode({ completed: undefined, offers: [gossip(10, npc(1), [X])] }))).toEqual([]);
  });
});
