import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { objectLoots } from "./_loots";

function makeSession(functions: SessionRecord["functions"], lootsOpenedAt: number[]): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events: lootsOpenedAt.map((t) => ({ t, tp: t, e: "LOOT_READY", a: { n: 0 } })),
    functions,
    functionsDelta: {},
  };
}

describe("objectLoots", () => {
  it("should count a loot of several slots from the same object once", () => {
    const source = { 1: "GameObject-0-5208-0-7-673206-0000399", 2: 1, n: 2 };
    const session = makeSession({
      GetLootSourceInfo: {
        1: [{ t: 5, tp: 5, v: source }],
        2: [{ t: 5, tp: 5, v: source }],
      },
    }, [5]);

    expect(objectLoots(session)).toEqual([{ objectID: 673206, t: 5 }]);
  });

  it("should not count the sample taken when the loot window closes", () => {
    const source = { 1: "GameObject-0-5208-0-7-673206-0000399", 2: 1, n: 2 };
    const session = makeSession({
      GetLootSourceInfo: { 1: [{ t: 5, tp: 5, v: source }, { t: 7, tp: 7, v: source }] },
    }, [5]);

    expect(objectLoots(session)).toEqual([{ objectID: 673206, t: 5 }]);
  });

  it("should ignore loot from npcs and empty slots", () => {
    const session = makeSession({
      GetLootSourceInfo: {
        1: [
          { t: 5, tp: 5, v: { 1: "Creature-0-5208-0-7-823-000031", 2: 1, n: 2 } },
          { t: 6, tp: 6 },
        ],
      },
    }, [5, 6]);

    expect(objectLoots(session)).toEqual([]);
  });

  it("should return every loot of the same object at different times", () => {
    const session = makeSession({
      GetLootSourceInfo: {
        1: [
          { t: 5, tp: 5, v: { 1: "GameObject-0-5208-0-7-673206-0000399", 2: 1, n: 2 } },
          { t: 9, tp: 9, v: { 1: "GameObject-0-5208-0-7-673206-0000400", 2: 1, n: 2 } },
        ],
      },
    }, [5, 9]);

    expect(objectLoots(session)).toEqual([
      { objectID: 673206, t: 5 },
      { objectID: 673206, t: 9 },
    ]);
  });
});
