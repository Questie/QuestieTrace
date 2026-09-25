// questKeys.triggerEnd - Questie field 9.
// type: table {text, {[zoneID] = {coordPair,...}}
//
// Event objectives are completed by the player in the world rather than by an
// entity interaction. C_QuestLog.GetQuestObjectives exposes their text and
// finished state; correlate a completed sample with the player's map position
// to obtain the location where the objective was completed. Samples repeat the
// finished state long after the actual completion, so a location is recorded
// only on an objective's incomplete->complete transition - or, when the
// objective is first seen already finished, on that first completed sample.

import { emulate, getParamKeys, getStream } from "../../../core/emulator";
import { getLocaleAt, playerPosAt, playerZoneAt, toQuestieMapPosition } from "../../probes";
import { sessionLabel, type FieldObserver, type Observation } from "../../observation";

export type QuestTriggerEndValue = [text: string, locations: Record<number, Array<[number, number]>>];

interface RawObjective {
  type?: unknown;
  text?: unknown;
  finished?: unknown;
}

function readObjectiveList(value: unknown): RawObjective[] {
  const objectives = emulate(value);
  if (!Array.isArray(objectives)) return [];
  return objectives.filter(
    (objective): objective is RawObjective =>
      objective !== null && typeof objective === "object" && !Array.isArray(objective),
  );
}

function coordinateKey(position: [number, number]): string {
  return `${position[0]},${position[1]}`;
}

interface ZoneTally {
  count: number;
  latestT: number;
  coords: Map<string, { position: [number, number]; count: number; latestT: number }>;
}/**
 * Merge repeated samples of the same completed event objective into Questie's
 * positional triggerEnd table.
 *
 * Questie's triggerEnd carries exactly one zone with exactly one coordinate
 * pair, so the merge always produces `[text, { [zone]: [[x, y]] }]`:
 *
 * 1. Only samples sharing the winning text are considered (same rule as
 *    before; the merge is still per-entity, so cross-quest text cannot leak).
 * 2. The zone is picked once for the whole merge: the quest's zoneOrSort zone
 *    (`preferredZone`, the AreaTable/QuestSort header Questie displays the
 *    quest under) wins when any sample observed it; otherwise the zone with
 *    the most observations, tie-broken by the most recent sample and then the
 *    lowest zone id for determinism.
 * 3. Within that zone, coordinate pairs are de-duplicated and the most
 *    frequently observed pair wins; ties break to the most recent sample.
 */
export function mergeQuestTriggerEnds(
  observations: Observation<QuestTriggerEndValue>[],
  preferredZone?: number,
): QuestTriggerEndValue {
  const latestText = [...observations]
    .sort((a, b) => b.provenance.t - a.provenance.t)
    .find((observation) => observation.value[0].trim().length > 0)?.value[0];
  if (latestText === undefined) return ["", {}];

  const zones = new Map<number, ZoneTally>();
  for (const observation of observations) {
    if (observation.value[0] !== latestText) continue;

    for (const [zoneKey, positions] of Object.entries(observation.value[1])) {
      const zoneID = Number(zoneKey);
      if (!Number.isFinite(zoneID)) continue;
      const tally = zones.get(zoneID) ?? { count: 0, latestT: 0, coords: new Map() };
      tally.count++;
      tally.latestT = Math.max(tally.latestT, observation.provenance.t);
      for (const position of positions) {
        const key = coordinateKey(position);
        const coord = tally.coords.get(key) ?? { position, count: 0, latestT: 0 };
        coord.count++;
        coord.latestT = Math.max(coord.latestT, observation.provenance.t);
        tally.coords.set(key, coord);
      }
      zones.set(zoneID, tally);
    }
  }

  let bestZone: { id: number; tally: ZoneTally } | null = null;
  for (const [id, tally] of zones) {
    if (!bestZone || betterZone(id, tally, bestZone.id, bestZone.tally, preferredZone)) {
      bestZone = { id, tally };
    }
  }
  if (!bestZone) return [latestText, {}];

  let bestCoord: { position: [number, number]; count: number; latestT: number } | null = null;
  for (const coord of bestZone.tally.coords.values()) {
    if (
      !bestCoord ||
      coord.count > bestCoord.count ||
      (coord.count === bestCoord.count && coord.latestT > bestCoord.latestT)
    ) {
      bestCoord = coord;
    }
  }
  if (!bestCoord) return [latestText, {}];

  return [latestText, { [bestZone.id]: [bestCoord.position] }];
}

/** Zone selection: preferred zone wins; then more observations, more recent, lower id. */
function betterZone(
  id: number,
  tally: ZoneTally,
  bestId: number,
  best: ZoneTally,
  preferredZone: number | undefined,
): boolean {
  if (preferredZone !== undefined) {
    const isPreferred = id === preferredZone;
    const bestIsPreferred = bestId === preferredZone;
    if (isPreferred !== bestIsPreferred) return isPreferred;
  }
  if (tally.count !== best.count) return tally.count > best.count;
  if (tally.latestT !== best.latestT) return tally.latestT > best.latestT;
  return id < bestId;
}

export const observeTriggerEnd: FieldObserver<QuestTriggerEndValue> = (session) => {
  const observations: Observation<QuestTriggerEndValue>[] = [];
  const objectivesRoot = session.functions["C_QuestLog.GetQuestObjectives"];
  if (!objectivesRoot || Array.isArray(objectivesRoot)) return observations;

  for (const questIdKey of getParamKeys(objectivesRoot)) {
    const questID = Number(questIdKey);
    if (!Number.isFinite(questID)) continue;

    const stream = getStream(session, "C_QuestLog.GetQuestObjectives", questIdKey) ?? [];
    // Objective text -> finished state of the previous positioned sample
    // (undefined = not sampled before). Repeated completed samples are stale
    // re-reads, not completions, so only the transition emits.
    const previousFinished = new Map<string, boolean | undefined>();

    for (const entry of stream) {
      const locale = getLocaleAt(session, entry.t);
      if (locale !== null && locale !== "enUS") continue;

      const zoneID = playerZoneAt(session, entry.t);
      const position = playerPosAt(session, entry.t);
      if (zoneID === null || position === null) continue;

      for (const objective of readObjectiveList(entry.v)) {
        if (objective.type !== "event" || objective.finished === undefined) continue;
        if (typeof objective.text !== "string") continue;
        const text = objective.text.trim();
        if (text.length === 0) continue;

        const finished = objective.finished === true;
        const wasFinished = previousFinished.get(text);
        previousFinished.set(text, finished);

        if (!finished) continue;
        // Emit on the incomplete->complete transition, or on the first
        // completed sample when the objective was never seen incomplete
        // (e.g. the trace starts with the objective already finished).
        if (wasFinished === true) continue;

        const questiePosition = toQuestieMapPosition(position);
        observations.push({
          entityId: questID,
          value: [text, { [zoneID]: [[questiePosition.x, questiePosition.y]] }],
          confidence: "high",
          provenance: { session: sessionLabel(session), t: entry.t },
        });
      }
    }
  }

  return observations;
};
