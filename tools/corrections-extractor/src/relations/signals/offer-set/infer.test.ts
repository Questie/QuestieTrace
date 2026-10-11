import { describe, expect, it } from "vitest";
import { catalogQuest, event, gossip, makeEpisode, npc } from "../../core/fixtures";
import type { CatalogQuest, QuestCatalog, QuestEpisode, QuestEvent } from "../../core/types";
import { aggregateQuest, finisherIndex, type AggregateParams, type QuestAssessment } from "./aggregate";
import { inferQuest, type InferParams, type InferredEdge } from "./infer";
import { observeCharacter, type CharacterView } from "./observe";
import { VariantIndex } from "./variants";

const ALLIANCE = 4294967373;
const HORDE = 8589934770;
const AGGREGATE: AggregateParams = { freshSeconds: 900, immediateSeconds: 30, latestTieSeconds: 2, maxContradictions: 0 };
const INFER: InferParams = { minCharacters: 2, minPendingRate: 0.5, minSecondarySupport: 2 };
const GIVER = npc(100);
const horde = { player: { raceId: 2, classId: 1, faction: "Horde" as const } };

function makeCatalog(quests: CatalogQuest[]): QuestCatalog {
  return { generatedAt: "", sources: [], quests: Object.fromEntries(quests.map((quest) => [quest.id, quest])) };
}

const starts = (id: number) => ({ starters: { npcs: [id], objects: [], items: [] } });
const ends = (id: number) => ({ finishers: { npcs: [id], objects: [] } });

/**
 * One character: arrives at the giver, then does each quest of `chain` in turn at that giver
 * (offered, turned in), then is offered `quest`. `initial` is already completed. `coCompleted`
 * maps a chain quest to copies the server flags completed with it (0.3s later, after the offer).
 */
function playChain(
  initial: number[],
  chain: number[],
  quest: number,
  overrides: Partial<QuestEpisode> = {},
  coCompleted: Record<number, number[]> = {},
): QuestEpisode {
  const offers = [gossip(1, GIVER, [])];
  const questEvents: QuestEvent[] = [];
  const changes: Array<{ t: number; add: number[] }> = [];
  let t = 10;
  for (const step of chain) {
    offers.push(gossip(t, GIVER, [step]));
    questEvents.push(event(t + 50, "turnedIn", step, GIVER));
    changes.push({ t: t + 50.3, add: [step, ...(coCompleted[step] ?? [])] });
    t += 100;
  }
  // Offered 0.1s after the last turn-in, before its completed-quest delta.
  offers.push(gossip(chain.length > 0 ? t - 49.9 : 5, GIVER, [quest]));
  return makeEpisode({ completed: { source: "modern", initial, changes }, offers, questEvents, ...overrides });
}

function assess(catalog: QuestCatalog, episodes: QuestEpisode[]): Map<number, QuestAssessment> {
  const views = new Map<number, CharacterView[]>();
  for (const episode of episodes) {
    for (const view of observeCharacter([episode], catalog, { completionToleranceSeconds: 1 })) {
      views.set(view.questId, [...(views.get(view.questId) ?? []), view]);
    }
  }
  const variants = new VariantIndex(catalog);
  const finishers = finisherIndex(catalog, new Map());
  const assessments = new Map<number, QuestAssessment>();
  for (const [questId, list] of views) assessments.set(questId, aggregateQuest(questId, list, catalog, AGGREGATE, finishers, variants));
  return assessments;
}

function infer(catalog: QuestCatalog, episodes: QuestEpisode[], questId: number, params: Partial<InferParams> = {}): InferredEdge[] {
  const assessments = assess(catalog, episodes);
  return inferQuest(assessments.get(questId)!, assessments, new VariantIndex(catalog), { ...INFER, ...params });
}

const targets = (edges: InferredEdge[]) => edges.map((edge) => edge.target).sort((a, b) => a - b);

describe("offer-set inference", () => {
  const UNIVERSAL = 1;
  const P1 = 10;
  const P2 = 11;
  const QUEST = 20;
  const catalog = makeCatalog([
    catalogQuest(UNIVERSAL, ends(999)),
    catalogQuest(P1, { ...starts(100), ...ends(100) }),
    catalogQuest(P2, { ...starts(100), ...ends(100) }),
    catalogQuest(QUEST, starts(100)),
  ]);

  it("keeps the quest turned in at the giver just before the offer and ignores what everyone did before", () => {
    const episodes = [playChain([UNIVERSAL], [P1], QUEST), playChain([UNIVERSAL], [P1], QUEST)];
    const edges = infer(catalog, episodes, QUEST);
    expect(targets(edges)).toEqual([P1]);
    expect(edges[0].features).toMatchObject({ field: "preQuestSingle", tier: "primary", link: true, latest: 2 });
    const universal = assess(catalog, episodes).get(QUEST)!.alive.find((entry) => entry.target === UNIVERSAL)!;
    expect(universal).toMatchObject({ support: 2, background: 2, pending: 0, latest: 0 });
  });

  it("drops a candidate once any character was offered the quest without it", () => {
    const withoutP1 = playChain([UNIVERSAL], [], QUEST);
    expect(targets(infer(catalog, [playChain([UNIVERSAL], [P1], QUEST), withoutP1, playChain([UNIVERSAL], [P1], QUEST)], QUEST))).toEqual([]);
  });

  it("lists only the nearest step of a chain", () => {
    const episodes = [playChain([UNIVERSAL], [P1, P2], QUEST), playChain([UNIVERSAL], [P1, P2], QUEST)];
    expect(targets(infer(catalog, episodes, QUEST))).toEqual([P2]);
  });

  it("makes a confirmed preQuestGroup when the giver's list lacked the quest with only one member done", () => {
    // playChain shows the giver's list after the first turn-in: it offers the second member, not QUEST.
    const episodes = [playChain([UNIVERSAL], [P1, P2], QUEST), playChain([UNIVERSAL], [P2, P1], QUEST)];
    const edges = infer(catalog, episodes, QUEST);
    expect(targets(edges)).toEqual([P1, P2]);
    expect(edges.map((edge) => [edge.features.field, edge.features.allOfConfirmed])).toEqual([
      ["preQuestGroup", true],
      ["preQuestGroup", true],
    ]);
  });

  it("leaves a group unconfirmed when play order is the only evidence (ANY-of fits too)", () => {
    // Both turned in at the giver with no list read in between, in either order, then QUEST offered.
    const turnInBoth = (order: number[]) =>
      makeEpisode({
        completed: { source: "modern", initial: [UNIVERSAL], changes: [] },
        questEvents: order.map((id, index) => event(10 + 100 * index, "turnedIn", id, GIVER)),
        offers: [gossip(10 + 100 * order.length, GIVER, [QUEST])],
      });
    const edges = infer(catalog, [turnInBoth([P1, P2]), turnInBoth([P2, P1])], QUEST);
    expect(targets(edges)).toEqual([P1, P2]);
    expect(edges.map((edge) => [edge.features.field, edge.features.allOfConfirmed])).toEqual([
      ["preQuestGroup", false],
      ["preQuestGroup", false],
    ]);
  });

  it("does not judge quests seen by too few characters", () => {
    expect(infer(catalog, [playChain([UNIVERSAL], [P1], QUEST)], QUEST)).toEqual([]);
  });

  it("emits no secondary edge when characters first saw the quest right after reaching its level", () => {
    // P1 finishes elsewhere (no link), so only the secondary tier could pick it up.
    const unlinked = makeCatalog([catalogQuest(P1, ends(555)), catalogQuest(QUEST, { ...starts(100), requiredLevel: 11 })]);
    const levelled = { levels: [{ t: 0, v: 10 }, { t: 150, v: 11 }] };
    const offeredAtLevel = (overrides: Partial<QuestEpisode>) =>
      makeEpisode({
        completed: { source: "modern", initial: [], changes: [{ t: 100, add: [P1] }] },
        offers: [gossip(160, GIVER, [QUEST])],
        levels: [{ t: 0, v: 11 }],
        ...overrides,
      });
    expect(targets(infer(unlinked, [offeredAtLevel({}), offeredAtLevel({})], QUEST))).toEqual([P1]);
    expect(infer(unlinked, [offeredAtLevel(levelled), offeredAtLevel({})], QUEST)).toEqual([]);
  });

  describe("same-name copies and faction variants", () => {
    const P_ALLIANCE = 30;
    const P_HORDE = 31;
    const SHARED = 40;
    const SCOUT = 50;
    const SCOUT_HORDE = 32;
    const AFTER_SCOUT = 60;
    const OUTDOORS = 70;
    const OUTDOORS_HORDE = 71;
    const CAMPING = 72;
    const variantCatalog = makeCatalog([
      catalogQuest(P_ALLIANCE, { name: "Report In", requiredRaces: ALLIANCE, ...ends(100) }),
      catalogQuest(P_HORDE, { name: "Report In", requiredRaces: HORDE, ...ends(100) }),
      catalogQuest(SHARED, starts(100)),
      catalogQuest(SCOUT, { name: "Scout Ahead", requiredRaces: ALLIANCE, ...ends(100) }),
      catalogQuest(SCOUT_HORDE, { name: "Scout Ahead", requiredRaces: HORDE, ...ends(200) }),
      catalogQuest(AFTER_SCOUT, starts(100)),
      catalogQuest(OUTDOORS, { name: "The Great Outdoors", requiredRaces: ALLIANCE, ...ends(100) }),
      catalogQuest(OUTDOORS_HORDE, { name: "The Great Outdoors", requiredRaces: HORDE, ...ends(200) }),
      catalogQuest(CAMPING, { requiredRaces: ALLIANCE, ...starts(100) }),
    ]);

    it("makes an OR of same-name variants when each faction's characters had their own copy", () => {
      const episodes = [
        playChain([], [P_ALLIANCE], SHARED),
        playChain([], [P_ALLIANCE], SHARED),
        playChain([], [P_HORDE], SHARED, horde),
        playChain([], [P_HORDE], SHARED, horde),
      ];
      const edges = infer(variantCatalog, episodes, SHARED);
      expect(targets(edges)).toEqual([P_ALLIANCE, P_HORDE]);
      expect(edges.every((edge) => edge.features.field === "preQuestSingle" && edge.features.shape === "or")).toBe(true);
    });

    it("adds the other faction's variant when only one faction was observed", () => {
      const episodes = [playChain([], [SCOUT], AFTER_SCOUT), playChain([], [SCOUT], AFTER_SCOUT)];
      expect(targets(infer(variantCatalog, episodes, AFTER_SCOUT))).toEqual([SCOUT_HORDE, SCOUT]);
    });

    it("adds only variants that the quest's own audience can take", () => {
      const NIGHT_ELF = 8;
      const TAUREN = 32;
      const SKYBORNE_ALLIANCE = 4294967296;
      const DRUID = 1024;
      const WARRIOR = 1;
      const TRIAL = 80;
      const trial = (id: number, requiredRaces: number, requiredClasses: number) =>
        catalogQuest(id, { name: "Trial", requiredRaces, requiredClasses, ...ends(100 + id) });
      const audienceCatalog = makeCatalog([
        trial(TRIAL, NIGHT_ELF, DRUID),
        trial(81, TAUREN, DRUID),
        trial(82, SKYBORNE_ALLIANCE, DRUID),
        trial(84, SKYBORNE_ALLIANCE, WARRIOR),
        catalogQuest(83, { requiredRaces: NIGHT_ELF + SKYBORNE_ALLIANCE, requiredClasses: DRUID, ...starts(100) }),
      ]);
      const nightElf = { player: { raceId: 4, classId: 11, faction: "Alliance" as const } };
      const episodes = [playChain([], [TRIAL], 83, nightElf), playChain([], [TRIAL], 83, nightElf)];
      // Quest 83 is for Night Elf and Skyborne druids: no one who can take it can take the Tauren
      // copy (race) or the Skyborne warrior copy (class).
      expect(targets(infer(audienceCatalog, episodes, 83))).toEqual([TRIAL, 82]);
    });

    it("lists copies the server completed together as one preQuestSingle, not a group", () => {
      // The copy lands with the delayed completed-quest delta, just after the next offer.
      const together = { [OUTDOORS]: [OUTDOORS_HORDE] };
      const episodes = [playChain([], [OUTDOORS], CAMPING, {}, together), playChain([], [OUTDOORS], CAMPING, {}, together)];
      const edges = infer(variantCatalog, episodes, CAMPING);
      expect(targets(edges)).toEqual([OUTDOORS, OUTDOORS_HORDE]);
      expect(edges.map((edge) => [edge.features.field, edge.features.shape])).toEqual([
        ["preQuestSingle", "direct"],
        ["preQuestSingle", "copy"],
      ]);
    });
  });
});
