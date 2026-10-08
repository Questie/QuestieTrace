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

function mergeScalar<T>(
  observations: Observation<T>[],
): { value: T; confidence: Confidence; alternativeCount: number } {
  interface Group {
    value: T;
    confidence: Confidence;
    count: number;
    latestT: number;
  }
  const groups = new Map<string, Group>();
  for (const obs of observations) {
    const key = JSON.stringify(obs.value);
    const existing = groups.get(key);
    if (!existing) {
      groups.set(key, { value: obs.value, confidence: obs.confidence, count: 1, latestT: obs.provenance.t });
      continue;
    }
    existing.count++;
    if (CONFIDENCE_RANK[obs.confidence] > CONFIDENCE_RANK[existing.confidence]) {
      existing.confidence = obs.confidence;
    }
    if (obs.provenance.t > existing.latestT) {
      existing.latestT = obs.provenance.t;
    }
  }

  const sorted = [...groups.values()].sort((a, b) => {
    if (CONFIDENCE_RANK[b.confidence] !== CONFIDENCE_RANK[a.confidence]) {
      return CONFIDENCE_RANK[b.confidence] - CONFIDENCE_RANK[a.confidence];
    }
    if (b.count !== a.count) {
      return b.count - a.count;
    }
    return b.latestT - a.latestT;
  });

  const winner = sorted[0];
  return { value: winner.value, confidence: winner.confidence, alternativeCount: sorted.length - 1 };
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
  const byEntity = new Map<number, Observation<T>[]>();
  for (const obs of observations) {
    const list = byEntity.get(obs.entityId);
    if (list) {
      list.push(obs);
    } else {
      byEntity.set(obs.entityId, [obs]);
    }
  }

  const facts = new Map<number, Fact<T> | Fact<R>>();
  for (const [entityId, entityObservations] of byEntity) {
    if (customMerge) {
      facts.set(entityId, {
        entityId,
        value: customMerge(entityObservations),
        confidence: highestConfidence(entityObservations),
        observationCount: entityObservations.length,
        alternativeCount: 0,
      });
      continue;
    }

    const { value, confidence, alternativeCount } = mergeScalar(entityObservations);
    facts.set(entityId, {
      entityId,
      value,
      confidence,
      observationCount: entityObservations.length,
      alternativeCount,
    });
  }
  return facts;
}
