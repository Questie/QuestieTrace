import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeObjectDrops } from "./npcDrops";

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

describe("observeObjectDrops", () => {
  it("should extract object drop IDs from GetLootSourceInfo correlated with GetLootSlotLink", () => {
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
              v: { 1: "GameObject-0-5208-0-7-2843-0000399", 2: 1, n: 2 },
            },
          ],
        },
      },
    );

    expect(observeObjectDrops(session)).toEqual([
      {
        entityId: 12345,
        value: { objectID: 2843 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should extract multiple object IDs from a multi-pair source tuple", () => {
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
                1: "GameObject-0-5208-0-7-2843-0000399",
                2: 1,
                3: "GameObject-0-5208-0-7-2844-0000400",
                4: 1,
                n: 4,
              },
            },
          ],
        },
      },
    );

    expect(observeObjectDrops(session)).toEqual([
      {
        entityId: 12345,
        value: { objectID: 2843 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
      {
        entityId: 12345,
        value: { objectID: 2844 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should skip npc-sourced loot GUIDs", () => {
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
              v: { 1: "Creature-0-5208-0-7-823-000031", 2: 1, n: 2 },
            },
          ],
        },
      },
    );

    expect(observeObjectDrops(session)).toEqual([]);
  });

  it("should skip items with no GetLootSourceInfo stream", () => {
    const session = makeSession({
      GetLootSlotLink: {
        1: [{ t: 5, tp: 5, v: "|Hitem:12345::::::::1::::::::::|h[Feather]|h[|r" }],
      },
    });

    expect(observeObjectDrops(session)).toEqual([]);
  });

  it("should handle multiple slots and multiple times", () => {
    const session = makeSession(
      {
        GetLootSlotLink: {
          1: [
            { t: 5, tp: 5, v: "|Hitem:111::::::::1::::::::::|h[Feather]|h[|r" },
            { t: 10, tp: 10, v: "|Hitem:222::::::::1::::::::::|h[Bone]|h[|r" },
          ],
        },
        GetLootSourceInfo: {
          1: [
            {
              t: 5,
              tp: 5,
              v: { 1: "GameObject-0-5208-0-7-2843-0000399", 2: 1, n: 2 },
            },
            {
              t: 10,
              tp: 10,
              v: { 1: "GameObject-0-5208-0-7-2844-0000400", 2: 1, n: 2 },
            },
          ],
        },
      },
    );

    expect(observeObjectDrops(session)).toEqual([
      { entityId: 111, value: { objectID: 2843 }, confidence: "high", provenance: { session: "test-session", t: 5 } },
      { entityId: 222, value: { objectID: 2844 }, confidence: "high", provenance: { session: "test-session", t: 10 } },
    ]);
  });

  it("should return empty array when no loot data exists", () => {
    const session = makeSession({});
    expect(observeObjectDrops(session)).toEqual([]);
  });
});
