// Pass 1 of the absence signal: everything known from moments a quest WAS available.
//
// A blocked moment (blocked.ts) only says "something stopped X here". What that something can
// be is decided by contrast with every moment X was offered to anyone: a quest that sat in some
// character's log while X was still offered cannot be what hides X while in the log, and so on.
// This pass also learns which givers really start X (as observed, not as the catalog claims),
// which quests are listed side by side, and which turn-ins hand off to which offers.

import type { GiverRef, QuestEpisode, SessionTime } from "../../core/types";
import type { AbsenceParams } from "./params";
import { EpisodeStates } from "./states";

export interface QuestOffers {
  /** Characters the quest was offered to (in any list, or accepted). */
  characters: Set<string>;
  /** Lowest character level at an offer; blocked moments below it may be level gating. */
  minLevel: number;
  factions: Set<string>;
  races: Set<number>;
  classes: Set<number>;
  /** Listed as daily/weekly/repeatable: completion does not stick, so absence says little. */
  periodic: boolean;
  /** Other quest -> characters that had it in the log at some moment this quest was offered. */
  inLogWhenOffered: Map<number, Set<string>>;
  /** Other quest -> characters that had it completed at some moment this quest was offered. */
  completedWhenOffered: Map<number, Set<string>>;
  /**
   * Quests completed at every giver offer of nearly every character (prerequisite suspects).
   * Filled by build(); see AbsenceParams.requiredTolerance.
   */
  required: Set<number>;
  /**
   * Same-name quest groups (Forever's zone and race copies) of which one member was completed at
   * nearly every offer, while no single member was: "any one of these" prerequisites.
   * Smallest member -> members seen completed. Filled by build().
   */
  requiredAny: Map<number, number[]>;
  /** Per character, the quests completed at each of their giver offers (sorted). Build input. */
  completedAtEachOffer: Map<string, Int32Array>;
  /** Per character, the earliest offer. */
  firstOfferedAt: Map<string, Moment>;
}

export interface OfferIndex {
  quests: Map<number, QuestOffers>;
  /** Giver key -> quests that giver was seen offering. */
  questsByGiver: Map<string, Set<number>>;
  /** "a:b" (a < b): listed together in one offer list at least once. */
  coOffered: Set<string>;
  /** "x>y" -> givers where y was offered within the hand-off window after x was turned in there. */
  handoffs: Map<string, Set<string>>;
  /** Quest -> givers it was seen turned in at. */
  finishersOf: Map<number, Set<string>>;
  /** Quest -> its completion bundle (sorted, 2+ quests that always complete together). */
  bundles: Map<number, number[]>;
  episodes: number;
}

export function giverKey(giver: GiverRef): string {
  return `${giver.kind}:${giver.id}`;
}

/** The quest standing in for its completion bundle (its smallest member), or the quest itself. */
export function representative(index: OfferIndex, questId: number): number {
  return index.bundles.get(questId)?.[0] ?? questId;
}

/** A representative's bundle, or just the quest. */
export function bundleOf(index: OfferIndex, questId: number): number[] {
  return index.bundles.get(questId) ?? [questId];
}

export function pairKey(a: number, b: number): string {
  return a < b ? `${a}:${b}` : `${b}:${a}`;
}

/** A point in one character's play history: which of their sessions, and when in it. */
export interface Moment {
  /** QuestEpisode.sessionOrder: sessionName is missing for a third of sessions, so never order by it. */
  session: number;
  t: SessionTime;
}

export function momentOf(episode: QuestEpisode, t: SessionTime): Moment {
  return { session: episode.sessionOrder, t };
}

export function isEarlier(a: Moment, b: Moment): boolean {
  return a.session < b.session || (a.session === b.session && a.t < b.t);
}

interface OfferMoment {
  t: SessionTime;
  /** Reached through a known giver. Giverless details are party shares and pushed quests. */
  fromGiver: boolean;
}

export class OfferIndexBuilder {
  private readonly index: OfferIndex = {
    quests: new Map(),
    questsByGiver: new Map(),
    coOffered: new Set(),
    handoffs: new Map(),
    finishersOf: new Map(),
    bundles: new Map(),
    episodes: 0,
  };
  /** Quest -> quests in every completed-set batch it arrived in, and how many batches that was. */
  private readonly batchMates = new Map<number, Set<number>>();
  private readonly batchCounts = new Map<number, number>();

  constructor(private readonly params: AbsenceParams) {}

  add(episode: QuestEpisode): void {
    this.index.episodes++;
    const states = new EpisodeStates(episode, this.params);
    const moments = new Map<number, OfferMoment[]>();
    const note = (questId: number, t: SessionTime, giver: GiverRef | undefined) => {
      const list = moments.get(questId);
      const moment = { t, fromGiver: giver !== undefined };
      if (list) list.push(moment);
      else moments.set(questId, [moment]);
      if (giver) addTo(this.index.questsByGiver, giverKey(giver), questId);
    };

    for (const snapshot of episode.offers) {
      const ids = snapshot.available.map((offered) => offered.id);
      for (const offered of snapshot.available) {
        note(offered.id, snapshot.t, snapshot.giver);
        if ((offered.frequency ?? 1) > 1 || offered.repeatable) this.questOffers(offered.id).periodic = true;
      }
      for (let i = 0; i < ids.length; i++) {
        for (let j = i + 1; j < ids.length; j++) if (ids[i] !== ids[j]) this.index.coOffered.add(pairKey(ids[i], ids[j]));
      }
    }
    // A successful accept passed the server's availability check, however the dialog was reached.
    for (const event of episode.questEvents) {
      if (event.kind === "accepted") note(event.questId, event.t, event.giver);
      if (event.kind === "turnedIn" && event.giver) addTo(this.index.finishersOf, event.questId, giverKey(event.giver));
    }

    for (const [questId, list] of moments) this.recordOffers(episode, states, questId, list);
    this.recordHandoffs(episode);
    this.recordBatches(states.episode);
  }

  /** `nameOf` groups same-name copies for requiredAny; without it only single prerequisites are found. */
  build(nameOf: (questId: number) => string | undefined = () => undefined): OfferIndex {
    for (const [questId, mates] of this.batchMates) {
      const bundle = [...mates].filter((mate) => this.batchMates.get(mate)?.has(questId) && (this.batchCounts.get(mate) ?? 0) >= this.params.minBundleBatches);
      if (bundle.length >= 2 && bundle.length <= this.params.maxBundle) this.index.bundles.set(questId, bundle.sort((a, b) => a - b));
    }
    for (const [questId, offers] of this.index.quests) {
      const characters = offers.completedAtEachOffer.size;
      const enough = characters - Math.floor(characters * this.params.requiredTolerance);
      const counts = new Map<number, number>();
      const groupCounts = new Map<string, number>();
      const groupMembers = new Map<string, Set<number>>();
      const ownName = nameOf(questId);
      for (const completed of offers.completedAtEachOffer.values()) {
        const groupsHere = new Set<string>();
        for (const other of completed) {
          counts.set(other, (counts.get(other) ?? 0) + 1);
          const name = nameOf(other);
          if (name === undefined || name === ownName) continue;
          groupsHere.add(name);
          addTo(groupMembers, name, other);
        }
        for (const name of groupsHere) groupCounts.set(name, (groupCounts.get(name) ?? 0) + 1);
      }
      for (const [other, count] of counts) if (count >= enough) offers.required.add(other);
      for (const [name, count] of groupCounts) {
        const members = [...groupMembers.get(name)!].sort((a, b) => a - b);
        if (members.length < 2 || count < enough || members.some((member) => offers.required.has(member))) continue;
        offers.requiredAny.set(members[0], members);
      }
    }
    return this.index;
  }

  private questOffers(questId: number): QuestOffers {
    let offers = this.index.quests.get(questId);
    if (!offers) {
      offers = {
        characters: new Set(),
        minLevel: Infinity,
        factions: new Set(),
        races: new Set(),
        classes: new Set(),
        periodic: false,
        inLogWhenOffered: new Map(),
        completedWhenOffered: new Map(),
        required: new Set(),
        requiredAny: new Map(),
        completedAtEachOffer: new Map(),
        firstOfferedAt: new Map(),
      };
      this.index.quests.set(questId, offers);
    }
    return offers;
  }

  private recordOffers(episode: QuestEpisode, states: EpisodeStates, questId: number, moments: OfferMoment[]): void {
    const offers = this.questOffers(questId);
    const character = episode.characterKey;
    offers.characters.add(character);
    const { player } = episode;
    if (player.faction) offers.factions.add(player.faction);
    if (player.raceId !== undefined) offers.races.add(player.raceId);
    if (player.classId !== undefined) offers.classes.add(player.classId);

    moments.sort((a, b) => a.t - b.t);
    const first = momentOf(episode, moments[0].t);
    const known = offers.firstOfferedAt.get(character);
    if (known === undefined || isEarlier(first, known)) offers.firstOfferedAt.set(character, first);

    for (const { t } of moments) {
      const level = states.timeline.levelAt(t);
      if (level !== undefined) offers.minLevel = Math.min(offers.minLevel, level);
      for (const other of states.log(t)) if (other !== questId) addTo(offers.inLogWhenOffered, other, character);
    }

    // Completions only grow within a session (daily resets aside), so the first and last
    // offer bound the union and the intersection over every offer in between.
    if (!episode.completed) return;
    const atFirst = states.completed(moments[0].t);
    const atLast = states.completed(moments[moments.length - 1].t);
    for (const other of atLast) if (other !== questId) addTo(offers.completedWhenOffered, other, character);
    for (const other of atFirst) if (other !== questId) addTo(offers.completedWhenOffered, other, character);

    // Prerequisites only from giver offers: shared and pushed quests skip the giver's checks.
    const fromGiver = moments.filter((moment) => moment.fromGiver);
    if (fromGiver.length === 0) return;
    const atFirstGiver = states.completed(fromGiver[0].t);
    const atLastGiver = states.completed(fromGiver[fromGiver.length - 1].t);
    let completed: Int32Array = Int32Array.from([...atFirstGiver].filter((other) => other !== questId && atLastGiver.has(other))).sort();
    const earlier = offers.completedAtEachOffer.get(character);
    if (earlier) completed = intersectSorted(earlier, completed);
    offers.completedAtEachOffer.set(character, completed);
  }

  private recordBatches(episode: QuestEpisode): void {
    for (const change of episode.completed?.changes ?? []) {
      const batch = change.add ?? [];
      for (const questId of batch) {
        this.batchCounts.set(questId, (this.batchCounts.get(questId) ?? 0) + 1);
        const mates = this.batchMates.get(questId);
        if (!mates) this.batchMates.set(questId, new Set(batch));
        else for (const mate of mates) if (!batch.includes(mate)) mates.delete(mate);
      }
    }
  }

  /** "Turned in x at giver g, and g offered y within the window": y is handed off by x's turn-in. */
  private recordHandoffs(episode: QuestEpisode): void {
    const window = this.params.handoffWindow;
    for (const turnIn of episode.questEvents) {
      if (turnIn.kind !== "turnedIn" || !turnIn.giver) continue;
      const giver = giverKey(turnIn.giver);
      const offeredAfter = new Set<number>();
      for (const snapshot of episode.offers) {
        if (snapshot.t < turnIn.t || snapshot.t > turnIn.t + window) continue;
        if (snapshot.giver && giverKey(snapshot.giver) === giver) for (const offered of snapshot.available) offeredAfter.add(offered.id);
      }
      for (const event of episode.questEvents) {
        if (event.t < turnIn.t || event.t > turnIn.t + window) continue;
        if ((event.kind === "detail" || event.kind === "accepted") && event.giver && giverKey(event.giver) === giver) offeredAfter.add(event.questId);
      }
      offeredAfter.delete(turnIn.questId);
      for (const next of offeredAfter) addTo(this.index.handoffs, `${turnIn.questId}>${next}`, giver);
    }
  }
}

function intersectSorted(a: Int32Array, b: Int32Array): Int32Array {
  const result: number[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      result.push(a[i]);
      i++;
      j++;
    } else if (a[i] < b[j]) i++;
    else j++;
  }
  return Int32Array.from(result);
}

function addTo<K, V>(map: Map<K, Set<V>>, key: K, value: V): void {
  const set = map.get(key);
  if (set) set.add(value);
  else map.set(key, new Set([value]));
}
