import { describe, expect, it } from "vitest";
import { catalogQuest, event, gossip, makeEpisode, npc } from "../../core/fixtures";
import type { CatalogQuest, QuestCatalog, QuestEpisode } from "../../core/types";
import { inferBreadcrumbs } from "./infer";

// B is given by npc 1 and turned in at npc 2, who starts T. A is a second lead-in into T.
const B = 100;
const T = 200;
const A = 300;
const OTHER = 400;

function catalogOf(...quests: CatalogQuest[]): QuestCatalog {
  return { generatedAt: "", sources: [], quests: Object.fromEntries(quests.map((quest) => [quest.id, quest])) };
}

const baseCatalog = catalogOf(
  catalogQuest(B, { starters: { npcs: [1], objects: [], items: [] }, finishers: { npcs: [2], objects: [] } }),
  catalogQuest(T, { starters: { npcs: [2], objects: [], items: [] }, finishers: { npcs: [3], objects: [] } }),
);

/** Turns in B at npc 2; the server pops T right away and the character accepts it. */
function doesBreadcrumb(): QuestEpisode {
  return makeEpisode({
    questLog: [
      { t: 0, v: [B] },
      { t: 101, v: [] },
      { t: 103, v: [T] },
    ],
    completed: { source: "modern", initial: [], changes: [{ t: 101, add: [B] }] },
    questEvents: [event(100, "turnedIn", B, npc(2)), event(100.5, "detail", T, npc(2)), event(102, "accepted", T, npc(2))],
  });
}

/** Walks up to npc 2 without ever doing B and takes T from its list. */
function skipsBreadcrumb(overrides: Partial<QuestEpisode> = {}): QuestEpisode {
  return makeEpisode({
    questLog: [
      { t: 0, v: [] },
      { t: 52, v: [T] },
    ],
    offers: [gossip(50, npc(2), [T])],
    questEvents: [event(51, "accepted", T, npc(2))],
    ...overrides,
  });
}

function edges(result: ReturnType<typeof inferBreadcrumbs>): string[] {
  return result.candidates.map((candidate) => `${candidate.questId}.${candidate.field}=${candidate.target}`).sort();
}

describe("inferBreadcrumbs", () => {
  it("emits both Questie fields for an optional lead-in whose turn-in pops the target", () => {
    const result = inferBreadcrumbs([doesBreadcrumb(), doesBreadcrumb(), skipsBreadcrumb(), skipsBreadcrumb()], baseCatalog);

    expect(edges(result)).toEqual([`${B}.breadcrumbForQuestId=${T}`, `${T}.breadcrumbs=${B}`]);
    expect(result.candidates[0].score).toBeGreaterThan(0.85);
    expect(result.candidates[0].support).toBe(4);
    expect(result.coveredQuestIds).toEqual([B, T]);
  });

  it("does not call a chain a breadcrumb: nobody reached T without B", () => {
    const result = inferBreadcrumbs([doesBreadcrumb(), doesBreadcrumb()], baseCatalog);

    expect(result.candidates).toEqual([]);
    expect(result.coveredQuestIds).toEqual([B, T]);
  });

  it("lets characters who only got T once B was done outweigh a stray free one", () => {
    // T missing from npc 2's full list before B, offered once B is turned in: what a chain looks like.
    const blockedUntilB = () =>
      makeEpisode({
        questLog: [
          { t: 0, v: [] },
          { t: 20, v: [B] },
          { t: 101, v: [] },
        ],
        completed: { source: "modern", initial: [], changes: [{ t: 101, add: [B] }] },
        offers: [gossip(10, npc(2), []), gossip(110, npc(2), [T])],
        questEvents: [event(19, "accepted", B, npc(1)), event(100, "turnedIn", B, npc(2))],
      });
    const clean = inferBreadcrumbs([doesBreadcrumb(), skipsBreadcrumb()], baseCatalog);
    const chainLike = inferBreadcrumbs([doesBreadcrumb(), skipsBreadcrumb(), blockedUntilB(), blockedUntilB()], baseCatalog);

    expect(clean.candidates[0].score).toBeGreaterThan(0.5);
    expect(chainLike.candidates[0].score).toBeLessThan(0.2);
    expect(chainLike.candidates[0].contradict).toBe(2);
  });

  it("vetoes a pair once two characters were offered T with B still in the log", () => {
    const offeredAlongside = () =>
      makeEpisode({
        questLog: [{ t: 0, v: [B] }],
        offers: [gossip(50, npc(2), [T], [B])],
      });
    const result = inferBreadcrumbs([doesBreadcrumb(), skipsBreadcrumb(), offeredAlongside(), offeredAlongside()], baseCatalog);

    expect(result.candidates).toEqual([]);
  });

  it("does not count T reached through another lead-in as optional (T needs A or B)", () => {
    const catalog = catalogOf(
      ...Object.values(baseCatalog.quests),
      catalogQuest(A, { starters: { npcs: [5], objects: [], items: [] }, finishers: { npcs: [2], objects: [] } }),
    );
    const viaA = () => skipsBreadcrumb({ completed: { source: "modern", initial: [A], changes: [] } });
    const result = inferBreadcrumbs([doesBreadcrumb(), viaA(), viaA()], catalog);

    expect(result.candidates).toEqual([]);
  });

  it("only counts characters who could have taken B as skipping it", () => {
    const catalog = catalogOf(
      { ...baseCatalog.quests[B], requiredRaces: 2 }, // Orc only; fixtures default to Human
      baseCatalog.quests[T],
    );
    const orc = { raceId: 2, classId: 1, faction: "Horde" as const };
    const result = inferBreadcrumbs([doesBreadcrumb(), skipsBreadcrumb()], catalog);
    const withOrc = inferBreadcrumbs([doesBreadcrumb(), skipsBreadcrumb({ player: orc })], catalog);

    expect(result.candidates).toEqual([]);
    expect(edges(withOrc)).toContain(`${B}.breadcrumbForQuestId=${T}`);
  });

  it("never links a quest offered by the target's own giver", () => {
    const catalog = catalogOf(
      baseCatalog.quests[T],
      catalogQuest(OTHER, { starters: { npcs: [2], objects: [], items: [] }, finishers: { npcs: [2], objects: [] } }),
    );
    const popsT = () =>
      makeEpisode({
        completed: { source: "modern", initial: [], changes: [{ t: 101, add: [OTHER] }] },
        questEvents: [event(100, "turnedIn", OTHER, npc(2)), event(100.5, "detail", T, npc(2))],
      });
    const result = inferBreadcrumbs([popsT(), popsT(), skipsBreadcrumb()], catalog);

    expect(result.candidates).toEqual([]);
  });

  it("keeps one target per breadcrumb: the follow-up of T inherits B being hidden, but not the pop", () => {
    // npc 2 also starts OTHER, a follow-up of T. Once both are done, npc 1 no longer offers B.
    const catalog = catalogOf(
      ...Object.values(baseCatalog.quests),
      catalogQuest(OTHER, { starters: { npcs: [2], objects: [], items: [] }, finishers: { npcs: [2], objects: [] } }),
    );
    const pastBoth = makeEpisode({
      completed: { source: "modern", initial: [T, OTHER], changes: [] },
      offers: [gossip(10, npc(1), [])],
    });
    const result = inferBreadcrumbs([doesBreadcrumb(), skipsBreadcrumb({ offers: [gossip(50, npc(2), [T, OTHER])] }), pastBoth], catalog);
    const forB = result.candidates.filter((candidate) => candidate.field === "breadcrumbForQuestId");

    expect(result.judgements.map((judgement) => judgement.target).sort()).toEqual([T, OTHER]);
    expect(forB.map((candidate) => candidate.target)).toEqual([T]);
    expect(forB[0].note).toContain(`other targets ${OTHER}`);
  });

  it("needs two characters whose B left the log as T was accepted, and ignores player abandons", () => {
    // B in the log, T accepted, B gone moments later without a turn-in.
    const leavesLog = (kind: "removed" | "abandoned") =>
      makeEpisode({
        questLog: [
          { t: 0, v: [B] },
          { t: 52, v: [T] },
        ],
        questEvents: [event(51, "accepted", T, npc(2)), event(51.5, kind, B)],
      });
    // Someone completed B (so "never did B" means something) and someone skipped it.
    const base = [makeEpisode({ completed: { source: "modern", initial: [B], changes: [] } }), skipsBreadcrumb()];

    expect(inferBreadcrumbs([...base, leavesLog("removed")], baseCatalog).candidates).toEqual([]);
    expect(edges(inferBreadcrumbs([...base, leavesLog("removed"), leavesLog("removed")], baseCatalog))).toContain(
      `${B}.breadcrumbForQuestId=${T}`,
    );
    const abandoned = inferBreadcrumbs([...base, leavesLog("abandoned"), leavesLog("abandoned")], baseCatalog);
    expect(abandoned.candidates).toEqual([]);
    expect([...(abandoned.tally.pairs.get(`${B}>${T}`)?.characters.keys() ?? [])]).toEqual(["free"]);
  });

  it("trusts IsBreadcrumbQuest: true stands in for optionality, false rules the pair out", () => {
    const flagged = (isBreadcrumb: boolean) => ({ ...doesBreadcrumb(), questInfo: { [B]: { isBreadcrumb } } });

    expect(edges(inferBreadcrumbs([flagged(true)], baseCatalog))).toContain(`${B}.breadcrumbForQuestId=${T}`);
    expect(inferBreadcrumbs([flagged(false), skipsBreadcrumb(), skipsBreadcrumb()], baseCatalog).candidates).toEqual([]);
  });
});
