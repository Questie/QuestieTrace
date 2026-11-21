// npcKeys.subName - Questie field 14.
// observeSubName keeps enUS subnames only; observeLocalizedSubName keeps every locale (for the lookup export).
//
// Source: UnitSubName.mouseover from Modules/Trackers/UnitState.lua (the line below the name
// in the unit tooltip). "" means the NPC has no subname. Unlike the other UnitState values, it
// is not sampled on UPDATE_MOUSEOVER_UNIT but from the tooltip, and only written on change: a
// value is only paired with the mouseover GUID sampled in the same frame before it, so a
// tooltip that was not shown for an NPC cannot hand it the previous NPC's subname.
//
// NPCs without a subname show their level line there instead ("Level 28", "Stufe ?? (Boss)",
// "等级 10（精英）"), which the tracker does not tell apart. Such a value means "no subname".

import { getStream } from "../../../core/emulator";
import { getLocaleAt } from "../../probes";
import { onlyLocale, sessionLabel, type FieldObserver, type Localized, type Observation } from "../../observation";
import { guidEncounters } from "../_guidEncounters";

// One word, the level (or "??"), and an optional rank such as "(Elite)" in ASCII or full-width
// brackets. Titles with a number elsewhere ("SI:7 Operative", "제2대 칸") do not match.
const LEVEL_LINE = /^\S+ (\d+|\?\?)( ?[(（][^)）]*[)）])?$/u;

export function isLevelLine(subName: string): boolean {
  return LEVEL_LINE.test(subName);
}

export const observeLocalizedSubName: FieldObserver<Localized<string>> = (session) => {
  const observations: Observation<Localized<string>>[] = [];
  const stream = getStream(session, "UnitSubName", "mouseover") ?? [];
  const guidSamples = getStream(session, "UnitGUID", "mouseover") ?? [];
  for (const encounter of guidEncounters(session, "npc", ["mouseover"])) {
    // A later GUID sample in the same frame starts another unit.
    const nextTp = guidSamples.find((entry) => entry.t === encounter.t && entry.tp > encounter.tp)?.tp ?? Infinity;
    const sample = stream.find((entry) => entry.t === encounter.t && entry.tp >= encounter.tp && entry.tp < nextTp);
    if (typeof sample?.v !== "string") continue;
    const locale = getLocaleAt(session, encounter.t);
    if (locale === null) continue;
    observations.push({
      entityId: encounter.entityId,
      value: { locale, value: isLevelLine(sample.v) ? "" : sample.v },
      confidence: "high",
      provenance: { session: sessionLabel(session), t: encounter.t },
    });
  }
  return observations;
};

export const observeSubName: FieldObserver<string> = (session) => onlyLocale(observeLocalizedSubName(session), "enUS");
