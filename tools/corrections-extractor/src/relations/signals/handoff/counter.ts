// Counter-evidence for "X needs P": the character was offered X while P was not completed.
//
// Questie's preQuestSingle is an OR list, so an offer without P is only a contradiction when no
// alternative explains it. Alternatives are structural only: a completed same-name variant of P
// (faction/race/profession copies), or P being closed to the character by race or class. X's other
// claimed prerequisites are deliberately not alternatives: a wrong P (say, a quest that happened to
// be turned in at X's giver just before X auto-opened) would then be excused by the real one.
// What is left splits by where P was: not started (P is no prerequisite at all; X may be a
// breadcrumb target or have another way in) or in the log (X needs P taken, not finished).

import { classAllowed, raceAllowed } from "../../core/eligibility";
import type { EpisodeTimeline } from "../../core/timeline";
import type { GiverRef, SessionTime } from "../../core/types";
import type { QuestLookup } from "./detect";

export type CounterKind = "notStarted" | "inLog" | "alternative";

export interface CounterObservation {
  kind: CounterKind;
  /** P */
  prerequisite: number;
  /** X */
  quest: number;
  t: SessionTime;
  giver?: GiverRef;
  /** Why an alternative explains the offer (alternative observations only). */
  because?: string;
}

/** For each quest X, the prerequisites P this signal claims for it. */
export type ClaimedPrerequisites = ReadonlyMap<number, ReadonlySet<number>>;

/** Other quests whose completion would also make X available without P (see the file comment). */
export type AlternativesOf = (prerequisite: number) => readonly number[];

interface OfferMoment {
  questId: number;
  t: SessionTime;
  giver?: GiverRef;
}

/**
 * Completed-quest deltas are sampled shortly after the change, so a quest completed without a
 * turn-in event can show up to this late (the same tolerance offer-set uses).
 */
export const COMPLETION_TOLERANCE_SECONDS = 1;

export function findCounterEvidence(
  timeline: EpisodeTimeline,
  claimed: ClaimedPrerequisites,
  alternativesOf: AlternativesOf,
  lookup: QuestLookup,
): CounterObservation[] {
  const episode = timeline.episode;
  // Without the completed history there is no way to say P was not completed.
  if (!episode.completed) return [];

  const turnInTimes = new Map<number, SessionTime[]>();
  for (const questEvent of episode.questEvents) {
    if (questEvent.kind === "turnedIn") turnInTimes.set(questEvent.questId, [...(turnInTimes.get(questEvent.questId) ?? []), questEvent.t]);
  }
  const completedForOffer = (questId: number, t: SessionTime): boolean => {
    if (timeline.isCompletedAt(questId, t)) return true;
    // Turn-in events are exact: an offer just before one (say, the list a player saw on arriving
    // to turn P in) was made while P was still open.
    const end = t + COMPLETION_TOLERANCE_SECONDS;
    if (turnInTimes.get(questId)?.some((turnInAt) => turnInAt > t && turnInAt <= end)) return false;
    return timeline.isCompletedAt(questId, end);
  };

  const observations: CounterObservation[] = [];
  const seen = new Set<string>();
  for (const offer of offerMoments(timeline)) {
    const prerequisites = claimed.get(offer.questId);
    if (!prerequisites) continue;
    for (const prerequisite of prerequisites) {
      if (completedForOffer(prerequisite, offer.t)) continue;
      // One observation per (pair, kind) per episode is enough; characters are counted later.
      const observation = classify(timeline, lookup, alternativesOf, prerequisite, offer);
      const key = `${prerequisite}:${offer.questId}:${observation.kind}`;
      if (seen.has(key)) continue;
      seen.add(key);
      observations.push(observation);
    }
  }
  return observations;
}

/** Every moment the character was offered a quest: offer lists, accept dialogs and accepts. */
function offerMoments(timeline: EpisodeTimeline): OfferMoment[] {
  const episode = timeline.episode;
  const moments: OfferMoment[] = [];
  for (const offer of episode.offers) {
    for (const quest of offer.available) moments.push({ questId: quest.id, t: offer.t, giver: offer.giver });
  }
  for (const questEvent of episode.questEvents) {
    if (questEvent.kind === "detail" || questEvent.kind === "accepted") {
      moments.push({ questId: questEvent.questId, t: questEvent.t, giver: questEvent.giver });
    }
  }
  return moments.sort((a, b) => a.t - b.t);
}

function classify(
  timeline: EpisodeTimeline,
  lookup: QuestLookup,
  alternativesOf: AlternativesOf,
  prerequisite: number,
  offer: OfferMoment,
): CounterObservation {
  const base = { prerequisite, quest: offer.questId, t: offer.t, giver: offer.giver };
  const player = timeline.episode.player;
  const prerequisiteQuest = lookup(prerequisite);
  if (prerequisiteQuest && (!raceAllowed(prerequisiteQuest.requiredRaces, player.raceId) || !classAllowed(prerequisiteQuest.requiredClasses, player.classId))) {
    return { ...base, kind: "alternative", because: `${prerequisite} is closed to this race/class` };
  }
  const alternative = alternativesOf(prerequisite).find((other) => timeline.isCompletedAt(other, offer.t));
  if (alternative !== undefined) return { ...base, kind: "alternative", because: `${alternative} completed` };
  return { ...base, kind: timeline.isInLogAt(prerequisite, offer.t) ? "inLog" : "notStarted" };
}
