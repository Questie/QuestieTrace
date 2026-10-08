// questKeys.questLevel - Questie field 5.
// Read from two possible streams, both keyed by questID:
//   - GetQuestLogTitle(questId) tuple position 2 (the "level" field)
//   - C_QuestLog.GetInfo(questId).level (modern API, table return)
//
// These are quest-log streams, not only quest-dialog encounters. Quests can
// appear in a log without GetQuestID() ever being sampled, so the streams are
// observed directly.

import { emulate, getParamKeys, getStream } from "../../../core/emulator";
import type { SessionRecord } from "../../../core/types";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

function readLevel(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (!value || typeof value !== "object") return null;

  const level = (value as Record<string, unknown>).level;
  return typeof level === "number" && Number.isFinite(level) ? level : null;
}

function readLevelFromGetQuestLogTitleEntry(value: unknown): number | null {
  const tuple = emulate(value);
  if (!Array.isArray(tuple) || tuple.length < 2) return null;
  return readLevel(tuple[1]); // position 2 (0-indexed: 1) = level
}

function readLevelFromGetInfoEntry(value: unknown): number | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return readLevel((value as Record<string, unknown>).level);
}

function appendStreamLevels(
  session: SessionRecord,
  observations: Observation<number>[],
  functionName: string,
  readEntry: (value: unknown) => number | null,
  skipKeys: ReadonlySet<string> = new Set<string>(),
): Set<string> {
  const observedKeys = new Set<string>();
  const root = session.functions[functionName];
  if (!root || Array.isArray(root)) return observedKeys;

  const pending: Observation<number>[] = [];
  for (const questIdKey of getParamKeys(root)) {
    if (skipKeys.has(questIdKey)) continue;
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    let hasLevel = false;
    const stream = getStream(session, functionName, questIdKey) ?? [];
    for (const entry of stream) {
      const questLevel = readEntry(entry.v);
      if (questLevel === null) continue;
      hasLevel = true;
      pending.push({
        entityId: questID,
        value: questLevel,
        confidence: "high",
        provenance: { session: sessionLabel(session), t: entry.t },
      });
    }
    if (hasLevel) observedKeys.add(questIdKey);
  }

  pending.sort((a, b) => a.provenance.t - b.provenance.t);
  observations.push(...pending);
  return observedKeys;
}

export const observeQuestLevel: FieldObserver<number> = (session) => {
  const observations: Observation<number>[] = [];

  // Prefer the modern API when both APIs recorded the same quest ID.
  const modernKeys = appendStreamLevels(session, observations, "C_QuestLog.GetInfo", readLevelFromGetInfoEntry);
  appendStreamLevels(
    session,
    observations,
    "GetQuestLogTitle",
    readLevelFromGetQuestLogTitleEntry,
    modernKeys,
  );

  return observations;
};
