import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeObjectivesText } from "./objectivesText";

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

/** Packed GetQuestLogQuestText return: (questDescription, questObjectives). */
function questText(description: string, objectives: string) {
  return { 1: description, 2: objectives, n: 2 };
}

describe("observeObjectivesText", () => {
  it("should take the objectives (second) return, not the description", () => {
    const session = makeSession({
      "GetQuestLogQuestText": {
        "33": [{ t: 10, tp: 10, v: questText("Eagan has a hunting problem.", "Bring 8 pieces of Tough Wolf Meat to Eagan Peltskinner outside Northshire Abbey.") }],
      },
    });

    expect(observeObjectivesText(session)).toEqual([
      {
        entityId: 33,
        value: ["Bring 8 pieces of Tough Wolf Meat to Eagan Peltskinner outside Northshire Abbey."],
        confidence: "high",
        provenance: { session: "test-session", t: 10 },
      },
    ]);
  });

  it("should keep the blank line between paragraphs, as Questie's DB rows do", () => {
    const session = makeSession({
      "GetQuestLogQuestText": {
        "25": [{ t: 10, tp: 10, v: questText("Long flavour text.", "Dispatch 12 Befouled Water Elementals at Mystral Lake.\n\nScout the gazebo on Mystral Lake.\n\nReturn to Mastok Wrilehiss at Splintertree Post.") }],
      },
    });

    expect(observeObjectivesText(session)[0].value).toEqual([
      "Dispatch 12 Befouled Water Elementals at Mystral Lake.",
      "",
      "Scout the gazebo on Mystral Lake.",
      "",
      "Return to Mastok Wrilehiss at Splintertree Post.",
    ]);
  });

  it("should drop leading and trailing blank lines but keep interior ones", () => {
    const session = makeSession({
      "GetQuestLogQuestText": {
        "7": [{ t: 10, tp: 10, v: questText("d", "\nKill 10 Kobold Vermin, then return to Marshal McBride.\n\n") }],
      },
    });

    expect(observeObjectivesText(session)[0].value).toEqual(["Kill 10 Kobold Vermin, then return to Marshal McBride."]);
  });

  it("should skip empty objectives text (auto-complete quests have none)", () => {
    const session = makeSession({
      "GetQuestLogQuestText": {
        "9": [{ t: 10, tp: 10, v: questText("d", "   \n  ") }],
      },
    });

    expect(observeObjectivesText(session)).toEqual([]);
  });

  it("should skip non-enUS sessions", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "deDE" }] },
      "GetQuestLogQuestText": {
        "33": [{ t: 10, tp: 10, v: questText("d", "Bring 8 pieces of Tough Wolf Meat.") }],
      },
    });

    expect(observeObjectivesText(session)).toEqual([]);
  });

  it("should treat a missing GetLocale stream as enUS", () => {
    const session = makeSession({
      "GetQuestLogQuestText": {
        "33": [{ t: 10, tp: 10, v: questText("d", "Bring 8 pieces of Tough Wolf Meat.") }],
      },
    });

    expect(observeObjectivesText(session)).toHaveLength(1);
  });

  it("should emit every sample, including repeats across a session", () => {
    const session = makeSession({
      "GetQuestLogQuestText": {
        "33": [
          { t: 10, tp: 10, v: questText("d", "Bring 8 pieces of Tough Wolf Meat.") },
          { t: 40, tp: 40, v: questText("d", "Bring 8 pieces of Tough Wolf Meat.") },
        ],
      },
    });

    expect(observeObjectivesText(session).map((observation) => observation.provenance.t)).toEqual([10, 40]);
  });

  it("should return an empty array when no GetQuestLogQuestText stream exists", () => {
    expect(observeObjectivesText(makeSession({}))).toEqual([]);
  });
});
