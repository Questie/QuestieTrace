// Groups a contributor's sessions into characters without any player names or GUIDs.
//
// Candidates are a contributor's sessions with the same race, class and faction. Walking them
// in start order, a session continues an existing character when it could be that character's
// next session: it starts after the character's last session ended, at the same or a higher
// level, and its completed-quest set at start contains (almost) everything the character had
// completed by the end of its last session. Several characters can qualify early on (two
// same-race alts share their starting-zone quests); the one with the longest history wins,
// because a short history fits into a long one but not the other way round.
//
// Known failure modes:
// - Two same race/class/faction alts that have completed (nearly) the same quests, e.g. two
//   fresh level-1 alts, merge into one character until their histories diverge.
// - A character whose completed history was reset or not loaded at capture start starts a new
//   character key.
// - Sessions without completed-quest data are only matched by level and time.
// - Different contributor ids for one player (e.g. reinstalls) are never joined.

import { createHash } from "crypto";
import type { PlayerContext } from "../core/types";

export const CHARACTER_PARAMS = {
  /** Completed quests of the previous session that may be missing (daily/weekly resets). */
  maxMissingQuests: 5,
  maxMissingRatio: 0.05,
  /** Clock skew tolerated between one session's end and the next one's start. */
  overlapSlackSeconds: 120,
} as const;

export interface CharacterInput {
  episodeKey: string;
  contributorId: string;
  player: PlayerContext;
  /** Epoch seconds at capture start (GetServerTime), when recorded. */
  start?: number;
  duration: number;
  /** Approximate epoch seconds used for ordering: `start`, else the session name stamp or submission time. */
  order: number;
  sessionName?: string;
  firstLevel?: number;
  lastLevel?: number;
  /** Completed quest ids at capture start and at its end, sorted ascending. */
  initialCompleted?: readonly number[];
  finalCompleted?: readonly number[];
}

interface Character {
  key: string;
  lastEnd?: number;
  lastLevel?: number;
  completed?: readonly number[];
}

/** How many ids of sorted `a` are missing from sorted `b`. */
function countMissing(a: readonly number[], b: readonly number[]): number {
  let missing = 0;
  let j = 0;
  for (const id of a) {
    while (j < b.length && b[j] < id) j++;
    if (j >= b.length || b[j] !== id) missing++;
  }
  return missing;
}

function fits(character: Character, session: CharacterInput): boolean {
  if (character.lastEnd !== undefined && session.start !== undefined && session.start < character.lastEnd - CHARACTER_PARAMS.overlapSlackSeconds) {
    return false;
  }
  if (character.lastLevel !== undefined && session.firstLevel !== undefined && session.firstLevel < character.lastLevel) return false;
  if (character.completed && session.initialCompleted) {
    const allowed = Math.max(CHARACTER_PARAMS.maxMissingQuests, CHARACTER_PARAMS.maxMissingRatio * character.completed.length);
    if (countMissing(character.completed, session.initialCompleted) > allowed) return false;
  }
  return true;
}

function compareSessions(a: CharacterInput, b: CharacterInput): number {
  if (a.order !== b.order) return a.order - b.order;
  return a.episodeKey < b.episodeKey ? -1 : a.episodeKey > b.episodeKey ? 1 : 0;
}

/**
 * Episode key -> 0-based position among its character's sessions, oldest first (QuestEpisode.sessionOrder).
 * Ordered by progress (completed history size at capture start), then capture start time, which
 * unlike the session name exists for unsaved sessions too, then name and key for determinism.
 */
export function orderSessions(sessions: CharacterInput[], characterOf: ReadonlyMap<string, string>): Map<string, number> {
  const byCharacter = new Map<string, CharacterInput[]>();
  for (const session of sessions) {
    const character = characterOf.get(session.episodeKey)!;
    const list = byCharacter.get(character);
    if (list) list.push(session);
    else byCharacter.set(character, [session]);
  }
  const progress = (session: CharacterInput) => session.initialCompleted?.length ?? 0;
  const result = new Map<string, number>();
  for (const list of byCharacter.values()) {
    list.sort(
      (a, b) =>
        progress(a) - progress(b) ||
        a.order - b.order ||
        (a.sessionName ?? "").localeCompare(b.sessionName ?? "") ||
        a.episodeKey.localeCompare(b.episodeKey),
    );
    list.forEach((session, index) => result.set(session.episodeKey, index));
  }
  return result;
}

/** Episode key -> character key. */
export function assignCharacters(sessions: CharacterInput[]): Map<string, string> {
  const groups = new Map<string, CharacterInput[]>();
  for (const session of sessions) {
    const { raceId, classId, faction } = session.player;
    const group = `${session.contributorId}|${raceId ?? "?"}|${classId ?? "?"}|${faction ?? "?"}`;
    const list = groups.get(group);
    if (list) list.push(session);
    else groups.set(group, [session]);
  }

  const result = new Map<string, string>();
  for (const list of groups.values()) {
    const characters: Character[] = [];
    for (const session of [...list].sort(compareSessions)) {
      let best: Character | undefined;
      for (const character of characters) {
        if (!fits(character, session)) continue;
        if (!best || (character.completed?.length ?? -1) > (best.completed?.length ?? -1)) best = character;
      }
      if (!best) {
        best = { key: createHash("sha256").update(`character|${session.contributorId}|${session.episodeKey}`).digest("hex").slice(0, 16) };
        characters.push(best);
      }
      if (session.start !== undefined) best.lastEnd = session.start + session.duration;
      if (session.lastLevel !== undefined) best.lastLevel = session.lastLevel;
      if (session.finalCompleted) best.completed = session.finalCompleted;
      result.set(session.episodeKey, best.key);
    }
  }
  return result;
}
