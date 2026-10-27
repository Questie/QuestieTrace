import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";
import { mergeRank, observeRank } from "./rank";

function vote(sessionKey: number, rank: number): Observation<number> {
  return { entityId: 1, value: rank, confidence: "high", provenance: { session: `s${sessionKey}`, sessionKey, t: 0 } };
}

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

describe("mergeRank", () => {
  it("should return the rank seen in the most sessions", () => {
    expect(mergeRank([vote(1, 1), vote(2, 1), vote(3, 4)])).toBe(1);
  });

  it("should count several observations in one session once", () => {
    expect(mergeRank([vote(1, 4), vote(1, 4), vote(1, 4), vote(2, 1), vote(3, 1)])).toBe(1);
  });

  it("should return undefined when the most common ranks tie", () => {
    expect(mergeRank([vote(1, 1), vote(2, 1), vote(3, 4), vote(4, 4)])).toBeUndefined();
  });

  it("should return undefined when the rank was seen in only one session", () => {
    expect(mergeRank([vote(1, 1), vote(1, 1)])).toBeUndefined();
  });

  it("should return undefined for rank 0", () => {
    expect(mergeRank([vote(1, 0), vote(2, 0)])).toBeUndefined();
  });
});
