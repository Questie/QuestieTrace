import { describe, expect, it } from "vitest";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";
import { observeRank } from "./rank";

describe("observeRank", () => {
  it.each([
    ["normal", 0],
    ["trivial", 0],
    ["minus", 0],
    ["elite", 1],
    ["rareelite", 2],
    ["worldboss", 3],
    ["rare", 4],
  ])("should map %s to rank %i", (classification, rank) => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitClassification: { target: [apiSample(5, 5, classification)] },
    });

    expect(observeRank(session)).toEqual([
      { entityId: 200001, value: rank, confidence: "high", provenance: { session: "test-session", t: 5 } },
    ]);
  });

  it("should skip unknown classifications", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(5, 5, creatureGuid(200001))] },
      UnitClassification: { target: [apiSample(5, 5, "unknown")] },
    });

    expect(observeRank(session)).toEqual([]);
  });

  it("should not pair a stale classification with a guid sampled at another event", () => {
    const session = makeSession(
      {
        UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(8, 8, creatureGuid(200002))] },
        UnitClassification: { target: [apiSample(3, 3, "elite")] },
      },
      [traceEvent("PLAYER_TARGET_CHANGED", 3, 3), traceEvent("QUEST_DETAIL", 8, 8)],
    );

    expect(observeRank(session).map((obs) => obs.entityId)).toEqual([200001]);
  });

  it("should record one observation per npc and rank in a session", () => {
    const session = makeSession({
      UnitGUID: { target: [apiSample(3, 3, creatureGuid(200001)), apiSample(4, 4), apiSample(5, 5, creatureGuid(200001))] },
      UnitClassification: { target: [apiSample(3, 3, "elite"), apiSample(4, 4, "normal"), apiSample(5, 5, "elite")] },
    });

    expect(observeRank(session).map((obs) => [obs.entityId, obs.value])).toEqual([[200001, 1]]);
  });
});
