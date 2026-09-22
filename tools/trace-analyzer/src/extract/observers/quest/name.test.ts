import { describe, expect, it } from "vitest";
import type { SessionRecord } from "../../../core/types";
import { observeName } from "./name";

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

describe("observeName (quest)", () => {
  it("should observe the quest's title at the same time as the questID encounter", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      GetTitleText: [{ t: 3, tp: 3, v: "A Threat Within" }],
    });

    expect(observeName(session)).toEqual([
      {
        entityId: 96659,
        value: "A Threat Within",
        confidence: "high",
        provenance: { session: "test-session", t: 3 },
      },
    ]);
  });

  it("should use the most recently observed title at or before the encounter's time", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      GetQuestID: [{ t: 5, tp: 5, v: 96659 }],
      GetTitleText: [{ t: 2, tp: 2, v: "A Threat Within" }],
    });

    expect(observeName(session)).toEqual([
      {
        entityId: 96659,
        value: "A Threat Within",
        confidence: "high",
        provenance: { session: "test-session", t: 5 },
      },
    ]);
  });

  it("should skip encounters where no title was observed", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "enUS" }] },
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
    });

    expect(observeName(session)).toEqual([]);
  });

  it("should skip encounters with non-enUS locale", () => {
    const session = makeSession({
      GetLocale: { player: [{ t: 0, tp: 0, v: "frFR" }] },
      GetQuestID: [{ t: 3, tp: 3, v: 96659 }],
      GetTitleText: [{ t: 3, tp: 3, v: "Une menace intérieure" }],
    });

    expect(observeName(session)).toEqual([]);
  });
});
