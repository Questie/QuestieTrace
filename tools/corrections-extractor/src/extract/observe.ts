// Which observers run on a session, and the staged observations they produce.
//
// Kept apart from the merge and emit code: the observer version (store/observer-version.ts)
// hashes this file, so changing it re-observes every submission, while index.ts can change freely.

import { getStream } from "../core/emulator";
import type { SessionRecord } from "../core/types";
import type { Observation } from "./observation";
import { sessionServerTime } from "./probes";
import { observeName as observeItemName } from "./observers/item/name";
import { observeNpcDrops } from "./observers/item/npcDrops";
import { observeObjectDrops } from "./observers/item/objectDrops";
import { observeLevel as observeNpcLevel, observeSpawns as observeNpcSpawns, observeFriendlyToFaction, observeRank as observeNpcRank } from "./observers/npc";
import { observeName as observeNpcName } from "./observers/npc/name";
import { observeName as observeObjectName } from "./observers/object/name";
import { observeSpawns as observeObjectSpawns } from "./observers/object";
import {
  observeName as observeQuestName,
  observeStartedBy,
  observeFinishedBy,
  observeTurnInPoi,
  observeQuestLevel,
  observeRequiredLevel,
  observeZoneOrSort,
  observeObjectives,
  observeObjectiveCount,
  observeTriggerEnd,
  observeObjectivesText,
  observeReputationReward,
} from "./observers/quest";
import { observeQuestXp } from "./observers/quest/questXp";
import { observeQuestFactions } from "./observers/quest/requiredRaces";

export const MAX_IDS = {
    npc: 18199,
    quest: 9665,
    item: 25818,
    object: 300142,
};

/**
 * Extracts the interfaceVersion from a session's GetBuildInfo capture.
 * Returns undefined if the session has no GetBuildInfo data.
 *
 * The function stream structure is always: { player: [ { t: 0, tp: 0, v: 16001 } ] }
 * as captured by PlayerIdentity.lua.
 */
function getInterfaceVersion(session: SessionRecord): number | undefined {
  const buildInfo = session.functions["GetBuildInfo"];
  if (!buildInfo || typeof buildInfo !== "object" || Array.isArray(buildInfo)) {
    return undefined;
  }
  const playerStream = buildInfo["player"];
  if (!Array.isArray(playerStream) || playerStream.length === 0) return undefined;
  const firstEntry = playerStream[0];
  if (!firstEntry || typeof firstEntry !== "object") return undefined;
  return typeof firstEntry.v === "number" ? firstEntry.v : undefined;
}

/** The interface version from the session's top-level buildInfo; undefined when absent or not a number. */
function getTopLevelInterfaceVersion(session: SessionRecord): number | undefined {
  const raw = session.buildInfo?.["4"];
  if (typeof raw !== "number" && typeof raw !== "string") return undefined;
  const version = Number(raw);
  return Number.isInteger(version) && version > 0 ? version : undefined;
}

/**
 * Checks if an interfaceVersion is from a WoW Forever build.
 * Forever builds have interfaceVersion starting with "16" (e.g. 16001, 16002, etc.)
 * TBC had 10xxx, WotLK had 11xxx, Cata had 12xxx, etc.
 */
function isForeverBuild(interfaceVersion: number | undefined): boolean {
  if (interfaceVersion === undefined) return false;
  return String(interfaceVersion).startsWith("16");
}

/** The client a session was recorded with; null where the session does not say. */
export interface SessionClient {
  interfaceVersion: number | null;
  build: number | null;
  locale: string | null;
}

export function sessionClient(session: SessionRecord): SessionClient {
  const build = Number(session.buildInfo?.["2"]);
  const locale = getStream(session, "GetLocale", "player")?.[0]?.v;
  return {
    interfaceVersion: getInterfaceVersion(session) ?? null,
    build: Number.isInteger(build) && build > 0 ? build : null,
    locale: typeof locale === "string" && locale.length > 0 ? locale : null,
  };
}

/**
 * Sessions count as Forever unless their version says otherwise: the GetBuildInfo stream is
 * read first, then the session's top-level buildInfo ("4" is the interface version). The stream
 * is the older, interface-version-only record; the top-level buildInfo (schema 15+) covers
 * sessions where the stream is missing. Sessions with neither count as Forever too (see
 * foldSessions).
 */
export function isForeverSession(session: SessionRecord): boolean {
  const version = getInterfaceVersion(session) ?? getTopLevelInterfaceVersion(session);
  return version === undefined || isForeverBuild(version);
}

/** Fields using the default scalar merge: only a per-value summary is kept. */
export const SCALAR_OBSERVERS = {
  zoneOrSort: observeZoneOrSort,
  questName: observeQuestName,
  questLevel: observeQuestLevel,
  objectivesText: observeObjectivesText,
  objectiveCount: observeObjectiveCount,
  npcName: observeNpcName,
  itemName: observeItemName,
  objectName: observeObjectName,
};

/** Fields with a custom merge, which needs every raw observation. */
export const OBSERVERS = {
  requiredLevel: observeRequiredLevel,
  objectives: observeObjectives,
  triggerEnd: observeTriggerEnd,
  startedBy: observeStartedBy,
  finishedBy: observeFinishedBy,
  reputationReward: observeReputationReward,
  questXp: observeQuestXp,
  questFactions: observeQuestFactions,
  turnInPoi: observeTurnInPoi,
  // npcZoneID and objectZoneID are merged from the spawn observations: their observers
  // are the spawn observers.
  npcSpawns: observeNpcSpawns,
  npcLevel: observeNpcLevel,
  npcFriendlyToFaction: observeFriendlyToFaction,
  npcRank: observeNpcRank,
  npcDrops: observeNpcDrops,
  objectDrops: observeObjectDrops,
  objectSpawns: observeObjectSpawns,
};

/**
 * The entity each npc, item and object field is about. Only entities above MAX_IDS are
 * exported, so their observations at or below it are dropped while folding.
 * npcFriendlyToFaction is kept for every NPC: Classic NPCs start Forever quests, and
 * their reactions decide those quests' requiredRaces. questXp is kept for every quest:
 * Forever's XP can differ from Classic's, so the XP of known quests is exported too.
 */
export const OBSERVER_ENTITIES: Partial<Record<keyof typeof OBSERVERS, keyof typeof MAX_IDS>> = {
  npcSpawns: "npc",
  npcLevel: "npc",
  npcRank: "npc",
  npcDrops: "item",
  objectDrops: "item",
  objectSpawns: "object",
};

export type ScalarObserverKey = keyof typeof SCALAR_OBSERVERS;
export type ObserverKey = keyof typeof OBSERVERS;
export type ObservedValue<F extends (session: SessionRecord) => unknown> =
  ReturnType<F> extends Observation<infer V>[] ? V : never;

/** One session's observations, ready to be added to an accumulator (see observeSession). */
export interface StagedSession {
  /** Stable key of the session (core/trace-data/session-key.ts); a per-run "#<n>" is used when unknown. */
  sessionKey?: string;
  contributorId?: string;
  sessionTime: number | null;
  scalars: [ScalarObserverKey, Observation<unknown>[]][];
  observations: [ObserverKey, Observation<unknown>[]][];
}

/**
 * Runs every observer over one session. Returns undefined for sessions that are not
 * observed: only sessions from WoW Forever builds are. Sessions without GetBuildInfo are
 * INCLUDED (they might be early Forever beta traces with valuable data) - any Classic IDs
 * they contain are filtered out by filterBelowMax in finalize.
 *
 * npc, item and object observations at or below `maxIds` are dropped right away: they are
 * never exported.
 */
export function observeSession(session: SessionRecord, maxIds: Partial<typeof MAX_IDS> = {}): StagedSession | undefined {
  if (!isForeverSession(session)) return undefined;
  const limits = { ...MAX_IDS, ...maxIds };
  return {
    sessionTime: sessionServerTime(session),
    scalars: (Object.keys(SCALAR_OBSERVERS) as ScalarObserverKey[]).map((key) => [key, SCALAR_OBSERVERS[key](session) as Observation<unknown>[]]),
    observations: (Object.keys(OBSERVERS) as ObserverKey[]).map((key) => {
      const entity = OBSERVER_ENTITIES[key];
      const maxId = entity === undefined ? -Infinity : limits[entity];
      return [key, (OBSERVERS[key](session) as Observation<unknown>[]).filter((observation) => observation.entityId > maxId)];
    }),
  };
}
