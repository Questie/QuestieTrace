import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import type { Observation } from "../../observation";
import { mergeQuestTriggerEnds, observeTriggerEnd, type QuestTriggerEndValue } from "./triggerEnd";

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

  it("should emit on the incomplete-to-complete transition, not for stale completed re-samples", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          { t: 5, tp: 5, v: [{ type: "event", text: "Light the campfire", finished: false }] },
          { t: 10, tp: 10, v: [{ type: "event", text: "Light the campfire", finished: true }] },
          // Stale: the objective stays finished in later re-samples.
          { t: 15, tp: 15, v: [{ type: "event", text: "Light the campfire", finished: true }] },
          { t: 20, tp: 20, v: [{ type: "event", text: "Light the campfire", finished: true }] },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 0, tp: 0, v: 1436 }] },
      "C_Map.GetPlayerMapPosition": {
        player: [
          { t: 5, tp: 5, v: { x: 0.1, y: 0.2 } },
          { t: 10, tp: 10, v: { x: 0.5611, y: 0.6164 } },
          { t: 15, tp: 15, v: { x: 0.9, y: 0.9 } },
          { t: 20, tp: 20, v: { x: 0.8, y: 0.8 } },
        ],
      },
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

  it("should keep the first completed sample when the objective is finished from the first positioned sample on", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          { t: 10, tp: 10, v: [{ type: "event", text: "Light the campfire", finished: true }] },
          { t: 15, tp: 15, v: [{ type: "event", text: "Light the campfire", finished: true }] },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 0, tp: 0, v: 1436 }] },
      "C_Map.GetPlayerMapPosition": {
        player: [
          { t: 10, tp: 10, v: { x: 0.5611, y: 0.6164 } },
          { t: 15, tp: 15, v: { x: 0.9, y: 0.9 } },
        ],
      },
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

  it("should track each objective's state independently and skip samples without position data", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          // No position at t=5: the incomplete state must NOT be remembered,
          // so the completed sample at t=10 is a first sighting and emits.
          { t: 5, tp: 5, v: [{ type: "event", text: "Unpositioned", finished: false }] },
          { t: 10, tp: 10, v: [{ type: "event", text: "Unpositioned", finished: true }] },
          // A different objective reaching completion in the same stream.
          { t: 15, tp: 15, v: [{ type: "event", text: "Light the beacon", finished: false }] },
          { t: 20, tp: 20, v: [{ type: "event", text: "Light the beacon", finished: true }] },
        ],
      },
      "C_Map.GetBestMapForUnit": { player: [{ t: 0, tp: 0, v: 1436 }] },
      "C_Map.GetPlayerMapPosition": {
        player: [
          { t: 10, tp: 10, v: { x: 0.3, y: 0.4 } },
          { t: 15, tp: 15, v: { x: 0.5, y: 0.5 } },
          { t: 20, tp: 20, v: { x: 0.6, y: 0.7 } },
        ],
      },
    });

    expect(observeTriggerEnd(session)).toEqual([
      {
        entityId: 33,
        value: ["Unpositioned", { 40: [[30, 40]] }],
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
      },
      {
        entityId: 33,
        value: ["Light the beacon", { 40: [[60, 70]] }],
        confidence: "high",
        provenance: { session: "test-session", t: 20 },
      },
    ]);
  });
});

describe("mergeQuestTriggerEnds", () => {
  const base = { entityId: 33, confidence: "high" as const };

  it("should export exactly one zone with one coordinate pair", () => {
    expect(
      mergeQuestTriggerEnds([
        { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64], [57, 62]] }], provenance: { session: "test", t: 10 } },
        { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64]], 12: [[30, 40]] }], provenance: { session: "test", t: 20 } },
      ]),
    ).toEqual(["Light the campfire", { 40: [[56.11, 61.64]] }]);
  });

  it("should prefer the quest's zoneOrSort zone when any sample observed it", () => {
    expect(
      mergeQuestTriggerEnds(
        [
          { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64]] }], provenance: { session: "test", t: 10 } },
          { ...base, value: ["Light the campfire", { 12: [[47.46, 62.18]] }], provenance: { session: "test", t: 20 } },
        ],
        12, // zoneOrSort resolved Elwynn Forest for this quest
      ),
    ).toEqual(["Light the campfire", { 12: [[47.46, 62.18]] }]);
  });

  it("should fall back to the most-observed zone when the preferred zone was never observed", () => {
    expect(
      mergeQuestTriggerEnds(
        [
          { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64]] }], provenance: { session: "test", t: 10 } },
          { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64]] }], provenance: { session: "test", t: 20 } },
          { ...base, value: ["Light the campfire", { 12: [[30, 40]] }], provenance: { session: "test", t: 30 } },
        ],
        999,
      ),
    ).toEqual(["Light the campfire", { 40: [[56.11, 61.64]] }]);
  });

  it("should break zone count ties by the most recent sample, then lowest zone id", () => {
    const earlier: Observation<QuestTriggerEndValue> = {
      ...base,
      value: ["Light the campfire", { 12: [[30, 40]] }],
      provenance: { session: "test", t: 10 },
    };
    const later: Observation<QuestTriggerEndValue> = {
      ...base,
      value: ["Light the campfire", { 40: [[56.11, 61.64]] }],
      provenance: { session: "test", t: 20 },
    };

    // Equal counts: the more recent sample (zone 40) wins.
    expect(mergeQuestTriggerEnds([earlier, later])).toEqual(["Light the campfire", { 40: [[56.11, 61.64]] }]);
    // Equal counts and equal recency: the lowest zone id wins deterministically.
    expect(mergeQuestTriggerEnds([earlier, { ...earlier }])).toEqual(["Light the campfire", { 12: [[30, 40]] }]);
  });

  it("should pick the most frequently observed coordinate pair within the winning zone", () => {
    expect(
      mergeQuestTriggerEnds([
        { ...base, value: ["Light the campfire", { 40: [[57, 62]] }], provenance: { session: "test", t: 10 } },
        { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64]] }], provenance: { session: "test", t: 20 } },
        { ...base, value: ["Light the campfire", { 40: [[56.11, 61.64]] }], provenance: { session: "test", t: 30 } },
      ]),
    ).toEqual(["Light the campfire", { 40: [[56.11, 61.64]] }]);
  });

  it("should return no location when there are no samples", () => {
    expect(mergeQuestTriggerEnds([])).toEqual(["", {}]);
  });
});
