// Test helpers: sessions described as a few readable facts. Only test files import this.

import { catalogQuest, event, makeEpisode } from "../../core/fixtures";
import type { CatalogQuest, QuestCatalog, QuestEpisode, QuestEvent } from "../../core/types";
import { buildHistories, unstableQuests } from "./history";
import { Population } from "./population";

export interface SessionSpec {
  /** QuestEpisode.sessionOrder: position among the character's sessions, oldest first. */
  order?: number;
  race?: number;
  level?: number;
  /** [t, level]: dinged at t. */
  levelUps?: Array<[number, number]>;
  /** Completed before the session started. */
  initial?: number[];
  /** In the log when the session started. */
  holding?: number[];
  /** [t, quest]: accepted at t. */
  accepts?: Array<[number, number]>;
  /** [t, quest]: turned in at t; the completed set gains it half a second later. */
  turnIns?: Array<[number, number]>;
  /** [t, quest]: the completed set gains it at t without a turn-in (server flag). */
  flags?: Array<[number, number]>;
}

const HORDE_RACES = new Set([2, 5, 6, 8, 96]);

export function session(characterKey: string, spec: SessionSpec): QuestEpisode {
  const race = spec.race ?? 1;
  const adds = new Map<number, number[]>();
  const add = (t: number, questId: number) => adds.set(t, [...(adds.get(t) ?? []), questId]);
  const questEvents: QuestEvent[] = [];
  for (const [t, questId] of spec.turnIns ?? []) {
    questEvents.push(event(t, "turnedIn", questId));
    add(t + 0.5, questId);
  }
  for (const [t, questId] of spec.flags ?? []) add(t, questId);
  for (const [t, questId] of spec.accepts ?? []) questEvents.push(event(t, "accepted", questId));
  questEvents.sort((a, b) => a.t - b.t);

  const log = new Set(spec.holding ?? []);
  const questLog = [{ t: 0, v: [...log] }];
  for (const { t, kind, questId } of questEvents) {
    if (kind === "accepted") log.add(questId);
    else log.delete(questId);
    questLog.push({ t, v: [...log] });
  }

  return makeEpisode({
    characterKey,
    sessionOrder: spec.order ?? 0,
    player: { raceId: race, classId: 1, faction: HORDE_RACES.has(race) ? "Horde" : "Alliance" },
    levels: [{ t: 0, v: spec.level ?? 5 }, ...(spec.levelUps ?? []).map(([t, v]) => ({ t, v }))],
    completed: {
      source: "modern",
      initial: spec.initial ?? [],
      changes: [...adds].sort(([a], [b]) => a - b).map(([t, ids]) => ({ t, add: ids })),
    },
    questLog,
    questEvents,
  });
}

export function catalogOf(quests: Array<number | CatalogQuest>): QuestCatalog {
  const entries = quests.map((quest) => (typeof quest === "number" ? catalogQuest(quest) : quest));
  return { generatedAt: "test", sources: [], quests: Object.fromEntries(entries.map((quest) => [String(quest.id), quest])) };
}

export function populationOf(episodes: QuestEpisode[], catalog: QuestCatalog): Population {
  const ignore = unstableQuests(episodes, catalog);
  const { histories } = buildHistories(episodes, { ignore, maxInitialLoss: 0.1 });
  return new Population(histories, catalog, ignore);
}
