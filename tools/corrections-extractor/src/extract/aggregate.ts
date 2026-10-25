// Merges per-session Observations for a single field into one Fact per entity.
//
// Default (scalar) merge policy: highest confidence wins; ties broken by
// most-frequently-observed value; further ties broken by most-recent
// observation. Fields whose correct merge isn't "pick one winner" (e.g.
// `spawns` accumulating per-zone coordinates) can pass a custom `merge`
// function instead - see decision 21 in the extraction plan.

import type { Confidence, Observation } from "./observation";

export interface Fact<T> {
  entityId: number;
  value: T;
  confidence: Confidence;
  observationCount: number;
  /** How many distinct alternative values were seen and NOT picked (scalar merge only; always 0 for custom merges). */
  alternativeCount: number;
}

const CONFIDENCE_RANK: Record<Confidence, number> = { high: 3, medium: 2, low: 1 };

function highestConfidence(observations: Observation<unknown>[]): Confidence {
  let best: Confidence = "low";
  for (const obs of observations) {
    if (CONFIDENCE_RANK[obs.confidence] > CONFIDENCE_RANK[best]) {
      best = obs.confidence;
    }
  }
  return best;
}

interface ScalarGroup<T> {
  value: T;
  confidence: Confidence;
  count: number;
  latestT: number;
  /** Distinct sessions (by provenance.sessionKey) with a server time that observed this value. */
  timedSessionCount: number;
  lastSessionKey: number | undefined;
  /** Newest provenance.sessionTime among those sessions, null when none had one. */
  newestSessionTime: number | null;
}

/**
 * Per entity, the distinct values observed for a scalar field, keyed by their
 * JSON form. Holds everything the scalar merge needs, so raw observations can
 * be dropped as soon as they are added.
 */
export type ScalarSummary<T> = Map<number, Map<string, ScalarGroup<T>>>;

export function addToScalarSummary<T>(summary: ScalarSummary<T>, observations: Observation<T>[]): void {
  for (const obs of observations) {
    let groups = summary.get(obs.entityId);
    if (!groups) {
      groups = new Map();
      summary.set(obs.entityId, groups);
    }
    const key = JSON.stringify(obs.value);
    const sessionKey = obs.provenance.sessionKey;
    const sessionTime = obs.provenance.sessionTime ?? null;
    const existing = groups.get(key);
    if (!existing) {
      groups.set(key, {
        value: obs.value,
        confidence: obs.confidence,
        count: 1,
        latestT: obs.provenance.t,
        timedSessionCount: sessionTime === null ? 0 : 1,
        lastSessionKey: sessionKey,
        newestSessionTime: sessionTime,
      });
      continue;
    }
    existing.count++;
    if (CONFIDENCE_RANK[obs.confidence] > CONFIDENCE_RANK[existing.confidence]) {
      existing.confidence = obs.confidence;
    }
    if (obs.provenance.t > existing.latestT) {
      existing.latestT = obs.provenance.t;
    }
    // Observations of one session are added together, so a new key means a new session.
    if (sessionTime !== null && (sessionKey === undefined || sessionKey !== existing.lastSessionKey)) {
      existing.timedSessionCount++;
    }
    existing.lastSessionKey = sessionKey;
    if (sessionTime !== null && (existing.newestSessionTime === null || sessionTime > existing.newestSessionTime)) {
      existing.newestSessionTime = sessionTime;
    }
  }
}

function defaultOrder<T>(a: ScalarGroup<T>, b: ScalarGroup<T>): number {
  if (CONFIDENCE_RANK[b.confidence] !== CONFIDENCE_RANK[a.confidence]) {
    return CONFIDENCE_RANK[b.confidence] - CONFIDENCE_RANK[a.confidence];
  }
  if (b.count !== a.count) {
    return b.count - a.count;
  }
  return b.latestT - a.latestT;
}

function toFact<T>(entityId: number, winner: ScalarGroup<T>, groups: ScalarGroup<T>[]): Fact<T> {
  return {
    entityId,
    value: winner.value,
    confidence: winner.confidence,
    observationCount: groups.reduce((total, group) => total + group.count, 0),
    alternativeCount: groups.length - 1,
  };
}

/** Picks one Fact per entity from a scalar summary (see the default merge policy above). */
export function aggregateScalarSummary<T>(summary: ScalarSummary<T>): Map<number, Fact<T>> {
  const facts = new Map<number, Fact<T>>();
  for (const [entityId, groups] of summary) {
    const sorted = [...groups.values()].sort(defaultOrder);
    facts.set(entityId, toFact(entityId, sorted[0], sorted));
  }
  return facts;
}

/** Sessions a value needs before it can win by being the newest. */
const NEWEST_MIN_SESSIONS = 2;

/**
 * Like aggregateScalarSummary, but for values that change over time (e.g. a
 * renamed NPC): among the values seen in at least NEWEST_MIN_SESSIONS sessions
 * with a server time, the one seen in the newest session wins. Without such a
 * value, the default merge policy decides. Requires provenance.sessionKey, as
 * set by foldSessions.
 */
export function aggregateNewestScalarSummary<T>(summary: ScalarSummary<T>): Map<number, Fact<T>> {
  const facts = new Map<number, Fact<T>>();
  for (const [entityId, groups] of summary) {
    const all = [...groups.values()];
    const candidates = all
      .filter((group) => group.timedSessionCount >= NEWEST_MIN_SESSIONS)
      .sort((a, b) => (b.newestSessionTime as number) - (a.newestSessionTime as number) || defaultOrder(a, b));
    facts.set(entityId, toFact(entityId, candidates[0] ?? all.sort(defaultOrder)[0], all));
  }
  return facts;
}

/**
 * Aggregates all Observations for one field (across every session) into one
 * Fact per entity id.
 *
 * When `customMerge` is provided, it receives all observations for one entity
 * and returns the merged value. The return type R may differ from the
 * observation value type T (e.g. observations carry individual pieces, merge
 * combines them into an array or compound object).
 */
export function aggregateField<T>(observations: Observation<T>[]): Map<number, Fact<T>>;
export function aggregateField<T, R>(
  observations: Observation<T>[],
  customMerge: (observationsForEntity: Observation<T>[]) => R,
): Map<number, Fact<R>>;
export function aggregateField<T, R>(
  observations: Observation<T>[],
  customMerge?: (observationsForEntity: Observation<T>[]) => R,
): Map<number, Fact<T> | Fact<R>> {
  if (!customMerge) {
    const summary: ScalarSummary<T> = new Map();
    addToScalarSummary(summary, observations);
    return aggregateScalarSummary(summary);
  }

  const byEntity = new Map<number, Observation<T>[]>();
  for (const obs of observations) {
    const list = byEntity.get(obs.entityId);
    if (list) {
      list.push(obs);
    } else {
      byEntity.set(obs.entityId, [obs]);
    }
  }

  const facts = new Map<number, Fact<R>>();
  for (const [entityId, entityObservations] of byEntity) {
    facts.set(entityId, {
      entityId,
      value: customMerge(entityObservations),
      confidence: highestConfidence(entityObservations),
      observationCount: entityObservations.length,
      alternativeCount: 0,
    });
  }
  return facts;
}
