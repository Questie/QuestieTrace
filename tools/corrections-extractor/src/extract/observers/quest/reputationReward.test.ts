import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { apiSample, makeSession, traceEvent } from "../../../test/fixtures";
import {
  mergeReputationReward,
  observeReputationReward,
  splitSpillover,
  type ReputationTurnInValue,
} from "./reputationReward";

const ORC = { n: 3, 1: "Orc", 2: "Orc", 3: 2 };
const HUMAN = { n: 3, 1: "Human", 2: "Human", 3: 1 };
const HORDE = { n: 2, 1: "Horde", 2: "Horde" };

function session(events: ReturnType<typeof traceEvent>[], race: unknown = ORC, locale = "enUS") {
  const raceSamples = race === null ? [] : [apiSample(0, 0, race)];
  return makeSession(
    {
      GetLocale: { player: [apiSample(0, 0, locale)] },
      UnitRace: { player: raceSamples },
      UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
      "C_Reputation.GetFactionDataByID": {
        68: [apiSample(0, 0, { name: "Undercity", factionID: 68 })],
        76: [apiSample(0, 0, { name: "Orgrimmar", factionID: 76 })],
        87: [apiSample(0, 0, { name: "Bloodsail Buccaneers", factionID: 87 })],
      },
    },
    events,
  );
}

function message(t: number, text: string) {
  return traceEvent("CHAT_MSG_COMBAT_FACTION_CHANGE", t, t, text);
}

function turnIn(t: number, questID: number) {
  return traceEvent("QUEST_TURNED_IN", t, t, questID, 0, 0);
}

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
    provenance: { session: `s${sessionKey}`, sessionKey, t: 0 },
  };
}

describe("observeReputationReward", () => {
  it("should record reputation messages around a quest turn-in", () => {
    const observations = observeReputationReward(
      session([
        message(9.9, "Reputation with Undercity increased by 100."),
        turnIn(10, 99001),
        message(10.1, "Reputation with Bloodsail Buccaneers decreased by 25."),
      ]),
    );

    expect(observations).toEqual([
      {
        entityId: 99001,
        value: { playerFaction: "H", gains: { 68: 100, 87: -25 }, spillover: {} },
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
      },
    ]);
  });

  it("should split off spillover gains", () => {
    const observations = observeReputationReward(
      session([
        turnIn(10, 99001),
        message(10, "Reputation with Undercity increased by 100."),
        message(10, "Reputation with Orgrimmar increased by 26."),
      ]),
    );

    expect(observations.map((obs) => obs.value)).toEqual([{ playerFaction: "H", gains: { 68: 100 }, spillover: { 76: 68 } }]);
  });

  it("should ignore messages outside the turn-in window", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10.8, "Reputation with Undercity increased by 100.")]),
    );

    expect(observations).toEqual([]);
  });

  it("should skip turn-ins close to another turn-in", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10, "Reputation with Undercity increased by 100."), turnIn(10.6, 99002)]),
    );

    expect(observations).toEqual([]);
  });

  it("should skip turn-ins with a faction name missing from the reputation streams", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10, "Reputation with Unknown Faction increased by 100.")]),
    );

    expect(observations).toEqual([]);
  });

  it("should skip human players", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10, "Reputation with Undercity increased by 110.")], HUMAN),
    );

    expect(observations).toEqual([]);
  });

  it("should skip sessions without a known race", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10, "Reputation with Undercity increased by 100.")], null),
    );

    expect(observations).toEqual([]);
  });

  it("should skip non-enUS sessions", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10, "Reputation with Undercity increased by 100.")], ORC, "deDE"),
    );

    expect(observations).toEqual([]);
  });
});

describe("splitSpillover", () => {
  it("should keep gains that are not a quarter of another gain", () => {
    expect(splitSpillover({ 68: 100, 76: 50 })).toEqual({ gains: { 68: 100, 76: 50 }, spillover: {} });
  });

  it("should split off gains rounded either way from a quarter of another gain", () => {
    expect(splitSpillover({ 69: 75, 47: 18, 54: 19 })).toEqual({ gains: { 69: 75 }, spillover: { 47: 69, 54: 69 } });
  });

  it("should pick the largest matching gain as the spillover source", () => {
    expect(splitSpillover({ 68: 96, 69: 100, 47: 25 })).toEqual({ gains: { 68: 96, 69: 100 }, spillover: { 47: 69 } });
  });

  it("should not treat a gain in the other direction as spillover", () => {
    expect(splitSpillover({ 68: 100, 87: -25 })).toEqual({ gains: { 68: 100, 87: -25 }, spillover: {} });
  });
});

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

  it("should add spillover as a rounded quarter of its decided source reward", () => {
    expect(
      mergeReputationReward([
        vote(1, "A", { 69: 75 }, { 47: 69, 54: 69 }),
        vote(2, "A", { 69: 75 }, { 47: 69, 54: 69 }),
        vote(3, "A", { 69: 75 }, { 47: 69 }),
      ]),
    ).toEqual([
      [47, 19],
      [54, 19],
      [69, 75],
    ]);
  });

  it("should keep the sign of negative spillover", () => {
    expect(mergeReputationReward([vote(1, "H", { 87: -150 }, { 21: 87 }), vote(2, "H", { 87: -150 }, { 21: 87 })])).toEqual([
      [21, -38],
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
