// npcKeys.friendlyToFaction - Questie field 13.
// type: string, default: nil
// comment: 'Contains "A" and/or "H" depending on NPC being friendly towards those factions. nil if hostile to both.'
//
// Questie treats neutral NPCs (e.g. wildlife) as friendly, so a faction counts as
// friendly when UnitReaction("player", npc) is 4 (neutral) or higher.
//
// Source: UnitReaction["player"][token] from Modules/Trackers/UnitState.lua, paired
// with UnitGUID[token] for the "target" and "mouseover" tokens. Both streams are only
// written on change, so a reaction is only trusted at a GUID sample when it was
// recorded at the same time, or when PLAYER_TARGET_CHANGED/UPDATE_MOUSEOVER_UNIT fired
// at that time (UnitState samples every token on both, so the last value is still
// current). Other GUID samples (quest/merchant/loot events) are not paired with a
// reaction. A failed UnitReaction call at such an event would leave the previous
// unit's value in place; the strict merge below is what guards against that.
//
// The merge is strict: a faction is only decided when at least
// MIN_SESSIONS_PER_FACTION sessions agree and no session disagrees, and a value is
// only emitted when both factions are decided. The one exception is a fix by
// Blizzard: when sessions ordered by server time switch from one reaction to the
// other exactly once, the newer reaction wins if at least MIN_SESSIONS_PER_FACTION
// sessions saw it. Any other disagreement (several switches, a switch between
// sessions with the same server time, a session that saw both, or a session
// without a server time) leaves the faction undecided. NPCs hostile to both are not
// exported, since nil is Questie's default and corrections cannot express it.

import { getStream, valueAt } from "../../../core/emulator";
import type { FunctionStreamEntry } from "../../../core/types";
import { playerFactionAt, sessionServerTime } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";
import { guidEncounters } from "../_guidEncounters";

export type Faction = "A" | "H";

export interface FactionReactionValue {
  faction: Faction;
  friendly: boolean;
  /** GetServerTime() at the start of the session, used to order sessions; null when not recorded. */
  sessionTime: number | null;
}

const MIN_SESSIONS_PER_FACTION = 2;
const FRIENDLY_REACTION_MIN = 4;
const REACTION_TOKENS = ["target", "mouseover"] as const;
const REACTION_SAMPLE_EVENTS = new Set(["PLAYER_TARGET_CHANGED", "UPDATE_MOUSEOVER_UNIT"]);

function reactionAt(stream: FunctionStreamEntry[], sampleTimes: Set<number>, t: number): number | null {
  if (!sampleTimes.has(t)) return null;
  const reaction = valueAt(stream, t);
  return typeof reaction === "number" ? reaction : null;
}

export const observeFriendlyToFaction: FieldObserver<FactionReactionValue> = (session) => {
  const observations: Observation<FactionReactionValue>[] = [];
  const seen = new Set<string>();
  const sessionTime = sessionServerTime(session);
  const sampleEventTimes = session.events.filter((event) => REACTION_SAMPLE_EVENTS.has(event.e)).map((event) => event.t);
  for (const token of REACTION_TOKENS) {
    const stream = getStream(session, "UnitReaction", "player", token) ?? [];
    const sampleTimes = new Set([...sampleEventTimes, ...stream.map((entry) => entry.t)]);
    for (const encounter of guidEncounters(session, "npc", [token])) {
      const faction = playerFactionAt(session, encounter.t);
      if (!faction) continue;
      const reaction = reactionAt(stream, sampleTimes, encounter.t);
      if (reaction === null) continue;
      const friendly = reaction >= FRIENDLY_REACTION_MIN;
      const key = `${encounter.entityId}:${faction}:${friendly}`;
      if (seen.has(key)) continue;
      seen.add(key);
      observations.push({
        entityId: encounter.entityId,
        value: { faction, friendly, sessionTime },
        confidence: "high",
        provenance: { session: sessionLabel(session), t: encounter.t },
      });
    }
  }
  return observations;
};

function decideFaction(observations: Observation<FactionReactionValue>[], faction: Faction): boolean | null {
  const sessions = new Map<number | string, { friendly: Set<boolean>; time: number | null }>();
  for (const obs of observations) {
    if (obs.value.faction !== faction) continue;
    const key = obs.provenance.sessionKey ?? obs.provenance.session;
    const session = sessions.get(key) ?? { friendly: new Set<boolean>(), time: obs.value.sessionTime };
    session.friendly.add(obs.value.friendly);
    sessions.set(key, session);
  }
  const all = [...sessions.values()];
  if (all.some((session) => session.friendly.size > 1)) return null;

  const votes = all.map((session) => ({ friendly: [...session.friendly][0], time: session.time }));
  if (votes.every((vote) => vote.friendly === votes[0]?.friendly)) {
    return votes.length >= MIN_SESSIONS_PER_FACTION ? votes[0].friendly : null;
  }

  if (votes.some((vote) => vote.time === null)) return null;
  votes.sort((a, b) => (a.time as number) - (b.time as number));
  const switchIndexes = votes.flatMap((vote, index) => (index > 0 && vote.friendly !== votes[index - 1].friendly ? [index] : []));
  if (switchIndexes.length !== 1 || votes[switchIndexes[0]].time === votes[switchIndexes[0] - 1].time) return null;
  const newest = votes[votes.length - 1].friendly;
  const newestCount = votes.filter((vote) => vote.friendly === newest).length;
  return newestCount >= MIN_SESSIONS_PER_FACTION ? newest : null;
}

/** Returns undefined when the value is not reliable enough to export or the NPC is hostile to both factions. */
export function mergeFriendlyToFaction(observations: Observation<FactionReactionValue>[]): string | undefined {
  const alliance = decideFaction(observations, "A");
  const horde = decideFaction(observations, "H");
  if (alliance === null || horde === null) return undefined;
  const value = (alliance ? "A" : "") + (horde ? "H" : "");
  return value === "" ? undefined : value;
}
