// Turns scored edges into one consistent RelationSet per quest. Every edge ends with exactly one
// Decision, so the review can say why anything with evidence was left out.
//
// Order matters, and follows how much each rule can be trusted:
//   1. threshold: edges at or above minScore are accepted, the rest wait. Edges touching a quest the
//      catalog does not know are never written: Questie has no such quest, and a prerequisite on
//      one would hide the quest for good if it cannot be completed in Forever.
//   2. breadcrumbs: one target per breadcrumb; a breadcrumb B -> T replaces any "T needs B" and any
//      exclusivity between B and T, and between their same-name copies, and implies
//      nextQuestInChain(B) = T (every breadcrumbForQuestId in Questie's Classic and Forever data
//      agrees, 214 of 215). Copies are parallel variants with the same structure, so a copy of B is
//      a breadcrumb for T's copies too, never their prerequisite. Where Forever completes all copies
//      at once (not everywhere: faction copies complete separately), prerequisite signals otherwise
//      see B's copies as prerequisites of every copy of T. (On the 2026-10 data this rule dropped
//      107 edges, 106 of them wrong by the ground truth.)
//   3. scalar fields: one target per quest, runners-up recorded.
//   4. ordering cycles: prerequisites, chains and breadcrumbs all order quests; the weakest edge of
//      each cycle goes. This also settles "B needs T" against a breadcrumb B -> T by score.
//   5. a quest is never exclusive with a quest it comes before or after (directly or through a
//      chain): the weakest edge of the pair and that path goes. Otherwise the later quest could
//      never be taken.
//   6. exclusive groups: a pair below the threshold is still accepted when both quests are already
//      in one accepted group, the pair has evidence of its own, and rule 5 allows it; nothing is
//      inferred from transitivity alone.
//   7. transitive reduction: X needing P2 and P1, where P2 needs P1, keeps only P2, unless the
//      direct edge to P1 is the stronger one.
//   8. OR vs AND: see chooseGroupKind.

import { SCALAR_RELATION_FIELDS, type RelationField, type RelationSet } from "../core/types";
import { edgeKey, type Claim, type EdgeKind } from "./edges";

export interface ResolvableEdge {
  key: string;
  kind: EdgeKind;
  quest: number;
  target: number;
  probability: number;
  claims: ReadonlyArray<Pick<Claim, "source" | "field">>;
}

export interface ResolveOptions {
  minScore: number;
  /** Whether the catalog (Questie's Forever data) knows the quest. */
  knownQuest: (questId: number) => boolean;
  /** Lower bar for an exclusive pair inside a group whose other pairs were accepted (rule 6). */
  cliqueMinScore: number;
  /** Same-name copies share a family; a list of copies of one quest is always an OR list. */
  familyOf: (questId: number) => string;
  /** Sources whose preQuestGroup claims count as evidence that a quest's prerequisites are ALL required. */
  andSourcesFor: (questId: number) => ReadonlySet<string>;
}

/**
 * - accepted / clique / derived: written to the RelationSet (clique: rule 6; derived: implied by
 *   an accepted breadcrumb, see `because`).
 * - below-threshold: not enough evidence.
 * - unknown-quest: one side is missing from the catalog.
 * - runner-up: a scalar field already took a stronger target (`because`).
 * - breadcrumb: replaced by the breadcrumb `because`.
 * - cycle, conflict, transitive: dropped by rules 4, 5 and 7, in favour of `because`.
 */
export type Outcome =
  "accepted" | "clique" | "derived" | "below-threshold" | "unknown-quest" | "runner-up" | "breadcrumb" | "cycle" | "conflict" | "transitive";

export interface Decision {
  outcome: Outcome;
  /** Key of the edge that caused the outcome. */
  because?: string;
}

export interface Resolution {
  relations: Map<number, RelationSet>;
  /** One decision per input edge key, plus one per derived edge. */
  decisions: Map<string, Decision>;
  /** nextQuestInChain edges implied by breadcrumbs that no signal claimed. */
  derived: ResolvableEdge[];
  /** Per-quest notes on OR vs AND, for the review. */
  notes: Map<number, string[]>;
}

const WRITTEN: ReadonlySet<Outcome> = new Set<Outcome>(["accepted", "clique", "derived"]);

const SCALAR_KINDS: readonly EdgeKind[] = ["next", "parentQuest", "availableUntilCompleted", "availableStartingWith", "disabledByQuest"];

const ORDERING_KINDS: readonly EdgeKind[] = ["prerequisite", "next", "breadcrumb"];

/** Ordering edges point from the earlier quest to the later one. */
function orderingArc(edge: ResolvableEdge): [number, number] {
  return edge.kind === "prerequisite" ? [edge.target, edge.quest] : [edge.quest, edge.target];
}

export function resolveRelations(edges: readonly ResolvableEdge[], options: ResolveOptions): Resolution {
  const byKey = new Map(edges.map((edge) => [edge.key, edge]));
  const decisions = new Map<string, Decision>();
  const notes = new Map<number, string[]>();
  const derived: ResolvableEdge[] = [];
  const isLive = (key: string) => WRITTEN.has(decisions.get(key)?.outcome ?? "below-threshold");
  const live = (...kinds: EdgeKind[]) =>
    [...byKey.values(), ...derived].filter((edge) => (kinds.length === 0 || kinds.includes(edge.kind)) && isLive(edge.key));
  const drop = (edge: ResolvableEdge, outcome: Outcome, because: ResolvableEdge) => decisions.set(edge.key, { outcome, because: because.key });
  const stronger = (a: ResolvableEdge, b: ResolvableEdge) => (a.probability !== b.probability ? a.probability > b.probability : a.key < b.key);
  const weakestOf = (list: ResolvableEdge[]) => list.reduce((a, b) => (stronger(a, b) ? b : a));
  const strongestOf = (list: ResolvableEdge[]) => list.reduce((a, b) => (stronger(a, b) ? a : b));

  // 1. threshold
  for (const edge of edges) {
    const known = options.knownQuest(edge.quest) && options.knownQuest(edge.target);
    const outcome: Outcome = !known ? "unknown-quest" : edge.probability >= options.minScore ? "accepted" : "below-threshold";
    decisions.set(edge.key, { outcome });
  }

  // 2. breadcrumbs
  const betweenFamilies = new Map<string, ResolvableEdge[]>();
  const familyKey = (kind: EdgeKind, from: number, to: number) => `${kind}|${options.familyOf(from)}|${options.familyOf(to)}`;
  for (const edge of edges) {
    const keys = [familyKey(edge.kind, edge.quest, edge.target)];
    if (edge.kind === "exclusive") keys.push(familyKey(edge.kind, edge.target, edge.quest));
    for (const key of keys) betweenFamilies.set(key, [...(betweenFamilies.get(key) ?? []), edge]);
  }
  keepBestPerQuest(live("breadcrumb"), stronger, drop);
  for (const breadcrumb of live("breadcrumb")) {
    // The breadcrumb's implied chain competes with any other chain claimed for the same quest.
    const rivals = live("next").filter((next) => next.quest === breadcrumb.quest && next.target !== breadcrumb.target);
    if (rivals.length > 0 && stronger(strongestOf(rivals), breadcrumb)) {
      drop(breadcrumb, "runner-up", strongestOf(rivals));
      continue;
    }
    for (const rival of rivals) drop(rival, "runner-up", breadcrumb);
    const [b, t] = [breadcrumb.quest, breadcrumb.target];
    // Some chains reuse one name for every step, so copies only count when the two names differ.
    const sameName = options.familyOf(b) === options.familyOf(t);
    const explained: Array<[EdgeKind, number, number, string]> = [
      ["prerequisite", t, b, edgeKey("prerequisite", t, b)],
      ["exclusive", b, t, edgeKey("exclusive", Math.min(b, t), Math.max(b, t))],
    ];
    for (const [kind, from, to, exact] of explained) {
      const replaced = sameName ? [byKey.get(exact)] : [byKey.get(exact), ...(betweenFamilies.get(familyKey(kind, from, to)) ?? [])];
      for (const edge of replaced) if (edge && decisions.get(edge.key)?.outcome !== "unknown-quest") drop(edge, "breadcrumb", breadcrumb);
    }
    const nextKey = edgeKey("next", b, t);
    if (!byKey.has(nextKey)) derived.push({ key: nextKey, kind: "next", quest: b, target: t, probability: breadcrumb.probability, claims: [] });
    if (!isLive(nextKey)) decisions.set(nextKey, { outcome: "derived", because: breadcrumb.key });
  }

  // 3. scalar fields (a breadcrumb's implied chain already beat its rivals in step 2)
  for (const kind of SCALAR_KINDS) keepBestPerQuest(live(kind), stronger, drop);

  // 4. ordering cycles
  for (let cycle = findCycle(live(...ORDERING_KINDS)); cycle; cycle = findCycle(live(...ORDERING_KINDS))) {
    drop(weakestOf(cycle), "cycle", strongestOf(cycle));
  }

  // 5. exclusive vs ordering
  const ordering = new Map<number, ResolvableEdge[]>();
  for (const edge of live(...ORDERING_KINDS)) {
    const [from] = orderingArc(edge);
    ordering.set(from, [...(ordering.get(from) ?? []), edge]);
  }
  const pathBetween = (a: number, b: number) => findPath(a, b, ordering, isLive) ?? findPath(b, a, ordering, isLive);
  for (const pair of live("exclusive").sort((a, b) => b.probability - a.probability || a.key.localeCompare(b.key))) {
    for (let path = pathBetween(pair.quest, pair.target); path && isLive(pair.key); path = pathBetween(pair.quest, pair.target)) {
      drop(weakestOf([pair, ...path]), "conflict", strongestOf([pair, ...path]));
    }
  }

  // 6. exclusive groups
  const group = new UnionFind();
  for (const pair of live("exclusive")) group.union(pair.quest, pair.target);
  for (const pair of edges) {
    if (pair.kind !== "exclusive" || decisions.get(pair.key)?.outcome !== "below-threshold" || pair.probability < options.cliqueMinScore) continue;
    if (group.find(pair.quest) === group.find(pair.target) && !pathBetween(pair.quest, pair.target)) decisions.set(pair.key, { outcome: "clique" });
  }

  // 7. transitive reduction
  const direct = new Map<number, ResolvableEdge[]>();
  for (const edge of live("prerequisite")) direct.set(edge.quest, [...(direct.get(edge.quest) ?? []), edge]);
  const reach = reachability(direct);
  for (const list of direct.values()) {
    for (const edge of list) {
      const via = list.find((other) => other !== edge && !stronger(edge, other) && reach(other.target).has(edge.target));
      if (via) drop(edge, "transitive", via);
    }
  }

  // 8. OR vs AND, then the RelationSets
  const relations = new Map<number, RelationSet>();
  const add = (questId: number, field: RelationField, value: number) => {
    const set = relations.get(questId) ?? {};
    const values = set[field] ?? [];
    if (!values.includes(value)) values.push(value);
    set[field] = values.sort((a, b) => a - b);
    relations.set(questId, set);
  };
  const prerequisitesOf = new Map<number, ResolvableEdge[]>();
  for (const edge of live("prerequisite")) prerequisitesOf.set(edge.quest, [...(prerequisitesOf.get(edge.quest) ?? []), edge]);
  for (const [questId, list] of prerequisitesOf) {
    const { field, note } = chooseGroupKind(list, options.familyOf, options.andSourcesFor(questId));
    if (note) notes.set(questId, [note]);
    for (const edge of list) add(questId, field, edge.target);
  }
  for (const edge of live()) {
    if (edge.kind === "prerequisite") continue;
    if (edge.kind === "exclusive") {
      add(edge.quest, "exclusiveTo", edge.target);
      add(edge.target, "exclusiveTo", edge.quest);
    } else if (edge.kind === "breadcrumb") {
      add(edge.quest, "breadcrumbForQuestId", edge.target);
      add(edge.target, "breadcrumbs", edge.quest);
    } else {
      add(edge.quest, edge.kind === "next" ? "nextQuestInChain" : edge.kind, edge.target);
    }
  }
  for (const set of relations.values()) {
    for (const field of Object.keys(set) as RelationField[]) {
      if (SCALAR_RELATION_FIELDS.has(field) && set[field]!.length > 1) throw new Error(`resolve: scalar ${field} got ${set[field]!.join(",")}`);
    }
  }
  return { relations, decisions, derived, notes };
}

/**
 * Prerequisites are an OR list (preQuestSingle) unless every distinct prerequisite (counting
 * same-name copies as one) has a preQuestGroup claim from a source trusted for AND. Measured on
 * the 11 judged quests with two or more accepted prerequisite families (2026-10): this rule got
 * all 11 right, "always OR" 4, "always AND" 7. OR is also the safer mistake: it shows a quest
 * early instead of hiding it.
 *
 * AND over a family with several copies is kept as OR: preQuestGroup would demand every copy,
 * and the copies are usually faction variants nobody can all complete.
 */
export function chooseGroupKind(
  prerequisites: readonly ResolvableEdge[],
  familyOf: (questId: number) => string,
  andSources: ReadonlySet<string>,
): { field: "preQuestSingle" | "preQuestGroup"; note?: string } {
  const families = new Map<string, ResolvableEdge[]>();
  for (const edge of prerequisites) {
    const family = familyOf(edge.target);
    families.set(family, [...(families.get(family) ?? []), edge]);
  }
  if (families.size < 2) return { field: "preQuestSingle" };
  const allVoteAnd = [...families.values()].every((members) =>
    members.some((edge) => edge.claims.some((claim) => claim.field === "preQuestGroup" && andSources.has(claim.source))),
  );
  if (!allVoteAnd) return { field: "preQuestSingle", note: `${families.size} distinct prerequisites without AND evidence for each: kept as OR` };
  if ([...families.values()].some((members) => members.length > 1)) {
    return { field: "preQuestSingle", note: "AND evidence, but a prerequisite has same-name copies: kept as OR" };
  }
  return { field: "preQuestGroup", note: `AND: each of ${families.size} prerequisites has preQuestGroup evidence` };
}

function keepBestPerQuest(
  edges: ResolvableEdge[],
  stronger: (a: ResolvableEdge, b: ResolvableEdge) => boolean,
  drop: (edge: ResolvableEdge, outcome: Outcome, because: ResolvableEdge) => void,
): void {
  const best = new Map<number, ResolvableEdge>();
  for (const edge of edges) {
    const current = best.get(edge.quest);
    if (!current || stronger(edge, current)) best.set(edge.quest, edge);
  }
  for (const edge of edges) if (best.get(edge.quest) !== edge) drop(edge, "runner-up", best.get(edge.quest)!);
}

/** One cycle among the ordering edges, as its edges, or undefined when they form a DAG. */
function findCycle(edges: ResolvableEdge[]): ResolvableEdge[] | undefined {
  const outgoing = new Map<number, ResolvableEdge[]>();
  for (const edge of edges) {
    const [from] = orderingArc(edge);
    outgoing.set(from, [...(outgoing.get(from) ?? []), edge]);
  }
  const state = new Map<number, "open" | "done">();
  // path[i] leads from pathNodes[i] to pathNodes[i + 1].
  const path: ResolvableEdge[] = [];
  const pathNodes: number[] = [];
  const visit = (node: number): ResolvableEdge[] | undefined => {
    state.set(node, "open");
    pathNodes.push(node);
    for (const edge of outgoing.get(node) ?? []) {
      const [, to] = orderingArc(edge);
      if (state.get(to) === "open") return [...path.slice(pathNodes.indexOf(to)), edge];
      if (state.has(to)) continue;
      path.push(edge);
      const found = visit(to);
      if (found) return found;
      path.pop();
    }
    pathNodes.pop();
    state.set(node, "done");
    return undefined;
  };
  for (const node of outgoing.keys()) {
    if (state.has(node)) continue;
    const found = visit(node);
    if (found) return found;
  }
  return undefined;
}

/** Edges of a path from `from` to `to` over live ordering edges (breadth-first), if there is one. */
function findPath(from: number, to: number, outgoing: ReadonlyMap<number, ResolvableEdge[]>, isLive: (key: string) => boolean): ResolvableEdge[] | undefined {
  const cameBy = new Map<number, ResolvableEdge | undefined>([[from, undefined]]);
  const queue = [from];
  while (queue.length > 0) {
    const node = queue.shift()!;
    if (node === to) {
      const path: ResolvableEdge[] = [];
      for (let edge = cameBy.get(to); edge; edge = cameBy.get(orderingArc(edge)[0])) path.unshift(edge);
      return path;
    }
    for (const edge of outgoing.get(node) ?? []) {
      const [, next] = orderingArc(edge);
      if (!isLive(edge.key) || cameBy.has(next)) continue;
      cameBy.set(next, edge);
      queue.push(next);
    }
  }
  return undefined;
}

/** Everything a quest transitively needs, over an acyclic prerequisite graph. */
function reachability(direct: ReadonlyMap<number, ResolvableEdge[]>): (questId: number) => Set<number> {
  const memo = new Map<number, Set<number>>();
  const reach = (questId: number): Set<number> => {
    const known = memo.get(questId);
    if (known) return known;
    const result = new Set<number>();
    memo.set(questId, result);
    for (const edge of direct.get(questId) ?? []) {
      result.add(edge.target);
      for (const further of reach(edge.target)) result.add(further);
    }
    return result;
  };
  return reach;
}

class UnionFind {
  private readonly parent = new Map<number, number>();

  find(x: number): number {
    const p = this.parent.get(x);
    if (p === undefined || p === x) return x;
    const root = this.find(p);
    this.parent.set(x, root);
    return root;
  }

  union(a: number, b: number): void {
    const [ra, rb] = [this.find(a), this.find(b)];
    if (ra !== rb) this.parent.set(ra, rb);
  }
}
