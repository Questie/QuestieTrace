import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";
import { mergeFriendlyToFaction, observeFriendlyToFaction, type FactionReactionValue } from "./friendlyToFaction";

const ALLIANCE = { n: 2, 1: "Alliance", 2: "Alliance" };
const HORDE = { n: 2, 1: "Horde", 2: "Horde" };

function vote(
  entityId: number,
  sessionKey: number,
  faction: "A" | "H",
  friendly: boolean,
  sessionTime: number | null = sessionKey * 1000,
): Observation<FactionReactionValue> {
  return {
    entityId,
    value: { faction, friendly, sessionTime },
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

  it("should use the newer reaction when sessions switch once over time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false),
        vote(1, 2, "A", true),
        vote(1, 3, "A", true),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBe("AH");
  });

  it("should decide without server times when all sessions agree", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", true, null),
        vote(1, 2, "A", true, null),
        vote(1, 3, "H", true, null),
        vote(1, 4, "H", true, null),
      ]),
    ).toBe("AH");
  });

  it("should return undefined when the switch happens between sessions with the same server time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false, 1000),
        vote(1, 2, "A", true, 1000),
        vote(1, 3, "A", true, 2000),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should order sessions by server time rather than by session key", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "H", true, 4000),
        vote(1, 2, "H", true, 3000),
        vote(1, 3, "H", false, 2000),
        vote(1, 4, "H", false, 1000),
        vote(1, 5, "A", true),
        vote(1, 6, "A", true),
      ]),
    ).toBe("AH");
  });

  it("should return undefined when the newer reaction was seen in only one session", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false),
        vote(1, 2, "A", false),
        vote(1, 3, "A", true),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should return undefined when sessions switch more than once over time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", true),
        vote(1, 2, "A", false),
        vote(1, 3, "A", true),
        vote(1, 4, "A", true),
        vote(1, 5, "H", true),
        vote(1, 6, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should return undefined when disagreeing sessions have no server time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false, null),
        vote(1, 2, "A", true),
        vote(1, 3, "A", true),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should return undefined when one session saw both reactions", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false),
        vote(1, 2, "A", true),
        vote(1, 2, "A", false),
        vote(1, 3, "A", true),
        vote(1, 4, "A", true),
        vote(1, 5, "H", true),
        vote(1, 6, "H", true),
      ]),
    ).toBeUndefined();
  });
});
