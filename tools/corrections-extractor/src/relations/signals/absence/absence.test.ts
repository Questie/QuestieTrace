import { describe, expect, it } from "vitest";
import { catalogQuest, event, gossip, makeEpisode, npc } from "../../core/fixtures";
import type { CatalogQuest, QuestCatalog, QuestEpisode, RelationField } from "../../core/types";
import { analyzeAbsence, type AbsenceResult } from "./analyze";
import { DEFAULT_PARAMS } from "./params";

function catalogOf(quests: CatalogQuest[]): QuestCatalog {
  return { generatedAt: "", sources: [], quests: Object.fromEntries(quests.map((quest) => [quest.id, quest])) };
}

function run(episodes: QuestEpisode[], quests: CatalogQuest[]): Promise<AbsenceResult> {
  return analyzeAbsence(() => episodes, catalogOf(quests), DEFAULT_PARAMS);
}

function edge(result: AbsenceResult, questId: number, field: RelationField, target: number) {
  return result.candidates.find((candidate) => candidate.questId === questId && candidate.field === field && candidate.target === target);
}

describe("absence signal", () => {
  const giver = npc(100);
  // Two variants of one quest (zone copies share a name); skill-gated like Forever's Camping 101.
  const X = 1001;
  const Y = 1002;
  const variants = [catalogQuest(X, { name: "Path of Ash", requiredSkill: [129, 1] }), catalogQuest(Y, { name: "Path of Ash", requiredSkill: [129, 1] })];

  it("reads exclusiveTo from a list that loses X right after Y is accepted, in that direction only", async () => {
    const chooser = makeEpisode({
      completed: { source: "modern", initial: [500], changes: [] },
      questLog: [
        { t: 0, v: [] },
        { t: 13, v: [Y] },
      ],
      questEvents: [event(11, "detail", Y, giver), event(12, "accepted", Y, giver)],
      offers: [gossip(10, giver, [X, Y]), gossip(20, giver, [])],
    });

    const result = await run([chooser], variants);

    const forward = edge(result, X, "exclusiveTo", Y)!;
    expect(forward.support).toBe(1);
    expect(forward.score).toBeGreaterThan(0.5);
    expect(forward.evidence[0].text).toContain("1002 in log (accepted at 12.0s)");
    // Nothing was observed about Y's availability, so nothing is claimed about what hides Y.
    expect(edge(result, Y, "exclusiveTo", X)).toBeUndefined();
  });

  it("drops a blocker once another character was offered X while in that state", async () => {
    const chooser = makeEpisode({
      questLog: [
        { t: 0, v: [] },
        { t: 13, v: [Y] },
      ],
      questEvents: [event(12, "accepted", Y, giver)],
      offers: [gossip(10, giver, [X, Y]), gossip(20, giver, [])],
    });
    const holdsBoth = makeEpisode({ questLog: [{ t: 0, v: [Y] }], offers: [gossip(5, giver, [X])] });

    const result = await run([chooser, holdsBoth], variants);

    expect(result.candidates.filter((candidate) => candidate.questId === X && candidate.target === Y)).toEqual([]);
  });

  describe("prerequisites: X stays off the list until P is turned in", () => {
    const P = 2000;
    const next = 2001;
    const quests = [catalogQuest(P, { name: "First Step" }), catalogQuest(next, { name: "Second Step" })];
    const veteran = () => makeEpisode({ completed: { source: "modern", initial: [P], changes: [] }, offers: [gossip(5, giver, [next])] });
    const learner = (logAtVisit: number[]) =>
      makeEpisode({
        completed: { source: "modern", initial: [], changes: [{ t: 31, add: [P] }] },
        questLog: [
          { t: 0, v: logAtVisit },
          { t: 21, v: [P] },
          { t: 31, v: [] },
        ],
        questEvents: [event(20, "accepted", P, npc(7)), event(30, "turnedIn", P, giver)],
        offers: [gossip(10, giver, [], logAtVisit), gossip(32, giver, [next])],
      });

    it("is proven by a block while P was not even taken", async () => {
      const result = await run([learner([]), veteran()], quests);

      const prerequisite = edge(result, next, "preQuestSingle", P)!;
      expect(prerequisite.note).toContain("group prerequisite");
      expect(prerequisite.score).toBeGreaterThan(0.9);
      expect(prerequisite.evidence[0].text).toContain("2000 not taken");
    });

    it("scores lower when P was in the log at every block, as a breadcrumb looks the same", async () => {
      const result = await run([learner([P]), veteran()], quests);

      const prerequisite = edge(result, next, "preQuestSingle", P)!;
      expect(prerequisite.note).toContain("group pendingPrerequisite");
      expect(prerequisite.score).toBeLessThan(0.9);
      // P in the log is the same fact as P not completed yet: no separate log-based blocker.
      expect(result.candidates.filter((candidate) => candidate.field !== "preQuestSingle")).toEqual([]);
    });
  });

  it("names the direct prerequisite, not the earlier steps of its chain", async () => {
    const [first, second, third] = [4000, 4001, 4002];
    const quests = [catalogQuest(first), catalogQuest(second), catalogQuest(third)];
    const done = (...ids: number[]) => ({ source: "modern" as const, initial: ids, changes: [] });
    const episodes = [
      makeEpisode({ completed: done(first, second), offers: [gossip(5, giver, [third])] }),
      makeEpisode({ completed: done(first, second), offers: [gossip(5, giver, [third])] }),
      makeEpisode({ completed: done(first), offers: [gossip(5, giver, [second])] }),
      makeEpisode({ completed: done(), offers: [gossip(5, giver, [first])] }),
    ];

    const result = await run(episodes, quests);

    // The newcomer's block splits between "first" and "second" missing; all of it lands on "second".
    const direct = edge(result, third, "preQuestSingle", second)!;
    expect(direct.note).toContain("shares 1.00");
    expect(direct.evidence[0].text).toContain("4001 not taken");
    expect(edge(result, third, "preQuestSingle", first)).toBeUndefined();
  });

  it("hands an earlier step's blame over once, split between the prerequisites that require it", async () => {
    const [root, left, right, goal] = [4100, 4101, 4102, 4103];
    const quests = [catalogQuest(root), catalogQuest(left), catalogQuest(right), catalogQuest(goal)];
    const done = (...ids: number[]) => ({ source: "modern" as const, initial: ids, changes: [] });
    const episodes = [
      makeEpisode({ completed: done(root, left, right), offers: [gossip(5, giver, [goal])] }),
      makeEpisode({ completed: done(root, left, right), offers: [gossip(5, giver, [goal])] }),
      makeEpisode({ completed: done(root), offers: [gossip(5, giver, [left, right])] }),
      makeEpisode({ completed: done(), offers: [gossip(5, giver, [root])] }),
    ];

    const result = await run(episodes, quests);

    // Thirds of the newcomer's block: root's third goes half to each side, never all of it twice.
    for (const side of [left, right]) expect(edge(result, goal, "preQuestSingle", side)!.note).toContain("shares 0.50");
    expect(edge(result, goal, "preQuestSingle", root)).toBeUndefined();
  });

  it("treats zone copies that always complete together as one suspect, and names every copy", async () => {
    const [copyA, copyB, next] = [6001, 6002, 6003];
    const finishers = { npcs: [giver.id], objects: [] };
    const quests = [catalogQuest(copyA, { name: "Welcome", finishers }), catalogQuest(copyB, { name: "Welcome", finishers }), catalogQuest(next, { name: "Onwards" })];
    // Turning in either copy completes both, in one completed-set batch.
    const veteran = () =>
      makeEpisode({
        completed: { source: "modern", initial: [], changes: [{ t: 3, add: [copyA, copyB] }] },
        offers: [gossip(5, giver, [next])],
      });
    const newcomer = makeEpisode({ offers: [gossip(5, giver, [])] });

    const result = await run([veteran(), veteran(), newcomer], quests);

    for (const copy of [copyA, copyB]) {
      const prerequisite = edge(result, next, "preQuestSingle", copy)!;
      expect(prerequisite.evidence[0].text).toContain("6001/6002 not taken");
      expect(prerequisite.support).toBe(1);
    }
  });

  it("reads 'any one of these copies' as prerequisites when no single copy is required", async () => {
    const [copyA, copyB, next] = [5001, 5002, 5003];
    // Both copies are turned in where the next quest starts.
    const finishers = { npcs: [giver.id], objects: [] };
    const quests = [catalogQuest(copyA, { name: "Welcome", finishers }), catalogQuest(copyB, { name: "Welcome", finishers }), catalogQuest(next, { name: "Onwards" })];
    const done = (...ids: number[]) => ({ source: "modern" as const, initial: ids, changes: [] });
    const episodes = [
      makeEpisode({ completed: done(copyA), offers: [gossip(5, giver, [next])] }),
      makeEpisode({ completed: done(copyB), offers: [gossip(5, giver, [next])] }),
      makeEpisode({ completed: done(), questLog: [{ t: 0, v: [copyA] }], offers: [gossip(5, giver, [], [copyA])] }),
    ];

    const result = await run(episodes, quests);

    for (const copy of [copyA, copyB]) {
      const prerequisite = edge(result, next, "preQuestSingle", copy)!;
      expect(prerequisite.note).toContain("missing any of 5001/5002");
      expect(prerequisite.evidence[0].text).toContain("none of 5001/5002 completed");
      expect(prerequisite.support).toBe(1);
    }
    expect(result.candidates.filter((candidate) => candidate.field !== "preQuestSingle")).toEqual([]);
  });

  it("reads breadcrumbs when only the lead-in in the log hides the target", async () => {
    const [lead, target] = [3000, 3001];
    const targetGiver = npc(302);
    const quests = [catalogQuest(lead, { name: "Report to the Outpost" }), catalogQuest(target, { name: "Outpost Trouble" })];
    const follower = makeEpisode({
      completed: { source: "modern", initial: [], changes: [{ t: 21, add: [lead] }] },
      questLog: [
        { t: 0, v: [lead] },
        { t: 21, v: [] },
      ],
      questEvents: [event(20, "turnedIn", lead, targetGiver)],
      offers: [gossip(10, targetGiver, [], [lead]), gossip(22, targetGiver, [target])],
    });
    const skipper = makeEpisode({ offers: [gossip(5, targetGiver, [target])] });

    const result = await run([follower, skipper], quests);

    expect(edge(result, target, "breadcrumbs", lead)!.score).toBeGreaterThan(0.4);
  });

  describe("sets aside lists that say nothing about relationships", () => {
    // An offered character shows X is available to Y-less characters; the blocked one opens the
    // giver with Y in the log. Each case changes one thing about the blocked character.
    const offered = () => makeEpisode({ offers: [gossip(5, giver, [X, Y])] });
    const plain = [catalogQuest(X, { name: "Path of Ash" }), catalogQuest(Y, { name: "Path of Ash" })];
    const blocked = (overrides: Partial<QuestEpisode> = {}) =>
      makeEpisode({ questLog: [{ t: 0, v: [Y] }], offers: [gossip(10, giver, [])], ...overrides });

    it("uses the plain case as evidence", async () => {
      const result = await run([offered(), blocked()], plain);
      const exclusive = edge(result, X, "exclusiveTo", Y)!;
      expect(exclusive.score).toBeGreaterThan(0.5);
      expect(exclusive.evidence[0].text).toContain("1002 in log");
    });

    it("a level below every level X was offered at, even when the catalog allows it", async () => {
      const result = await run([offered(), blocked({ levels: [{ t: 0, v: 8 }] })], [catalogQuest(X, { name: "Path of Ash", requiredLevel: 5 }), plain[1]]);
      expect(result.blocked.skipped.get("belowOfferedLevel")).toBeGreaterThan(0);
      expect(edge(result, X, "exclusiveTo", Y)).toBeUndefined();
    });

    it("a faction X was never offered to", async () => {
      const result = await run([offered(), blocked({ player: { raceId: 2, classId: 1, faction: "Horde" } })], plain);
      expect(result.blocked.skipped.get("unofferedFaction")).toBeGreaterThan(0);
      expect(edge(result, X, "exclusiveTo", Y)).toBeUndefined();
    });

    it("a list read while X's own dialog was just accepted", async () => {
      const result = await run([offered(), blocked({ questEvents: [event(9.5, "accepted", X, npc(999))] })], plain);
      expect(result.blocked.skipped.get("self")).toBeGreaterThan(0);
      expect(edge(result, X, "exclusiveTo", Y)).toBeUndefined();
    });

    it("an empty gossip sub-menu right after the quest list", async () => {
      const result = await run([offered(), blocked({ offers: [gossip(8, giver, [Y + 1]), gossip(10, giver, [])] })], plain);
      expect(result.blocked.skipped.get("gossipSubMenu")).toBe(1);
      expect(result.blocked.blockedMoments).toBe(1);
    });

    it("a standalone quest detail, which may be a follow-up pushed after a turn-in", async () => {
      const detail = { t: 10, source: "detail" as const, giver, available: [{ id: Y + 1 }], listComplete: true };
      const result = await run([offered(), blocked({ offers: [detail] })], plain);
      expect(result.blocked.blockedMoments).toBe(0);
    });

    describe("a skill-gated quest needs an offer to the same character in an earlier session", () => {
      // Session names are missing for a third of sessions, so order comes from sessionOrder only.
      const named = { sessionName: "2026-10-01_10-00-00" };
      const offeredIn = (sessionOrder: number) =>
        makeEpisode({ characterKey: "returning", sessionOrder, ...(sessionOrder === 0 ? named : {}), offers: [gossip(5, giver, [X, Y])] });
      const blockedIn = (sessionOrder: number) =>
        blocked({ characterKey: "returning", sessionOrder, ...(sessionOrder === 0 ? named : {}) });

      it("uses a block in a later, unnamed session", async () => {
        const result = await run([offeredIn(0), blockedIn(1)], variants);
        const exclusive = edge(result, X, "exclusiveTo", Y)!;
        expect(exclusive.score).toBeGreaterThan(0.5);
        expect(exclusive.note).toContain("transitions 1");
      });

      it("sets aside a block in an earlier session than the offer", async () => {
        const result = await run([offeredIn(1), blockedIn(0)], variants);
        expect(result.blocked.skipped.get("untrackedRequirements")).toBeGreaterThan(0);
        expect(edge(result, X, "exclusiveTo", Y)).toBeUndefined();
      });
    });

    it("a skill-gated quest the character was never offered a same-requirement copy of", async () => {
      const result = await run([offered(), blocked()], variants);
      expect(result.blocked.skipped.get("untrackedRequirements")).toBeGreaterThan(0);
      expect(edge(result, X, "exclusiveTo", Y)).toBeUndefined();
    });
  });
});
