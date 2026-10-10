import { describe, expect, it } from "vitest";
import { makeEpisode, npc } from "../../core/fixtures";
import type { QuestEpisode } from "../../core/types";
import { allOfMembers, nextInChainCandidates, prerequisiteCandidates } from "./candidates";
import type { HandoffKind } from "./detect";
import { HandoffTally } from "./tally";

function observe(tally: HandoffTally, episode: QuestEpisode, kind: HandoffKind, turnedIn: number, offered: number, confounders: string[] = []): void {
  tally.addHandoff(episode, { kind, turnedIn, offered, giver: npc(1), turnInAt: 10, offeredAt: 10.5, previousListAt: 5, confounders });
}

function candidates(tally: HandoffTally) {
  return [...prerequisiteCandidates(tally.pairs.values(), allOfMembers(tally.pairs.values())), ...nextInChainCandidates(tally.pairs.values())];
}

describe("handoff candidates", () => {
  it("claims nextQuestInChain only from pops, and preQuestSingle from pops and lists", () => {
    const tally = new HandoffTally();
    observe(tally, makeEpisode(), "pop", 1, 2);
    observe(tally, makeEpisode(), "list", 1, 3);
    expect(candidates(tally).map((c) => [c.field, c.questId, c.target])).toEqual([
      ["preQuestSingle", 2, 1],
      ["preQuestSingle", 3, 1],
      ["nextQuestInChain", 1, 2],
    ]);
  });

  it("counts a character once however many of its sessions show the hand-off", () => {
    const tally = new HandoffTally();
    observe(tally, makeEpisode({ characterKey: "alice" }), "pop", 1, 2);
    observe(tally, makeEpisode({ characterKey: "alice" }), "pop", 1, 2);
    observe(tally, makeEpisode({ characterKey: "bob" }), "pop", 1, 2);
    expect(candidates(tally).map((c) => c.support)).toEqual([2, 2]);
  });

  it("scores clean evidence above confounded evidence and drops contradicted prerequisites", () => {
    const tally = new HandoffTally();
    observe(tally, makeEpisode(), "list", 1, 2);
    observe(tally, makeEpisode(), "list", 1, 3, ["level 10 -> 11"]);
    observe(tally, makeEpisode(), "list", 1, 4);
    tally.addCounter(makeEpisode(), { kind: "notStarted", prerequisite: 1, quest: 4, t: 1 });
    const [clean, confounded, ...rest] = prerequisiteCandidates(tally.pairs.values(), new Set());
    expect(clean.score).toBeGreaterThan(confounded.score);
    expect(rest).toEqual([]);
  });

  it("claims preQuestGroup when every hand-off from one prerequisite found the others done", () => {
    // X = 9 after 1 and 2. All-of: whoever finished 1 last had 2 done, and the other way round.
    // Any-of: one character got 9 right after 1 with 2 not done.
    const fields = (secondDoneAfterFirst: boolean) => {
      const tally = new HandoffTally();
      const first = makeEpisode();
      const second = makeEpisode();
      observe(tally, first, "pop", 1, 9);
      observe(tally, second, "pop", 2, 9);
      tally.addCompanionStates(first, { kind: "pop", turnedIn: 1, offered: 9, giver: npc(1), turnInAt: 10, offeredAt: 10.5, confounders: [] }, [[2, secondDoneAfterFirst]]);
      tally.addCompanionStates(second, { kind: "pop", turnedIn: 2, offered: 9, giver: npc(1), turnInAt: 10, offeredAt: 10.5, confounders: [] }, [[1, true]]);
      return prerequisiteCandidates(tally.pairs.values(), allOfMembers(tally.pairs.values())).map((c) => c.field);
    };
    expect(fields(true)).toEqual(["preQuestGroup", "preQuestGroup"]);
    expect(fields(false)).toEqual(["preQuestSingle", "preQuestSingle"]);
  });

  it("drops a contradicted pop as nextQuestInChain when the same turn-in popped something else", () => {
    // P = 1 popped 2 for three characters; 3 popped once but was offered elsewhere without 1:
    // the giver auto-opening its only quest, not a chain.
    const tally = new HandoffTally();
    for (let i = 0; i < 3; i++) observe(tally, makeEpisode(), "pop", 1, 2);
    observe(tally, makeEpisode(), "pop", 1, 3);
    tally.addCounter(makeEpisode(), { kind: "notStarted", prerequisite: 1, quest: 3, t: 1 });
    expect(nextInChainCandidates(tally.pairs.values()).map((c) => [c.target, c.contradict])).toEqual([[2, 1]]);
  });

  it("splits nextQuestInChain confidence when characters saw different pops after the same turn-in", () => {
    const tally = new HandoffTally();
    for (let i = 0; i < 3; i++) observe(tally, makeEpisode(), "pop", 1, 2);
    for (let i = 0; i < 2; i++) observe(tally, makeEpisode(), "pop", 1, 3);
    const [toTwo, toThree] = nextInChainCandidates(tally.pairs.values());
    expect([toTwo.support, toTwo.contradict, toThree.support, toThree.contradict]).toEqual([3, 2, 2, 3]);
    expect(toTwo.score).toBeGreaterThan(toThree.score);
  });
});
