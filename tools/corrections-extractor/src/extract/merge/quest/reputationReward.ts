// questKeys.reputationReward - Questie field 26. Observed in observers/quest/reputationReward.ts.
//
// Gains are changed by things the trace cannot see in full, so observations are
// merged strictly:
// - Spillover (25% of another faction's gain) is printed rounded either way by the
//   game, so it is not merged by amount. A spillover faction is kept
//   when it passes the presence and session checks below and spills over from the
//   same decided reward in at least MIN_AMOUNT_SHARE of its turn-ins; its amount is
//   that reward / 4, unrounded. A quest that really rewards about a quarter of
//   another reward is treated as spillover too, so its amount may be off by under 1.
// - A faction is kept when it shows up in at least half of the quest's turn-ins with
//   reputation, in at least MIN_SESSIONS sessions, and one amount covers at least
//   MIN_AMOUNT_SHARE of those sessions (rested/event bonuses change some of them).
// - Some quests reward different factions per player faction, which the static
//   corrections cannot express, so a quest is dropped when both Alliance and Horde
//   turned it in and their results differ, including when one side has too little
//   data to decide.

import type { Observation } from "../../observation";
import { SPILLOVER_RATIO, type ReputationTurnInValue } from "../../observers/quest/reputationReward";

export type QuestReputationRewardValue = Array<[number, number]>;

const MIN_SESSIONS = 2;
const MIN_AMOUNT_SHARE = 0.8;
const MIN_PRESENCE_SHARE = 0.5;

function spilloverAmount(amount: number): number {
  return amount / SPILLOVER_RATIO;
}

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
