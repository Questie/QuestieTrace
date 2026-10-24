import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";
import { mergeFriendlyToFaction, observeFriendlyToFaction, type FactionReactionValue } from "./friendlyToFaction";

const ALLIANCE = { n: 2, 1: "Alliance", 2: "Alliance" };
const HORDE = { n: 2, 1: "Horde", 2: "Horde" };

function vote(entityId: number, sessionKey: number, faction: "A" | "H", friendly: boolean): Observation<FactionReactionValue> {
  return {
    entityId,
    value: { faction, friendly },
    confidence: "high",
    provenance: { session: `s${sessionKey}`, sessionKey, t: 0 },
  };
}

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
        value: { faction: "A", friendly: true },
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
      [200001, { faction: "H", friendly: false }],
      [200002, { faction: "H", friendly: false }],
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

  it("should skip encounters without a known player faction", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitReaction: { player: { target: [apiSample(5, 5, 4)] } },
    });

    expect(observeFriendlyToFaction(session)).toEqual([]);
  });
});

describe("mergeFriendlyToFaction", () => {
  it("should return AH when both factions are friendly in at least two sessions each", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", true), vote(1, 2, "A", true), vote(1, 3, "H", true), vote(1, 4, "H", true)]),
    ).toBe("AH");
  });

  it("should return only the friendly faction when the other is hostile", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", false), vote(1, 2, "A", false), vote(1, 3, "H", true), vote(1, 4, "H", true)]),
    ).toBe("H");
  });

  it("should return undefined when the npc is hostile to both factions", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", false), vote(1, 2, "A", false), vote(1, 3, "H", false), vote(1, 4, "H", false)]),
    ).toBeUndefined();
  });

  it("should return undefined when only one faction was observed", () => {
    expect(mergeFriendlyToFaction([vote(1, 1, "A", true), vote(1, 2, "A", true)])).toBeUndefined();
  });

  it("should return undefined when a faction was observed in only one session", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", true), vote(1, 1, "A", true), vote(1, 3, "H", true), vote(1, 4, "H", true)]),
    ).toBeUndefined();
  });

  it("should return undefined when sessions disagree for a faction", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", true),
        vote(1, 2, "A", true),
        vote(1, 5, "A", false),
        vote(1, 3, "H", true),
        vote(1, 4, "H", true),
      ]),
    ).toBeUndefined();
  });
});
