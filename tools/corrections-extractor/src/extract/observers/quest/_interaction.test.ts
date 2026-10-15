import { describe, expect, it } from "vitest";
import { AMBIGUOUS_INTERACTION, questEventLookup } from "./_interaction";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";

describe("questEventLookup", () => {
  it("should return no lookup when the previous event has an indistinguishable timestamp", () => {
    const session = makeSession({}, [
      traceEvent("GOSSIP_SHOW", 3, 3),
      traceEvent("GOSSIP_SHOW", 3, 3),
    ]);

    expect(questEventLookup(session, 1)).toBeUndefined();
  });

  it("should return no lookup when the next event has an indistinguishable timestamp", () => {
    const session = makeSession({}, [
      traceEvent("GOSSIP_SHOW", 3, 3),
      traceEvent("GOSSIP_SHOW", 3, 3),
    ]);

    expect(questEventLookup(session, 0)).toBeUndefined();
  });

  it("should read a parameterized stream", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [apiSample(3, 3.002, creatureGuid(276171))],
        },
      },
      [traceEvent("GOSSIP_SHOW", 3, 3.001)]
    );

    expect(questEventLookup(session, 0)?.("UnitGUID", "npc")).toBe(creatureGuid(276171));
  });

  it("should return undefined for a stream that was never recorded", () => {
    const session = makeSession({}, [traceEvent("GOSSIP_SHOW", 3, 3.001)]);

    const lookup = questEventLookup(session, 0);

    expect(lookup).toBeDefined();
    expect(lookup?.("GetQuestID")).toBeUndefined();
  });

  it("should include synchronous probes recorded after the event in the same frame", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(1, 1, 99196),
          apiSample(3, 3.002, 99197),
        ],
      },
      [traceEvent("QUEST_DETAIL", 3, 3.001)]
    );

    expect(questEventLookup(session, 0)?.("GetQuestID")).toBe(99197);
  });

  it("should exclude probes recorded after the next event in the same frame", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(3, 3.002, 99196),
          apiSample(3, 3.012, 99197),
        ],
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3.001),
        traceEvent("MERCHANT_SHOW", 3, 3.011),
      ]
    );

    expect(questEventLookup(session, 0)?.("GetQuestID")).toBe(99196);
  });

  it("should exclude probes from later frames", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(3, 3.002, 99196),
          apiSample(4, 4.002, 99197),
        ],
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3.001),
        traceEvent("QUEST_DETAIL", 4, 4.001),
      ]
    );

    expect(questEventLookup(session, 0)?.("GetQuestID")).toBe(99196);
  });

  it("should mark a probe that ties with the next event as ambiguous", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(1, 1, 99196),
          apiSample(3, 3.002, 99197),
        ],
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3.001),
        traceEvent("QUEST_FINISHED", 3, 3.002),
      ]
    );

    expect(questEventLookup(session, 0)?.("GetQuestID")).toBe(AMBIGUOUS_INTERACTION);
  });

  it("should keep a recorded nil instead of an older value", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(2, 2),
          ],
        },
      },
      [traceEvent("GOSSIP_SHOW", 3, 3.001)]
    );

    const lookup = questEventLookup(session, 0);

    expect(lookup).toBeDefined();
    expect(lookup?.("UnitGUID", "npc")).toBeUndefined();
  });

  it("should keep an unchanged value from an earlier frame", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [apiSample(1, 1, creatureGuid(276171))],
        },
      },
      [traceEvent("QUEST_TURNED_IN", 60, 60, 99196)]
    );

    expect(questEventLookup(session, 0)?.("UnitGUID", "npc")).toBe(creatureGuid(276171));
  });
});
