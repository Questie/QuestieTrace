import { describe, expect, it } from "vitest";
import { claimKey, provenanceOf } from "./scores";

describe("claimKey", () => {
  it("identifies a claim by surface, quest, field and target only", () => {
    const base = { surface: "series", questId: 2, field: "preQuestSingle", target: 1 } as const;
    const samePairFromAnotherPage = { ...base, shape: "group", pageId: 9 } as const;
    expect(claimKey(samePairFromAnotherPage)).toBe(claimKey(base));
    const others = [{ ...base, surface: "unlocks" }, { ...base, questId: 1, target: 2 }, { ...base, field: "nextQuestInChain" }] as const;
    for (const other of others) expect(claimKey(other)).not.toBe(claimKey(base));
  });
});

describe("provenanceOf", () => {
  it("separates Forever-only claims from inherited ones", () => {
    const era = new Set(["series|2|preQuestSingle|1"]);
    expect(provenanceOf("forever", "series|2|preQuestSingle|1", era)).toBe("forever-era-edge");
    expect(provenanceOf("forever", "series|3|preQuestSingle|1", era)).toBe("forever-new-edge");
    expect(provenanceOf("era", "series|3|preQuestSingle|1", era)).toBe("era-page");
    expect(provenanceOf("unknown", "series|3|preQuestSingle|1", era)).toBe("era-page");
  });
});
