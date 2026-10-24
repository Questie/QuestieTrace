// questKeys.reputationReward - Questie field 26.
// type: table, default: nil
// comment: "table: {{faction(int), value(int)},...}, a list of reputation rewarded upon quest completion"
//
// Source: the "Reputation with %s increased by %d." chat messages
// (CHAT_MSG_COMBAT_FACTION_CHANGE) the client prints around QUEST_TURNED_IN. The
// message can arrive shortly before or after the turn-in event, so messages within
// MESSAGE_WINDOW seconds count, and a turn-in is skipped when another one happened
// within TURN_IN_ISOLATION seconds. The faction name is mapped to its id with the
// session's own reputation streams. Only enUS sessions are used, since the message
// is localized.
//
// Gains are changed by things the trace cannot see in full, so observations are
// filtered and merged strictly:
// - Human sessions, and sessions without a known race, are skipped (Diplomacy adds 10%).
// - Spillover (25% of another faction's gain, rounded either way) is randomly
//   rounded by the game, so it is not merged by amount. A spillover faction is kept
//   when it passes the presence and session checks below and spills over from the
//   same decided reward in at least MIN_AMOUNT_SHARE of its turn-ins; its amount is
//   that reward / 4, rounded. A quest that really rewards a quarter of another
//   reward is treated as spillover too, which gives the same result.
// - A faction is kept when it shows up in at least half of the quest's turn-ins with
//   reputation, in at least MIN_SESSIONS sessions, and one amount covers at least
//   MIN_AMOUNT_SHARE of those sessions (rested/event bonuses change some of them).
// - Turn-ins without any message are ignored: they happen when every faction is capped.
// - Some quests reward different factions per player faction, which the static
//   corrections cannot express, so a quest is dropped when both Alliance and Horde
//   turned it in and their results differ, including when one side has too little
//   data to decide.

import { getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { getLocaleAt, playerFactionAt, playerRaceAt } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export type QuestReputationRewardValue = Array<[number, number]>;

export interface ReputationTurnInValue {
  playerFaction: "A" | "H";
  /** factionID -> reputation change, without spillover. */
  gains: Record<number, number>;
  /** factionID -> factionID of the gain it spilled over from. */
  spillover: Record<number, number>;
}

const MESSAGE_WINDOW = 0.5;
const TURN_IN_ISOLATION = 1;
const MIN_SESSIONS = 2;
const MIN_AMOUNT_SHARE = 0.8;
const MIN_PRESENCE_SHARE = 0.5;
const SPILLOVER_RATIO = 4;
const SPILLOVER_TOLERANCE = 1;
const REPUTATION_MESSAGE = /^Reputation with (.+) (increased|decreased) by (\d+)\.$/;
const FACTION_APIS = ["C_Reputation.GetFactionDataByID", "GetFactionInfoByID"] as const;

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

function spilloverAmount(amount: number): number {
  return Math.sign(amount) * Math.round(Math.abs(amount) / SPILLOVER_RATIO);
}

function turnInQuestId(args: unknown): number | null {
  const questID = (args as Record<number, unknown> | undefined)?.[1];
  return typeof questID === "number" && questID > 0 ? questID : null;
}

export const observeReputationReward: FieldObserver<ReputationTurnInValue> = (session) => {
  const observations: Observation<ReputationTurnInValue>[] = [];
  const events = session.events;
  let factionIds: Map<string, number> | null = null;

  for (let i = 0; i < events.length; i++) {
    const turnIn = events[i];
    if (turnIn.e !== "QUEST_TURNED_IN") continue;
    const questID = turnInQuestId(turnIn.a);
    if (questID === null) continue;
    if (getLocaleAt(session, turnIn.t) !== "enUS") continue;
    const race = playerRaceAt(session, turnIn.t);
    if (race === null || race === "Human") continue;
    const playerFaction = playerFactionAt(session, turnIn.t);
    if (!playerFaction) continue;

    let start = i;
    while (start > 0 && events[start - 1].t > turnIn.t - TURN_IN_ISOLATION) start--;
    const gains: Record<number, number> = {};
    let usable = true;
    for (let j = start; j < events.length && events[j].t < turnIn.t + TURN_IN_ISOLATION; j++) {
      const event = events[j];
      if (j !== i && event.e === "QUEST_TURNED_IN") {
        usable = false;
        break;
      }
      if (event.e !== "CHAT_MSG_COMBAT_FACTION_CHANGE" || Math.abs(event.t - turnIn.t) > MESSAGE_WINDOW) continue;
      const match = REPUTATION_MESSAGE.exec(String(event.a[1]));
      if (!match) continue;
      factionIds ??= factionIdsByName(session);
      const factionID = factionIds.get(match[1]);
      if (factionID === undefined || factionID in gains) {
        usable = false;
        break;
      }
      gains[factionID] = (match[2] === "increased" ? 1 : -1) * Number(match[3]);
    }
    if (!usable || Object.keys(gains).length === 0) continue;

    observations.push({
      entityId: questID,
      value: { playerFaction, ...splitSpillover(gains) },
      confidence: "high",
      provenance: { session: sessionLabel(session), t: turnIn.t },
    });
  }
  return observations;
};

function mergeForPlayerFaction(observations: Observation<ReputationTurnInValue>[]): QuestReputationRewardValue | undefined {
  if (observations.length === 0) return undefined;
  const sessionsByFaction = new Map<number, Map<number, Set<number | string>>>();
  const turnInsByFaction = new Map<number, number>();
  for (const obs of observations) {
    const session = obs.provenance.sessionKey ?? obs.provenance.session;
    for (const [key, amount] of Object.entries(obs.value.gains)) {
      const factionID = Number(key);
      turnInsByFaction.set(factionID, (turnInsByFaction.get(factionID) ?? 0) + 1);
      const amounts = sessionsByFaction.get(factionID) ?? new Map<number, Set<number | string>>();
      const sessions = amounts.get(amount) ?? new Set<number | string>();
      sessions.add(session);
      amounts.set(amount, sessions);
      sessionsByFaction.set(factionID, amounts);
    }
  }

  const rewards: QuestReputationRewardValue = [];
  for (const [factionID, amounts] of sessionsByFaction) {
    if ((turnInsByFaction.get(factionID) ?? 0) < observations.length * MIN_PRESENCE_SHARE) continue;
    const allSessions = new Set([...amounts.values()].flatMap((sessions) => [...sessions]));
    if (allSessions.size < MIN_SESSIONS) continue;
    const [amount, sessions] = [...amounts.entries()].sort((a, b) => b[1].size - a[1].size)[0];
    if (sessions.size < MIN_SESSIONS || sessions.size < allSessions.size * MIN_AMOUNT_SHARE) continue;
    rewards.push([factionID, amount]);
  }

  const decided = new Map(rewards);
  const spilloverSources = new Map<number, Map<number, number>>();
  const spilloverSessions = new Map<number, Set<number | string>>();
  for (const obs of observations) {
    const session = obs.provenance.sessionKey ?? obs.provenance.session;
    for (const [key, source] of Object.entries(obs.value.spillover)) {
      const factionID = Number(key);
      const sources = spilloverSources.get(factionID) ?? new Map<number, number>();
      sources.set(source, (sources.get(source) ?? 0) + 1);
      spilloverSources.set(factionID, sources);
      const sessions = spilloverSessions.get(factionID) ?? new Set<number | string>();
      sessions.add(session);
      spilloverSessions.set(factionID, sessions);
    }
  }
  for (const [factionID, sources] of spilloverSources) {
    if (decided.has(factionID)) continue;
    const turnIns = [...sources.values()].reduce((sum, count) => sum + count, 0);
    if (turnIns < observations.length * MIN_PRESENCE_SHARE) continue;
    if ((spilloverSessions.get(factionID)?.size ?? 0) < MIN_SESSIONS) continue;
    const [source, count] = [...sources.entries()].sort((a, b) => b[1] - a[1])[0];
    const sourceAmount = decided.get(source);
    if (sourceAmount === undefined || count < turnIns * MIN_AMOUNT_SHARE) continue;
    rewards.push([factionID, spilloverAmount(sourceAmount)]);
  }
  if (rewards.length === 0) return undefined;
  return rewards.sort((a, b) => a[0] - b[0]);
}

/** Returns undefined when no reliable reward could be derived. */
export function mergeReputationReward(observations: Observation<ReputationTurnInValue>[]): QuestReputationRewardValue | undefined {
  const allianceObservations = observations.filter((obs) => obs.value.playerFaction === "A");
  const hordeObservations = observations.filter((obs) => obs.value.playerFaction === "H");
  const alliance = mergeForPlayerFaction(allianceObservations);
  const horde = mergeForPlayerFaction(hordeObservations);
  if (allianceObservations.length > 0 && hordeObservations.length > 0 && JSON.stringify(alliance) !== JSON.stringify(horde)) {
    return undefined;
  }
  return alliance ?? horde;
}
