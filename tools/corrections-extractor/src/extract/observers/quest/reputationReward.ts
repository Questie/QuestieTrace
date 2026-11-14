// questKeys.reputationReward - Questie field 26.
// type: table, default: nil
// comment: "table: {{faction(int), value(int)},...}, a list of reputation rewarded upon quest completion"
//
// Source: the change of each faction's reputation around QUEST_TURNED_IN, read from
// C_Reputation.GetFactionDataByID (currentStanding) or, when a faction has no such
// stream, GetFactionInfoByID (barValue). The client updates reputation shortly
// before or after the turn-in event, so a gain is the value at turn-in +
// CHANGE_WINDOW minus the last value strictly before turn-in - CHANGE_WINDOW. A turn-in is skipped
// when another one happened within TURN_IN_ISOLATION seconds. This works in every
// client language.
//
// A turn-in can unlock a faction, which is then first seen inside the window and
// has no value before it, or only shows up in the reputation streams minutes later.
// The second case is caught by counting the reputation messages with an amount in
// the window (one per changed faction, in every language): more messages than
// changes means a faction is missing. The first value of such a faction is not a fixed starting
// point, so these turn-ins fall back to the "Reputation with %s increased by %d." chat messages
// (CHAT_MSG_COMBAT_FACTION_CHANGE) within CHANGE_WINDOW. The faction name is
// mapped to its id with the session's own reputation streams. The message is
// localized, so the fallback only works in enUS sessions; other sessions skip
// such a turn-in.
//
// Gains are changed by things the trace cannot see in full, so observations are
// filtered and merged strictly:
// - Human sessions, and sessions without a known race, are skipped (Diplomacy adds 10%).
// - Turn-ins without any change are ignored: they happen when every faction is capped.
// The merge rules are in merge/quest/reputationReward.ts.

import { getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { getLocaleAt, playerFactionAt, playerRaceAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export interface ReputationTurnInValue {
  playerFaction: "A" | "H";
  /** factionID -> reputation change, without spillover. */
  gains: Record<number, number>;
  /** factionID -> factionID of the gain it spilled over from. */
  spillover: Record<number, number>;
}

const CHANGE_WINDOW = 0.5;
const TURN_IN_ISOLATION = 1;
export const SPILLOVER_RATIO = 4;
const SPILLOVER_TOLERANCE = 1;
const REPUTATION_MESSAGE = /^Reputation with (.+) (increased|decreased) by (\d+)\.$/;
const FACTION_APIS = ["C_Reputation.GetFactionDataByID", "GetFactionInfoByID"] as const;

type FactionApi = (typeof FACTION_APIS)[number];
type StandingSample = { t: number; standing: number | null };

/** Reputation value of one faction sample, or null when the sample has none. */
function standingOf(api: FactionApi, value: unknown): number | null {
  if (!value || typeof value !== "object") return null;
  const standing =
    api === "GetFactionInfoByID"
      ? (value as Record<number, unknown>)[6]
      : (value as Record<string, unknown>).currentStanding;
  return typeof standing === "number" ? standing : null;
}

/** factionID -> standing streams, C_Reputation before the legacy api. */
function standingStreams(session: SessionRecord): Map<number, StandingSample[][]> {
  const streams = new Map<number, StandingSample[][]>();
  for (const api of FACTION_APIS) {
    const root = session.functions[api];
    if (!root || Array.isArray(root)) continue;
    for (const key of getParamKeys(root)) {
      const factionID = Number(key);
      if (!Number.isInteger(factionID)) continue;
      const entries = getStream(session, api, key) ?? [];
      if (entries.length === 0) continue;
      const factionStreams = streams.get(factionID) ?? [];
      factionStreams.push(entries.map((entry) => ({ t: entry.t, standing: standingOf(api, entry.v) })));
      streams.set(factionID, factionStreams);
    }
  }
  return streams;
}

function factionIdsByName(session: SessionRecord): Map<string, number> {
  const ids = new Map<string, number>();
  for (const api of FACTION_APIS) {
    const root = session.functions[api];
    if (!root || Array.isArray(root)) continue;
    for (const key of getParamKeys(root)) {
      const factionID = Number(key);
      if (!Number.isInteger(factionID)) continue;
      for (const entry of getStream(session, api, key) ?? []) {
        const value = entry.v as Record<string | number, unknown> | undefined;
        if (!value || typeof value !== "object") continue;
        const name = api === "GetFactionInfoByID" ? value[1] : value.name;
        if (typeof name === "string" && name.length > 0) ids.set(name, factionID);
      }
    }
  }
  return ids;
}

/** Gains from the enUS chat `messages` of a turn-in, or null when a message cannot be used. */
function chatGains(messages: string[], factionIds: Map<string, number>): Record<number, number> | null {
  const gains: Record<number, number> = {};
  for (const text of messages) {
    const match = REPUTATION_MESSAGE.exec(text);
    if (!match) continue;
    const factionID = factionIds.get(match[1]);
    if (factionID === undefined || factionID in gains) return null;
    gains[factionID] = (match[2] === "increased" ? 1 : -1) * Number(match[3]);
  }
  return gains;
}

/** Gains from the reputation values around `t`, or null when a faction was first seen within CHANGE_WINDOW. */
function standingGains(streams: Map<number, StandingSample[][]>, t: number): Record<number, number> | null {
  const gains: Record<number, number> = {};
  for (const [factionID, factionStreams] of streams) {
    const boundaries = factionStreams.map((stream) => ({
      before: standingAt(stream, t - CHANGE_WINDOW, true),
      after: standingAt(stream, t + CHANGE_WINDOW, false),
    }));
    // The first stream with a standing at both boundaries wins; otherwise the preferred one decides.
    const { before, after } =
      boundaries.find((b) => typeof b.before === "number" && typeof b.after === "number") ?? boundaries[0];
    if (after === undefined) continue;
    if (before === undefined) return null;
    if (before === null || after === null || before === after) continue;
    gains[factionID] = after - before;
  }
  return gains;
}

/** Last standing at or before `t` (strictly before when `strict`), undefined when none was sampled yet. */
function standingAt(stream: StandingSample[], t: number, strict: boolean): number | null | undefined {
  let result: number | null | undefined;
  for (const entry of stream) {
    if (strict ? entry.t >= t : entry.t > t) break;
    result = entry.standing;
  }
  return result;
}

/** Splits off gains that are a spillover of a larger gain in the same direction from the same turn-in. The largest matching gain is the source. */
export function splitSpillover(allGains: Record<number, number>): Pick<ReputationTurnInValue, "gains" | "spillover"> {
  const entries = Object.entries(allGains).map(([factionID, amount]) => [Number(factionID), amount] as const);
  const gains: Record<number, number> = {};
  const spillover: Record<number, number> = {};
  for (const [factionID, amount] of entries) {
    const source = entries
      .filter(
        ([, other]) =>
          Math.sign(other) === Math.sign(amount) &&
          Math.abs(other) > Math.abs(amount) &&
          Math.abs(amount - other / SPILLOVER_RATIO) <= SPILLOVER_TOLERANCE,
      )
      .sort((a, b) => Math.abs(b[1]) - Math.abs(a[1]))[0];
    if (source) spillover[factionID] = source[0];
    else gains[factionID] = amount;
  }
  return { gains, spillover };
}

function turnInQuestId(args: unknown): number | null {
  const questID = (args as Record<number, unknown> | undefined)?.[1];
  return typeof questID === "number" && questID > 0 ? questID : null;
}

export const observeReputationReward: FieldObserver<ReputationTurnInValue> = (session) => {
  const observations: Observation<ReputationTurnInValue>[] = [];
  const events = session.events;
  let streams: Map<number, StandingSample[][]> | null = null;
  let factionIds: Map<string, number> | null = null;

  for (let i = 0; i < events.length; i++) {
    const turnIn = events[i];
    if (turnIn.e !== "QUEST_TURNED_IN") continue;
    const questID = turnInQuestId(turnIn.a);
    if (questID === null) continue;
    const race = playerRaceAt(session, turnIn.t);
    if (race === null || race === "Human") continue;
    const playerFaction = playerFactionAt(session, turnIn.t);
    if (!playerFaction) continue;

    let start = i;
    while (start > 0 && events[start - 1].t > turnIn.t - TURN_IN_ISOLATION) start--;
    const messages: string[] = [];
    let isolated = true;
    for (let j = start; j < events.length && events[j].t < turnIn.t + TURN_IN_ISOLATION; j++) {
      const event = events[j];
      if (j !== i && event.e === "QUEST_TURNED_IN") {
        isolated = false;
        break;
      }
      if (event.e === "CHAT_MSG_COMBAT_FACTION_CHANGE" && Math.abs(event.t - turnIn.t) <= CHANGE_WINDOW) {
        messages.push(String(event.a[1]));
      }
    }
    if (!isolated) continue;

    streams ??= standingStreams(session);
    let gains = standingGains(streams, turnIn.t);
    const amountMessages = messages.filter((text) => /\d/.test(text)).length;
    if (gains === null || amountMessages > Object.keys(gains).length) {
      if (getLocaleAt(session, turnIn.t) !== "enUS") continue;
      factionIds ??= factionIdsByName(session);
      gains = chatGains(messages, factionIds);
    }
    if (gains === null || Object.keys(gains).length === 0) continue;

    observations.push({
      entityId: questID,
      value: { playerFaction, ...splitSpillover(gains) },
      confidence: "high",
      provenance: { session: sessionLabel(session), t: turnIn.t },
    });
  }
  return observations;
};
