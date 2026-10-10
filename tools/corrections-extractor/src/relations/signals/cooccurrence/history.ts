// One merged completion history per character: which quests it completed and took, and where in
// its (sessions-ordered) life each one first happened. Everything this signal infers is a
// statement about these histories across characters.
//
// Order is only partly observable. A quest already completed (or already in the log) when a
// session started happened at an unknown time since the previous session, so two such quests of
// the same session are unordered against each other, but both precede everything that happens
// during that session and follow everything from earlier sessions.

import { EpisodeTimeline } from "../../core/timeline";
import type { CatalogQuest, PlayerContext, QuestCatalog, QuestEpisode } from "../../core/types";

/** Larger than any session's duration in seconds, so session index dominates stamp order. */
const SESSION_SPAN = 1e7;

/** Quests first seen in the log this early in a session, without an accept, were held at its start. */
const LATE_LOG_SECONDS = 60;

/** Where one fact sits in a character's history. */
export interface Stamp {
  /** Sort key: session index * SESSION_SPAN + session time, or just below the session for state present at its start. */
  pos: number;
  /** Already true when the session started (happened at an unknown time since the previous session). */
  atStart: boolean;
  /** `t` is the event's own time (a turn-in or accept), not a later sample of the completed set or log. */
  exact: boolean;
  /** Episode where it was first seen, for evidence. */
  episode: string;
  t: number;
}

export type Order = "before" | "after" | "unordered";

/** Lag allowed between two exact event times before their order counts (event timestamps jitter). */
const EXACT_TOLERANCE_SECONDS = 1;

/**
 * Whether completion `done` happened before or after `taken`. Completion times are never early
 * (a turn-in is exact, a completed-quest delta is sampled up to ~1 s late plus server lag), so any
 * in-session lead counts as "before" - a follow-up accepted a second after its prerequisite's
 * turn-in is the most common case. A sampled completion up to `toleranceSeconds` late is
 * "unordered", not "after": "accepted the follow-up 0.5 s before the prerequisite's delta arrived"
 * proves nothing. A turn-in after the accept does: the server would not have offered X yet.
 */
export function order(done: Stamp, taken: Stamp, toleranceSeconds: number): Order {
  if (done.pos === taken.pos) return "unordered";
  if (done.pos < taken.pos) return "before";
  const sameSession = Math.floor(done.pos / SESSION_SPAN) === Math.floor(taken.pos / SESSION_SPAN);
  const tolerance = done.exact ? EXACT_TOLERANCE_SECONDS : toleranceSeconds;
  if (sameSession && !done.atStart && !taken.atStart && done.pos - taken.pos <= tolerance) return "unordered";
  return "after";
}

/** Both happened during the same session (neither was already true when it started). */
export function sameSession(a: Stamp, b: Stamp): boolean {
  return !a.atStart && !b.atStart && a.episode === b.episode;
}

export interface CharacterHistory {
  key: string;
  player: PlayerContext;
  /** Highest level seen in any session; a lower bound on what the character reached. */
  maxLevel: number | undefined;
  /** Episode keys in history order. */
  episodes: string[];
  /** First completion of each quest. Only quests the client's completed set actually contained. */
  done: Map<number, Stamp>;
  /** First moment each quest was in the log, accepted, or completed. */
  taken: Map<number, Stamp>;
  /** One timeline per kept session, by episode key, for point-in-time checks (levels, log, completion). */
  timelines: Map<string, EpisodeTimeline>;
}

/**
 * The character had both quests active at once: both in the log, or one in the log while the
 * other was completed. Exclusive quests never are.
 */
export function heldTogether(history: CharacterHistory, a: number, b: number): boolean {
  for (const timeline of history.timelines.values()) {
    for (const entry of timeline.episode.questLog) {
      const hasA = entry.v.includes(a);
      const hasB = entry.v.includes(b);
      if (hasA && (hasB || timeline.isCompletedAt(b, entry.t))) return true;
      if (hasB && timeline.isCompletedAt(a, entry.t)) return true;
    }
  }
  return false;
}

export interface HistoryReport {
  episodes: number;
  /** Sessions dropped because they carry no completed-quest data, or an empty set that cannot be real. */
  withoutCompleted: number;
  /** Sessions dropped because their initial completed set lost much of what earlier sessions showed. */
  brokenInitialSets: number;
  characters: number;
  /** Characters dropped because their sessions disagree on race or class (a grouping error). */
  inconsistentCharacters: number;
}

/** Quests whose completion state is not a one-way "done forever" flag, so co-occurrence says nothing. */
export function unstableQuests(episodes: readonly QuestEpisode[], catalog: QuestCatalog): Set<number> {
  const unstable = new Set<number>();
  for (const quest of Object.values(catalog.quests)) {
    if (isRepeatable(quest)) unstable.add(quest.id);
  }
  for (const episode of episodes) {
    // Daily/weekly resets remove a few quests from the completed set. A change that drops most of
    // the set is a bad read (seen mid-session, followed by the set coming back), not a reset.
    let size = episode.completed?.initial.length ?? 0;
    for (const change of episode.completed?.changes ?? []) {
      const removed = change.remove ?? [];
      if (removed.length <= Math.max(2, size / 2)) for (const id of removed) unstable.add(id);
      size += (change.add?.length ?? 0) - removed.length;
    }
    for (const offer of episode.offers) {
      for (const quest of offer.available) if (quest.repeatable || (quest.frequency ?? 1) > 1) unstable.add(quest.id);
    }
  }
  return unstable;
}

function isRepeatable(quest: CatalogQuest): boolean {
  return (quest.specialFlags ?? 0) % 2 === 1;
}

export interface HistoryOptions {
  /** Quests to leave out of `done`/`taken` entirely (see unstableQuests). */
  ignore: ReadonlySet<number>;
  /**
   * A session whose initial completed set misses more than this share of the stable quests earlier
   * sessions already showed is treated as a broken read and dropped.
   */
  maxInitialLoss: number;
}

export function buildHistories(
  episodes: readonly QuestEpisode[],
  options: HistoryOptions,
): { histories: CharacterHistory[]; report: HistoryReport } {
  const report: HistoryReport = {
    episodes: episodes.length,
    withoutCompleted: 0,
    brokenInitialSets: 0,
    characters: 0,
    inconsistentCharacters: 0,
  };

  const byCharacter = new Map<string, QuestEpisode[]>();
  for (const episode of episodes) {
    if (!episode.completed || completedLooksMissing(episode)) {
      report.withoutCompleted++;
      continue;
    }
    const list = byCharacter.get(episode.characterKey);
    if (list) list.push(episode);
    else byCharacter.set(episode.characterKey, [episode]);
  }

  const histories: CharacterHistory[] = [];
  for (const [key, sessions] of byCharacter) {
    const player = mergePlayer(sessions);
    if (!player) {
      report.inconsistentCharacters++;
      continue;
    }
    // Ingest orders a character's sessions by completed-history size, then name (a third have no
    // name), so every signal agrees on what happened first.
    sessions.sort((a, b) => a.sessionOrder - b.sessionOrder || a.key.localeCompare(b.key));
    const history = buildHistory(key, player, sessions, options, report);
    if (history.done.size > 0 || history.taken.size > 0) histories.push(history);
  }
  report.characters = histories.length;
  return { histories, report };
}

/** The character's race/class/faction, or undefined when its sessions disagree. */
function mergePlayer(sessions: readonly QuestEpisode[]): PlayerContext | undefined {
  const merged: PlayerContext = {};
  for (const { player } of sessions) {
    for (const field of ["raceId", "classId", "faction"] as const) {
      const value = player[field];
      if (value === undefined) continue;
      if (merged[field] !== undefined && merged[field] !== value) return undefined;
      (merged as Record<string, unknown>)[field] = value;
    }
  }
  return merged;
}

function buildHistory(
  key: string,
  player: PlayerContext,
  sessions: readonly QuestEpisode[],
  options: HistoryOptions,
  report: HistoryReport,
): CharacterHistory {
  const history: CharacterHistory = { key, player, maxLevel: undefined, episodes: [], done: new Map(), taken: new Map(), timelines: new Map() };
  const first = (map: Map<number, Stamp>, questId: number, stamp: Stamp) => {
    if (options.ignore.has(questId)) return;
    const existing = map.get(questId);
    if (!existing || stamp.pos < existing.pos) map.set(questId, stamp);
  };

  for (const episode of sessions) {
    const completed = episode.completed!;
    if (initialSetLooksBroken(completed.initial, history.done, options.maxInitialLoss)) {
      report.brokenInitialSets++;
      continue;
    }
    const index = history.episodes.length;
    const base = index * SESSION_SPAN;
    const atStart: Stamp = { pos: base - 1, atStart: true, exact: false, episode: episode.key, t: 0 };
    const at = (t: number, exact: boolean): Stamp => ({ pos: base + t, atStart: false, exact, episode: episode.key, t });
    const turnIns = new Set(episode.questEvents.filter((event) => event.kind === "turnedIn").map((event) => `${event.questId}@${event.t}`));
    const accepted = new Set(episode.questEvents.filter((event) => event.kind === "accepted").map((event) => event.questId));
    history.episodes.push(episode.key);

    // Turn-ins date a completion more precisely than the delayed delta, but only quests the
    // completed set really contained count as done (repeatables are turned in, never flagged).
    const timeline = new EpisodeTimeline(episode);
    history.timelines.set(episode.key, timeline);
    const completedIds = new Set(completed.initial);
    for (const change of completed.changes) for (const id of change.add ?? []) completedIds.add(id);
    for (const questId of completedIds) {
      const when = timeline.completionTime(questId);
      if (when === undefined) continue;
      const stamp = when === "initial" ? atStart : at(when, turnIns.has(`${questId}@${when}`));
      first(history.done, questId, stamp);
      first(history.taken, questId, stamp);
    }

    // The log often loads a few seconds into the session (5% of sessions start with an empty
    // snapshot): quests that show up early without being accepted were already held.
    episode.questLog.forEach((entry, entryIndex) => {
      for (const questId of entry.v) {
        const loadedLate = entry.t <= LATE_LOG_SECONDS && !accepted.has(questId);
        first(history.taken, questId, entryIndex === 0 || loadedLate ? atStart : at(entry.t, false));
      }
    });
    for (const event of episode.questEvents) {
      if (event.kind === "accepted") first(history.taken, event.questId, at(event.t, true));
    }

    for (const level of episode.levels) {
      if (history.maxLevel === undefined || level.v > history.maxLevel) history.maxLevel = level.v;
    }
  }
  return history;
}

/**
 * Old traces using the legacy GetQuestsCompleted often recorded an empty set for the whole session.
 * Such a session would make every quest the character held look taken without its prerequisites.
 * An empty set is believable only for a fresh character whose turn-ins all show up as additions.
 */
function completedLooksMissing(episode: QuestEpisode): boolean {
  const completed = episode.completed!;
  if (completed.initial.length > 0) return false;
  if ((episode.levels[0]?.v ?? 1) > 5) return true;
  const added = new Set(completed.changes.flatMap((change) => change.add ?? []));
  return episode.questEvents.some((event) => event.kind === "turnedIn" && !added.has(event.questId));
}

/** An initial completed set that forgot many quests earlier sessions had completed is a bad read. */
function initialSetLooksBroken(initial: readonly number[], known: ReadonlyMap<number, Stamp>, maxLoss: number): boolean {
  if (known.size === 0) return false;
  const present = new Set(initial);
  let missing = 0;
  for (const questId of known.keys()) if (!present.has(questId)) missing++;
  return missing > Math.max(2, known.size * maxLoss);
}
