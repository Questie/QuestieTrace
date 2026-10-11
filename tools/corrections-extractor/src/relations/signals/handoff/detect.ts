// Finds hand-offs in one episode: the character turns in P at giver G, and G then offers X.
//
// Two shapes, from strongest to weakest:
// - pop: G shows X's accept dialog right after the turn-in, without the player picking X from a
//   list. Servers do this for the turned-in quest's NextQuestInChain when the same giver starts
//   it. On Forever a giver whose only available quest is X also opens it directly, so a pop shows
//   that X was available, and only suggests nextQuestInChain(P) = X (see ./candidates.ts).
// - list: G's complete offer list after the turn-in contains X, while G's complete list before it
//   did not. Servers leave unavailable quests out, so X became available in between. Forever
//   usually re-sends G's list 0.2-0.5s after a turn-in that pops nothing, so most of these lists
//   are immediate. P's completion is one explanation; anything else that changed in between (a
//   level-up, another completion, a quest leaving the log) is recorded as a confounder.

import type { EpisodeTimeline } from "../../core/timeline";
import type { CatalogQuest, GiverRef, OfferSnapshot, QuestEpisode, SessionTime } from "../../core/types";

export interface HandoffParams {
  /**
   * Latest an auto-offered accept dialog may follow its turn-in, in seconds. Forever pops arrive
   * 0.1-0.5s after the turn-in; a thin tail of lagged pops reaches about 3s.
   */
  popWindow: number;
  /** Oldest the "before" list may be, in seconds before the turn-in. */
  listBefore: number;
  /** Latest the "after" list may be, in seconds after the turn-in. */
  listAfter: number;
}

export type QuestLookup = (questId: number) => CatalogQuest | undefined;

export interface TurnIn {
  questId: number;
  t: SessionTime;
  /** From the turn-in event, or else from the quest's completion dialog just before it. */
  giver?: GiverRef;
  /** Seconds to the next accept dialog at the same giver with no offer list in between (timing report). */
  nextDetailDelay?: number;
  /** Seconds since the giver's last complete offer list (timing report). */
  previousListDelay?: number;
  /** Seconds to the giver's next complete offer list (timing report). */
  nextListDelay?: number;
}

export type HandoffKind = "pop" | "list";

export interface HandoffObservation {
  kind: HandoffKind;
  /** P: the quest that was turned in. */
  turnedIn: number;
  /** X: the quest the giver offered afterwards. */
  offered: number;
  giver: GiverRef;
  turnInAt: SessionTime;
  offeredAt: SessionTime;
  /** List observations: when the before-list was read. */
  previousListAt?: SessionTime;
  /** Other changes that could also explain X being offered. Empty means a clean observation. */
  confounders: string[];
}

export interface EpisodeHandoffs {
  turnIns: TurnIn[];
  observations: HandoffObservation[];
  /** Quests on compared before/after lists, newly offered or not: the signal judged them. */
  comparedQuests: Set<number>;
}

/** Turn-ins for which the completion dialog's giver may stand in for a missing turn-in giver. */
const COMPLETION_DIALOG_MAX_AGE = 120;

export function sameGiver(a: GiverRef | undefined, b: GiverRef | undefined): boolean {
  return !!a && !!b && a.kind === b.kind && a.id === b.id;
}

export function giverLabel(giver: GiverRef): string {
  return `${giver.kind} ${giver.id}`;
}

/** Repeatable quests never show as completed, so they cannot gate anything we can observe. */
export function isRepeatable(quest: CatalogQuest | undefined): boolean {
  return quest?.specialFlags !== undefined && quest.specialFlags % 2 === 1;
}

function isGiverList(offer: OfferSnapshot): boolean {
  return (offer.source === "gossip" || offer.source === "greeting") && offer.giver !== undefined;
}

export function findHandoffs(timeline: EpisodeTimeline, lookup: QuestLookup, params: HandoffParams): EpisodeHandoffs {
  const episode = timeline.episode;
  const turnIns = resolveTurnIns(episode);
  const anyLists = episode.offers.filter(isGiverList);
  const lists = anyLists.filter((offer) => offer.listComplete);
  const repeatableInLists = repeatableInEpisode(episode);
  const isRepeatableQuest = (questId: number) => repeatableInLists.has(questId) || isRepeatable(lookup(questId));
  const observations: HandoffObservation[] = [];
  const comparedQuests = new Set<number>();

  for (const turnIn of turnIns) {
    if (!turnIn.giver) continue;
    const giver = turnIn.giver;
    const before = lastListBefore(lists, giver, turnIn.t);
    const after = firstListAfter(lists, giver, turnIn.t);
    if (before) turnIn.previousListDelay = turnIn.t - before.t;
    if (after) turnIn.nextListDelay = after.t - turnIn.t;
    // Any list from the giver, even one ingest could not read completely, means the player picked.
    const pop = nextDetail(episode, turnIn, firstListAfter(anyLists, giver, turnIn.t));
    if (pop) turnIn.nextDetailDelay = pop.t - turnIn.t;
    if (isRepeatableQuest(turnIn.questId)) continue;

    if (pop && pop.t - turnIn.t <= params.popWindow && isNewOffer(timeline, pop.questId, turnIn)) {
      observations.push({
        kind: "pop",
        turnedIn: turnIn.questId,
        offered: pop.questId,
        giver,
        turnInAt: turnIn.t,
        offeredAt: pop.t,
        confounders: otherTurnInsAt(turnIns, giver, pop.t - params.popWindow, pop.t, turnIn).map((other) => `also turned in ${other.questId}`),
      });
    }

    if (before && after && turnIn.t - before.t <= params.listBefore && after.t - turnIn.t <= params.listAfter) {
      observations.push(...listHandoffs(timeline, lookup, isRepeatableQuest, turnIn, giver, before, after));
      for (const quest of [...before.available, ...after.available]) comparedQuests.add(quest.id);
    }
  }
  return { turnIns, observations, comparedQuests };
}

/** Turn-ins in time order, with the giver filled in from the completion dialog when the event lacks one. */
function resolveTurnIns(episode: QuestEpisode): TurnIn[] {
  const turnIns: TurnIn[] = [];
  const lastDialogGiver = new Map<number, { t: SessionTime; giver: GiverRef }>();
  for (const questEvent of episode.questEvents) {
    if ((questEvent.kind === "complete" || questEvent.kind === "progress") && questEvent.giver) {
      lastDialogGiver.set(questEvent.questId, { t: questEvent.t, giver: questEvent.giver });
    } else if (questEvent.kind === "turnedIn") {
      const dialog = lastDialogGiver.get(questEvent.questId);
      const giver = questEvent.giver ?? (dialog && questEvent.t - dialog.t <= COMPLETION_DIALOG_MAX_AGE ? dialog.giver : undefined);
      turnIns.push({ questId: questEvent.questId, t: questEvent.t, giver });
    }
  }
  return turnIns;
}

/** Quests any offer list in this episode flagged repeatable, daily or weekly. */
function repeatableInEpisode(episode: QuestEpisode): Set<number> {
  const result = new Set<number>();
  for (const offer of episode.offers) {
    for (const quest of offer.available) {
      if (quest.repeatable || (quest.frequency ?? 1) > 1) result.add(quest.id);
    }
  }
  return result;
}

function lastListBefore(lists: OfferSnapshot[], giver: GiverRef, t: SessionTime): OfferSnapshot | undefined {
  for (let i = lists.length - 1; i >= 0; i--) {
    if (lists[i].t < t && sameGiver(lists[i].giver, giver)) return lists[i];
  }
  return undefined;
}

function firstListAfter(lists: OfferSnapshot[], giver: GiverRef, t: SessionTime): OfferSnapshot | undefined {
  return lists.find((list) => list.t > t && sameGiver(list.giver, giver));
}

/**
 * The first accept dialog at the turn-in's giver after the turn-in, if nothing else happened at
 * that giver first. A dialog after a fresh list was picked by the player, and one after another
 * turn-in dialog belongs to that turn-in.
 */
function nextDetail(episode: QuestEpisode, turnIn: TurnIn, nextList: OfferSnapshot | undefined): { questId: number; t: SessionTime } | undefined {
  for (const questEvent of episode.questEvents) {
    if (questEvent.t <= turnIn.t) continue;
    if (nextList && questEvent.t > nextList.t) return undefined;
    if (questEvent.questId === turnIn.questId || !sameGiver(questEvent.giver, turnIn.giver)) continue;
    if (questEvent.kind === "detail") return { questId: questEvent.questId, t: questEvent.t };
    if (questEvent.kind === "progress" || questEvent.kind === "complete" || questEvent.kind === "turnedIn") return undefined;
  }
  return undefined;
}

/** False when the quest was already completed or in the log, i.e. the dialog is not a fresh offer. */
function isNewOffer(timeline: EpisodeTimeline, questId: number, turnIn: TurnIn): boolean {
  return !timeline.isCompletedAt(questId, turnIn.t) && !timeline.isInLogAt(questId, turnIn.t);
}

function otherTurnInsAt(turnIns: TurnIn[], giver: GiverRef, from: SessionTime, to: SessionTime, self: TurnIn): TurnIn[] {
  return turnIns.filter((other) => other !== self && other.t >= from && other.t <= to && sameGiver(other.giver, giver));
}

function listHandoffs(
  timeline: EpisodeTimeline,
  lookup: QuestLookup,
  isRepeatableQuest: (questId: number) => boolean,
  turnIn: TurnIn,
  giver: GiverRef,
  before: OfferSnapshot,
  after: OfferSnapshot,
): HandoffObservation[] {
  const previouslyOffered = new Set(before.available.map((quest) => quest.id));
  const shared = sharedConfounders(timeline, turnIn, before.t, after.t);
  const levelBefore = timeline.levelAt(before.t);
  const levelAfter = timeline.levelAt(after.t);
  const observations: HandoffObservation[] = [];

  for (const { id: questId } of after.available) {
    if (questId === turnIn.questId || previouslyOffered.has(questId)) continue;
    // Hidden before because it was already taken or done, not because it was locked.
    if (timeline.isInLogAt(questId, before.t) || timeline.isCompletedAt(questId, before.t)) continue;
    // Repeatable quests come back on their own (after a turn-in or a daily/weekly reset).
    if (isRepeatableQuest(questId)) continue;

    const confounders = [...shared];
    if (levelBefore !== undefined && levelAfter !== undefined && levelAfter > levelBefore) {
      const requiredLevel = lookup(questId)?.requiredLevel;
      if (requiredLevel === undefined || requiredLevel > levelBefore) confounders.push(`level ${levelBefore} -> ${levelAfter}`);
    }
    observations.push({
      kind: "list",
      turnedIn: turnIn.questId,
      offered: questId,
      giver,
      turnInAt: turnIn.t,
      offeredAt: after.t,
      previousListAt: before.t,
      confounders,
    });
  }
  return observations;
}

/** Changes between two lists, other than this turn-in, that could unlock any quest. */
function sharedConfounders(timeline: EpisodeTimeline, turnIn: TurnIn, from: SessionTime, to: SessionTime): string[] {
  const confounders: string[] = [];
  for (const completion of timeline.completionsDuringSession()) {
    if (completion.t > from && completion.t <= to && completion.v !== turnIn.questId) confounders.push(`also completed ${completion.v}`);
  }
  // A reset can reopen a quest that blocked others while completed (exclusiveTo).
  for (const change of timeline.episode.completed?.changes ?? []) {
    if (change.t > from && change.t <= to) for (const questId of change.remove ?? []) confounders.push(`${questId} reset`);
  }
  const logBefore = timeline.logAt(from);
  const logAfter = timeline.logAt(to);
  for (const questId of logBefore) {
    if (questId === turnIn.questId || logAfter.has(questId) || timeline.isCompletedAt(questId, to)) continue;
    confounders.push(`${questId} left the log`);
  }
  // Some quests open only while another is taken (parentQuest, availableStartingWith).
  for (const questId of logAfter) {
    if (!logBefore.has(questId)) confounders.push(`${questId} entered the log`);
  }
  return confounders;
}
