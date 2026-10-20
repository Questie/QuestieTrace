import { describe, expect, it } from "vitest";
import { observeStartedBy, mergeStartedBy } from "./startedBy";
import { apiSample, creatureGuid, makeSession, objectGuid, traceEvent } from "../../../test/fixtures";

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
      [traceEvent("GOSSIP_SHOW", 3, 3)]
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
      [traceEvent("GOSSIP_SHOW", 3, 3)]
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
      [traceEvent("GOSSIP_SHOW", 3, 3)]
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
        traceEvent("GOSSIP_SHOW", 3, 3),
        traceEvent("QUEST_DETAIL", 4, 4, 0),
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

describe("observeStartedBy interaction correlation", () => {
  it("should prefer a questnpc object over a conflicting npc creature", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(3, 3.002, objectGuid(410369))],
          npc: [apiSample(3, 3.002, creatureGuid(276172))],
        },
      },
      [
        traceEvent("QUEST_ACCEPTED", 3, 3.001, 99196),
      ]
    );

    expect(observeStartedBy(session).map((o) => o.value)).toEqual([{ objectId: 410369 }]);
  });

  it("should fall back to npc when questnpc is explicitly nil", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.002),
          ],
          npc: [apiSample(3, 3.002, creatureGuid(276172))],
        },
      },
      [
        traceEvent("QUEST_ACCEPTED", 3, 3.001, 99196),
      ]
    );

    expect(observeStartedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276172 }]);
  });

  it("should not fall back to an npc object like a mailbox when a quest is shared", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 1486)],
        UnitGUID: {
          npc: [apiSample(1, 1, objectGuid(659159))],
        },
      },
      [
        traceEvent("MAIL_SHOW", 1, 1),
        traceEvent("QUEST_DETAIL", 3, 3, 0),
        traceEvent("QUEST_ACCEPTED", 4, 4, 1486),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should include synchronous probes without borrowing a later same-frame quest or giver", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(3, 3.002, 99196),
          apiSample(3, 3.012, 99197),
        ],
        UnitGUID: {
          questnpc: [
            apiSample(3, 3.003, creatureGuid(276171)),
            apiSample(3, 3.013, creatureGuid(276172)),
          ],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3.001),
        traceEvent("QUEST_DETAIL", 3, 3.011),
      ]
    );

    expect(observeStartedBy(session).map((o) => [o.entityId, o.value])).toEqual([
      [99196, { creatureId: 276171 }],
      [99197, { creatureId: 276172 }],
    ]);
  });

  it("should use the next raw event as the boundary, even when it is not a quest event", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.012, creatureGuid(276172)),
          ],
        },
      },
      [
        traceEvent("QUEST_ACCEPTED", 3, 3.001, 99196),
        traceEvent("MERCHANT_SHOW", 3, 3.011),
      ]
    );

    expect(observeStartedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276171 }]);
  });

  it("should not let a later same-frame close erase the current giver or reuse it after close", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, objectGuid(410369)),
            apiSample(3, 3.012),
          ],
        },
      },
      [
        traceEvent("QUEST_ACCEPTED", 3, 3.001, 99196),
        traceEvent("QUEST_FINISHED", 3, 3.011),
        traceEvent("QUEST_ACCEPTED", 3, 3.021, 99197),
      ]
    );

    expect(observeStartedBy(session).map((o) => [o.entityId, o.value])).toEqual([[99196, { objectId: 410369 }]]);
  });

  it("should keep an unchanged object giver during a long dialog", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(1, 1, objectGuid(410369))],
        },
      },
      [
        traceEvent("QUEST_PROGRESS", 1, 1),
        traceEvent("QUEST_ACCEPTED", 60, 60, 99196),
      ]
    );

    expect(observeStartedBy(session).map((o) => o.value)).toEqual([{ objectId: 410369 }]);
  });

  it("should skip events with indistinguishable timestamps rather than guessing their giver", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [
            apiSample(3, 3, creatureGuid(276171)),
            apiSample(3, 3, creatureGuid(276172)),
          ],
        },
      },
      [
        traceEvent("QUEST_ACCEPTED", 3, 3, 99196),
        traceEvent("QUEST_ACCEPTED", 3, 3, 99197),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it.each<string | undefined>([undefined, creatureGuid(276172)])("should not resurrect an older giver when a probe ties with the next event (%s)", (tiedGuid) => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.002, tiedGuid),
          ],
          npc: [apiSample(1, 1, creatureGuid(276173))],
        },
      },
      [
        traceEvent("QUEST_ACCEPTED", 3, 3.001, 99196),
        traceEvent("MERCHANT_SHOW", 3, 3.002),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should not resurrect an older quest ID when its probe ties with the next event", () => {
    const session = makeSession(
      {
        GetQuestID: [
          apiSample(1, 1, 99196),
          apiSample(3, 3.002, 99197),
        ],
        UnitGUID: {
          questnpc: [apiSample(1, 1, creatureGuid(276171))],
        },
      },
      [
        traceEvent("QUEST_DETAIL", 3, 3.001),
        traceEvent("QUEST_FINISHED", 3, 3.002),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should skip gossip events with indistinguishable timestamps", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.002, creatureGuid(276172)),
          ],
        },
        "C_GossipInfo.GetAvailableQuests": [apiSample(1, 1, [{ questID: 99196 }])],
      },
      [
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 3, 3.001),
      ]
    );

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should not infer an interaction from a list without a GOSSIP_SHOW event", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [apiSample(3, 3.002, creatureGuid(276171))],
      },
      "C_GossipInfo.GetAvailableQuests": [apiSample(3, 3.002, [{ questID: 99196 }])],
    });

    expect(observeStartedBy(session)).toEqual([]);
  });

  it("should replay one unchanged gossip list at two NPCs within the same frame", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [
            apiSample(3, 3.002, creatureGuid(276171)),
            apiSample(3, 3.012, creatureGuid(276172)),
          ],
        },
        "C_GossipInfo.GetAvailableQuests": [apiSample(3, 3.002, [{ questID: 99196 }])],
      },
      [
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 3, 3.011),
      ]
    );

    expect(observeStartedBy(session).map((o) => [o.entityId, o.value])).toEqual([
      [99196, { creatureId: 276171 }],
      [99196, { creatureId: 276172 }],
    ]);
  });

  it("should replay the last observed list after closing and opening gossip at another NPC", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [
            apiSample(1, 1.002, creatureGuid(276171)),
            apiSample(2, 2.002),
            apiSample(3, 3.002, creatureGuid(276172)),
          ],
        },
        "C_GossipInfo.GetAvailableQuests": [apiSample(1, 1.002, [{ questID: 99196 }])],
      },
      [
        traceEvent("GOSSIP_SHOW", 1, 1.001),
        traceEvent("GOSSIP_CLOSED", 2, 2.001),
        traceEvent("GOSSIP_SHOW", 3, 3.001),
      ]
    );

    expect(observeStartedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276171 }, { creatureId: 276172 }]);
  });

  it.each<unknown[] | undefined>([[], undefined])("should not carry quests past an observed empty or nil list (%s)", (clearingList) => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [
            apiSample(1, 1.002, creatureGuid(276171)),
            apiSample(3, 3.002, creatureGuid(276172)),
          ],
        },
        "C_GossipInfo.GetAvailableQuests": [
          apiSample(1, 1.002, [{ questID: 99196 }]),
          apiSample(3, 3.002, clearingList),
        ],
      },
      [
        traceEvent("GOSSIP_SHOW", 1, 1.001),
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 4, 4.001),
      ]
    );

    expect(observeStartedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276171 }]);
  });

  it("should not read a later same-frame quest list into an earlier gossip interaction", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [
            apiSample(3, 3.002, creatureGuid(276171)),
            apiSample(3, 3.012, creatureGuid(276172)),
          ],
        },
        "C_GossipInfo.GetAvailableQuests": [
          apiSample(3, 3.002, [{ questID: 99196 }]),
          apiSample(3, 3.012, [{ questID: 99197 }]),
        ],
      },
      [
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 3, 3.011),
      ]
    );

    expect(observeStartedBy(session).map((o) => [o.entityId, o.value])).toEqual([
      [99196, { creatureId: 276171 }],
      [99197, { creatureId: 276172 }],
    ]);
  });
});
