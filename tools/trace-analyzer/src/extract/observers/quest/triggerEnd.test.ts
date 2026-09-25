import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { mergeQuestTriggerEnds, observeTriggerEnd } from "./triggerEnd";

function makeSession(functions: SessionRecord["functions"]): SessionRecord {
  return {
    schemaVersion: 9,
    name: "test-session",
    startedAt: 0,
    startedAtPrecise: 0,
    stoppedAt: 100,
    stoppedAtPrecise: 100,
    duration: 100,
    durationPrecise: 100,
    events: [],
    functions,
    functionsDelta: {},
  };
}

describe("observeTriggerEnd", () => {
  it("should extract a completed event objective at the player's map position", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          {
            t: 10,
            tp: 10,
            v: [{ type: "event", text: "Light the campfire", finished: true }],
          },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 10, tp: 10, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 10, tp: 10, v: { x: 0.5611, y: 0.6164 } }] },
    });

    expect(observeTriggerEnd(session)).toEqual([
      {
        entityId: 33,
        value: ["Light the campfire", { 40: [[56.11, 61.64]] }],
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
      },
    ]);
  });

  it("should ignore incomplete and non-event objectives", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          {
            t: 10,
            tp: 10,
            v: [
              { type: "event", text: "Not completed yet", finished: false },
              { type: "monster", text: "0/1 Kobold Vermin slain", finished: true },
            ],
          },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 10, tp: 10, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 10, tp: 10, v: { x: 0.5, y: 0.5 } }] },
    });

    expect(observeTriggerEnd(session)).toEqual([]);
  });

  it("should skip non-enUS objective text", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 10, tp: 10, v: [{ type: "event", text: "Das Lagerfeuer anzünden", finished: true }] }],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 10, tp: 10, v: 1436 }] }, // uiMapID 1436 = Westfall (area 40),
      "C_Map.GetPlayerMapPosition": { player: [{ t: 10, tp: 10, v: { x: 0.5, y: 0.5 } }] },
    });

    expect(observeTriggerEnd(session)).toEqual([]);
  });
});

describe("mergeQuestTriggerEnds", () => {
  it("should merge repeated completion samples and de-duplicate coordinates", () => {
    expect(
      mergeQuestTriggerEnds([
        {
          entityId: 33,
          value: ["Light the campfire", { 40: [[56.11, 61.64]] }],
          confidence: "high",
          provenance: { session: "test", t: 10 },
        },
        {
          entityId: 33,
          value: ["Light the campfire", { 40: [[56.11, 61.64], [57, 62]], 12: [[30, 40]] }],
          confidence: "high",
          provenance: { session: "test", t: 20 },
        },
      ]),
    ).toEqual(["Light the campfire", { 12: [[30, 40]], 40: [[56.11, 61.64], [57, 62]] }]);
  });
});
