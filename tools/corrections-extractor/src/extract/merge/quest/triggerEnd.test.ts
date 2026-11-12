import { describe, expect, it } from "vitest";
import type { Observation } from "../../observation";
import type { QuestTriggerEndValue } from "../../observers/quest/triggerEnd";
import { mergeQuestTriggerEnds } from "./triggerEnd";

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
