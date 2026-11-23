// npcKeys.subName - Questie field 14.
// observeSubName keeps enUS subnames only; observeLocalizedSubName keeps every locale (for the lookup export).
//
// Source: UnitSubName.mouseover from Modules/Trackers/UnitState.lua (the line below the name
// in the unit tooltip), written on change. "" means the NPC has no subname.
//
// NPCs without a subname show their level line there instead ("Level 28", "Stufe ?? (Boss)",
// "等级 10（精英）"), which older versions of the tracker did not tell apart. Such a value means
// "no subname". A subname with a redacted player name ("<name>'s Escort") is not observed at all.
//
// Which NPC a subname belongs to depends on the tracker version:
// - With UnitSubNameGUID.mouseover (schema 21+), every subname has the GUID of the NPC its tooltip
//   was shown for, so it is used as recorded.
// - Without it, the NPC is inferred. The mouseover GUID stream cannot be used: UPDATE_MOUSEOVER_UNIT
//   was not registered, so mouseovers were only sampled as a side effect of other events and a
//   title was often paired with a neighbouring NPC. Instead, players hover an NPC before they
//   target it with the mouse, so the last subname before PLAYER_TARGET_CHANGED to an NPC is that
//   NPC's. It is only trusted when it is recent and the player is not in combat (target changes
//   in combat are mostly tab targeting). Compared with the Classic NPC database, this was right for
//   99% of the titled NPCs on a sample of 848 recent sessions. Targeting without hovering still pairs
//   a wrong title now and then, which the agreement between sessions in the merge evens out.

import { getStream } from "../../../core/emulator";
import type { FunctionStreamEntry, SessionRecord } from "../../../core/types";
import { parseGuid } from "../../guid";
import { getLocaleAt } from "../../probes";
import { onlyLocale, sessionLabel, type FieldObserver, type Localized, type Observation } from "../../observation";
import { guidEncounters } from "../_guidEncounters";

// One word, the level (or "??"), and an optional rank such as "(Elite)" in ASCII or full-width
// brackets. Titles with a number elsewhere ("SI:7 Operative", "제2대 칸") do not match.
const LEVEL_LINE = /^\S+ (\d+|\?\?)( ?[(（][^)）]*[)）])?$/u;

/** What the tracker's privacy filter replaces player names with (Core.SanitizeText). */
const REDACTED_NAME = "<name>";

/** Longest time (seconds) between a subname and the target change it is inferred for. */
const MAX_SECONDS_BEFORE_TARGET = 1;

export function isLevelLine(subName: string): boolean {
  return LEVEL_LINE.test(subName);
}

function toObservation(session: SessionRecord, entityId: number, sample: FunctionStreamEntry, token: string): Observation<Localized<string>> | undefined {
  if (typeof sample.v !== "string") return undefined;
  const noSubName = isLevelLine(sample.v);
  // The tracker redacts player names to "<name>", so such a title belongs to a player-related unit (e.g. an escort).
  if (!noSubName && sample.v.includes(REDACTED_NAME)) return undefined;
  const locale = getLocaleAt(session, sample.t);
  if (locale === null) return undefined;
  return {
    entityId,
    value: { locale, value: noSubName ? "" : sample.v },
    confidence: "high",
    provenance: { session: sessionLabel(session), t: sample.t },
    token,
  };
}

function observeRecordedGuids(session: SessionRecord, subNames: FunctionStreamEntry[], guids: FunctionStreamEntry[]): Observation<Localized<string>>[] {
  const guidAt = new Map(guids.map((entry) => [`${entry.t}:${entry.tp}`, entry.v]));
  const observations: Observation<Localized<string>>[] = [];
  for (const sample of subNames) {
    const parsed = parseGuid(guidAt.get(`${sample.t}:${sample.tp}`) as string | undefined);
    if (parsed.kind !== "npc" || parsed.id === null) continue;
    const observation = toObservation(session, parsed.id, sample, "mouseover");
    if (observation) observations.push(observation);
  }
  return observations;
}

/** Seconds (tp) at which the player entered (true) or left (false) combat, in event order. */
function combatChanges(session: SessionRecord): { tp: number; inCombat: boolean }[] {
  return session.events
    .filter((event) => event.e === "PLAYER_REGEN_DISABLED" || event.e === "PLAYER_REGEN_ENABLED")
    .map((event) => ({ tp: event.tp, inCombat: event.e === "PLAYER_REGEN_DISABLED" }));
}

function observeBeforeTargetChange(session: SessionRecord, subNames: FunctionStreamEntry[]): Observation<Localized<string>>[] {
  const samples = [...subNames].sort((a, b) => a.tp - b.tp);
  const combat = combatChanges(session);
  const observations: Observation<Localized<string>>[] = [];
  for (const encounter of guidEncounters(session, "npc", ["target"])) {
    let sample: FunctionStreamEntry | undefined;
    for (const candidate of samples) {
      if (candidate.tp > encounter.tp) break;
      sample = candidate;
    }
    if (!sample || encounter.tp - sample.tp > MAX_SECONDS_BEFORE_TARGET) continue;
    let inCombat = false;
    for (const change of combat) {
      if (change.tp > encounter.tp) break;
      inCombat = change.inCombat;
    }
    if (inCombat) continue;
    const observation = toObservation(session, encounter.entityId, sample, "target");
    if (observation) {
      observation.provenance.t = encounter.t;
      observations.push(observation);
    }
  }
  return observations;
}

export const observeLocalizedSubName: FieldObserver<Localized<string>> = (session) => {
  const subNames = getStream(session, "UnitSubName", "mouseover") ?? [];
  const guids = getStream(session, "UnitSubNameGUID", "mouseover");
  return guids ? observeRecordedGuids(session, subNames, guids) : observeBeforeTargetChange(session, subNames);
};

export const observeSubName: FieldObserver<string> = (session) => onlyLocale(observeLocalizedSubName(session), "enUS");
