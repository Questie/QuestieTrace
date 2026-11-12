import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import type { ReputationTurnInValue } from "../../observers/quest/reputationReward";
import { mergeReputationReward } from "./reputationReward";

function vote(
  sessionKey: number,
  playerFaction: "A" | "H",
  gains: Record<number, number>,
  spillover: Record<number, number> = {},
): Observation<ReputationTurnInValue> {
  return {
    entityId: 1,
    value: { playerFaction, gains, spillover },
    confidence: "high",
    provenance: { session: `s${sessionKey}`, sessionKey: `k${sessionKey}`, t: 0 },
  };
}

describe("mergeReputationReward", () => {
  it("should return the reward agreed on by at least two sessions", () => {
    expect(mergeReputationReward([vote(1, "H", { 68: 100 }), vote(2, "H", { 68: 100 })])).toEqual([[68, 100]]);
  });

  it("should return undefined when only one session was observed", () => {
    expect(mergeReputationReward([vote(1, "H", { 68: 100 }), vote(1, "H", { 68: 100 })])).toBeUndefined();
  });

  it("should tolerate a minority of bonus amounts", () => {
    const observations = [1, 2, 3, 4].map((key) => vote(key, "H", { 68: 100 }));
    observations.push(vote(5, "H", { 68: 110 }));
    expect(mergeReputationReward(observations)).toEqual([[68, 100]]);
  });

  it("should return undefined when amounts disagree too often", () => {
    expect(
      mergeReputationReward([vote(1, "H", { 68: 100 }), vote(2, "H", { 68: 100 }), vote(3, "H", { 68: 50 })]),
    ).toBeUndefined();
  });

  it("should drop a faction that shows up in less than half of the turn-ins", () => {
    expect(
      mergeReputationReward([
        vote(1, "H", { 68: 100, 87: 5 }),
        vote(2, "H", { 68: 100, 87: 5 }),
        vote(3, "H", { 68: 100 }),
        vote(4, "H", { 68: 100 }),
        vote(5, "H", { 68: 100 }),
      ]),
    ).toEqual([[68, 100]]);
  });

  it("should return undefined when alliance and horde rewards differ", () => {
    expect(
      mergeReputationReward([
        vote(1, "A", { 2778: 50 }),
        vote(2, "A", { 2778: 50 }),
        vote(3, "H", { 2779: 50 }),
        vote(4, "H", { 2779: 50 }),
      ]),
    ).toBeUndefined();
  });

  it("should return the shared reward when alliance and horde agree", () => {
    expect(
      mergeReputationReward([vote(1, "A", { 59: 75 }), vote(2, "A", { 59: 75 }), vote(3, "H", { 59: 75 }), vote(4, "H", { 59: 75 })]),
    ).toEqual([[59, 75]]);
  });

  it("should add spillover as an unrounded quarter of its decided source reward", () => {
    expect(
      mergeReputationReward([
        vote(1, "A", { 69: 75 }, { 47: 69, 54: 69 }),
        vote(2, "A", { 69: 75 }, { 47: 69, 54: 69 }),
        vote(3, "A", { 69: 75 }, { 47: 69 }),
      ]),
    ).toEqual([
      [47, 18.75],
      [54, 18.75],
      [69, 75],
    ]);
  });

  it("should keep the sign of negative spillover", () => {
    expect(mergeReputationReward([vote(1, "H", { 87: -150 }, { 21: 87 }), vote(2, "H", { 87: -150 }, { 21: 87 })])).toEqual([
      [21, -37.5],
      [87, -150],
    ]);
  });

  it("should prefer a decided base reward over spillover for the same faction", () => {
    expect(
      mergeReputationReward([
        vote(1, "A", { 69: 100, 47: 25 }),
        vote(2, "A", { 69: 100, 47: 25 }),
        vote(3, "A", { 69: 100 }, { 47: 69 }),
      ]),
    ).toEqual([
      [47, 25],
      [69, 100],
    ]);
  });

  it("should drop spillover seen in only one session", () => {
    expect(mergeReputationReward([vote(1, "A", { 69: 100 }, { 47: 69 }), vote(2, "A", { 69: 100 })])).toEqual([[69, 100]]);
  });

  it("should drop spillover whose source reward was not decided", () => {
    expect(
      mergeReputationReward([vote(1, "A", { 69: 100 }, { 47: 69 }), vote(2, "A", { 69: 50 }, { 47: 69 })]),
    ).toBeUndefined();
  });

  it("should sort rewards by faction id", () => {
    expect(mergeReputationReward([vote(1, "H", { 87: -25, 68: 100 }), vote(2, "H", { 87: -25, 68: 100 })])).toEqual([
      [68, 100],
      [87, -25],
    ]);
  });
});
