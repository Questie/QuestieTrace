import { describe, expect, it } from "vitest";
import { observeFinishedBy, mergeFinishedBy } from "./finishedBy";
import { apiSample, creatureGuid, makeSession, traceEvent } from "../../../test/fixtures";

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
      },
    ]);
  });

  it("should capture a creature finisher from npc GUID when questnpc is absent", () => {
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

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { creatureId: 197 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture an object finisher from GameObject GUID on npc token", () => {
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

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 96659,
        value: { objectId: 2843 },
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should capture both creature and object finishers for the same quest", () => {
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
    expect(results).toHaveLength(2);
    expect(results.map((r) => r.value)).toContainEqual({ creatureId: 197 });
    expect(results.map((r) => r.value)).toContainEqual({ objectId: 2843 });
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

    // Both tokens produce observations; questnpc comes first in array so its observation appears first
    const results = observeFinishedBy(session);
    const creatures = results.filter((r) => r.value.creatureId);
    expect(creatures).toHaveLength(2);
    // questnpc is processed first (priority)
    expect(creatures[0].value.creatureId).toBe(197);
    expect(creatures[1].value.creatureId).toBe(200);
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
      },
    ]);
  });

  it("should capture an object finisher from QUEST_TURNED_IN event", () => {
    const session = makeSession(
      {
        UnitGUID: {
          npc: [apiSample(3, 3, "GameObject-0-5208-0-7-2843-0000399")],
        },
      },
      [
        traceEvent("QUEST_TURNED_IN", 3, 3, 99196),
      ]
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 99196,
        value: { objectId: 2843 },
        confidence: "low",
        provenance: { session: "test-session", t: 3 },
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

  it("should produce observations from both QUEST_COMPLETE and QUEST_TURNED_IN for the same quest", () => {
    const session = makeSession(
      {
        GetQuestID: [apiSample(3, 3, 96659)],
        UnitGUID: {
          questnpc: [apiSample(3, 3, creatureGuid(197))],
        },
      },
      [
        traceEvent("QUEST_COMPLETE", 3, 3),
        traceEvent("QUEST_TURNED_IN", 4, 4, 96659),
      ]
    );

    // Both events produce observations; mergeFinishedBy unions them by type
    const results = observeFinishedBy(session);
    const creatures = results.filter((r) => r.value.creatureId);
    expect(creatures).toHaveLength(2); // one from QUEST_COMPLETE, one from QUEST_TURNED_IN
    // mergeFinishedBy will deduplicate the creature IDs
    const merged = mergeFinishedBy(results);
    expect(merged.creatures).toEqual([197]);
    expect(merged.creatures).toHaveLength(1);
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
      []
    );

    expect(observeFinishedBy(session)).toEqual([
      {
        entityId: 99196,
        value: { creatureId: 276171 },
        confidence: "medium",
        provenance: { session: "test-session", t: 3 },
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
      []
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
      []
    );

    expect(observeFinishedBy(session)).toEqual([]);
  });

  it("should merge duplicate creature finishers via mergeFinishedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 10 } },
      { entityId: 96659, value: { creatureId: 200 }, confidence: "medium" as const, provenance: { session: "s", t: 15 } },
    ];

    const result = mergeFinishedBy(obs);
    expect(result.creatures).toEqual(expect.arrayContaining([197, 200]));
    expect(result.creatures).toHaveLength(2);
    expect(result.objects).toEqual([]);
  });

  it("should merge mixed finisher types via mergeFinishedBy", () => {
    const obs = [
      { entityId: 96659, value: { creatureId: 197 }, confidence: "medium" as const, provenance: { session: "s", t: 3 } },
      { entityId: 96659, value: { objectId: 2843 }, confidence: "low" as const, provenance: { session: "s", t: 5 } },
    ];

    const result = mergeFinishedBy(obs);
    expect(result.creatures).toEqual([197]);
    expect(result.objects).toEqual([2843]);
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
      },
    ]);
  });
});
