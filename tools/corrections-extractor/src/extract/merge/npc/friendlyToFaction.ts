// npcKeys.friendlyToFaction - Questie field 13. Observed in observers/npc/friendlyToFaction.ts.
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

import type { Observation } from "../../observation";
import type { Faction, FactionReactionValue } from "../../observers/npc/friendlyToFaction";

const MIN_SESSIONS_PER_FACTION = 2;

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
