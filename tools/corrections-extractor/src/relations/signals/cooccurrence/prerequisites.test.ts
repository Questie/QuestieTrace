import { describe, expect, it } from "vitest";
import { catalogQuest } from "../../core/fixtures";
import { catalogOf, populationOf, session } from "./fixtures";
import { inferPrerequisites, type PrerequisiteParams } from "./prerequisites";

const params: PrerequisiteParams = {
  minTakers: 3,
  minOrdered: 1,
  maxNullProbability: 0.05,
  orderToleranceSeconds: 10,
  handoffSeconds: 30,
};

const edgesFor = (result: ReturnType<typeof inferPrerequisites>, questId: number) =>
  result.edges.filter((edge) => edge.questId === questId).map(({ prereq, field, rank, shape }) => ({ prereq, field, rank, shape }));

/** Characters who did nothing relevant: the baseline that makes a prerequisite non-universal. */
const bystanders = (count: number, race = 1) =>
  Array.from({ length: count }, (_, i) => session(`bystander-${race}-${i}`, { race, level: 10, initial: [8] }));

describe("inferPrerequisites", () => {
  it("emits the hand-off predecessor, prunes the transitive one, and ranks a zone-mate below it", () => {
    // 1 -> 2 -> 3, and every taker of 3 also finished zone-mate 9 while holding 2.
    const takers = ["a", "b", "c", "d"].map((key) =>
      session(key, { initial: [1], accepts: [[10, 2], [101, 3]], turnIns: [[50, 9], [100, 2], [200, 3]] }),
    );
    const population = populationOf([...takers, ...bystanders(3)], catalogOf([1, 2, 3, 8, 9]));

    expect(edgesFor(inferPrerequisites(population, params), 3)).toEqual([
      { prereq: 2, field: "preQuestSingle", rank: 1, shape: "full" },
      { prereq: 9, field: "preQuestGroup", rank: 2, shape: "full" },
    ]);
  });

  it("drops a prerequisite as soon as one character held the quest before completing it", () => {
    const handedOff = ["a", "b", "c"].map((key) => session(key, { initial: [8], turnIns: [[100, 2]], accepts: [[101, 3]] }));
    const heldFirst = session("d", { initial: [8], accepts: [[10, 3]], turnIns: [[100, 2]] });

    expect(edgesFor(inferPrerequisites(populationOf([...handedOff, ...bystanders(3)], catalogOf([2, 3, 8])), params), 3)).toEqual([
      { prereq: 2, field: "preQuestSingle", rank: 1, shape: "full" },
    ]);
    expect(edgesFor(inferPrerequisites(populationOf([...handedOff, heldFirst, ...bystanders(3)], catalogOf([2, 3, 8])), params), 3)).toEqual([]);
  });

  it("ignores a quest that nearly every comparable character completed anyway", () => {
    // Everyone did 7, takers of 3 included: no lift, however consistent the order.
    const takers = ["a", "b", "c"].map((key) => session(key, { initial: [8], turnIns: [[100, 7]], accepts: [[101, 3]] }));
    const didSeven = ["d", "e", "f", "g", "h", "i"].map((key) => session(key, { initial: [8, 7] }));

    expect(edgesFor(inferPrerequisites(populationOf([...takers, ...didSeven], catalogOf([3, 7, 8])), params), 3)).toEqual([]);
    expect(edgesFor(inferPrerequisites(populationOf([...takers, ...bystanders(6)], catalogOf([3, 7, 8])), params), 3)).toEqual([
      { prereq: 7, field: "preQuestSingle", rank: 1, shape: "full" },
    ]);
  });

  it("emits faction variants as alternatives when together they cover every taker", () => {
    const quests = [catalogQuest(30), catalogQuest(31, { requiredRaces: 4294967373 }), catalogQuest(32, { requiredRaces: 8589934770 }), 8];
    const alliance = ["a", "b", "c"].map((key) => session(key, { race: 1, initial: [8], turnIns: [[100, 31]], accepts: [[101, 30]] }));
    const horde = ["d", "e", "f"].map((key) => session(key, { race: 2, initial: [8], turnIns: [[100, 32]], accepts: [[101, 30]] }));
    const population = populationOf([...alliance, ...horde, ...bystanders(2, 1), ...bystanders(2, 2)], catalogOf(quests));

    expect(edgesFor(inferPrerequisites(population, params), 30)).toEqual([
      { prereq: 31, field: "preQuestSingle", rank: 1, shape: "variant" },
      { prereq: 32, field: "preQuestSingle", rank: 1, shape: "variant" },
    ]);
  });

  it("does not count a hand-off that the prerequisite's level-up explains", () => {
    // Turning in 2 dings level 10, which unlocks 3 on its own.
    const quests = [2, 8, catalogQuest(3, { requiredLevel: 10 })];
    const dinged = ["a", "b", "c"].map((key) => session(key, { level: 9, initial: [8], turnIns: [[100, 2]], levelUps: [[100.2, 10]], accepts: [[101, 3]] }));
    const [edge] = inferPrerequisites(populationOf([...dinged, ...bystanders(3)], catalogOf(quests)), params).edges.filter((e) => e.questId === 3);

    expect(edge).toMatchObject({ prereq: 2, ordered: 3, handoffs: 0, levelUps: 3 });
  });

  it("keeps a prerequisite that a faction-only step implies for one faction but not the other", () => {
    // Alliance goes 5 -> 6 -> 30, Horde goes 5 -> 30: the Alliance-only 6 must not prune 5.
    const quests = [catalogQuest(30), 5, catalogQuest(6, { requiredRaces: 4294967373 }), 8];
    const alliance = ["a", "b", "c"].map((key) =>
      session(key, { race: 1, initial: [8], turnIns: [[50, 5], [100, 6]], accepts: [[60, 6], [101, 30]] }),
    );
    const horde = ["d", "e", "f"].map((key) => session(key, { race: 2, initial: [8], turnIns: [[100, 5]], accepts: [[101, 30]] }));
    const population = populationOf([...alliance, ...horde, ...bystanders(2, 1), ...bystanders(2, 2)], catalogOf(quests));

    expect(edgesFor(inferPrerequisites(population, params), 30)).toContainEqual({ prereq: 5, field: "preQuestSingle", rank: 1, shape: "full" });
  });

  it("treats co-flagged copies as one step and never as each other's prerequisite", () => {
    // Turning in 41 flags its copy 42 half a second later, then 43 is handed off.
    const takers = ["a", "b", "c"].map((key) => session(key, { initial: [8], turnIns: [[100, 41]], flags: [[100.5, 42]], accepts: [[102, 43]] }));
    const population = populationOf([...takers, ...bystanders(3)], catalogOf([8, 41, 42, 43]));
    const coFlagged = (a: number, b: number) => (a === 41 && b === 42) || (a === 42 && b === 41);
    const result = inferPrerequisites(population, params, coFlagged);

    expect(edgesFor(result, 43).map(({ prereq, rank }) => ({ prereq, rank }))).toEqual([
      { prereq: 41, rank: 1 },
      { prereq: 42, rank: 1 },
    ]);
    expect(edgesFor(result, 42)).toEqual([]);
    expect(edgesFor(inferPrerequisites(population, params), 42).map(({ prereq }) => prereq)).toEqual([41]);
  });
});
