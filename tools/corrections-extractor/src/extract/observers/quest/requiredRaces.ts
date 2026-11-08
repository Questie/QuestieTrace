// questKeys.requiredRaces - Questie field 6.
// type: bitmask, default: 0 (every race)
//
// A wrong mask hides a quest from players who can take it, so a restriction is only
// exported with proof that a whole faction cannot get the quest:
// - every starter of the quest is an NPC (items and objects give no reaction to check),
// - each of them was hostile to that faction in at least MIN_HOSTILE_SESSIONS sessions and
//   friendly to it in none,
// - each of them was friendly to the other faction and never hostile to it, which rules out
//   reputation-based reactions such as Timbermaw's (unfriendly to both factions), and
// - no session of that faction ever saw the quest, while one of the other faction did.
// The exported mask is Questie's ALL_ALLIANCE / ALL_HORDE for Forever, never only the races seen.
// Remaining risk: a starter of the excluded faction that no trace has seen yet.
// Race-level restrictions are not inferred (yet). Unfactioned characters are ignored.

import { getStream } from "../../../core/emulator";
import { playerFactionAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import type { Faction, FactionReactionValue } from "../npc/friendlyToFaction";
import { questEncounters } from "./_encounters";

const MIN_HOSTILE_SESSIONS = 20;

// QuestieDB raceKeys.ALL_ALLIANCE / ALL_HORDE for Forever (src/corrections/enum/expansions.lua):
// the Classic races plus Skyborne. Added, not OR-ed: JavaScript's bitwise operators truncate to 32 bits.
const SKYBORNE_ALLIANCE = 4294967296;
const SKYBORNE_HORDE = 8589934592;
export const ALLIANCE_RACES = 1 + 4 + 8 + 64 + SKYBORNE_ALLIANCE;
export const HORDE_RACES = 2 + 16 + 32 + 128 + SKYBORNE_HORDE;

export type ReactionSummary = Record<Faction, { hostileSessions: number; friendlySessions: number }>;

interface QuestListEntry {
  questID?: number;
}

function acceptedQuestId(args: unknown): number | null {
  if (!args || typeof args !== "object") return null;
  const packed = args as Record<string, unknown>;
  const value = packed.n === 1 ? packed[1] : packed[2];
  return typeof value === "number" && value > 0 ? value : null;
}

/** The player's faction for every quest the session saw: accepted, in a quest frame or in a gossip list. */
export const observeQuestFactions: FieldObserver<Faction> = (session) => {
  const sightings: { questID: number; t: number }[] = questEncounters(session);
  for (const event of session.events) {
    if (event.e !== "QUEST_ACCEPTED") continue;
    const questID = acceptedQuestId(event.a);
    if (questID !== null) sightings.push({ questID, t: event.t });
  }
  for (const name of ["C_GossipInfo.GetAvailableQuests", "C_GossipInfo.GetActiveQuests"]) {
    for (const entry of getStream(session, name) ?? []) {
      if (!Array.isArray(entry.v)) continue;
      for (const quest of entry.v as QuestListEntry[]) {
        if (typeof quest?.questID === "number" && quest.questID > 0) sightings.push({ questID: quest.questID, t: entry.t });
      }
    }
  }

  const observations: Observation<Faction>[] = [];
  const seen = new Set<string>();
  for (const { questID, t } of sightings) {
    const faction = playerFactionAt(session, t);
    if (!faction) continue;
    const key = `${questID}:${faction}`;
    if (seen.has(key)) continue;
    seen.add(key);
    observations.push({ entityId: questID, value: faction, confidence: "high", provenance: { session: sessionLabel(session), t } });
  }
  return observations;
};

export function mergeQuestFactions(observations: Observation<Faction>[]): Set<Faction> {
  return new Set(observations.map((obs) => obs.value));
}

/** Per faction, how many sessions saw the NPC hostile and how many saw it friendly. */
export function summarizeReactions(observations: Observation<FactionReactionValue>[]): ReactionSummary {
  const sessions = { A: { hostile: new Set<unknown>(), friendly: new Set<unknown>() }, H: { hostile: new Set<unknown>(), friendly: new Set<unknown>() } };
  for (const obs of observations) {
    const key = obs.provenance.sessionKey ?? obs.provenance.session;
    sessions[obs.value.faction][obs.value.friendly ? "friendly" : "hostile"].add(key);
  }
  return {
    A: { hostileSessions: sessions.A.hostile.size, friendlySessions: sessions.A.friendly.size },
    H: { hostileSessions: sessions.H.hostile.size, friendlySessions: sessions.H.friendly.size },
  };
}

function excluded(faction: Faction, allowed: Faction, seenFactions: Set<Faction>, starters: ReactionSummary[]): boolean {
  if (seenFactions.has(faction) || !seenFactions.has(allowed)) return false;
  return (
    starters.length > 0 &&
    starters.every(
      (starter) =>
        starter[faction].friendlySessions === 0 &&
        starter[faction].hostileSessions >= MIN_HOSTILE_SESSIONS &&
        starter[allowed].friendlySessions > 0 &&
        starter[allowed].hostileSessions === 0,
    )
  );
}

/**
 * The race mask to export, or undefined to keep every race.
 * @param starters reaction summaries of every NPC starting the quest; pass none when the quest
 *   has item or object starters.
 */
export function mergeRequiredRaces(seenFactions: Set<Faction>, starters: ReactionSummary[]): number | undefined {
  if (excluded("H", "A", seenFactions, starters)) return ALLIANCE_RACES;
  if (excluded("A", "H", seenFactions, starters)) return HORDE_RACES;
  return undefined;
}
