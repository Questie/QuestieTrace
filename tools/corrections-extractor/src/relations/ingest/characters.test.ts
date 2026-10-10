import { describe, expect, it } from "vitest";
import { assignCharacters, orderSessions, type CharacterInput } from "./characters";

const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => from + i);

function session(episodeKey: string, start: number, levels: [number, number], initial: number[], final: number[]): CharacterInput {
  return {
    episodeKey,
    contributorId: "contributor",
    player: { raceId: 2, classId: 1, faction: "Horde" },
    start,
    order: start,
    duration: 3600,
    firstLevel: levels[0],
    lastLevel: levels[1],
    initialCompleted: initial,
    finalCompleted: final,
  };
}

describe("assignCharacters", () => {
  it("keeps a same race/class alt apart from the main even though the main has done all the alt's quests", () => {
    // Both start in the same zone, so the alt's quests 1..10 are a subset of the main's history.
    const characters = assignCharacters([
      session("main-1", 0, [30, 31], range(1, 300), range(1, 310)),
      session("alt-1", 10_000, [1, 5], [], range(1, 10)),
      session("main-2", 20_000, [31, 32], range(1, 310), range(1, 320)),
      session("alt-2", 30_000, [5, 8], range(1, 10), range(1, 20)),
    ]);
    expect(characters.get("main-2")).toBe(characters.get("main-1"));
    expect(characters.get("alt-2")).toBe(characters.get("alt-1"));
    expect(characters.get("alt-1")).not.toBe(characters.get("main-1"));
  });

  it("splits sessions that overlap in time or would lose completed quests", () => {
    const characters = assignCharacters([
      session("a", 0, [10, 10], range(1, 50), range(1, 60)),
      session("overlapping", 1800, [10, 10], range(1, 60), range(1, 60)),
      session("lost-history", 10_000, [12, 12], range(1, 20), range(1, 20)),
    ]);
    expect(new Set(characters.values()).size).toBe(3);
  });

  it("orders a character's sessions by progress, then start time, whether or not they have a name", () => {
    const named = { ...session("named", 2000, [10, 10], range(1, 40), range(1, 45)), sessionName: "2026-10-01_12-00-00" };
    // In-progress exports carry no name; this one started later with more history.
    const unnamedLater = session("unnamed-later", 9000, [10, 11], range(1, 45), range(1, 50));
    // Same progress as `named` but captured earlier: the start time breaks the tie.
    const unnamedTie = session("unnamed-tie", 1000, [10, 10], range(1, 40), range(1, 40));
    const sessions = [unnamedLater, named, unnamedTie];
    const order = orderSessions(sessions, new Map(sessions.map((s) => [s.episodeKey, "character"])));
    expect([...order].sort((a, b) => a[1] - b[1]).map(([key]) => key)).toEqual(["unnamed-tie", "named", "unnamed-later"]);
  });
});
