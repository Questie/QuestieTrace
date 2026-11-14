import { describe, expect, it } from "vitest";
import { apiSample, makeSession, traceEvent } from "../../../test/fixtures";
import { observeReputationReward, splitSpillover } from "./reputationReward";

const ORC = { n: 3, 1: "Orc", 2: "Orc", 3: 2 };
const HUMAN = { n: 3, 1: "Human", 2: "Human", 3: 1 };
const HORDE = { n: 2, 1: "Horde", 2: "Horde" };

type Standing = [t: number, value: number];

const FACTION_NAMES: Record<number, string> = { 68: "Undercity", 76: "Orgrimmar", 87: "Bloodsail Buccaneers", 529: "Argent Dawn" };

function session(
  events: ReturnType<typeof traceEvent>[],
  standings: Record<number, Standing[]>,
  race: unknown = ORC,
  locale = "enUS",
) {
  const raceSamples = race === null ? [] : [apiSample(0, 0, race)];
  const factions: Record<number, ReturnType<typeof apiSample>[]> = {};
  for (const [factionID, samples] of Object.entries(standings)) {
    factions[Number(factionID)] = samples.map(([t, value]) => apiSample(t, t, { name: FACTION_NAMES[Number(factionID)], factionID: Number(factionID), currentStanding: value }));
  }
  return makeSession(
    {
      GetLocale: { player: [apiSample(0, 0, locale)] },
      UnitRace: { player: raceSamples },
      UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
      "C_Reputation.GetFactionDataByID": factions,
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

describe("observeReputationReward", () => {
  it("should record reputation changes around a quest turn-in", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001)], {
        68: [[0, 3000], [9.9, 3100]],
        76: [[0, 5000]],
        87: [[0, 400], [10.1, 375]],
      }),
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

  it("should read the bar value of the legacy faction api", () => {
    const legacy = (t: number, barValue: number) =>
      apiSample(t, t, { n: 14, 1: "Undercity", 3: 5, 4: 3000, 5: 9000, 6: barValue, 14: 68 });
    const observations = observeReputationReward(
      makeSession(
        {
          UnitRace: { player: [apiSample(0, 0, ORC)] },
          UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
          GetFactionInfoByID: { 68: [legacy(0, 3000), legacy(10, 3250)] },
        },
        [turnIn(10, 99001)],
      ),
    );

    expect(observations.map((obs) => obs.value)).toEqual([{ playerFaction: "H", gains: { 68: 250 }, spillover: {} }]);
  });

  it("should fall back to the legacy faction api when the modern samples have no standing", () => {
    const legacy = (t: number, barValue: number) =>
      apiSample(t, t, { n: 14, 1: "Unterstadt", 3: 5, 4: 3000, 5: 9000, 6: barValue, 14: 68 });
    const observations = observeReputationReward(
      makeSession(
        {
          GetLocale: { player: [apiSample(0, 0, "deDE")] },
          UnitRace: { player: [apiSample(0, 0, ORC)] },
          UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
          "C_Reputation.GetFactionDataByID": { 68: [apiSample(0, 0, null)] },
          GetFactionInfoByID: { 68: [legacy(0, 3000), legacy(10, 3250)] },
        },
        [turnIn(10, 99001), message(10, "Euer Ruf bei der Fraktion Unterstadt hat sich um 250 verbessert.")],
      ),
    );

    expect(observations.map((obs) => obs.value)).toEqual([{ playerFaction: "H", gains: { 68: 250 }, spillover: {} }]);
  });

  it("should split off spillover gains", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001)], {
        68: [[0, 3000], [10, 3100]],
        76: [[0, 5000], [10, 5026]],
      }),
    );

    expect(observations.map((obs) => obs.value)).toEqual([{ playerFaction: "H", gains: { 68: 100 }, spillover: { 76: 68 } }]);
  });

  it("should ignore changes outside the turn-in window", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001)], { 68: [[0, 3000], [9.4, 3050], [10.8, 3100]] }),
    );

    expect(observations).toEqual([]);
  });

  it("should skip turn-ins close to another turn-in", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), turnIn(10.6, 99002)], { 68: [[0, 3000], [10, 3100]] }),
    );

    expect(observations).toEqual([]);
  });

  it("should use the chat messages when a faction is first seen in the turn-in window", () => {
    const observations = observeReputationReward(
      session(
        [
          message(9.9, "Reputation with Argent Dawn increased by 150."),
          turnIn(10, 99001),
          message(10.1, "Reputation with Bloodsail Buccaneers decreased by 25."),
        ],
        {
          87: [[0, 400], [10.1, 375]],
          529: [[10, 350]],
        },
      ),
    );

    expect(observations.map((obs) => obs.value)).toEqual([
      { playerFaction: "H", gains: { 87: -25, 529: 150 }, spillover: {} },
    ]);
  });

  it("should use the chat messages when a faction gaining reputation has no reputation stream yet", () => {
    const observations = observeReputationReward(
      session(
        [
          turnIn(10, 99001),
          message(10.1, "Reputation with Undercity increased by 50."),
          message(10.1, "Reputation with Argent Dawn increased by 100."),
        ],
        {
          68: [[0, 3000], [10.1, 3050]],
          529: [[400, 100]],
        },
      ),
    );

    expect(observations.map((obs) => obs.value)).toEqual([
      { playerFaction: "H", gains: { 68: 50, 529: 100 }, spillover: {} },
    ]);
  });

  it("should skip turn-ins with more reputation messages than changes outside enUS sessions", () => {
    const observations = observeReputationReward(
      session(
        [
          turnIn(10, 99001),
          message(10.1, "Euer Ruf bei der Fraktion Unterstadt hat sich um 50 verbessert."),
          message(10.1, "Euer Ruf bei der Fraktion Der Irdene Ring hat sich um 100 verbessert."),
        ],
        { 68: [[0, 3000], [10.1, 3050]] },
        ORC,
        "deDE",
      ),
    );

    expect(observations).toEqual([]);
  });

  it("should not count reputation messages without an amount as changes", () => {
    const observations = observeReputationReward(
      session(
        [
          turnIn(10, 99001),
          message(10.1, "Euer Ruf bei der Fraktion Unterstadt hat sich um 50 verbessert."),
          message(10.1, "Ihr seid jetzt Freundlich mit der Fraktion Unterstadt."),
        ],
        { 68: [[0, 2950], [10.1, 3000]] },
        ORC,
        "deDE",
      ),
    );

    expect(observations.map((obs) => obs.value)).toEqual([{ playerFaction: "H", gains: { 68: 50 }, spillover: {} }]);
  });

  it("should skip turn-ins with a faction first seen in the turn-in window outside enUS sessions", () => {
    const observations = observeReputationReward(
      session(
        [turnIn(10, 99001), message(10, "Euer Ruf bei der Fraktion Argentumdämmerung hat sich um 150 verbessert.")],
        { 529: [[10, 350]] },
        ORC,
        "deDE",
      ),
    );

    expect(observations).toEqual([]);
  });

  it("should skip turn-ins with a chat faction name missing from the reputation streams", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001), message(10, "Reputation with Unknown Faction increased by 100.")], {
        529: [[10, 350]],
      }),
    );

    expect(observations).toEqual([]);
  });

  it("should skip human players", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001)], { 68: [[0, 3000], [10, 3110]] }, HUMAN),
    );

    expect(observations).toEqual([]);
  });

  it("should skip sessions without a known race", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001)], { 68: [[0, 3000], [10, 3100]] }, null),
    );

    expect(observations).toEqual([]);
  });

  it("should record turn-ins of non-enUS sessions", () => {
    const observations = observeReputationReward(
      session([turnIn(10, 99001)], { 68: [[0, 3000], [10, 3100]] }, ORC, "deDE"),
    );

    expect(observations.map((obs) => obs.value)).toEqual([{ playerFaction: "H", gains: { 68: 100 }, spillover: {} }]);
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
