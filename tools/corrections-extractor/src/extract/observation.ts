// Shared types for the observer layer.
//
// Observers only *observe*: they walk a single session and emit raw, per-session
// Observation records with provenance. Merging observations across sessions into
// a final Fact (confidence/frequency/recency tie-break, or a field's custom merge)
// happens once, in `aggregate.ts` - never inside an observer.

export type Confidence = "high" | "medium" | "low";

export interface Provenance {
  session: string;
  /** Time (seconds) within the session at which this value was observed. */
  t: number;
}

export interface Observation<T> {
  /** Entity id this observation is about (npcID/questID/itemID/objectID). */
  entityId: number;
  value: T;
  confidence: Confidence;
  provenance: Provenance;
}

/**
 * A field observer produces zero or more Observations for a single field, for a
 * single session. `entityId` may repeat across the returned array (e.g. the same
 * NPC seen multiple times in one session).
 */
export type FieldObserver<T> = (session: import("../core/types").SessionRecord) => Observation<T>[];

/** `session.name` is optional (see core/types.ts); observers use this for `Provenance.session`. */
export function sessionLabel(session: import("../core/types").SessionRecord): string {
  return session.name ?? "(unnamed session)";
}
