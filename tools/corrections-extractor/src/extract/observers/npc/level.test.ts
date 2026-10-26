import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";
import { mergeLevelRange, observeLevel } from "./level";

function sighting(sessionKey: number, level: number): Observation<number> {
  return { entityId: 1, value: level, confidence: "high", provenance: { session: `s${sessionKey}`, sessionKey, t: 0 } };
}

describe("observeLevel", () => {
  it("should record the level sampled with the target guid", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitLevel: { target: [apiSample(5, 5, 12)] },
    });

    expect(observeLevel(session)).toEqual([
      { entityId: 200001, value: 12, confidence: "high", provenance: { session: "test-session", t: 5 } },
    ]);
  });

  it("should use an unchanged level when the mouseover event fired at the guid sample", () => {
    const session = makeSession(
      {
        UnitGUID: { mouseover: [apiSample(3, 3, creatureGuid(200001)), apiSample(8, 8, creatureGuid(200002))] },
        UnitLevel: { mouseover: [apiSample(3, 3, 12)] },
      },
      [traceEvent("UPDATE_MOUSEOVER_UNIT", 3, 3), traceEvent("UPDATE_MOUSEOVER_UNIT", 8, 8)],
    );

    expect(observeLevel(session).map((obs) => [obs.entityId, obs.value])).toEqual([
      [200001, 12],
      [200002, 12],
    ]);
  });

  it("should not pair a stale level with a guid sampled at another event", () => {
    const session = makeSession(
      {
        UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(8, 8, creatureGuid(200002))] },
        UnitLevel: { target: [apiSample(3, 3, 12)] },
      },
      [traceEvent("PLAYER_TARGET_CHANGED", 3, 3), traceEvent("QUEST_DETAIL", 8, 8)],
    );

    expect(observeLevel(session).map((obs) => obs.entityId)).toEqual([200001]);
  });

  it("should pair each guid with its own level when the target changed twice at the same time", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5.1, creatureGuid(200001)), apiSample(5, 5.3, creatureGuid(200002))] },
      UnitLevel: { target: [apiSample(5, 5.2, 12), apiSample(5, 5.4, 30)] },
    });

    expect(observeLevel(session).map((obs) => [obs.entityId, obs.value])).toEqual([
      [200001, 12],
      [200002, 30],
    ]);
  });

  it("should skip unknown levels", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitLevel: { target: [apiSample(5, 5, -1)] },
    });

    expect(observeLevel(session)).toEqual([]);
  });

  it("should record each level of an npc once per session", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(4, 4), apiSample(5, 5, creatureGuid(200001))] },
      UnitLevel: { target: [apiSample(3, 3, 12), apiSample(4, 4, 30), apiSample(5, 5, 12)] },
    });

    expect(observeLevel(session)).toHaveLength(1);
  });
});

describe("mergeLevelRange", () => {
  it("should return the lowest and highest level", () => {
    expect(mergeLevelRange([sighting(1, 12), sighting(2, 12), sighting(3, 13), sighting(4, 13)])).toEqual({ min: 12, max: 13 });
  });

  it("should ignore levels seen in only one session", () => {
    expect(mergeLevelRange([sighting(1, 12), sighting(2, 12), sighting(3, 20)])).toEqual({ min: 12, max: 12 });
  });

  it("should ignore levels seen far less often than the most common level", () => {
    const observations = Array.from({ length: 30 }, (_, index) => sighting(index, 23));
    observations.push(sighting(100, 22), sighting(101, 22));
    expect(mergeLevelRange(observations)).toEqual({ min: 23, max: 23 });
  });

  it("should count one session once per level", () => {
    expect(mergeLevelRange([sighting(1, 12), sighting(1, 12)])).toBeUndefined();
  });
});
