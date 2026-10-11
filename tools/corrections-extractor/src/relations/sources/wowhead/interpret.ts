// Turns the relation surfaces of one parsed Wowhead quest page (./parse.ts) into claims about
// Questie relation fields (semantics on RELATION_FIELDS in ../../core/types.ts).
//
// Mapping, per surface:
//
//   series                Adjacent non-empty rows P -> N. For each quest b in N, its compatible
//                         members of P become b.preQuestSingle. Wowhead does not say AND or OR for
//                         several different quests in one row; Questie's data almost never uses
//                         preQuestGroup for them, so they are preQuestSingle with shape "group".
//                         For each a in P with exactly one compatible quest b in N:
//                         a.nextQuestInChain = b.
//                         When P is the first row and its quests have no objectives, a is often a
//                         breadcrumb rather than a prerequisite (shape "lead-in"): such edges also
//                         yield a.breadcrumbForQuestId = b and b.breadcrumbs = a.
//   requires              On page Q: Q.preQuestSingle, or preQuestGroup when the listed quests (other
//                         than Q itself) have different names.
//   requires-any          On page Q: Q.preQuestSingle for each listed quest except Q itself.
//   requires-in-progress  On page Q: Q.parentQuest (Q needs the listed quest in the log).
//   unlocks               On page Q: X.preQuestSingle = Q for each listed X except Q itself.
//   disables              On page Q: X.exclusiveTo = Q for each listed X except Q itself
//                         (taking Q makes X unavailable; Questie needs that edge on X).
//   storyline             Adjacent steps (runs of same-name quests are one step): next.preQuestSingle = prev.
//
// "Compatible" drops pairs no character can do both of (opposite factions, disjoint race or class
// masks). Series rows merge parallel race/class chains, so this matters a lot there.
// Scores live in ./scores.ts, keyed by surface, shape and provenance.

import type { RelationField } from "../../core/types";
import type { MarkupPanel, QuestPageFacts, QuestRecord, SeriesMember } from "./parse";

export type Surface = "series" | "requires" | "requires-any" | "requires-in-progress" | "unlocks" | "disables" | "storyline";

export const SURFACES: readonly Surface[] = ["series", "requires", "requires-any", "requires-in-progress", "unlocks", "disables", "storyline"];

/**
 * How the claim's source structure looked, which decides how far to trust its field mapping.
 * - single: exactly one quest on the other side of the edge.
 * - variants: several same-name quests (race/faction/class variants of one step).
 * - group: several different quests (AND or OR is not shown).
 * - with-self: a list that includes the page's own quest (repeatable turn-in groups).
 * - lead-in: Series first-row quests without objectives ("go talk to X"), which are as often
 *   breadcrumbs as prerequisites.
 */
export type ClaimShape = "single" | "variants" | "group" | "with-self" | "lead-in";

export interface Claim {
  surface: Surface;
  questId: number;
  field: RelationField;
  target: number;
  shape: ClaimShape;
  /** The page that showed it. */
  pageId: number;
  /** For series claims: the ordered pair (before, after) of adjacent rows the claim was read from. */
  adjacency?: [number, number];
  /** One line a reviewer can check against the page. */
  text: string;
}

const PANEL_SURFACES: Array<[MarkupPanel, Surface]> = [
  ["Requires", "requires"],
  ["Requires Any", "requires-any"],
  ["Requires In Progress", "requires-in-progress"],
  ["Unlocks", "unlocks"],
  ["Disables", "disables"],
];

/** Quest facts by id. Pass one merged across all pages; a page alone often lacks its neighbours' restrictions. */
export type QuestLookup = ReadonlyMap<number, QuestRecord>;

export function claimsFromPage(page: QuestPageFacts, quests: QuestLookup = page.quests): Claim[] {
  const context: Context = { page, quests };
  const claims: Claim[] = [...seriesClaims(context), ...storylineClaims(context)];
  for (const [panel, surface] of PANEL_SURFACES) {
    const ids = page.panels[panel];
    if (ids && ids.length > 0) claims.push(...panelClaims(context, surface, panel, ids));
  }
  return claims;
}

interface Context {
  page: QuestPageFacts;
  quests: QuestLookup;
}

// --------------------------------------------------------------------------- Series

function seriesClaims(context: Context): Claim[] {
  const { page } = context;
  const claims: Claim[] = [];
  const rows = page.series;
  for (let i = 0; i + 1 < rows.length; i++) {
    const prev = rows[i].members;
    const next = rows[i + 1].members;
    if (prev.length === 0 || next.length === 0) continue;
    const firstRow = rows.slice(0, i).every((row) => row.members.length === 0);
    const rowText = `Series row ${rows[i].index} [${describeMembers(context, prev)}] -> row ${rows[i + 1].index} [${describeMembers(context, next)}]`;
    const claim = (questId: number, field: RelationField, target: number, shape: ClaimShape, adjacency: [number, number]): Claim => ({
      surface: "series",
      questId,
      field,
      target,
      shape,
      pageId: page.questId,
      adjacency,
      text: rowText,
    });

    for (const b of next) {
      const before = prev.filter((a) => a.questId !== b.questId && compatible(context, a.questId, b.questId)).map((a) => a.questId);
      if (before.length === 0) continue;
      const leadIn = firstRow && before.every((a) => context.quests.get(a)?.hasObjectives === false);
      const shape: ClaimShape = leadIn ? "lead-in" : before.length === 1 ? "single" : sameName(context, before) ? "variants" : "group";
      for (const a of before) claims.push(claim(b.questId, "preQuestSingle", a, shape, [a, b.questId]));
    }

    for (const a of prev) {
      const after = next.filter((b) => b.questId !== a.questId && compatible(context, a.questId, b.questId)).map((b) => b.questId);
      if (after.length !== 1) continue;
      const b = after[0];
      const leadIn = firstRow && context.quests.get(a.questId)?.hasObjectives === false;
      claims.push(claim(a.questId, "nextQuestInChain", b, leadIn ? "lead-in" : "single", [a.questId, b]));
      if (leadIn) {
        claims.push(claim(a.questId, "breadcrumbForQuestId", b, "lead-in", [a.questId, b]));
        claims.push(claim(b, "breadcrumbs", a.questId, "lead-in", [a.questId, b]));
      }
    }
  }
  return claims;
}

function describeMembers(context: Context, members: SeriesMember[]): string {
  return members.map((member) => questLabel(context, member.questId)).join(" | ");
}

// --------------------------------------------------------------------------- Storyline

function storylineClaims(context: Context): Claim[] {
  const storyline = context.page.storyline;
  if (!storyline) return [];

  // Consecutive same-name quests are variants of one step (e.g. 92750/92751), not a sequence.
  const steps: number[][] = [];
  for (const id of storyline.questIds) {
    const last = steps.at(-1);
    if (last && sameName(context, [last[0], id])) last.push(id);
    else steps.push([id]);
  }

  const claims: Claim[] = [];
  for (let i = 0; i + 1 < steps.length; i++) {
    const describe = (step: number[]) => step.map((id) => questLabel(context, id)).join(" | ");
    const text = `Storyline ${storyline.id} "${storyline.name}" step ${i + 1} [${describe(steps[i])}] -> step ${i + 2} [${describe(steps[i + 1])}]`;
    for (const b of steps[i + 1]) {
      const before = steps[i].filter((a) => compatible(context, a, b));
      const shape: ClaimShape = before.length === 1 ? "single" : "variants";
      for (const a of before) claims.push({ surface: "storyline", questId: b, field: "preQuestSingle", target: a, shape, pageId: context.page.questId, text });
    }
  }
  return claims;
}

// --------------------------------------------------------------------------- Markup panels

function panelClaims(context: Context, surface: Surface, panel: MarkupPanel, ids: number[]): Claim[] {
  const self = context.page.questId;
  const others = ids.filter((id) => id !== self);
  const withSelf = others.length < ids.length;
  const differentQuests = others.length > 1 && !sameName(context, others);
  const listShape: ClaimShape = withSelf ? "with-self" : others.length === 1 ? "single" : differentQuests ? "group" : "variants";
  const text = `${panel}: ${ids.map((id) => questLabel(context, id)).join(", ")}`;
  const claim = (questId: number, field: RelationField, target: number): Claim => ({ surface, questId, field, target, shape: listShape, pageId: self, text });

  switch (surface) {
    case "requires":
      return others.map((id) => claim(self, differentQuests ? "preQuestGroup" : "preQuestSingle", id));
    case "requires-any":
      return others.map((id) => claim(self, "preQuestSingle", id));
    case "requires-in-progress":
      return others.map((id) => claim(self, "parentQuest", id));
    case "unlocks":
      return others.map((id) => claim(id, "preQuestSingle", self));
    case "disables":
      return others.map((id) => claim(id, "exclusiveTo", self));
    default:
      return [];
  }
}

// --------------------------------------------------------------------------- Helpers

/** False when no character could do both quests: opposite factions, or disjoint race or class masks. */
function compatible(context: Context, a: number, b: number): boolean {
  const qa = context.quests.get(a);
  const qb = context.quests.get(b);
  if (!qa || !qb) return true;
  if ((qa.side === 1 && qb.side === 2) || (qa.side === 2 && qb.side === 1)) return false;
  if (qa.races !== undefined && qb.races !== undefined && (qa.races & qb.races) === 0n) return false;
  if (qa.classes !== undefined && qb.classes !== undefined && (qa.classes & qb.classes) === 0) return false;
  return true;
}

/** True when every quest has a known name and all names are equal. */
function sameName(context: Context, ids: number[]): boolean {
  const names = ids.map((id) => context.quests.get(id)?.name);
  return names.every((name) => name !== undefined && name === names[0]);
}

function questLabel(context: Context, id: number): string {
  const name = context.quests.get(id)?.name;
  return name === undefined ? String(id) : `${id} ${name}`;
}
