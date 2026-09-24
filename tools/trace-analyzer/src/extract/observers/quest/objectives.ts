// questKeys.objectives - Questie field 10, positional table
// {creatureObjective, objectObjective, itemObjective, reputationObjective,
// killCreditObjective, spellObjective}. Only the first three are populated
// here; the remaining positions are never written (see mergeQuestObjectives).
//
// C_QuestLog.GetQuestObjectives exposes the objective type, display text, and
// progress, but not the entity IDs required by Questie. This observer matches
// the three objective types supported by the trace against IDs observed
// elsewhere in the same session:
//   item    -> GetLootSlotLink + GetLootSlotInfo
//   monster -> UnitName + UnitGUID
//   object  -> UnitName + UnitGUID
//
// Matching is by exact (case/whitespace-normalized) name equality against
// evidence collected anywhere in the session - kills/loot for a quest
// typically happen well after the quest was accepted, so evidence is not
// restricted to a time window around the objective sample.
//
// Note: Blizzard's "object" objective type is not limited to "interact with a
// GameObject" objectives - it also covers various in-place actions (emotes,
// casting a quest spell, etc.) whose text never matches a GameObject's display
// name, e.g. "Use the /sit emote near the campfire". Those simply produce no
// match, which is expected and harmless.

import { emulate, getParamKeys, getStream, valueAt } from "../../../core/emulator";
import type { FunctionStreamEntry, SessionRecord } from "../../../core/types";
import { getLocaleAt, unitNameAt } from "../../probes";
import { parseGuid } from "../../guid";
import { parseItemLink } from "../../itemLink";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export type ObjectiveKind = "item" | "monster" | "object";

export interface ObjectiveMatch {
  kind: ObjectiveKind;
  name: string;
  text: string;
  id: number;
}

// Questie questKeys.objectives positional indices (see file header comment).
// Only these three of the six positions are ever populated by this observer.
export const CREATURE_OBJECTIVE_INDEX = 1;
export const OBJECT_OBJECTIVE_INDEX = 2;
export const ITEM_OBJECTIVE_INDEX = 3;

/** Positional Questie objectives table: {1: creatures, 2: objects, 3: items}. */
export type QuestObjectivesValue = Partial<
  Record<typeof CREATURE_OBJECTIVE_INDEX | typeof OBJECT_OBJECTIVE_INDEX | typeof ITEM_OBJECTIVE_INDEX, number[]>
>;

interface RawObjective {
  type?: unknown;
  text?: unknown;
}

interface Evidence {
  kind: "item" | "npc" | "object";
  name: string;
  id: number;
  /**
   * Best-effort quest attribution for this piece of evidence, used only as a
   * same-quest tie-break in `candidateForObjective`. Provenance differs by
   * evidence kind: for items this is the quest id `GetLootSlotInfo` itself
   * reports for the loot slot; for units it is whatever `GetQuestID()`
   * (quest-dialog frame) last reported at the interaction's timestamp - a
   * much weaker signal, since it reflects "what dialog was open", not "what
   * quest this kill was for".
   */
  questId: number | null;
}

function normalizeName(name: string): string {
  return name.trim().toLocaleLowerCase();
}

/**
 * Strip the objective's progress counter and, for kill objectives, a trailing
 * " slain" suffix. Real enUS Classic objective text uses two different
 * layouts depending on objective kind:
 *   - monster/object: progress *prefix*, e.g. "0/8 Juvenile Vuldren slain",
 *     "0/1 Destroy the Demon Seed"
 *   - item: progress *suffix*, e.g. "Anaya's Pendant: 0/1"
 * Both progress forms are stripped unconditionally since only one can ever
 * match a given text. The " slain" suffix is only stripped for monster
 * objectives so an item/object name that happens to end in "slain" is left
 * untouched.
 */
function objectiveName(text: string, kind: ObjectiveKind): string | null {
  const withoutPrefix = text.replace(/^\s*\d+\s*\/\s*\d+\s*/, "");
  const withoutSuffix = withoutPrefix.replace(/\s*:\s*\d+\s*\/\s*\d+\s*$/, "");
  const withoutKillSuffix = kind === "monster" ? withoutSuffix.replace(/\s+slain$/i, "") : withoutSuffix;
  const trimmed = withoutKillSuffix.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function objectiveKind(type: unknown): ObjectiveKind | null {
  if (type === "item") return "item";
  if (type === "monster") return "monster";
  if (type === "object") return "object";
  return null;
}

function questIdAt(session: SessionRecord, t: number): number | null {
  const value = valueAt(getStream(session, "GetQuestID") ?? [], t);
  return typeof value === "number" && value !== 0 ? value : null;
}

function readObjectiveList(value: unknown): RawObjective[] {
  const objectives = emulate(value);
  if (!Array.isArray(objectives)) return [];
  return objectives.filter((objective): objective is RawObjective => {
    return objective !== null && typeof objective === "object" && !Array.isArray(objective);
  });
}

function collectItemEvidence(session: SessionRecord): Evidence[] {
  const linkStreams = session.functions["GetLootSlotLink"];
  if (!linkStreams) return [];

  // Older traces stored these as one flat stream. Newer traces may store them
  // as streams keyed by loot slot, so support both representations.
  const linkGroups: FunctionStreamEntry[][] = Array.isArray(linkStreams)
    ? [linkStreams]
    : getParamKeys(linkStreams)
        .map((slot) => getStream(session, "GetLootSlotLink", slot) ?? [])
        .filter((entries) => entries.length > 0);
  const infoRoot = session.functions["GetLootSlotInfo"];
  const infoGroups: FunctionStreamEntry[][] = !infoRoot
    ? []
    : Array.isArray(infoRoot)
      ? [infoRoot]
      : getParamKeys(infoRoot)
          .map((slot) => getStream(session, "GetLootSlotInfo", slot) ?? [])
          .filter((entries) => entries.length > 0);

  const evidence: Evidence[] = [];
  for (let groupIndex = 0; groupIndex < linkGroups.length; groupIndex++) {
    const linkEntries = linkGroups[groupIndex];
    const infoEntries = infoGroups[groupIndex] ?? [];
    for (const linkEntry of linkEntries) {
      if (typeof linkEntry.v !== "string") continue;
      const parsed = parseItemLink(linkEntry.v);
      if (!parsed) continue;

      const info = infoEntries.length > 0 ? emulate(valueAt(infoEntries, linkEntry.t)) : null;
      const questId = Array.isArray(info) && typeof info[6] === "number" ? info[6] : null;
      evidence.push({
        kind: "item",
        name: parsed.name,
        id: parsed.itemID,
        questId: questId && questId !== 0 ? questId : null,
      });
    }
  }
  return evidence;
}

function collectUnitEvidence(session: SessionRecord): Evidence[] {
  const evidence: Evidence[] = [];
  for (const token of ["target", "npc", "questnpc"]) {
    const guidStream = getStream(session, "UnitGUID", token);
    if (!guidStream) continue;

    for (const entry of guidStream) {
      if (typeof entry.v !== "string") continue;
      const parsed = parseGuid(entry.v);
      if (!parsed?.id || (parsed.kind !== "npc" && parsed.kind !== "object")) continue;

      const name = unitNameAt(session, token, entry.t);
      if (!name) continue;

      evidence.push({
        kind: parsed.kind === "npc" ? "npc" : "object",
        name,
        id: parsed.id,
        questId: questIdAt(session, entry.t),
      });
    }
  }
  return evidence;
}

/**
 * Find the best evidence match by exact normalized name. Prefers evidence
 * observed for the same quest; ties (including cross-quest evidence, which is
 * still accepted since kills/loot are not always attributed back to a
 * specific quest) are broken by the lowest id for determinism.
 */
function candidateForObjective(objective: { kind: ObjectiveKind; name: string }, evidence: Evidence[], questId: number): number | null {
  let best: { id: number; sameQuest: boolean } | null = null;

  const evidenceKind = objective.kind === "item" ? "item" : objective.kind === "monster" ? "npc" : "object";
  const wantedName = normalizeName(objective.name);
  for (const item of evidence) {
    if (item.kind !== evidenceKind) continue;
    if (normalizeName(item.name) !== wantedName) continue;
    if (item.questId !== null && item.questId !== questId) continue;

    const sameQuest = item.questId === questId;
    if (!best || (sameQuest && !best.sameQuest) || (sameQuest === best.sameQuest && item.id < best.id)) {
      best = { id: item.id, sameQuest };
    }
  }

  return best?.id ?? null;
}

export const observeObjectives: FieldObserver<ObjectiveMatch[]> = (session) => {
  const observations: Observation<ObjectiveMatch[]>[] = [];
  const itemEvidence = collectItemEvidence(session);
  const unitEvidence = collectUnitEvidence(session);

  const objectivesRoot = session.functions["C_QuestLog.GetQuestObjectives"];
  if (!objectivesRoot || Array.isArray(objectivesRoot)) return observations;

  for (const questIdKey of getParamKeys(objectivesRoot)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    const stream = getStream(session, "C_QuestLog.GetQuestObjectives", questIdKey) ?? [];
    for (const entry of stream) {
      const locale = getLocaleAt(session, entry.t);
      // Older traces may not contain GetLocale. Explicit non-enUS sessions are
      // still rejected; absent locale is treated as the legacy enUS default.
      if (locale !== null && locale !== "enUS") continue;

      const objectiveList = readObjectiveList(entry.v);
      const matches: ObjectiveMatch[] = [];

      for (const rawObjective of objectiveList) {
        const kind = objectiveKind(rawObjective.type);
        const text = typeof rawObjective.text === "string" ? rawObjective.text : null;
        const name = kind && text ? objectiveName(text, kind) : null;
        if (!kind || !text || !name) continue;

        const source = kind === "item" ? itemEvidence : unitEvidence;
        const id = candidateForObjective({ kind, name }, source, questID);
        if (id !== null) matches.push({ kind, name, text, id });
      }

      if (matches.length > 0) {
        observations.push({
          entityId: questID,
          value: matches,
          confidence: "low",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};

/** Merge matched objectives across observations into Questie's positional buckets. */
export function mergeQuestObjectives(observations: Observation<ObjectiveMatch[]>[]): QuestObjectivesValue {
  const creatures = new Set<number>();
  const objects = new Set<number>();
  const items = new Set<number>();

  for (const observation of observations) {
    for (const match of observation.value) {
      const target = match.kind === "monster" ? creatures : match.kind === "object" ? objects : items;
      target.add(match.id);
    }
  }

  const value: QuestObjectivesValue = {};
  if (creatures.size > 0) value[CREATURE_OBJECTIVE_INDEX] = [...creatures].sort((a, b) => a - b);
  if (objects.size > 0) value[OBJECT_OBJECTIVE_INDEX] = [...objects].sort((a, b) => a - b);
  if (items.size > 0) value[ITEM_OBJECTIVE_INDEX] = [...items].sort((a, b) => a - b);
  return value;
}
