import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import { mergeFinishedBy } from "./finishedBy";
import { mergeStartedBy } from "./startedBy";
import type { FinishedByFinisher } from "../../observers/quest/finishedBy";
import type { StartedByStarter } from "../../observers/quest/startedBy";

function seen<T>(value: T, sessions: number, observationsPerSession = 1): Observation<T>[] {
  return Array.from({ length: sessions * observationsPerSession }, (_, i) => ({
    entityId: 1,
    value,
    confidence: "medium" as const,
    provenance: { session: "s", sessionKey: `k${Math.floor(i / observationsPerSession)}`, t: i },
  }));
}

describe("mergeFinishedBy session support", () => {
  it("should keep finishers seen in at least 5 sessions", () => {
    const observations = [...seen<FinishedByFinisher>({ creatureId: 1077 }, 5), ...seen<FinishedByFinisher>({ objectId: 2843 }, 6)];

    expect(mergeFinishedBy(observations)).toEqual({ creatures: [1077], objects: [2843] });
  });

  it("should drop finishers seen in fewer than 5 sessions, however often", () => {
    const observations = [...seen<FinishedByFinisher>({ creatureId: 1077 }, 5), ...seen<FinishedByFinisher>({ creatureId: 1076 }, 4, 10)];

    expect(mergeFinishedBy(observations)).toEqual({ creatures: [1077], objects: [] });
  });

  it("should give no value when no finisher has enough sessions", () => {
    expect(mergeFinishedBy(seen<FinishedByFinisher>({ creatureId: 1076 }, 4))).toBeUndefined();
  });
});

describe("mergeStartedBy session support", () => {
  it("should keep starters seen in at least 5 sessions", () => {
    const observations = [
      ...seen<StartedByStarter>({ creatureId: 197 }, 5),
      ...seen<StartedByStarter>({ objectId: 2843 }, 5),
      ...seen<StartedByStarter>({ itemId: 750 }, 5),
    ];

    expect(mergeStartedBy(observations)).toEqual({ creatures: [197], objects: [2843], items: [750] });
  });

  it("should drop starters seen in fewer than 5 sessions, however often", () => {
    const observations = [
      ...seen<StartedByStarter>({ creatureId: 197 }, 5),
      ...seen<StartedByStarter>({ creatureId: 200 }, 4, 10),
      ...seen<StartedByStarter>({ itemId: 750 }, 1),
    ];

    expect(mergeStartedBy(observations)).toEqual({ creatures: [197], objects: [], items: [] });
  });

  it("should give no value when no starter has enough sessions", () => {
    expect(mergeStartedBy(seen<StartedByStarter>({ itemId: 750 }, 4))).toBeUndefined();
  });
});
