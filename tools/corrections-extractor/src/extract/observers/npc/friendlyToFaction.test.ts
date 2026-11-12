import { describe, expect, it } from "vitest";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";
import { observeFriendlyToFaction } from "./friendlyToFaction";

const ALLIANCE = { n: 2, 1: "Alliance", 2: "Alliance" };
const HORDE = { n: 2, 1: "Horde", 2: "Horde" };

describe("observeFriendlyToFaction", () => {
  it("should pair a reaction recorded at the same time as the target guid", () => {
    const session = makeSession({
      UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] },
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitReaction: { player: { target: [apiSample(5, 5, 4)] } },
    });

    expect(observeFriendlyToFaction(session)).toEqual([
      {
        entityId: 200001,
        value: { faction: "A", friendly: true, sessionTime: null },
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should use an unchanged reaction when the token change event fired at the guid sample", () => {
    const session = makeSession(
      {
        UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
        UnitGUID: { mouseover: [apiSample(3, 3, creatureGuid(200001)), apiSample(8, 8, creatureGuid(200002))] },
        UnitReaction: { player: { mouseover: [apiSample(3, 3, 2)] } },
      },
      [traceEvent("UPDATE_MOUSEOVER_UNIT", 3, 3), traceEvent("UPDATE_MOUSEOVER_UNIT", 8, 8)],
    );

    expect(observeFriendlyToFaction(session).map((obs) => [obs.entityId, obs.value])).toEqual([
      [200001, { faction: "H", friendly: false, sessionTime: null }],
      [200002, { faction: "H", friendly: false, sessionTime: null }],
    ]);
  });

  it("should not pair a stale reaction with a guid sampled at another event", () => {
    const session = makeSession(
      {
        UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] },
        UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(8, 8, creatureGuid(200002))] },
        UnitReaction: { player: { target: [apiSample(3, 3, 5)] } },
      },
      [traceEvent("PLAYER_TARGET_CHANGED", 3, 3), traceEvent("QUEST_DETAIL", 8, 8)],
    );

    expect(observeFriendlyToFaction(session).map((obs) => obs.entityId)).toEqual([200001]);
  });

  it("should treat neutral as friendly and unfriendly as hostile", () => {
    const session = makeSession({
      UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] },
      UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(5, 5, creatureGuid(200002))] },
      UnitReaction: { player: { target: [apiSample(3, 3, 3), apiSample(5, 5, 4)] } },
    });

    expect(observeFriendlyToFaction(session).map((obs) => [obs.entityId, obs.value.friendly])).toEqual([
      [200001, false],
      [200002, true],
    ]);
  });

  it("should use either unit change event to pair a reaction for both tokens", () => {
    const session = makeSession(
      {
        UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] },
        UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(8, 8, creatureGuid(200002))] },
        UnitReaction: { player: { target: [apiSample(3, 3, 5)] } },
      },
      [traceEvent("UPDATE_MOUSEOVER_UNIT", 8, 8)],
    );

    expect(observeFriendlyToFaction(session).map((obs) => obs.entityId)).toEqual([200001, 200002]);
  });

  it("should record one observation per npc, faction and reaction kind in a session", () => {
    const session = makeSession({
      UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] },
      UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(4, 4), apiSample(5, 5, creatureGuid(200001))] },
      UnitReaction: { player: { target: [apiSample(3, 3, 5), apiSample(5, 5, 6)] } },
    });

    expect(observeFriendlyToFaction(session)).toHaveLength(1);
  });

  it("should record the first server time of the session", () => {
    const session = makeSession({
      GetServerTime: [apiSample(0, 0, 1790000000), apiSample(50, 50, 1790000050)],
      UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] },
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitReaction: { player: { target: [apiSample(5, 5, 4)] } },
    });

    expect(observeFriendlyToFaction(session).map((obs) => obs.value.sessionTime)).toEqual([1790000000]);
  });

  it("should skip encounters without a known player faction", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitReaction: { player: { target: [apiSample(5, 5, 4)] } },
    });

    expect(observeFriendlyToFaction(session)).toEqual([]);
  });
});
