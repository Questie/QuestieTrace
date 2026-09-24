import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeSpawns, type SpawnObservationValue } from "./spawns";

function makeSession(functions: SessionRecord["functions"], events: SessionRecord["events"] = []): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events,
    functions,
    functionsDelta: {},
  };
}

describe("observeSpawns", () => {
  it("should convert normalized map coordinates to Questie percentages", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [
          { t: 10, tp: 10, v: "Creature-0-5208-0-7-823-000031" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 10, tp: 10, v: 40 }],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 10, tp: 10, v: { x: 0.5611, y: 0.6164 } }],
      },
    });

    expect(observeSpawns(session)).toEqual([
      {
        entityId: 823,
        value: { zoneID: 40, x: 56.11, y: 61.64 },
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
        token: "npc",
      },
    ]);
  });

  it("should skip encounters where player zone is unknown", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [
          { t: 10, tp: 10, v: "Creature-0-5208-0-7-823-000031" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 10, tp: 10, v: { x: 0.3001, y: 0.8602 } }],
      },
    });

    expect(observeSpawns(session)).toEqual([]);
  });

  it("should skip encounters where player position is unknown", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [
          { t: 10, tp: 10, v: "Creature-0-5208-0-7-823-000031" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 10, tp: 10, v: 40 }],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [],
      },
    });

    expect(observeSpawns(session)).toEqual([]);
  });

  it("should emit multiple observations for the same npc in different zones", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [
          { t: 10, tp: 10, v: "Creature-0-5208-0-7-823-000031" },
          { t: 20, tp: 20, v: "Creature-0-5208-0-7-823-000031" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [
          { t: 10, tp: 10, v: 40 },
          { t: 20, tp: 20, v: 12 },
        ],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [
          { t: 10, tp: 10, v: { x: 0.3001, y: 0.8602 } },
          { t: 20, tp: 20, v: { x: 0.4746, y: 0.6218 } },
        ],
      },
    });

    expect(observeSpawns(session)).toEqual([
      {
        entityId: 823,
        value: { zoneID: 40, x: 30.01, y: 86.02 },
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
        token: "npc",
      },
      {
        entityId: 823,
        value: { zoneID: 12, x: 47.46, y: 62.18 },
        confidence: "high",
        provenance: { session: "test-session", t: 20 },
        token: "npc",
      },
    ]);
  });

  it("should return empty array when no npc encounters exist", () => {
    const session = makeSession({
      UnitGUID: {},
      "C_Map.GetBestMapForUnit": {},
      "C_Map.GetPlayerMapPosition": {},
    });

    expect(observeSpawns(session)).toEqual([]);
  });

  it("should use questnpc token encounters too", () => {
    const session = makeSession({
      UnitGUID: {
        questnpc: [
          { t: 5, tp: 5, v: "Creature-0-5208-0-7-197-000032" },
        ],
      },
      "C_Map.GetBestMapForUnit": {
        player: [{ t: 5, tp: 5, v: 12 }],
      },
      "C_Map.GetPlayerMapPosition": {
        player: [{ t: 5, tp: 5, v: { x: 0.4746, y: 0.6218 } }],
      },
    });

    expect(observeSpawns(session)).toEqual([
      {
        entityId: 197,
        value: { zoneID: 12, x: 47.46, y: 62.18 },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
        token: "questnpc",
      },
    ]);
  });
});
