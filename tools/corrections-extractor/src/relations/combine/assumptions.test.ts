import { describe, expect, it } from "vitest";
import type { RelationField } from "../core/types";
import { assumption } from "./assumptions";
import type { Claim } from "./edges";
import type { EdgeView, QuestView } from "./views";

function claim(signal: string, field: RelationField, score: number, support: number): Claim {
  return { source: `${signal}/${field}`, signal, field, score, support, contradict: 0, evidence: [] };
}

function edge(overrides: Partial<EdgeView> & Pick<EdgeView, "kind" | "field" | "target">): EdgeView {
  return { key: "k", probability: 0.99, heldOut: false, outcome: "accepted", claims: [], context: [], contributions: [], ...overrides };
}

function view(questId: number, relations: QuestView["relations"], edges: EdgeView[]): QuestView {
  return { questId, domain: "forever", reference: "none", relations, edges, referenceOnly: [], hints: [], notes: [] };
}

describe("assumption", () => {
  it("names the signal that moved the score most, not the one with the highest raw score", () => {
    const prerequisite = edge({
      kind: "prerequisite",
      field: "preQuestSingle",
      target: 76156,
      probability: 0.99523,
      claims: [claim("offer-set", "preQuestSingle", 0.98, 13), claim("handoff", "preQuestSingle", 0.97, 8), claim("absence", "preQuestSingle", 0.94, 1)],
      contributions: [
        { signal: "base", logOdds: -1.1 },
        { signal: "offer-set", logOdds: 2.4 },
        { signal: "handoff", logOdds: 2.7 },
        { signal: "absence", logOdds: -0.7 },
      ],
    });

    expect(assumption(view(76160, { preQuestSingle: [76156] }, [prerequisite]), "preQuestSingle")).toBe(
      "Assumption: offered right after 76156 was turned in (8 characters), 3 signals agree, score 0.995",
    );
  });

  it("gives one reason per target in field order, and ignores edges that were not written", () => {
    const written = (target: number, probability: number) =>
      edge({ kind: "prerequisite", field: "preQuestSingle", target, probability, claims: [claim("cooccurrence", "preQuestSingle", 0.7, 1)] });
    const edges = [written(79008, 0.97), written(79007, 0.96), { ...written(79009, 0.5), outcome: "below-threshold" as const }];

    expect(assumption(view(79192, { preQuestSingle: [79007, 79008] }, edges), "preQuestSingle")).toBe(
      "Assumption: everyone who took it had done 79007 (1 character), score 0.960; everyone who took it had done 79008 (1 character), score 0.970",
    );
    expect(assumption(view(79192, { preQuestSingle: [79007, 79008] }, edges), "nextQuestInChain")).toBeUndefined();
  });

  it("merges targets whose reasons differ only by id, so large exclusive groups stay readable", () => {
    const coflag = (target: number, support: number) =>
      edge({ kind: "exclusive", field: "exclusiveTo", target, probability: 0.993, claims: [claim("cooccurrence", "exclusiveTo", 0.9, support)] });
    const edges = [coflag(96102, 171), coflag(97925, 171), coflag(97937, 0)];

    expect(assumption(view(96101, { exclusiveTo: [96102, 97925, 97937] }, edges), "exclusiveTo")).toBe(
      "Assumption: the server completes it and 96102, 97925 together (171 characters), score 0.993; " +
        "the server completes it and 97937 together (no independent character), score 0.993",
    );
  });

  it("explains a chain implied by a breadcrumb by the breadcrumb, not its own weaker claims", () => {
    const next = edge({ kind: "next", field: "nextQuestInChain", target: 96392, probability: 0.97, outcome: "derived", claims: [claim("handoff", "nextQuestInChain", 0.66, 53)] });

    expect(assumption(view(96408, { nextQuestInChain: [96392] }, [next]), "nextQuestInChain")).toBe("Assumption: implied by its breadcrumb to 96392, score 0.970");
  });

  it("words a breadcrumb from each side, and absence by the side it observed", () => {
    // Breadcrumb 463 -> 276. Absence saw 276 hidden while 463 was in the log, whichever quest the comment is on.
    const claims = [claim("breadcrumbs", "breadcrumbForQuestId", 0.9, 16), claim("absence", "breadcrumbs", 0.9, 3)];
    const fromBreadcrumb = edge({ kind: "breadcrumb", field: "breadcrumbForQuestId", target: 276, claims, contributions: [{ signal: "breadcrumbs", logOdds: 2 }] });
    const fromTarget = edge({ kind: "breadcrumb", field: "breadcrumbs", target: 463, claims, contributions: [{ signal: "absence", logOdds: 2 }] });

    expect(assumption(view(463, { breadcrumbForQuestId: [276] }, [fromBreadcrumb]), "breadcrumbForQuestId")).toBe(
      "Assumption: optional lead-in to 276 (16 characters), 2 signals agree, score 0.990",
    );
    expect(assumption(view(276, { breadcrumbs: [463] }, [fromTarget]), "breadcrumbs")).toBe(
      "Assumption: 276 left off offer lists while 463 was in the log (3 characters), 2 signals agree, score 0.990",
    );
  });
});
