import { describe, expect, it } from "vitest";
import { apiSample, makeSession, traceEvent } from "../../../test/fixtures";
import { observeQuestFactions } from "./requiredRaces";

const ALLIANCE = { n: 2, 1: "Alliance", 2: "Alliance" };
const HORDE = { n: 2, 1: "Horde", 2: "Horde" };
const NEUTRAL = { n: 2, 1: "Neutral", 2: "Neutral" };

describe("observeQuestFactions", () => {
  it("should record the player's faction for an accepted quest", () => {
    const session = makeSession(
      { UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] } },
      [traceEvent("QUEST_ACCEPTED", 5, 5, 99001)],
    );

    expect(observeQuestFactions(session).map((obs) => [obs.entityId, obs.value])).toEqual([[99001, "A"]]);
  });

  it("should record quests shown in a quest frame and in gossip lists", () => {
    const session = makeSession({
      UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
      GetQuestID: [apiSample(2, 2, 99001)],
      "C_GossipInfo.GetAvailableQuests": [apiSample(3, 3, [{ questID: 99002 }])],
      "C_GossipInfo.GetActiveQuests": [apiSample(4, 4, [{ questID: 99003 }])],
    });

    expect(observeQuestFactions(session).map((obs) => [obs.entityId, obs.value])).toEqual([
      [99001, "H"],
      [99002, "H"],
      [99003, "H"],
    ]);
  });

  it("should record quests that are in the quest log", () => {
    const session = makeSession({
      UnitFactionGroup: { player: [apiSample(0, 0, HORDE)] },
      QuestLog: [apiSample(0, 0, [99001, 99002]), apiSample(5, 5, [99002])],
    });

    expect(observeQuestFactions(session).map((obs) => [obs.entityId, obs.value])).toEqual([
      [99001, "H"],
      [99002, "H"],
    ]);
  });

  it("should use the first known faction for sightings before the first faction sample", () => {
    const session = makeSession({
      UnitFactionGroup: { player: [apiSample(5, 5, HORDE)] },
      QuestLog: [apiSample(0, 0, [99001])],
    });

    expect(observeQuestFactions(session).map((obs) => [obs.entityId, obs.value])).toEqual([[99001, "H"]]);
  });

  it("should record each quest only once per session", () => {
    const session = makeSession(
      { UnitFactionGroup: { player: [apiSample(0, 0, ALLIANCE)] }, GetQuestID: [apiSample(2, 2, 99001)] },
      [traceEvent("QUEST_ACCEPTED", 5, 5, 99001)],
    );

    expect(observeQuestFactions(session)).toHaveLength(1);
  });

  it("should ignore unfactioned players", () => {
    const session = makeSession(
      { UnitFactionGroup: { player: [apiSample(0, 0, NEUTRAL)] } },
      [traceEvent("QUEST_ACCEPTED", 5, 5, 99001)],
    );

    expect(observeQuestFactions(session)).toEqual([]);
  });
});
