// What one episode says about each linked (breadcrumb B, target T) pair.
//
// A breadcrumb is a linked pair (see links.ts) that is both OPTIONAL and DEPENDENT:
//   optional:  T is available to characters who never did B (a chain's target is not),
//   dependent: B and T gate each other (an unrelated quest at the same giver does not).
// Each observation below is one of those two kinds of support, or a contradiction of them.

import { classAllowed, hasUntrackedRequirements, isStaticallyEligible, raceAllowed } from "../../core/eligibility";
import { EpisodeTimeline } from "../../core/timeline";
import type { CatalogQuest, Evidence, GiverRef, QuestCatalog, QuestEpisode, SessionTime } from "../../core/types";
import { findAutoPops, giverKey, pairKey, type BreadcrumbParams, type Links } from "./links";

export type ObservationKind =
  // optional
  | "free" //       T offered, taken or done while B was neither done nor in the log
  | "flagged" //    the client's IsBreadcrumbQuest(B) returned true
  // dependent
  | "autoPop" //    the server opened T's dialog right after B's turn-in
  | "reveal" //     T missing at its giver while B was in the log, offered once B was turned in
  | "blockB" //     B missing at its giver while T was done or in the log
  | "serverDrop" // B left the log without a turn-in as T was accepted
  // contradictions
  | "unlock" //     T missing while B was neither done nor in the log, offered once B was done (a chain)
  | "showB" //      B offered while T was done or in the log
  | "showT" //      T offered or accepted while B was in the log
  | "notFlagged"; // IsBreadcrumbQuest(B) returned false

export interface Observation {
  breadcrumb: number;
  target: number;
  kind: ObservationKind;
  evidence: Evidence;
}

/** How far before an accept to read the log, so the accepted quest's own log change is not seen yet. */
const BEFORE = 0.001;

interface PendingAbsence {
  breadcrumb: number;
  target: number;
  t: SessionTime;
  breadcrumbInLog: boolean;
  giver: GiverRef;
}

function describeGiver(giver: GiverRef | undefined): string {
  return giver ? `${giver.kind} ${giver.id}` : "unknown giver";
}

function pushTo<K, V>(map: Map<K, V[]>, key: K, value: V): void {
  const list = map.get(key);
  if (list) list.push(value);
  else map.set(key, [value]);
}

function time(t: SessionTime): string {
  return `${t.toFixed(1)}s`;
}

/** Race and class allow the quest, and are actually known whenever the quest restricts them. */
function raceClassAllow(quest: CatalogQuest | undefined, episode: QuestEpisode): boolean {
  if (!quest) return true;
  const { raceId, classId } = episode.player;
  if (quest.requiredRaces && raceId === undefined) return false;
  if (quest.requiredClasses && classId === undefined) return false;
  return raceAllowed(quest.requiredRaces, raceId) && classAllowed(quest.requiredClasses, classId);
}

export function observeEpisode(episode: QuestEpisode, links: Links, catalog: QuestCatalog, params: BreadcrumbParams): Observation[] {
  const timeline = new EpisodeTimeline(episode);
  const found = new Map<string, Observation>();
  const done = (questId: number, t: SessionTime) => timeline.isCompletedAt(questId, t);
  const inLog = (questId: number, t: SessionTime) => timeline.isInLogAt(questId, t);

  // One observation per pair and kind is enough: support is counted per character.
  const add = (breadcrumb: number, target: number, kind: ObservationKind, t: SessionTime, text: string) => {
    const key = `${pairKey(breadcrumb, target)}|${kind}`;
    if (!found.has(key)) found.set(key, { breadcrumb, target, kind, evidence: { ref: episode.key, t, text } });
  };

  /** "Not offered" only counts when nothing but relationships could have hidden the quest. */
  const absenceEligible = (questId: number, t: SessionTime): boolean => {
    const quest = catalog.quests[questId];
    if (!quest || hasUntrackedRequirements(quest) || ((quest.specialFlags ?? 0) & 2) !== 0) return false;
    if (!raceClassAllow(quest, episode)) return false;
    const level = timeline.levelAt(t);
    if (quest.requiredLevel !== undefined && level === undefined) return false;
    if (!isStaticallyEligible(quest, episode.player, level, { levelSlack: -params.absenceLevelMargin })) return false;
    return !done(questId, t) && !inLog(questId, t);
  };

  /**
   * T reached without B. Only counts when B's completion is observable at all, the character could
   * have taken B, and no other lead-in of T explains it (T requiring "A or B" is not a breadcrumb).
   */
  const freeOf = (breadcrumb: number, target: number, t: SessionTime, logT: SessionTime): boolean => {
    if (!links.everCompleted.has(breadcrumb) || done(breadcrumb, t) || inLog(breadcrumb, logT)) return false;
    if (!raceClassAllow(catalog.quests[breadcrumb], episode)) return false;
    for (const other of links.byTarget.get(target) ?? []) {
      if (other !== breadcrumb && (done(other, t) || inLog(other, logT))) return false;
    }
    return true;
  };

  // Session start: T already done or in the log while B never was.
  const startT = episode.questLog[0]?.t ?? 0;
  const startingTargets = new Set([...(episode.completed?.initial ?? []), ...(episode.questLog[0]?.v ?? [])]);
  for (const target of startingTargets) {
    for (const breadcrumb of links.byTarget.get(target) ?? []) {
      if (freeOf(breadcrumb, target, startT, startT)) {
        add(breadcrumb, target, "free", startT, `${target} done or in the log at session start; ${breadcrumb} never done`);
      }
    }
  }

  // Offer lists: what was offered, and what a complete list left out.
  const availableAt = new Map<number, SessionTime[]>();
  const pending: PendingAbsence[] = [];
  for (const offer of episode.offers) {
    const t = offer.t;
    const where = describeGiver(offer.giver);
    for (const { id } of offer.available) {
      pushTo(availableAt, id, t);
      for (const breadcrumb of links.byTarget.get(id) ?? []) {
        if (done(breadcrumb, t)) continue;
        if (inLog(breadcrumb, t)) add(breadcrumb, id, "showT", t, `${id} offered by ${where} at ${time(t)} while ${breadcrumb} was in the log`);
        else if (freeOf(breadcrumb, id, t, t)) add(breadcrumb, id, "free", t, `${id} offered by ${where} at ${time(t)}; ${breadcrumb} never done`);
      }
      for (const target of links.byBreadcrumb.get(id) ?? []) {
        if (!done(id, t) && (done(target, t) || inLog(target, t))) {
          add(id, target, "showB", t, `${id} offered by ${where} at ${time(t)} while ${target} was done or in the log`);
        }
      }
    }

    if (!offer.listComplete || !offer.giver || offer.giver.kind === "item") continue;
    const listed = new Set([...offer.available.map((quest) => quest.id), ...(offer.active ?? [])]);
    for (const missing of links.startsAt.get(giverKey(offer.giver)) ?? []) {
      if (listed.has(missing) || !absenceEligible(missing, t)) continue;
      for (const breadcrumb of links.byTarget.get(missing) ?? []) {
        if (done(breadcrumb, t) || !links.everCompleted.has(breadcrumb)) continue;
        pending.push({ breadcrumb, target: missing, t, breadcrumbInLog: inLog(breadcrumb, t), giver: offer.giver });
      }
      for (const target of links.byBreadcrumb.get(missing) ?? []) {
        if (done(target, t) || inLog(target, t)) {
          add(missing, target, "blockB", t, `${missing} missing from ${where}'s full list at ${time(t)} while ${target} was done or in the log`);
        }
      }
    }
  }

  // Accepts: T taken without B, or with B in the log (which the server should refuse or resolve by dropping B).
  for (const accept of episode.questEvents) {
    if (accept.kind !== "accepted") continue;
    const t = accept.t;
    pushTo(availableAt, accept.questId, t);
    for (const breadcrumb of links.byTarget.get(accept.questId) ?? []) {
      if (done(breadcrumb, t)) continue;
      if (inLog(breadcrumb, t - BEFORE)) {
        // "removed" is ingest's "left the log, reason unknown"; a player abandoning B says nothing either way.
        const removal = episode.questEvents.find(
          (other) =>
            other.questId === breadcrumb &&
            (other.kind === "removed" || other.kind === "abandoned") &&
            Math.abs(other.t - t) <= params.serverDropSeconds,
        );
        if (removal?.kind === "abandoned") continue;
        const dropped = removal !== undefined;
        add(
          breadcrumb,
          accept.questId,
          dropped ? "serverDrop" : "showT",
          t,
          dropped
            ? `${breadcrumb} left the log without a turn-in as ${accept.questId} was accepted at ${time(t)}`
            : `${accept.questId} accepted at ${time(t)} while ${breadcrumb} was in the log`,
        );
      } else if (freeOf(breadcrumb, accept.questId, t, t - BEFORE)) {
        add(breadcrumb, accept.questId, "free", t, `${accept.questId} accepted from ${describeGiver(accept.giver)} at ${time(t)}; ${breadcrumb} never done`);
      }
    }
  }

  for (const pop of findAutoPops(episode, params)) {
    const after = (pop.detailT - pop.turnInT).toFixed(1);
    add(pop.breadcrumb, pop.target, "autoPop", pop.turnInT, `turned in ${pop.breadcrumb} at ${describeGiver(pop.giver)}, ${pop.target} opened ${after}s later`);
  }

  // A missing T (or one hidden while B was in the log) means something only when the same character
  // is offered T right after B was completed, with no other completion or level-up in between.
  const completions = timeline.completionsDuringSession();
  for (const absence of pending) {
    const { breadcrumb, target, t } = absence;
    const shown = (availableAt.get(target) ?? []).find((t2) => t2 > t && done(breadcrumb, t2));
    if (shown === undefined || timeline.levelAt(shown) !== timeline.levelAt(t)) continue;
    if (completions.some((completion) => completion.t > t && completion.t <= shown && completion.v !== breadcrumb)) continue;
    const where = describeGiver(absence.giver);
    if (absence.breadcrumbInLog) {
      add(breadcrumb, target, "reveal", t, `${target} missing from ${where}'s full list at ${time(t)} with ${breadcrumb} in the log, offered at ${time(shown)} after its turn-in`);
    } else {
      add(breadcrumb, target, "unlock", t, `${target} missing from ${where}'s full list at ${time(t)} before ${breadcrumb}, offered at ${time(shown)} once it was done`);
    }
  }

  for (const [questId, info] of Object.entries(episode.questInfo ?? {})) {
    if (info.isBreadcrumb === undefined) continue;
    const breadcrumb = Number(questId);
    for (const target of links.byBreadcrumb.get(breadcrumb) ?? []) {
      add(breadcrumb, target, info.isBreadcrumb ? "flagged" : "notFlagged", 0, `IsBreadcrumbQuest(${breadcrumb}) = ${info.isBreadcrumb}`);
    }
  }

  return [...found.values()];
}
