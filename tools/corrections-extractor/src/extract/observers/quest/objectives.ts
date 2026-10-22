// questKeys.objectives - Questie field 10, positional table
// {creatureObjective, objectObjective, itemObjective, reputationObjective,
// killCreditObjective, spellObjective}. Only the first three are populated
// here; the remaining positions are never written (see mergeQuestObjectives).
//
// C_QuestLog.GetQuestObjectives exposes the objective type, display text, and
// progress, but not the entity IDs required by Questie. This observer matches
// the three objective types supported by the trace against IDs observed
// elsewhere in the same session:
//   item    -> GetLootSlotLink + GetLootSlotInfo, the turn-in window's
//              GetQuestItemLink("required", i), and GetMerchantItemLink
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
  /** Position in the C_QuestLog.GetQuestObjectives result (0-based). */
  index: number;
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
   * Whether this evidence should win over same-named alternatives for the
   * quest. Items: `GetLootSlotInfo` flags it as a quest item. Units: the
   * `GetQuestID()` (quest-dialog frame) last reported at the interaction's
   * timestamp - a weak "what dialog was open" signal - matches the quest.
   */
  preferredFor: (questId: number) => boolean;
  /**
   * Quest this item starts, from `GetLootSlotInfo`. Such an item ranks below
   * same-named alternatives for that quest, but remains a valid fallback.
   */
  startsQuestId: number | null;
  /** Quest whose turn-in window listed this item as required. Wins over all other evidence. */
  requiredForQuestId: number | null;
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

/** Unfiltered so array positions keep matching the API's objective indices. */
function readObjectiveList(value: unknown): unknown[] {
  const objectives = emulate(value);
  return Array.isArray(objectives) ? objectives : [];
}

function isRawObjective(objective: unknown): objective is RawObjective {
  return objective !== null && typeof objective === "object" && !Array.isArray(objective);
}

function collectItemEvidence(session: SessionRecord): Evidence[] {
  const linkStreams = session.functions["GetLootSlotLink"];
  if (!linkStreams) return [];

  // Older traces stored these as one flat stream. Newer traces may store them
  // as streams keyed by loot slot, so support both representations. Keyed
  // streams must be paired by the same slot; filtering each root separately
  // would shift info from another slot onto links with no matching samples.
  const infoRoot = session.functions["GetLootSlotInfo"];
  const linkGroups: Array<{ links: FunctionStreamEntry[]; infos: FunctionStreamEntry[] }> = Array.isArray(linkStreams)
    ? [{ links: linkStreams, infos: Array.isArray(infoRoot) ? infoRoot : [] }]
    : getParamKeys(linkStreams)
        .map((slot) => ({
          links: getStream(session, "GetLootSlotLink", slot) ?? [],
          infos: infoRoot && !Array.isArray(infoRoot) ? getStream(session, "GetLootSlotInfo", slot) ?? [] : [],
        }))
        .filter(({ links }) => links.length > 0);

  const evidence: Evidence[] = [];
  for (const { links: linkEntries, infos: infoEntries } of linkGroups) {
    for (const linkEntry of linkEntries) {
      if (typeof linkEntry.v !== "string") continue;
      const parsed = parseItemLink(linkEntry.v);
      if (!parsed) continue;

      // GetLootSlotInfo returns (icon, name, quantity, currencyID, quality,
      // locked, isQuestItem, questID, isActive); questID is the quest the item starts.
      const info = infoEntries.length > 0 ? emulate(valueAt(infoEntries, linkEntry.t)) : null;
      const isQuestItem = Array.isArray(info) && info[6] === true;
      const startsQuestId = Array.isArray(info) && typeof info[7] === "number" && info[7] !== 0 ? info[7] : null;
      evidence.push({
        kind: "item",
        name: parsed.name,
        id: parsed.itemID,
        preferredFor: () => isQuestItem,
        startsQuestId,
        requiredForQuestId: null,
      });
    }
  }
  return evidence;
}

/** Item links of one function keyed by index, e.g. GetMerchantItemLink[index] or GetQuestItemLink.required[index]. */
function linkEntriesByIndex(session: SessionRecord, functionName: string, ...prefix: string[]): FunctionStreamEntry[] {
  let root: unknown = session.functions[functionName];
  for (const key of prefix) root = root && !Array.isArray(root) ? (root as Record<string, unknown>)[key] : undefined;
  if (!root || Array.isArray(root) || typeof root !== "object") return [];
  return Object.keys(root).flatMap((index) => getStream(session, functionName, ...prefix, index) ?? []);
}

/**
 * Required items of a quest's turn-in window (with the quest from GetQuestID)
 * and items a merchant sells, e.g. vendor-bought objectives like Empty Vial.
 */
function collectLinkEvidence(session: SessionRecord): Evidence[] {
  const evidence: Evidence[] = [];
  for (const entry of linkEntriesByIndex(session, "GetQuestItemLink", "required")) {
    const parsed = typeof entry.v === "string" ? parseItemLink(entry.v) : null;
    if (!parsed) continue;
    evidence.push({
      kind: "item",
      name: parsed.name,
      id: parsed.itemID,
      preferredFor: () => false,
      startsQuestId: null,
      requiredForQuestId: questIdAt(session, entry.t),
    });
  }
  for (const entry of linkEntriesByIndex(session, "GetMerchantItemLink")) {
    const parsed = typeof entry.v === "string" ? parseItemLink(entry.v) : null;
    if (!parsed) continue;
    evidence.push({
      kind: "item",
      name: parsed.name,
      id: parsed.itemID,
      preferredFor: () => false,
      startsQuestId: null,
      requiredForQuestId: null,
    });
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

      const dialogQuestId = questIdAt(session, entry.t);
      evidence.push({
        kind: parsed.kind === "npc" ? "npc" : "object",
        name,
        id: parsed.id,
        preferredFor: (questId) => dialogQuestId === questId,
        startsQuestId: null,
        requiredForQuestId: null,
      });
    }
  }
  return evidence;
}

/**
 * Find the best evidence match by exact normalized name, ranked: items the
 * quest's turn-in window requires, then preferred evidence (see
 * `Evidence.preferredFor`), then other evidence, then items that
 * start the quest. Starter items stay eligible since some quests ask to bring
 * back the very item that started them. Remaining ties go to the lowest id.
 */
function candidateForObjective(objective: { kind: ObjectiveKind; name: string }, evidence: Evidence[], questId: number): number | null {
  let best: { id: number; rank: number } | null = null;

  const evidenceKind = objective.kind === "item" ? "item" : objective.kind === "monster" ? "npc" : "object";
  const wantedName = normalizeName(objective.name);
  for (const item of evidence) {
    if (item.kind !== evidenceKind) continue;
    if (normalizeName(item.name) !== wantedName) continue;

    const rank = item.requiredForQuestId === questId ? 3 : item.preferredFor(questId) ? 2 : item.startsQuestId === questId ? 0 : 1;
    if (!best || rank > best.rank || (rank === best.rank && item.id < best.id)) {
      best = { id: item.id, rank };
    }
  }

  return best?.id ?? null;
}

export const observeObjectives: FieldObserver<ObjectiveMatch[]> = (session) => {
  const observations: Observation<ObjectiveMatch[]>[] = [];
  const itemEvidence = [...collectItemEvidence(session), ...collectLinkEvidence(session)];
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

      for (const [index, rawObjective] of objectiveList.entries()) {
        if (!isRawObjective(rawObjective)) continue;
        const kind = objectiveKind(rawObjective.type);
        const text = typeof rawObjective.text === "string" ? rawObjective.text : null;
        const name = kind && text ? objectiveName(text, kind) : null;
        if (!kind || !text || !name) continue;

        const source = kind === "item" ? itemEvidence : unitEvidence;
        const id = candidateForObjective({ kind, name }, source, questID);
        if (id !== null) matches.push({ kind, name, text, id, index });
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
  // Order by the objective's index in the C_QuestLog.GetQuestObjectives result,
  // since unmatched objectives leave gaps in each observation's match list.
  // Different observations may resolve the same objective slot to different
  // ids (e.g. several creatures sharing a name), so each slot is reduced to a
  // single id: the one matched in the most sessions, lowest id on ties.
  const sessionsBySlot: Record<ObjectiveKind, Map<number, Map<number, Set<number | string>>>> = {
    monster: new Map(),
    object: new Map(),
    item: new Map(),
  };
  for (const observation of observations) {
    for (const match of observation.value) {
      const slots = sessionsBySlot[match.kind];
      const candidates = slots.get(match.index) ?? new Map<number, Set<number | string>>();
      const sessions = candidates.get(match.id) ?? new Set<number | string>();
      sessions.add(observation.provenance.sessionKey ?? observation.provenance.session);
      candidates.set(match.id, sessions);
      slots.set(match.index, candidates);
    }
  }

  const pick = (candidates: Map<number, Set<number | string>>) =>
    [...candidates].sort((a, b) => b[1].size - a[1].size || a[0] - b[0])[0][0];

  const ordered = (kind: ObjectiveKind) => {
    const ids = [...sessionsBySlot[kind]].sort((a, b) => a[0] - b[0]).map(([, candidates]) => pick(candidates));
    return [...new Set(ids)];
  };

  const creatures = ordered("monster");
  const objects = ordered("object");
  const items = ordered("item");

  const value: QuestObjectivesValue = {};
  if (creatures.length > 0) value[CREATURE_OBJECTIVE_INDEX] = creatures;
  if (objects.length > 0) value[OBJECT_OBJECTIVE_INDEX] = objects;
  if (items.length > 0) value[ITEM_OBJECTIVE_INDEX] = items;
  return value;
}
