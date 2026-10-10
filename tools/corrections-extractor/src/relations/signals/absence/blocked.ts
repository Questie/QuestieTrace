// Pass 2 of the absence signal: moments a giver's complete list lacked a quest it starts, while
// the character could otherwise have taken it, and what could explain each one.
//
// An explanation of "X missing from the list at t" is one of:
// - inLog Y:     Y was in the log at t, and Y was never in the log while X was offered.
// - completed Y: Y was completed at t, and Y was never completed while X was offered.
// - missing P:   P was never taken by t, but P was completed at every offer of X.
// - pending P:   the same, but P was in the log at t. A breadcrumb that nearly everyone does
//                looks exactly like this, so only "missing" proves a prerequisite.
// Completed and missing suspects stand for their whole completion bundle (offers.ts), keyed by
// its representative: zone copies that complete together are one cause, not several.
// - missingAny G: no member of same-name group G was completed at t, but one was at every offer.
// Everything consistent with all offer moments (offers.ts) is a suspect. A character offered X a
// minute earlier usually leaves one suspect (what changed in between), a character with no offer
// of their own leaves hundreds. Suspects get a prior weight (locality.ts: unrelated quests count
// little); attribute.ts then decides who takes the blame across all moments of X.

import { hasUntrackedRequirements, isStaticallyEligible } from "../../core/eligibility";
import type { CatalogQuest, OfferSnapshot, QuestCatalog, QuestEpisode, SessionTime } from "../../core/types";
import { Locality } from "./locality";
import { bundleOf, giverKey, isEarlier, momentOf, representative, type Moment, type OfferIndex, type QuestOffers } from "./offers";
import type { AbsenceParams } from "./params";
import { EpisodeStates } from "./states";

/** missingAny is keyed by the smallest member of a QuestOffers.requiredAny group. */
export type BlockKind = "inLog" | "completed" | "missing" | "pending" | "missingAny";

export interface Suspect {
  /** explanationKey(kind, quest) */
  key: string;
  /** Prior weight: 1 for a structurally related quest, AbsenceParams.unrelatedWeight otherwise. */
  weight: number;
  /** Reviewable state description, e.g. "92742 in log (accepted at 886.1s)". */
  detail: string;
}

export interface BlockedMoment {
  character: string;
  ref: string;
  t: SessionTime;
  /** "npc:123 list at 45.0s lacked 678" */
  what: string;
  /** The character was offered X before this moment, so its untracked requirements were met. */
  transition: boolean;
  suspects: Suspect[];
  /** Total prior weight of the suspects too unlikely to keep individually. */
  residualWeight: number;
}

export interface BlockedQuest {
  /** Characters with at least one blocked moment that passed gating. */
  characters: Set<string>;
  /** Characters with a blocked moment nothing consistent explains (an unmodelled gate). */
  unexplained: Set<string>;
  moments: BlockedMoment[];
}

export function explanationKey(kind: BlockKind, questId: number): string {
  return `${kind}:${questId}`;
}

const EVENT_FLAG = 2;
const REPEATABLE_FLAG = 1;

export class BlockedCollector {
  readonly quests = new Map<number, BlockedQuest>();
  /** Why candidate blocked moments were set aside, for the report. */
  readonly skipped = new Map<string, number>();
  blockedMoments = 0;
  private readonly questsBySignature = new Map<string, number[]>();
  private readonly firstOfferBySignature = new Map<string, Moment | undefined>();
  readonly locality: Locality;

  constructor(
    private readonly index: OfferIndex,
    private readonly catalog: QuestCatalog,
    private readonly params: AbsenceParams,
  ) {
    this.locality = new Locality(index, catalog);
    for (const questId of index.quests.keys()) {
      const quest = catalog.quests[questId];
      if (!quest || !hasUntrackedRequirements(quest)) continue;
      const signature = requirementSignature(quest);
      const list = this.questsBySignature.get(signature);
      if (list) list.push(questId);
      else this.questsBySignature.set(signature, [questId]);
    }
  }

  add(episode: QuestEpisode): void {
    if (!episode.completed) return this.skip("noCompletedHistory");
    if (episode.questLog.length === 0) return this.skip("noQuestLog");
    const states = new EpisodeStates(episode, this.params);
    const offeredHere = offerTimesOf(episode);
    const lastState = new Map<number, { completed: ReadonlySet<number>; log: string }>();
    const lastListWithQuests = new Map<string, SessionTime>();

    for (const snapshot of episode.offers) {
      if (!snapshot.listComplete || !snapshot.giver || !this.params.blockedSources.includes(snapshot.source)) continue;
      const giver = giverKey(snapshot.giver);
      const listed = new Set([...snapshot.available.map((offered) => offered.id), ...(snapshot.active ?? [])]);
      if (listed.size > 0) {
        lastListWithQuests.set(giver, snapshot.t);
      } else if (snapshot.source === "gossip") {
        const previous = lastListWithQuests.get(giver);
        if (previous !== undefined && snapshot.t - previous <= this.params.subMenuWindow && !states.anyEventWithin(previous, snapshot.t)) {
          this.skip("gossipSubMenu");
          continue;
        }
      }
      for (const questId of this.index.questsByGiver.get(giver) ?? []) {
        if (listed.has(questId)) continue;
        const offers = this.index.quests.get(questId)!;
        const reason = this.gate(episode, states, questId, offers, snapshot.t);
        if (reason) {
          this.skip(reason);
          continue;
        }
        // Repeated reads of one giver with nothing changed in between are one observation.
        const completed = states.completed(snapshot.t);
        const log = states.log(snapshot.t);
        const logKey = [...log].sort().join(",");
        const last = lastState.get(questId);
        if (last && last.completed === completed && last.log === logKey) continue;
        lastState.set(questId, { completed, log: logKey });

        this.blockedMoments++;
        this.record(episode, states, questId, offers, snapshot, giver, completed, log, offeredHere.get(questId) ?? []);
      }
    }
  }

  /** Why the character might not have been able to take the quest anyway; undefined if eligible. */
  private gate(episode: QuestEpisode, states: EpisodeStates, questId: number, offers: QuestOffers, t: SessionTime): string | undefined {
    const quest = this.catalog.quests[questId];
    if (!quest) return "uncataloged";
    const flags = quest.specialFlags ?? 0;
    if (offers.periodic || flags & REPEATABLE_FLAG) return "periodic";
    if (flags & EVENT_FLAG) return "seasonal";

    const { player } = episode;
    const level = states.timeline.levelAt(t);
    if (level === undefined) return "unknownLevel";
    if (!isStaticallyEligible(quest, player, level)) return "catalogRaceClassLevel";
    // Catalog levels can be off; only trust levels at which someone was actually offered the quest.
    if (level < offers.minLevel) return "belowOfferedLevel";
    // Faction variants are never offered to the other faction; that is not exclusivity.
    if (!player.faction || !offers.factions.has(player.faction)) return "unofferedFaction";
    const { openDemographics } = this.params;
    if (offers.races.size < openDemographics && (player.raceId === undefined || !offers.races.has(player.raceId))) return "unofferedRace";
    if (offers.classes.size < openDemographics && (player.classId === undefined || !offers.classes.has(player.classId))) return "unofferedClass";
    // Skill, reputation and spell requirements are not in the trace. An earlier offer to this
    // character of a quest with the very same requirements (often a zone copy of this quest)
    // shows they met them; skills and reputation rarely go back down.
    if (hasUntrackedRequirements(quest)) {
      const first = this.firstOfferWithRequirements(requirementSignature(quest), episode.characterKey);
      if (first === undefined || !isEarlier(first, momentOf(episode, t))) return "untrackedRequirements";
    }

    const slack = this.params.selfSlack;
    if (states.isCompleted(questId, t + slack)) return "self";
    if (states.inLog(questId, t) || states.timeline.isInLogAt(questId, t - slack) || states.timeline.isInLogAt(questId, t + slack)) return "self";
    if (states.touched(questId, t - slack, t + slack)) return "self";
    return undefined;
  }

  /** The character's earliest offer of any quest with these untracked requirements. */
  private firstOfferWithRequirements(signature: string, character: string): Moment | undefined {
    const key = `${character}|${signature}`;
    if (this.firstOfferBySignature.has(key)) return this.firstOfferBySignature.get(key);
    let first: Moment | undefined;
    for (const questId of this.questsBySignature.get(signature) ?? []) {
      const offered = this.index.quests.get(questId)?.firstOfferedAt.get(character);
      if (offered !== undefined && (first === undefined || isEarlier(offered, first))) first = offered;
    }
    this.firstOfferBySignature.set(key, first);
    return first;
  }

  private record(
    episode: QuestEpisode,
    states: EpisodeStates,
    questId: number,
    offers: QuestOffers,
    snapshot: OfferSnapshot,
    giver: string,
    completed: ReadonlySet<number>,
    log: ReadonlySet<number>,
    offeredHere: SessionTime[],
  ): void {
    const { maxContradictions, minShare, unrelatedWeight } = this.params;
    const consistent = (contradicting: Set<string> | undefined) => (contradicting?.size ?? 0) <= maxContradictions;
    const { required, requiredAny } = offers;
    const inRequiredGroup = (other: number) => [...requiredAny.values()].some((members) => members.includes(other));
    const suspects: Array<[BlockKind, number]> = [];
    // A required quest in the log is simply not completed yet: "pending" covers it.
    for (const other of log) {
      if (other === questId || required.has(other) || inRequiredGroup(other)) continue;
      if (consistent(offers.inLogWhenOffered.get(other))) suspects.push(["inLog", other]);
    }
    const completedBundles = new Set<number>();
    for (const other of completed) {
      const stand = representative(this.index, other);
      if (other === questId || completedBundles.has(stand)) continue;
      completedBundles.add(stand);
      if (consistent(offers.completedWhenOffered.get(stand))) suspects.push(["completed", stand]);
    }
    const missingBundles = new Set<number>();
    for (const other of required) {
      const stand = representative(this.index, other);
      if (completed.has(other) || missingBundles.has(stand)) continue;
      missingBundles.add(stand);
      suspects.push([bundleOf(this.index, stand).some((member) => log.has(member)) ? "pending" : "missing", stand]);
    }
    for (const [representative, members] of requiredAny) {
      if (!members.some((member) => completed.has(member))) suspects.push(["missingAny", representative]);
    }

    const blocked = this.blockedQuest(questId);
    const character = episode.characterKey;
    blocked.characters.add(character);
    if (suspects.length === 0) {
      blocked.unexplained.add(character);
      return;
    }
    const t = snapshot.t;
    const membersOf = (kind: BlockKind, other: number) =>
      kind === "missingAny" ? requiredAny.get(other)! : kind === "inLog" ? [other] : bundleOf(this.index, other);
    const related = (kind: BlockKind, other: number) => membersOf(kind, other).some((member) => this.locality.reason(questId, member));
    const weights = suspects.map(([kind, other]) => (related(kind, other) ? 1 : unrelatedWeight));
    const totalWeight = weights.reduce((sum, weight) => sum + weight, 0);
    const kept: Suspect[] = [];
    let residualWeight = 0;
    for (let i = 0; i < suspects.length; i++) {
      const [kind, other] = suspects[i];
      if (weights[i] / totalWeight < minShare) residualWeight += weights[i];
      else kept.push({ key: explanationKey(kind, other), weight: weights[i], detail: describeState(states, kind, membersOf(kind, other), t) });
    }
    const earlierOffer = offeredHere.filter((offeredAt) => offeredAt < t).pop();
    const firstOffer = offers.firstOfferedAt.get(character);
    blocked.moments.push({
      character,
      ref: episode.key,
      t,
      what: `${giver} list at ${t.toFixed(1)}s lacked ${questId}` + (earlierOffer !== undefined ? ` (offered at ${earlierOffer.toFixed(1)}s)` : ""),
      transition: firstOffer !== undefined && isEarlier(firstOffer, momentOf(episode, t)),
      suspects: kept,
      residualWeight,
    });
  }

  private blockedQuest(questId: number): BlockedQuest {
    let blocked = this.quests.get(questId);
    if (!blocked) {
      blocked = { characters: new Set(), unexplained: new Set(), moments: [] };
      this.quests.set(questId, blocked);
    }
    return blocked;
  }

  private skip(reason: string): void {
    this.skipped.set(reason, (this.skipped.get(reason) ?? 0) + 1);
  }
}

function requirementSignature(quest: CatalogQuest): string {
  return JSON.stringify([quest.requiredSkill, quest.requiredMinRep, quest.requiredMaxRep, quest.requiredSpell]);
}

function describeState(states: EpisodeStates, kind: BlockKind, members: number[], t: SessionTime): string {
  const questId = members[0];
  const ids = members.join("/");
  if (kind === "missing") return `${ids} not taken`;
  if (kind === "pending") return `${ids} in log, not completed`;
  if (kind === "missingAny") return `none of ${ids} completed`;
  const events = states.episode.questEvents.filter((event) => event.questId === questId && event.t <= t);
  if (kind === "inLog") {
    const accepted = events.filter((event) => event.kind === "accepted").pop();
    return `${questId} in log` + (accepted ? ` (accepted at ${accepted.t.toFixed(1)}s)` : "");
  }
  const completedAt = states.timeline.completionTime(questId);
  return `${ids} completed` + (typeof completedAt === "number" ? ` at ${completedAt.toFixed(1)}s` : " before the session");
}

/** Per quest, the times it was listed or accepted in this episode. */
function offerTimesOf(episode: QuestEpisode): Map<number, SessionTime[]> {
  const result = new Map<number, SessionTime[]>();
  const add = (questId: number, t: SessionTime) => {
    const times = result.get(questId);
    if (times) times.push(t);
    else result.set(questId, [t]);
  };
  for (const snapshot of episode.offers) for (const offered of snapshot.available) add(offered.id, snapshot.t);
  for (const event of episode.questEvents) if (event.kind === "accepted") add(event.questId, event.t);
  for (const times of result.values()) times.sort((a, b) => a - b);
  return result;
}
