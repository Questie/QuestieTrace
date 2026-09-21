import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../core/types";
import {
  playerPosAt,
  playerPosNear,
  playerZoneAt,
  playerZoneNear,
  questIdAt,
  questTitleAt,
  unitGuidAt,
  unitLevelAt,
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
  it("should read the player's zone at or before a given time", () => {
    const session = makeSession({
      "C_Map.GetBestMapForUnit": {
        player: [
          { t: 0, tp: 0, v: 1429 },
          { t: 10, tp: 10, v: 40 },
        ],
      },
    });
    expect(playerZoneAt(session, 5)).toBe(1429);
    expect(playerZoneAt(session, 10)).toBe(40);
  });

  it("should find the player's zone nearest a given time within the window", () => {
    const session = makeSession({
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 20, tp: 20, v: 12 }],
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

  it("should read a UnitLevel stream, normalizing 0/negative to null", () => {
    const session = makeSession({
      UnitLevel: { target: [{ t: 0, tp: 0, v: 5 }] },
    });
    expect(unitLevelAt(session, "target", 5)).toBe(5);
    expect(unitLevelAt(session, "missing", 5)).toBeNull();
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
