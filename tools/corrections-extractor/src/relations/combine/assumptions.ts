// The `-- Assumption:` comment on each field of foreverQuestRelationTraces.lua: why a written
// relation is believed, in one line a QuestieDB maintainer can read without the review report.
//
// Per target it names the strongest trace observation (the claim whose signal added the most
// log-odds), the independent characters behind it, how many trace signals agree, and the combined
// score. Targets whose reasons differ only by id share one, so a large exclusive group stays one
// readable line. Wowhead never appears: it is context only (see inputs.ts). relations-review.md has
// the full evidence for every relation.

import type { RelationField } from "../core/types";
import type { Claim } from "./edges";
import { isWritten, type EdgeView, type QuestView } from "./views";

/** What one signal saw, keyed by signal name. The commented quest is "it"; `other` is the quest (or quests) in the field. */
type Phrases = Partial<Record<string, (other: string) => string>>;

const PREREQUISITE: Phrases = {
  handoff: (p) => `offered right after ${p} was turned in`,
  "offer-set": (p) => `${p} was done whenever it was offered`,
  cooccurrence: (p) => `everyone who took it had done ${p}`,
  absence: (p) => `left off complete offer lists until ${p} was done`,
};

/** Keyed by the field the comment sits on, so both sides of a breadcrumb read naturally. */
const PHRASES: Partial<Record<RelationField, Phrases>> = {
  preQuestSingle: PREREQUISITE,
  preQuestGroup: PREREQUISITE,
  nextQuestInChain: {
    handoff: (n) => `turning it in led straight to ${n}`,
    absence: (n) => `left off offer lists once ${n} was taken or done`,
  },
  exclusiveTo: {
    cooccurrence: (x) => `the server completes it and ${x} together`,
    absence: (x) => `offer lists show it and ${x} exclude each other`,
  },
  breadcrumbForQuestId: { breadcrumbs: (t) => `optional lead-in to ${t}` },
  breadcrumbs: { breadcrumbs: (b) => `${b} is an optional lead-in to it` },
  availableUntilCompleted: { absence: (x) => `left off offer lists once ${x} was done` },
  disabledByQuest: { absence: (x) => `left off offer lists while ${x} was in the log` },
};

function observation(claim: Claim, edge: EdgeView, questId: number, other: string): string {
  // Absence sees a breadcrumb B -> T from one side only, and the claim's field says which: B hidden
  // once T is taken (breadcrumbForQuestId) or T hidden while B is in the log (breadcrumbs).
  if (edge.kind === "breadcrumb" && claim.signal === "absence") {
    const [b, t] = edge.field === "breadcrumbForQuestId" ? [questId, other] : [other, questId];
    return claim.field === "breadcrumbs" ? `${t} left off offer lists while ${b} was in the log` : `${b} left off offer lists once ${t} was taken or done`;
  }
  return PHRASES[edge.field]?.[claim.signal]?.(other) ?? `${claim.signal} signal`;
}

/** The claim whose signal added the most log-odds; score, then support, break ties and cover edges without contributions. */
function strongest(edge: EdgeView): Claim | undefined {
  const logOdds = (claim: Claim) => edge.contributions.find((term) => term.signal === claim.signal)?.logOdds ?? -Infinity;
  return [...edge.claims].sort((a, b) => logOdds(b) - logOdds(a) || b.score - a.score || b.support - a.support)[0];
}

function characters(support: number): string {
  // Support 0: only observations a signal could not pin to one independent character.
  return support === 0 ? "no independent character" : `${support} character${support === 1 ? "" : "s"}`;
}

function reason(edge: EdgeView, questId: number, other: string): string {
  const score = `score ${edge.probability.toFixed(3)}`;
  if (edge.outcome === "derived") return `implied by its breadcrumb to ${other}, ${score}`;
  const claim = strongest(edge);
  if (!claim) return score;
  const parts = [`${observation(claim, edge, questId, other)} (${characters(claim.support)})`];
  const signals = new Set(edge.claims.map((each) => each.signal)).size;
  if (signals > 1) parts.push(`${signals} signals agree`);
  parts.push(score);
  if (edge.outcome === "clique") parts.push("kept below the min score for its accepted exclusive group");
  return parts.join(", ");
}

/** One field's comment: a reason per target, in the order the field lists them; undefined without written edges. */
export function assumption(view: QuestView, field: RelationField): string | undefined {
  // Keyed by the reason with the target left out, so identical reasons merge into one.
  const groups = new Map<string, { edge: EdgeView; targets: number[] }>();
  for (const target of view.relations[field] ?? []) {
    const edge = view.edges.find((each) => isWritten(each) && each.field === field && each.target === target);
    if (!edge) continue;
    const key = reason(edge, view.questId, "#");
    const group = groups.get(key);
    if (group) group.targets.push(target);
    else groups.set(key, { edge, targets: [target] });
  }
  if (groups.size === 0) return undefined;
  const reasons = [...groups.values()].map(({ edge, targets }) => reason(edge, view.questId, targets.join(", ")));
  return `Assumption: ${reasons.join("; ")}`;
}
