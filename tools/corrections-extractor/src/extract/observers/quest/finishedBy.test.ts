import { describe, expect, it } from "vitest";
import { observeFinishedBy } from "./finishedBy";
import { apiSample, creatureGuid, makeSession, objectGuid, traceEvent } from "../../../test/fixtures";

describe("observeFinishedBy", () => {
  it("should capture a creature finisher from questnpc GUID at QUEST_COMPLETE time", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
        token: "questnpc",
      },
    ]);
  });

  it("should not take a creature finisher from npc GUID at QUEST_COMPLETE when questnpc is absent", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          npc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should not take an object finisher from GameObject GUID on npc token", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          npc: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should prefer the questnpc creature over a conflicting npc object", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
          npc: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    const results = observeFinishedBy(session);
    expect(results.map((r) => r.value)).toEqual([{ creatureId: 197 }]);
  });

  it("should prefer questnpc over npc when both are present", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
          npc: [apiSample(3, 3, "Creature-0-5208-0-7-200-000040")],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    const results = observeFinishedBy(session);
    expect(results.map((r) => r.value)).toEqual([{ creatureId: 197 }]);
  });

  it("should capture multiple creature finishers across different quest events", () => {
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
        traceEvent("QUEST_COMPLETE", 3, 3),
        traceEvent("QUEST_COMPLETE", 10, 10),
      ]
    );

    const results = observeFinishedBy(session);
    expect(results).toHaveLength(2);
    expect(results.filter((r) => r.value.creatureId).map((r) => r.entityId)).toEqual([96659, 12345]);
    expect(results.filter((r) => r.value.creatureId).map((r) => r.value.creatureId)).toEqual([197, 200]);
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
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should return no observations when no quest-completion events exist", () => {
    const session = makeSession({
      GetQuestID: [apiSample(3, 3, 96659)],
      UnitGUID: {
        questnpc: [apiSample(3, 3, creatureGuid(197))],
      },
    });

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should return no observations when GetQuestID is not active at event time", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should capture an object finisher at QUEST_COMPLETE", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 99196)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 99196,
        value: { objectId: 2843 },
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
        token: "questnpc",
      },
    ]);
  });

  it("should capture a creature finisher from QUEST_TURNED_IN event (questID from arg 1)", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(276171))],
        },
      },
      [
        traceEvent("QUEST_TURNED_IN", 3, 3, 99196),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 99196,
        value: { creatureId: 276171 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
        token: "questnpc",
      },
    ]);
  });

  it("should ignore QUEST_TURNED_IN with invalid questID", () => {
    const session = makeSession(
      {
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(276171))],
        },
      },
      [
        traceEvent("QUEST_TURNED_IN", 3, 3, 0),
        traceEvent("QUEST_TURNED_IN", 4, 4, "99196"),
        traceEvent("QUEST_TURNED_IN", 5, 5),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should not take a finisher at QUEST_TURNED_IN when the quest was already shown in a dialog", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(1, 1.001, 1104), apiSample(2, 2.002, 0), apiSample(3, 3.001, 1105)],
        UnitGUID: {
          questnpc: [
            apiSample(1, 1.001, creatureGuid(4453)),
            apiSample(2, 2.001),
            apiSample(3, 3.001, creatureGuid(4454)),
          ],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 1, 1),
        traceEvent("QUEST_FINISHED", 2, 2),
        traceEvent("QUEST_COMPLETE", 3, 3),
        traceEvent("QUEST_TURNED_IN", 4, 4, 1104),
      ]
    );

    expect(observeFinishedBy(session).map((o) => [o.entityId, o.value])).toEqual([
      [1104, { creatureId: 4453 }],
      [1105, { creatureId: 4454 }],
    ]);
  });

  it("should capture a creature finisher from C_GossipInfo.GetActiveQuests snapshot", () => {
    const session = makeSession(
      {
        "C_GossipInfo.GetActiveQuests": [
          apiSample(3, 3, [
            { questID: 99196, title: "A Donation of Wool", repeatable: false, isLegendary: false },
          ]),
        ],
        UnitGUID: {
          npc: [apiSample(3, 3, creatureGuid(276171))],
        },
      },
      [traceEvent("GOSSIP_SHOW", 3, 3)]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 99196,
        value: { creatureId: 276171 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
        token: "npc",
      },
    ]);
  });

  it("should capture multiple quests from a single gossip active quests snapshot", () => {
    const session = makeSession(
      {
        "C_GossipInfo.GetActiveQuests": [
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

    const results = observeFinishedBy(session);
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.entityId).sort()).toEqual([99196, 99197]);
    expect(results.map((r) => r.value.creatureId)).toEqual([276171, 276171]);
  });

  it("should ignore gossip active quest entries without valid questID", () => {
    const session = makeSession(
      {
        "C_GossipInfo.GetActiveQuests": [
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

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should attribute correctly even when the GUID sample is much older than the quest event (long dialog read)", () => {
    // questnpc/npc streams are only re-written on change (see UnitInteraction.lua dedup), so a player
    // reading the quest text for a long time before completing leaves the last GUID sample far in the
    // past relative to the anchor event - this must not be treated as stale.
    const session = makeSession(
      {
        GetQuestID: [apiSample(60, 60, 96659)],
        UnitGUID: {
          questnpc: [apiSample(1, 1, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 60, 60),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 60 },
        token: "questnpc",
      },
    ]);
  });
});

describe("observeFinishedBy interaction correlation", () => {
  it("should prefer a questnpc object over a conflicting npc creature", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [apiSample(3, 3.002, objectGuid(410369))],
          npc: [apiSample(3, 3.002, creatureGuid(276172))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3.001),
      ]
    );

    expect(observeFinishedBy(session).map((o) => o.value)).toEqual([{ objectId: 410369 }]);
  });

  it("should not fall back to npc at QUEST_COMPLETE when questnpc is explicitly nil", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.002),
          ],
          npc: [apiSample(3, 3.002, creatureGuid(276172))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3.001),
      ]
    );

    expect(observeFinishedBy(session).map((o) => o.value)).toEqual([]);
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
        traceEvent("QUEST_COMPLETE", 3, 3.001),
        traceEvent("QUEST_COMPLETE", 3, 3.011),
      ]
    );

    expect(observeFinishedBy(session).map((o) => [o.entityId, o.value])).toEqual([
      [99196, { creatureId: 276171 }],
      [99197, { creatureId: 276172 }],
    ]);
  });

  it("should use the next raw event as the boundary, even when it is not a quest event", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.012, creatureGuid(276172)),
          ],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3.001),
        traceEvent("MERCHANT_SHOW", 3, 3.011),
      ]
    );

    expect(observeFinishedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276171 }]);
  });

  it("should not let a later same-frame close erase the current giver or reuse it after close", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, objectGuid(410369)),
            apiSample(3, 3.012),
          ],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3.001),
        traceEvent("QUEST_FINISHED", 3, 3.011),
        traceEvent("QUEST_COMPLETE", 3, 3.021),
      ]
    );

    expect(observeFinishedBy(session).map((o) => [o.entityId, o.value])).toEqual([[99196, { objectId: 410369 }]]);
  });

  it("should keep an unchanged object giver during a long dialog", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [apiSample(1, 1, objectGuid(410369))],
        },
      },
      [
        traceEvent("QUEST_PROGRESS", 1, 1),
        traceEvent("QUEST_COMPLETE", 60, 60),
      ]
    );

    expect(observeFinishedBy(session).map((o) => o.value)).toEqual([{ objectId: 410369 }]);
  });

  it("should skip events with indistinguishable timestamps rather than guessing their giver", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [
            apiSample(3, 3, creatureGuid(276171)),
            apiSample(3, 3, creatureGuid(276172)),
          ],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
        traceEvent("QUEST_COMPLETE", 3, 3),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it.each<string | undefined>([undefined, creatureGuid(276172)])("should not resurrect an older giver when a probe ties with the next event (%s)", (tiedGuid) => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(0, 0, 99196)],
        UnitGUID: {
          questnpc: [
            apiSample(1, 1, creatureGuid(276171)),
            apiSample(3, 3.002, tiedGuid),
          ],
          npc: [apiSample(1, 1, creatureGuid(276173))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3.001),
        traceEvent("MERCHANT_SHOW", 3, 3.002),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
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
        traceEvent("QUEST_COMPLETE", 3, 3.001),
        traceEvent("QUEST_FINISHED", 3, 3.002),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
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
        "C_GossipInfo.GetActiveQuests": [apiSample(1, 1, [{ questID: 99196 }])],
      },
      [
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 3, 3.001),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should not infer an interaction from a list without a GOSSIP_SHOW event", () => {
    const session = makeSession({
      UnitGUID: {
        npc: [apiSample(3, 3.002, creatureGuid(276171))],
      },
      "C_GossipInfo.GetActiveQuests": [apiSample(3, 3.002, [{ questID: 99196 }])],
    });

    expect(observeFinishedBy(session)).toEqual([]);
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
        "C_GossipInfo.GetActiveQuests": [apiSample(3, 3.002, [{ questID: 99196 }])],
      },
      [
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 3, 3.011),
      ]
    );

    expect(observeFinishedBy(session).map((o) => [o.entityId, o.value])).toEqual([
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
        "C_GossipInfo.GetActiveQuests": [apiSample(1, 1.002, [{ questID: 99196 }])],
      },
      [
        traceEvent("GOSSIP_SHOW", 1, 1.001),
        traceEvent("GOSSIP_CLOSED", 2, 2.001),
        traceEvent("GOSSIP_SHOW", 3, 3.001),
      ]
    );

    expect(observeFinishedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276171 }, { creatureId: 276172 }]);
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
        "C_GossipInfo.GetActiveQuests": [
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

    expect(observeFinishedBy(session).map((o) => o.value)).toEqual([{ creatureId: 276171 }]);
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
        "C_GossipInfo.GetActiveQuests": [
          apiSample(3, 3.002, [{ questID: 99196 }]),
          apiSample(3, 3.012, [{ questID: 99197 }]),
        ],
      },
      [
        traceEvent("GOSSIP_SHOW", 3, 3.001),
        traceEvent("GOSSIP_SHOW", 3, 3.011),
      ]
    );

    expect(observeFinishedBy(session).map((o) => [o.entityId, o.value])).toEqual([
      [99196, { creatureId: 276171 }],
      [99197, { creatureId: 276172 }],
    ]);
  });
});
