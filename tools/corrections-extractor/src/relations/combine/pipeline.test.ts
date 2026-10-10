import { describe, expect, it } from "vitest";
import { catalogQuest } from "../core/fixtures";
import type { CandidateFile, CatalogQuest, GroundTruth, QuestCatalog, RelationCandidate, RelationSet } from "../core/types";
import { prepareInputs } from "./inputs";
import { combine, type Combined } from "./pipeline";

function file(signal: string, coveredQuestIds: number[], candidates: Array<Omit<RelationCandidate, "contradict" | "evidence">>): CandidateFile {
  return {
    signal,
    generatedAt: "",
    inputCount: 1,
    params: {},
    coveredQuestIds,
    candidates: candidates.map((candidate) => ({ ...candidate, contradict: 0, evidence: [] })),
  };
}

// 40 authored quests 92000+i, each claimed to need 91000+i; every fourth claim is wrong. Five more
// quests 93000+i have no reference, so the model fitted on all ground truth scores them.
const range = (n: number) => Array.from({ length: n }, (_, i) => i);
const judged = range(40);
const quests: Record<string, CatalogQuest> = {};
for (const i of [...judged, 40, 41, 42, 43, 44]) {
  quests[String(92000 + i)] = catalogQuest(92000 + i, { name: `Quest ${i}` });
  quests[String(91000 + i)] = catalogQuest(91000 + i, { name: `Step ${i}` });
  quests[String(90000 + i)] = catalogQuest(90000 + i, { name: `Other ${i}` });
}
for (const i of range(5)) quests[String(93000 + i)] = catalogQuest(93000 + i, { name: `Open ${i}` });
const catalog: QuestCatalog = { generatedAt: "", sources: [], quests };
const truth: GroundTruth = {
  generatedAt: "",
  sources: [],
  tiers: {
    authoredForever: Object.fromEntries(judged.map((i): [string, RelationSet] => [String(92000 + i), { preQuestSingle: [i % 4 === 0 ? 91999 : 91000 + i] }])),
    inheritedClassic: {},
  },
};
const allQuests = Object.keys(quests).map(Number);

const handoff = file("handoff", allQuests, [
  ...judged.map((i) => ({
    questId: 92000 + i,
    field: "preQuestSingle" as const,
    target: 91000 + i,
    score: i % 4 === 0 ? 0.5 + (i % 3) * 0.05 : 0.9 + (i % 3) * 0.03,
    support: (i % 7) + 1,
  })),
  ...range(5).map((i) => ({ questId: 93000 + i, field: "preQuestSingle" as const, target: 91000 + i, score: 0.95, support: 9 })),
]);
const offerSet = file(
  "offer-set",
  allQuests,
  judged.filter((i) => i % 3 !== 0).map((i) => ({ questId: 92000 + i, field: "preQuestSingle" as const, target: 91000 + i, score: 0.9, support: 4 })),
);
// Claims some of the same edges, some of its own, and covers everything: any leak shows up as a
// column, a silence or a new edge.
const wowhead = file("wowhead-series", allQuests, [
  ...judged.filter((i) => i % 2 === 0).map((i) => ({ questId: 92000 + i, field: "preQuestSingle" as const, target: 91000 + i, score: 0.85, support: 3 })),
  ...range(5).map((i) => ({ questId: 93000 + i, field: "preQuestSingle" as const, target: 90000 + i, score: 0.95, support: 5 })),
  ...range(10).map((i) => ({ questId: 91000 + i, field: "nextQuestInChain" as const, target: 92000 + i, score: 0.95, support: 5 })),
  ...range(3).map((i) => ({ questId: 90000 + i, field: "breadcrumbForQuestId" as const, target: 92000 + i, score: 0.35, support: 2 })),
]);

function snapshot(result: Combined) {
  return {
    probabilities: Object.fromEntries(result.scored.map((edge) => [edge.key, edge.probability])),
    relations: Object.fromEntries(result.accepted.relations),
    decisions: Object.fromEntries(result.accepted.decisions),
  };
}

describe("combine", () => {
  it("is unchanged by Wowhead candidate files: same probabilities, same accepted relations", () => {
    const withWowhead = prepareInputs([handoff, offerSet, wowhead], catalog, truth);
    const withoutWowhead = prepareInputs([handoff, offerSet], catalog, truth);

    const shipped = snapshot(combine(withWowhead));

    expect(shipped).toEqual(snapshot(combine(withoutWowhead)));
    // Not vacuous: something was accepted, and Wowhead did claim things.
    expect(Object.keys(shipped.relations).length).toBeGreaterThan(0);
    expect(withWowhead.context.size).toBeGreaterThan(0);
  });
});
