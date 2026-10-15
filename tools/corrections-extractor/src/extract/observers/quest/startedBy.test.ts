import { describe, expect, it } from "vitest";
import { observeStartedBy, mergeStartedBy } from "./startedBy";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";

describe("observeStartedBy", () => {
  it("should capture a creature starter from questnpc GUID at QUEST_DETAIL time", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 0),
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture a creature starter from npc GUID when questnpc is absent", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          npc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 0),
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture an item starter from QUEST_DETAIL questStartItemID", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 750),
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { itemId: 750 },
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture both creature and item starters for the same quest", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 750),
      ]
    );

    const results = observeStartedBy(session);
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.value)).toContainEqual({ creatureId: 197 });
    expect(results.map((r) => r.value)).toContainEqual({ itemId: 750 });
  });

  it("should read the quest ID from the one-argument QUEST_ACCEPTED form", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [traceEvent("QUEST_ACCEPTED", 3, 3, 96659)]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should read the quest ID from argument 2 for the two-argument QUEST_ACCEPTED form", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 11111)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [traceEvent("QUEST_ACCEPTED", 3, 3, 11111, 96659)]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it.each([{ 1: 0, n: 1 }, { 1: "96659", n: 1 }, { n: 0 }])(
    "should ignore invalid QUEST_ACCEPTED payload %#",
    (args) => {
      const session = makeSession(
        {
          GetQuestID: [apiSample(3, 3, 11111)],
          UnitGUID: {
            questnpc: [apiSample(3, 3, creatureGuid(197))],
          },
        },
        [{ t: 3, tp: 3, e: "QUEST_ACCEPTED", a: args }]
      );

      expect(observeStartedBy(session)).toEqual([]);
    }
  );

  it("should capture multiple creature starters across different quest events", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(3, 3, 96659),
          apiSample(10, 10, 12345),
        ],
        UnitGUID: {
          questnpc: [
            apiSample(3, 3, creatureGuid(197)),
            apiSample(10, 10, "Creature-0-5208-0-7-200-000040"),
          ],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 12345),
        traceEvent("QUEST_DETAIL", 10, 10, 67890),
      ]
    );

    const results = observeStartedBy(session);
    expect(results).toHaveLength(4);
    // Each quest event produces both a creature observation and an item observation
    expect(results.filter((r) => r.value.creatureId).map((r) => r.entityId)).toEqual([96659, 12345]);
    expect(results.filter((r) => r.value.creatureId).map((r) => r.value.creatureId)).toEqual([197, 200]);
    expect(results.filter((r) => r.value.itemId).map((r) => r.value.itemId)).toEqual([12345, 67890]);
  });

  it("should ignore target GUID (only questnpc and npc are used)", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          target: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 0),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should ignore GameObject GUID on target token (only questnpc/npc used)", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          target: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 0),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should attribute correctly even when the GUID sample is much older than the quest event (long dialog read)", () => {
    // questnpc/npc streams are only re-written on change (see UnitInteraction.lua dedup), so a player
    // reading gossip/quest text for a long time before accepting leaves the last GUID sample far in the
    // past relative to the anchor event - this must not be treated as stale.
    const session = makeSession(
      {
        GetQuestID: [apiSample(60, 60, 96659)],
        UnitGUID: {
          questnpc: [apiSample(1, 1, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 60, 60, 0),
      ]
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 60 },
      },
    ]);
  });

  it("should ignore player GUIDs seen on quest tokens", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, "Player-5284-0362")],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 0),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should return no observations when no quest-start events exist", () => {
    const session = makeSession({
      GetQuestID: [apiSample(3, 3, 96659)],
      UnitGUID: {
        questnpc: [apiSample(3, 3, creatureGuid(197))],
      },
    });

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should return no observations when GetQuestID is not active at event time", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 12345),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should capture a creature starter from C_GossipInfo.GetAvailableQuests snapshot", () => {
    const session = makeSession(
      {
        "C_GossipInfo.GetAvailableQuests": [
          apiSample(3, 3, [
            { questID: 99196, title: "A Donation of Wool", repeatable: false, isLegendary: false },
          ]),
        ],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(276171))],
        },
      },
      []
    );

    expect(observeStartedBy(session)).toEqual([
      {
        entityId: 99196,
        value: { creatureId: 276171 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture multiple quests from a single gossip available quests snapshot", () => {
    const session = makeSession(
      {
        "C_GossipInfo.GetAvailableQuests": [
          apiSample(3, 3, [
            { questID: 99196, title: "A Donation of Wool", repeatable: false },
            { questID: 99197, title: "Another Quest", repeatable: false },
          ]),
        ],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(276171))],
        },
      },
      []
    );

    const results = observeStartedBy(session);
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.entityId).sort()).toEqual([99196, 99197]);
    expect(results.map((r) => r.value.creatureId)).toEqual([276171, 276171]);
  });

  it("should ignore gossip quest entries without valid questID", () => {
    const session = makeSession(
      {
        "C_GossipInfo.GetAvailableQuests": [
          apiSample(3, 3, [
            { title: "No ID here", repeatable: false },
            { questID: 0, title: "Zero ID", repeatable: false },
          ]),
        ],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(276171))],
        },
      },
      []
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should produce observations from both QUEST_DETAIL and gossip for the same quest", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        "C_GossipInfo.GetAvailableQuests": [
          apiSample(3, 3, [{ questID: 96659, title: "From Gossip", repeatable: false }]),
        ],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3, 0),
      ]
    );

    // Both sources produce observations; mergeStartedBy unions them by type
    const results = observeStartedBy(session);
    const creatures = results.filter((r) => r.value.creatureId);
    expect(creatures).toHaveLength(2); // one from QUEST_DETAIL, one from gossip
    // mergeStartedBy will deduplicate the creature IDs
    const merged = mergeStartedBy(results);
    expect(merged.creatures).toEqual([197]);
    expect(merged.creatures).toHaveLength(1);
  });

  it("should merge duplicate creature starters via mergeStartedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 10 } },
      { entityId: 96659, value: { creatureId: 200 }, confidence: "medium" as const, provenance: { session: "s", t: 15 } },
    ];

    const result = mergeStartedBy(obs);
    expect(result.creatures).toEqual(expect.arrayContaining([197, 200]));
    expect(result.creatures).toHaveLength(2);
    expect(result.objects).toEqual([]);
    expect(result.items).toEqual([]);
  });

  it("should merge mixed starter types via mergeStartedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { objectId: 2843 }, confidence: "low" as const, provenance: { session: "s", t: 5 } },
      { entityId: 96659, value: { itemId: 750 }, confidence: "high" as const, provenance: { session: "s", t: 7 } },
    ];

    const result = mergeStartedBy(obs);
    expect(result.creatures).toEqual([197]);
    expect(result.objects).toEqual([2843]);
    expect(result.items).toEqual([750]);
  });
});
