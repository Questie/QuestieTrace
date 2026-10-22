import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeObjectiveCount } from "./objectiveCount";

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

describe("observeObjectiveCount", () => {
  it("should observe the number of objectives the API reports for a quest", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "92109": [
          {
            t: 3,
            tp: 3,
            v: [
              { type: "item", text: "0/5 Empty Vial" },
              { type: "item", text: "0/5 Peacebloom" },
              { type: "item", text: "0/5 Silverleaf" },
            ],
          },
        ],
      },
    });

    expect(observeObjectiveCount(session)).toEqual([
      { entityId: 92109, value: 3, confidence: "high", provenance: { session: "test-session", t: 3 } },
    ]);
  });

  it("should count objectives from non-enUS sessions", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      "C_QuestLog.GetQuestObjectives": {
        "33": [{ t: 3, tp: 3, v: [{ type: "item", text: "Wolfsfleisch: 0/8" }] }],
      },
    });

    expect(observeObjectiveCount(session).map((o) => o.value)).toEqual([1]);
  });

  it("should not count event or log objectives", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": {
        "33": [
          {
            t: 3,
            tp: 3,
            v: [
              { type: "item", text: "Tough Wolf Meat: 0/8" },
              { type: "event", text: "Light the campfire" },
              { type: "log", text: "Some flavor text" },
              { type: "reputation", text: "Friendly with the Argent Dawn" },
            ],
          },
        ],
      },
    });

    expect(observeObjectiveCount(session).map((o) => o.value)).toEqual([2]);
  });

  it("should ignore empty objective lists", () => {
    const session = makeSession({
      "C_QuestLog.GetQuestObjectives": { "33": [{ t: 3, tp: 3, v: [] }] },
    });

    expect(observeObjectiveCount(session)).toEqual([]);
  });
});
