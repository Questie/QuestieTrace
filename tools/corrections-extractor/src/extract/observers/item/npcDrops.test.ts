import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeNpcDrops } from "./npcDrops";

function makeSession(
  functions: Partial<SessionRecord["functions"]>,
  events: SessionRecord["events"] = [],
): SessionRecord {
  return {
    schemaVersion: 11,
    recordingContractVersion: 1,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    duration: 100,
    durationPrecise: 100,
    events,
    functions: functions as SessionRecord["functions"],
    functionsDelta: {},
  };
}

describe("observeNpcDrops", () => {
  it("should extract npc drop IDs from GetLootSourceInfo correlated with GetLootSlotLink", () => {
    const session = makeSession(
      {
        GetLootSlotLink: {
          1: [{ t: 5, tp: 5, v: "|Hitem:12345::::::::1::::::::::|h[Tough Wolf Meat]|h[|r" }],
        },
        GetLootSourceInfo: {
          1: [
            {
              t: 5,
              tp: 5,
              v: { 1: "Creature-0-5208-0-7-823-000031", 2: 1, n: 2 },
            },
          ],
        },
      },
    );

    expect(observeNpcDrops(session)).toEqual([
      {
        entityId: 12345,
        value: { npcID: 823 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should extract multiple npc IDs from a multi-pair source tuple", () => {
    const session = makeSession(
      {
        GetLootSlotLink: {
          1: [{ t: 5, tp: 5, v: "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }],
        },
        GetLootSourceInfo: {
          1: [
            {
              t: 5,
              tp: 5,
              v: {
                1: "Creature-0-5208-0-7-823-000031",
                2: 1,
                3: "Creature-0-5208-0-7-824-000032",
                4: 1,
                n: 4,
              },
            },
          ],
        },
      },
    );

    expect(observeNpcDrops(session)).toEqual([
      {
        entityId: 12345,
        value: { npcID: 823 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
      {
        entityId: 12345,
        value: { npcID: 824 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should skip object-sourced loot GUIDs", () => {
    const session = makeSession(
      {
        GetLootSlotLink: {
          1: [{ t: 5, tp: 5, v: "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }],
        },
        GetLootSourceInfo: {
          1: [
            {
              t: 5,
              tp: 5,
              v: { 1: "GameObject-0-5208-0-7-2843-0000399", 2: 1, n: 2 },
            },
          ],
        },
      },
    );

    expect(observeNpcDrops(session)).toEqual([]);
  });

  it("should skip items with no GetLootSourceInfo stream", () => {
    const session = makeSession({
      GetLootSlotLink: {
        1: [{ t: 5, tp: 5, v: "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }],
      },
    });

    expect(observeNpcDrops(session)).toEqual([]);
  });

  it("should handle multiple slots and multiple times", () => {
    const session = makeSession(
      {
        GetLootSlotLink: {
          1: [
            { t: 5, tp: 5, v: "|Hitem:111::::::::1::::::::::|h[Feather]|h[|r" },
            { t: 10, tp: 10, v: "|Hitem:222::::::::1::::::::::|h[Bone]|h[|r" },
          ],
          2: [
            { t: 5, tp: 5, v: "|Hitem:333::::::::1::::::::::|h[Hide]|h[|r" },
          ],
        },
        GetLootSourceInfo: {
          1: [
            {
              t: 5,
              tp: 5,
              v: { 1: "Creature-0-5208-0-7-823-000031", 2: 1, n: 2 },
            },
            {
              t: 10,
              tp: 10,
              v: { 1: "Creature-0-5208-0-7-824-000032", 2: 1, n: 2 },
            },
          ],
          2: [
            {
              t: 5,
              tp: 5,
              v: { 1: "Creature-0-5208-0-7-825-000033", 2: 1, n: 2 },
            },
          ],
        },
      },
    );

    expect(observeNpcDrops(session)).toEqual([
      { entityId: 111, value: { npcID: 823 }, confidence: "high", provenance: { session: "test-session", t: 5 } },
      { entityId: 222, value: { npcID: 824 }, confidence: "high", provenance: { session: "test-session", t: 10 } },
      { entityId: 333, value: { npcID: 825 }, confidence: "high", provenance: { session: "test-session", t: 5 } },
    ]);
  });

  it("should return empty array when no loot data exists", () => {
    const session = makeSession({});
    expect(observeNpcDrops(session)).toEqual([]);
  });

  it("should skip slots where link and source are not at the same time", () => {
    const session = makeSession(
      {
        GetLootSlotLink: {
          1: [{ t: 5, tp: 5, v: "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }],
        },
        GetLootSourceInfo: {
          1: [{ t: 10, tp: 10, v: { 1: "Creature-0-5208-0-7-823-000031", 2: 1, n: 2 } }],
        },
      },
    );

    // Link at t=5, source at t=10 — no correlation
    expect(observeNpcDrops(session)).toEqual([]);
  });
});
