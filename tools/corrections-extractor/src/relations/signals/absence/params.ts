// Tunables of the absence signal. Written into the candidate file for reproducibility.

import type { OfferSnapshot } from "../../core/types";

export interface AbsenceParams {
  /** Seconds of log/completion sampling lag to correct for with quest events (see states.ts). */
  stateSlack: number;
  /** A completed-set delta adding more quests than this is a catch-up read of old completions. */
  catchUpBatch: number;
  /** A list read within this many seconds of any dialog/accept/turn-in of X says nothing about X. */
  selfSlack: number;
  /**
   * Quests that always complete in one batch (Forever's zone copies) are one suspect. A bundle
   * must be seen in this many batches, and be no larger than maxBundle, to rule out bulk adds.
   */
  minBundleBatches: number;
  maxBundle: number;
  /** Seconds after a turn-in in which an offer at the same giver counts as a hand-off. */
  handoffWindow: number;
  /**
   * Snapshot sources trusted as complete lists for absence. A standalone QUEST_DETAIL is also how
   * finishers push a follow-up after a turn-in, so it is not the giver's whole list.
   */
  blockedSources: Array<OfferSnapshot["source"]>;
  /**
   * Gossip sub-menus carry no quest list. An empty gossip list read within this many seconds of
   * a non-empty one from the same giver, with no quest event in between, is taken for one.
   */
  subMenuWindow: number;
  /** Distinct offered races/classes beyond which the observed demographics stop gating. */
  openDemographics: number;
  /** Characters allowed to contradict a hypothesis before it is dropped (sampling glitches). */
  maxContradictions: number;
  /**
   * Share of offered characters that may lack a quest that still counts as required. One odd
   * offer (an alternate starter, a stale read) must not erase a prerequisite for everyone.
   */
  requiredTolerance: number;
  /** Blame weight of a suspect with no structural tie to the blocked quest (see locality.ts). */
  unrelatedWeight: number;
  /** Suspects whose prior share of a moment is below this are lumped into the unknown cause. */
  minShare: number;
  /** Prior weight of "something we cannot see" as the cause of any blocked moment (attribute.ts). */
  unknownWeight: number;
  emIterations: number;
  /** Candidates scoring below this are left out of the file (they are almost never right). */
  minScore: number;
}

export const DEFAULT_PARAMS: AbsenceParams = {
  stateSlack: 2,
  catchUpBatch: 60,
  selfSlack: 10,
  minBundleBatches: 2,
  maxBundle: 10,
  handoffWindow: 30,
  blockedSources: ["gossip", "greeting"],
  subMenuWindow: 60,
  openDemographics: 3,
  maxContradictions: 0,
  requiredTolerance: 0.1,
  unrelatedWeight: 0.02,
  minShare: 0.05,
  unknownWeight: 0.1,
  emIterations: 30,
  minScore: 0.05,
};
