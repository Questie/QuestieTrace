// Per-quest views of the resolution: every edge as seen from each quest it writes to (a
// breadcrumb is breadcrumbForQuestId on one side and breadcrumbs on the other), with its status
// against the quest's reference data. relations.json and the review both render these.
//
// Context-only claims (Wowhead, see inputs.ts) ride along for reviewers: on the edges trace signals
// claim, and as hints on Forever-new quests where only Wowhead claims something. They are never
// part of an edge's score or outcome.

import type { CatalogQuest, RelationField, RelationSet } from "../core/types";
import type { Contribution, ScoredEdge } from "./crossval";
import type { Claim, Domain, Edge, EdgeKind, QuestDomains } from "./edges";
import type { Outcome, Resolution } from "./resolve";

/**
 * Against the quest's reference relations (authored for Forever-new quests, inherited for Classic):
 * - agree: the reference has this target in this field.
 * - new: the reference has nothing in this field (prerequisites: in either list; always the case without reference data).
 * - conflict: the reference has other targets in this field.
 */
export type Status = "agree" | "new" | "conflict";

export interface EdgeView {
  key: string;
  kind: EdgeKind;
  field: RelationField;
  target: number;
  probability: number;
  heldOut: boolean;
  outcome: Outcome;
  because?: string;
  /** Only for edges written to the RelationSet. */
  status?: Status;
  claims: Claim[];
  /** Context-only claims on the same relationship (untrusted, never scored). */
  context: Claim[];
  contributions: Contribution[];
}

/** A relationship only context-only sources claim. */
export interface HintView {
  field: RelationField;
  target: number;
  claims: Claim[];
}

export interface QuestView {
  questId: number;
  name?: string;
  domain: Domain;
  /** Which reference data the quest has: authored (Forever-new), inherited (Classic) or none. */
  reference: "authored" | "inherited" | "none";
  relations: RelationSet;
  edges: EdgeView[];
  /** Reference edges the resolution did not write. */
  referenceOnly: Array<{ field: RelationField; target: number }>;
  /** Forever-new quests only: what context-only sources claim that no trace signal does. */
  hints: HintView[];
  notes: string[];
}

const PREREQUISITE_FIELDS: readonly RelationField[] = ["preQuestSingle", "preQuestGroup"];

function perspectives(edge: { kind: EdgeKind; quest: number; target: number }, relations: ReadonlyMap<number, RelationSet>) {
  switch (edge.kind) {
    case "prerequisite": {
      const field: RelationField = relations.get(edge.quest)?.preQuestGroup?.includes(edge.target) ? "preQuestGroup" : "preQuestSingle";
      return [{ questId: edge.quest, field, target: edge.target }];
    }
    case "next":
      return [{ questId: edge.quest, field: "nextQuestInChain" as const, target: edge.target }];
    case "breadcrumb":
      return [
        { questId: edge.quest, field: "breadcrumbForQuestId" as const, target: edge.target },
        { questId: edge.target, field: "breadcrumbs" as const, target: edge.quest },
      ];
    case "exclusive":
      return [
        { questId: edge.quest, field: "exclusiveTo" as const, target: edge.target },
        { questId: edge.target, field: "exclusiveTo" as const, target: edge.quest },
      ];
    default:
      return [{ questId: edge.quest, field: edge.kind as RelationField, target: edge.target }];
  }
}

function statusOf(field: RelationField, target: number, reference: RelationSet | undefined): Status {
  // The two prerequisite fields are one slot (Questie ignores preQuestGroup when preQuestSingle is
  // set), so the same target under the other list kind is a conflict, not agreement.
  const slot = PREREQUISITE_FIELDS.includes(field) ? PREREQUISITE_FIELDS : [field];
  if (slot.every((name) => (reference?.[name] ?? []).length === 0)) return "new";
  return (reference?.[field] ?? []).includes(target) ? "agree" : "conflict";
}

export function buildQuestViews(
  scored: readonly ScoredEdge[],
  resolution: Resolution,
  context: ReadonlyMap<string, Edge>,
  truth: ReadonlyMap<number, RelationSet>,
  domains: QuestDomains,
  quests: Readonly<Record<string, CatalogQuest>>,
  minShown: number,
): Map<number, QuestView> {
  const views = new Map<number, QuestView>();
  const viewOf = (questId: number): QuestView => {
    let view = views.get(questId);
    if (!view) {
      view = {
        questId,
        name: quests[String(questId)]?.name,
        domain: domains.isForeverNew(questId) ? "forever" : "classic",
        reference: domains.referenceOf(questId),
        relations: resolution.relations.get(questId) ?? {},
        edges: [],
        referenceOnly: [],
        hints: [],
        notes: resolution.notes.get(questId) ?? [],
      };
      views.set(questId, view);
    }
    return view;
  };

  const derived: ScoredEdge[] = resolution.derived.map((edge) => ({
    ...edge,
    claims: [],
    heldOut: false,
    contributions: [],
  }));
  const probabilityOf = new Map(scored.map((edge) => [edge.key, edge.probability]));
  for (const edge of [...scored, ...derived]) {
    const decision = resolution.decisions.get(edge.key);
    if (!decision) continue;
    const written = decision.outcome === "accepted" || decision.outcome === "clique" || decision.outcome === "derived";
    if (!written && edge.probability < minShown) continue;
    // A chain implied by a breadcrumb is as certain as the breadcrumb, whatever its own evidence says.
    const probability = decision.outcome === "derived" ? Math.max(edge.probability, probabilityOf.get(decision.because!) ?? 0) : edge.probability;
    for (const { questId, field, target } of perspectives(edge, resolution.relations)) {
      viewOf(questId).edges.push({
        key: edge.key,
        kind: edge.kind,
        field,
        target,
        probability,
        heldOut: edge.heldOut,
        outcome: decision.outcome,
        because: decision.because,
        status: written ? statusOf(field, target, truth.get(questId)) : undefined,
        claims: edge.claims,
        context: context.get(edge.key)?.claims ?? [],
        contributions: edge.contributions,
      });
    }
  }

  const scoredKeys = new Set([...scored, ...derived].map((edge) => edge.key));
  for (const edge of context.values()) {
    if (scoredKeys.has(edge.key)) continue;
    for (const { questId, field, target } of perspectives(edge, resolution.relations)) {
      if (domains.isForeverNew(questId)) viewOf(questId).hints.push({ field, target, claims: edge.claims });
    }
  }

  // Authored quests the resolution says nothing about still get a view, for the authored-only list.
  for (const questId of truth.keys()) if (domains.referenceOf(questId) === "authored") viewOf(questId);
  for (const view of views.values()) {
    view.edges.sort((a, b) => b.probability - a.probability);
    const reference = truth.get(view.questId);
    if (!reference) continue;
    for (const [field, targets] of Object.entries(reference) as Array<[RelationField, number[]]>) {
      const writtenFields = PREREQUISITE_FIELDS.includes(field) ? PREREQUISITE_FIELDS : [field];
      const written = new Set(writtenFields.flatMap((name) => view.relations[name] ?? []));
      for (const target of targets) if (!written.has(target)) view.referenceOnly.push({ field, target });
    }
  }
  return views;
}
