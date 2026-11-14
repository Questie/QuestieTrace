import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import type { FactionReactionValue } from "../../observers/npc/friendlyToFaction";
import { mergeFriendlyToFaction } from "./friendlyToFaction";

function vote(
  entityId: number,
  sessionKey: number,
  faction: "A" | "H",
  friendly: boolean,
  sessionTime: number | null = sessionKey * 1000,
): Observation<FactionReactionValue> {
  return {
    entityId,
    value: { faction, friendly, sessionTime },
    confidence: "high",
    provenance: { session: `s${sessionKey}`, sessionKey: `k${sessionKey}`, t: 0 },
  };
}

describe("mergeFriendlyToFaction", () => {
  it("should return AH when both factions are friendly in at least two sessions each", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", true), vote(1, 2, "A", true), vote(1, 3, "H", true), vote(1, 4, "H", true)]),
    ).toBe("AH");
  });

  it("should return only the friendly faction when the other is hostile", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", false), vote(1, 2, "A", false), vote(1, 3, "H", true), vote(1, 4, "H", true)]),
    ).toBe("H");
  });

  it("should return undefined when the npc is hostile to both factions", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", false), vote(1, 2, "A", false), vote(1, 3, "H", false), vote(1, 4, "H", false)]),
    ).toBeUndefined();
  });

  it("should return undefined when only one faction was observed", () => {
    expect(mergeFriendlyToFaction([vote(1, 1, "A", true), vote(1, 2, "A", true)])).toBeUndefined();
  });

  it("should return undefined when a faction was observed in only one session", () => {
    expect(
      mergeFriendlyToFaction([vote(1, 1, "A", true), vote(1, 1, "A", true), vote(1, 3, "H", true), vote(1, 4, "H", true)]),
    ).toBeUndefined();
  });

  it("should return undefined when sessions disagree for a faction", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", true),
        vote(1, 2, "A", true),
        vote(1, 5, "A", false),
        vote(1, 3, "H", true),
        vote(1, 4, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should use the newer reaction when sessions switch once over time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false),
        vote(1, 2, "A", true),
        vote(1, 3, "A", true),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBe("AH");
  });

  it("should decide without server times when all sessions agree", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", true, null),
        vote(1, 2, "A", true, null),
        vote(1, 3, "H", true, null),
        vote(1, 4, "H", true, null),
      ]),
    ).toBe("AH");
  });

  it("should return undefined when the switch happens between sessions with the same server time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false, 1000),
        vote(1, 2, "A", true, 1000),
        vote(1, 3, "A", true, 2000),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should order sessions by server time rather than by session key", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "H", true, 4000),
        vote(1, 2, "H", true, 3000),
        vote(1, 3, "H", false, 2000),
        vote(1, 4, "H", false, 1000),
        vote(1, 5, "A", true),
        vote(1, 6, "A", true),
      ]),
    ).toBe("AH");
  });

  it("should return undefined when the newer reaction was seen in only one session", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false),
        vote(1, 2, "A", false),
        vote(1, 3, "A", true),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should return undefined when sessions switch more than once over time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", true),
        vote(1, 2, "A", false),
        vote(1, 3, "A", true),
        vote(1, 4, "A", true),
        vote(1, 5, "H", true),
        vote(1, 6, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should return undefined when disagreeing sessions have no server time", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false, null),
        vote(1, 2, "A", true),
        vote(1, 3, "A", true),
        vote(1, 4, "H", true),
        vote(1, 5, "H", true),
      ]),
    ).toBeUndefined();
  });

  it("should return undefined when one session saw both reactions", () => {
    expect(
      mergeFriendlyToFaction([
        vote(1, 1, "A", false),
        vote(1, 2, "A", true),
        vote(1, 2, "A", false),
        vote(1, 3, "A", true),
        vote(1, 4, "A", true),
        vote(1, 5, "H", true),
        vote(1, 6, "H", true),
      ]),
    ).toBeUndefined();
  });
});
