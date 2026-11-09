// Sessions -> per-locale Questie l10n lookup modules (lookup<Entity>/<locale>.lua).
//
// Uses the same Forever-session and new-ID filtering as the corrections export
// (see index.ts). enUS is skipped: its names already go into forever*Traces.lua.

import type { SessionRecord } from "../core/types";
import { addScalarGroup, addToScalarSummary, aggregateNewestScalarSummary, aggregateScalarSummary, type ScalarGroup, type ScalarSummary } from "./aggregate";
import type { Localized, Observation } from "./observation";
import { MAX_IDS } from "./observe";
import { LOOKUP_OBSERVERS as OBSERVERS, observeLookupSession, type LookupObserverKey as ObserverKey, type StagedLookupSession } from "./observe-lookups";
import { writeLookupLua } from "./schema/lookup-writer";

export { observeLookupSession, type StagedLookupSession } from "./observe-lookups";

const SKIPPED_LOCALE = "enUS";

/** Names can change over time (e.g. a renamed NPC), so the newest one wins. */
const NEWEST_WINS = new Set<ObserverKey>(["npcName", "questName", "itemName", "objectName"]);

/** Per field, a scalar summary per locale. */
export interface LookupAccumulator {
  sessionCount: number;
  summaries: Record<ObserverKey, Map<string, ScalarSummary<unknown>>>;
}

/** Directory name -> locale -> Lua module content. */
export type LookupBundle = Record<string, Record<string, string>>;

export function createLookupAccumulator(): LookupAccumulator {
  const summaries = {} as LookupAccumulator["summaries"];
  for (const key of Object.keys(OBSERVERS) as ObserverKey[]) {
    summaries[key] = new Map();
  }
  return { sessionCount: 0, summaries };
}

/** Adds a summarized value group of a lookup field under `locale` (see addScalarGroup). */
export function addLookupScalarGroup(acc: LookupAccumulator, field: string, locale: string, entityId: number, group: Omit<ScalarGroup<unknown>, "lastSessionKey">): void {
  const byLocale = acc.summaries[field as ObserverKey] as Map<string, ScalarSummary<unknown>> | undefined;
  if (!byLocale) throw new Error(`Unknown lookup field ${field}`);
  if (locale === SKIPPED_LOCALE) return;
  let summary = byLocale.get(locale);
  if (!summary) {
    summary = new Map();
    byLocale.set(locale, summary);
  }
  addScalarGroup(summary, entityId, group);
}

function addLocalized(byLocale: Map<string, ScalarSummary<unknown>>, observations: Observation<Localized<unknown>>[]): void {
  for (const obs of observations) {
    if (obs.value.locale === SKIPPED_LOCALE) continue;
    let summary = byLocale.get(obs.value.locale);
    if (!summary) {
      summary = new Map();
      byLocale.set(obs.value.locale, summary);
    }
    addToScalarSummary(summary, [{ ...obs, value: obs.value.value }]);
  }
}

/** All observers run before anything is added, so a throwing observer leaves `acc` untouched. */
export function foldLookupSessions(acc: LookupAccumulator, sessions: SessionRecord[]): void {
  const staged = sessions.map(observeLookupSession).filter((s): s is StagedLookupSession => s !== undefined);
  addStagedLookupSessions(acc, staged);
}

/** Adds observed sessions, in order. */
export function addStagedLookupSessions(acc: LookupAccumulator, staged: StagedLookupSession[]): void {
  for (const session of staged) {
    acc.sessionCount++;
    const sessionKey = session.sessionKey ?? `#${acc.sessionCount}`;
    for (const [key, observations] of session.observations) {
      for (const observation of observations) {
        observation.provenance.sessionKey = sessionKey;
        observation.provenance.sessionTime = session.sessionTime;
        if (session.contributorId !== undefined) observation.provenance.contributorId = session.contributorId;
      }
      addLocalized(acc.summaries[key], observations);
    }
  }
}

export function finalizeLookups(acc: LookupAccumulator): LookupBundle {
  const maxIds = MAX_IDS;

  function valuesFor(key: ObserverKey, locale: string, maxId: number): Map<number, unknown> {
    const values = new Map<number, unknown>();
    const summary = acc.summaries[key].get(locale);
    if (!summary) return values;
    const aggregate = NEWEST_WINS.has(key) ? aggregateNewestScalarSummary : aggregateScalarSummary;
    for (const [id, fact] of aggregate(summary)) {
      if (id > maxId) values.set(id, fact.value);
    }
    return values;
  }

  function locales(...keys: ObserverKey[]): string[] {
    return [...new Set(keys.flatMap((key) => [...acc.summaries[key].keys()]))].sort();
  }

  const bundle: LookupBundle = { lookupNpcs: {}, lookupQuests: {}, lookupItems: {}, lookupObjects: {} };

  for (const locale of locales("npcName")) {
    const names = valuesFor("npcName", locale, maxIds.npc);
    // Subnames are not traced, so they are always nil.
    const entries = new Map([...names].map(([id, name]) => [id, [name, null]]));
    if (entries.size > 0) bundle.lookupNpcs[locale] = writeLookupLua("npcNameLookup", locale, entries);
  }

  for (const locale of locales("questName")) {
    const names = valuesFor("questName", locale, maxIds.quest);
    const objectives = valuesFor("objectivesText", locale, maxIds.quest);
    // Quests without an observed title are left out: Questie needs the title.
    const entries = new Map([...names].map(([id, name]) => [id, [name, objectives.get(id) ?? null]]));
    if (entries.size > 0) bundle.lookupQuests[locale] = writeLookupLua("questLookup", locale, entries);
  }

  for (const locale of locales("itemName")) {
    const entries = valuesFor("itemName", locale, maxIds.item);
    if (entries.size > 0) bundle.lookupItems[locale] = writeLookupLua("itemLookup", locale, entries);
  }

  for (const locale of locales("objectName")) {
    const entries = valuesFor("objectName", locale, maxIds.object);
    if (entries.size > 0) bundle.lookupObjects[locale] = writeLookupLua("objectLookup", locale, entries);
  }

  return bundle;
}
