import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../core/types";
import {
  completedQuestsAt,
  playerPosAt,
  playerPosNear,
  playerZoneAt,
  playerZoneNear,
  questIdAt,
  questTitleAt,
  unitGuidAt,
  unitNameAt,
} from "./probes";

function makeSession(functions: SessionRecord["functions"]): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events: [],
    functions,
    functionsDelta: {},
  };
}

describe("probes", () => {
  it("should read the player's zone at or before a given time, translated to areaIDs", () => {
    const session = makeSession({
      "C_Map.GetBestMapForUnit": {
        // uiMapIDs as reported by the API: 1429 = Elwynn Forest (area 12),
        // 1436 = Westfall (area 40).
        player: [
          { t: 0, tp: 0, v: 1429 },
          { t: 10, tp: 10, v: 1436 },
        ],
      },
    });
    expect(playerZoneAt(session, 5)).toBe(12);
    expect(playerZoneAt(session, 10)).toBe(40);
  });

  it("should return null for uiMapIDs without a Questie area (continents, dungeons)", () => {
    const session = makeSession({
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 0, tp: 0, v: 1415 }], // Eastern Kingdoms (continent)
      },
    });
    expect(playerZoneAt(session, 5)).toBeNull();
  });

  it("should find the player's zone nearest a given time within the window", () => {
    const session = makeSession({
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 20, tp: 20, v: 1429 }],
      },
    });
    expect(playerZoneNear(session, 22)).toBe(12);
    expect(playerZoneNear(session, 100, 5)).toBeNull();
  });

  it("should read the player's map position", () => {
    const session = makeSession({
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 0, tp: 0, v: { x: 0.5, y: 0.25 } }],
      },
    });
    expect(playerPosAt(session, 5)).toEqual({ x: 0.5, y: 0.25 });
    expect(playerPosNear(session, 5)).toEqual({ x: 0.5, y: 0.25 });
  });

  it("should read a UnitGUID stream, and unpack UnitName's packed (name, realm) args", () => {
    const session = makeSession({
      UnitGUID: { target: [{ t: 0, tp: 0, v: "Creature-0-5208-0-7-823-000031" }] },
      UnitName: { target: [{ t: 0, tp: 0, v: { 1: "Kobold Vermin", n: 2 } }] },
    });
    expect(unitGuidAt(session, "target", 5)).toBe("Creature-0-5208-0-7-823-000031");
    expect(unitNameAt(session, "target", 5)).toBe("Kobold Vermin");
  });

  it("should strip embedded line breaks from UnitName results entirely", () => {
    // Some NPCs (e.g. "Telenos \<newline>Leafwhisper" seen in traces) are returned
    // by the client with an embedded line break; the display name has none.
    const session = makeSession({
      UnitName: {
        target: [
          { t: 0, tp: 0, v: { 1: "Telenos \nLeafwhisper", n: 2 } },
          { t: 5, tp: 5, v: { 1: "Maelgwyn \r\nShadowtale", n: 2 } },
          { t: 10, tp: 10, v: { 1: "  Padded   Name  ", n: 2 } },
        ],
      },
    });
    expect(unitNameAt(session, "target", 1)).toBe("Telenos Leafwhisper");
    expect(unitNameAt(session, "target", 6)).toBe("Maelgwyn Shadowtale");
    expect(unitNameAt(session, "target", 11)).toBe("Padded   Name");
  });

  it("should normalize GetQuestID's 0 (no active quest frame) to null", () => {
    const session = makeSession({
      GetQuestID: [
        { t: 0, tp: 0, v: 0 },
        { t: 5, tp: 5, v: 783 },
      ],
    });
    expect(questIdAt(session, 2)).toBeNull();
    expect(questIdAt(session, 10)).toBe(783);
  });

  it("should read the quest detail title text, ignoring empty strings", () => {
    const session = makeSession({
      GetTitleText: [
        { t: 0, tp: 0, v: "" },
        { t: 5, tp: 5, v: "A Threat Within" },
      ],
    });
    expect(questTitleAt(session, 2)).toBeNull();
    expect(questTitleAt(session, 10)).toBe("A Threat Within");
  });

  it("should return null for probes on missing streams", () => {
    const session = makeSession({});
    expect(playerZoneAt(session, 0)).toBeNull();
    expect(playerPosAt(session, 0)).toBeNull();
    expect(unitGuidAt(session, "target", 0)).toBeNull();
    expect(questIdAt(session, 0)).toBeNull();
  });
});

describe("completedQuestsAt", () => {
  const MODERN = "C_QuestLog.GetAllCompletedQuestIDs";

  function withDelta(functionsDelta: SessionRecord["functionsDelta"]): SessionRecord {
    return { ...makeSession({}), functionsDelta };
  }

  it("should apply every change recorded at or before the time to the initial set", () => {
    const session = withDelta({
      [MODERN]: {
        t: 0,
        tp: 0,
        initial: [1, 2],
        delta: [
          { t: 5, tp: 5, add: [3] },
          { t: 10, tp: 10, add: [4], remove: [1] },
          { t: 20, tp: 20, add: [5] },
        ],
      },
    });

    expect(completedQuestsAt(session, 0)).toEqual(new Set([1, 2]));
    expect(completedQuestsAt(session, 10)).toEqual(new Set([2, 3, 4]));
    expect(completedQuestsAt(session, 15)).toEqual(new Set([2, 3, 4]));
  });

  it("should fall back to GetQuestsCompleted when the modern API was not recorded", () => {
    const session = withDelta({ GetQuestsCompleted: { t: 0, tp: 0, initial: [7], delta: [] } });

    expect(completedQuestsAt(session, 0)).toEqual(new Set([7]));
  });

  it("should include a change recorded in the same frame as the time asked for", () => {
    const session = withDelta({ [MODERN]: { t: 0, tp: 0, initial: [], delta: [{ t: 5, tp: 5.2, add: [3] }] } });

    expect(completedQuestsAt(session, 5)).toEqual(new Set([3]));
    expect(completedQuestsAt(session, 4.99)).toEqual(new Set());
  });

  it("should use GetQuestsCompleted when the modern API was recorded but stayed empty", () => {
    const session = withDelta({
      [MODERN]: { t: 0, tp: 0, initial: [], delta: [] },
      GetQuestsCompleted: { t: 0, tp: 0, initial: [2], delta: [] },
    });

    expect(completedQuestsAt(session, 0)).toEqual(new Set([2]));
  });

  it("should prefer the modern API when both were recorded", () => {
    const session = withDelta({
      [MODERN]: { t: 0, tp: 0, initial: [1], delta: [] },
      GetQuestsCompleted: { t: 0, tp: 0, initial: [2], delta: [] },
    });

    expect(completedQuestsAt(session, 0)).toEqual(new Set([1]));
  });

  it("should return undefined when completed quests were not recorded, unlike an empty set", () => {
    expect(completedQuestsAt(makeSession({}), 0)).toBeUndefined();
    expect(completedQuestsAt(withDelta({ [MODERN]: { t: 0, tp: 0, initial: [], delta: [] } }), 0)).toEqual(new Set());
  });

  it("should not change the session's stream", () => {
    const stream = { t: 0, tp: 0, initial: [1], delta: [{ t: 1, tp: 1, remove: [1] }] };
    completedQuestsAt(withDelta({ [MODERN]: stream }), 5);

    expect(stream.initial).toEqual([1]);
  });
});
