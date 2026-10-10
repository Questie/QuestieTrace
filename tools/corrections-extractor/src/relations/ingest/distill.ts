// Distills one raw SessionRecord into an episode draft: the character context, completed /
// quest-log / level timelines, quest dialog events and giver offer lists.
//
// All output times are session-relative `tp` (GetTimePreciseSec), not `t`: `t` is shared by
// everything in one frame, so it cannot order a turn-in before the follow-up detail it causes.
// See streams.ts for how samples are matched to the event that caused them.

import { createHash } from "crypto";
import type { CompletedTimeline, GiverRef, OfferedQuest, OfferSnapshot, PlayerContext, QuestClientInfo, QuestEvent, QuestEventKind, TimedValue } from "../core/types";
import type { EventEntry, FunctionStreamEntry, SessionRecord } from "../../core/types";
import { parseGuid } from "../../extract/guid";
import {
  asPositiveInt,
  entriesBetween,
  entryTime,
  eventArg,
  lastIndexBefore,
  leafStream,
  packed,
  sampledBefore,
  valueAtOrBefore,
  valueBefore,
} from "./streams";
import { emptyDistillStats, giverKey, type DistilledSession, type DistillStats, type PendingGreeting, type TitleObservation } from "./types";

/** Tunables, reported in ingest.json so episode semantics are reproducible. */
export const DISTILL_PARAMS = {
  /** A detail this soon after a turn-in is taken as the server's follow-up offer, not a fresh list. */
  afterTurnInSeconds: 5,
  /** A gossip/greeting list from the same giver this recent means a detail was not opened directly. */
  listContextSeconds: 120,
  /** Any other dialog with a compatible giver this recent means a detail was not a fresh right-click. */
  quietBeforeDetailSeconds: 2,
  /** A detail this soon after an accept or a dialog closing in an earlier frame was pushed by the server, not clicked. */
  pushedDetailSeconds: 2,
  /** An accept this soon after a list, of a quest the list lacks, was already in flight: the list may omit it. */
  inFlightAcceptSeconds: 2,
  /** An empty gossip page this soon after a non-empty one from the same giver, with no quest event between, is a sub-menu. */
  subMenuSeconds: 60,
  /** How far back accepted/turnedIn look for the dialog of the same quest to inherit its giver. */
  carryGiverSeconds: 60,
  /** A detail/turn-in dialog whose quest id never showed in GetQuestID borrows it from the next accept/turn-in this soon. */
  followUpSeconds: 5,
  /** ...and the accept must answer the dialog's close this soon (the server confirms an accept after the window closes). */
  followUpAfterCloseSeconds: 1.5,
  /** QUEST_REMOVED this close to the quest's own turn-in or completion is the turn-in, not an abandon. */
  removalTurnInSeconds: 5,
  /** QUEST_REMOVED this close to another quest's accept/turn-in may be server-side (breadcrumb/chain replacement). */
  removalOtherQuestSeconds: 2,
  /** A completed-set removal at least this large that is re-added soon after is an empty read during a loading screen. */
  completedReadGapMinQuests: 20,
  completedReadGapSeconds: 30,
  completedReadGapReAddedRatio: 0.9,
} as const;

const DIALOG_OPEN = new Set(["QUEST_DETAIL", "QUEST_PROGRESS", "QUEST_COMPLETE", "QUEST_GREETING", "GOSSIP_SHOW"]);
const DIALOG_CLOSE = new Set(["QUEST_FINISHED", "GOSSIP_CLOSED"]);
const FACTIONS = new Set(["Alliance", "Horde", "Neutral"]);
/** Session names are "YYYY-MM-DD_HH-MM-SS"; older builds also took free text from /qlt, which is never copied. */
const SESSION_NAME = /^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}$/;

/** How far a recorded duration may run past the last event before it is distrusted. */
const DURATION_SLACK_SECONDS = 3600;

/** Stable pseudonymous id of a recording session: its two capture-start clocks. */
export function sessionKey(session: SessionRecord): string {
  return createHash("sha256").update(`${session.startedAt}|${session.startedAtPrecise}`).digest("hex").slice(0, 16);
}

// ---------------------------------------------------------------------------
// Character context and timelines
// ---------------------------------------------------------------------------

function firstPlayerValue(session: SessionRecord, name: string): unknown {
  return leafStream(session, name, "player")?.find((entry) => entry.v !== undefined)?.v;
}

function readPlayer(session: SessionRecord): PlayerContext {
  const player: PlayerContext = {};
  const raceId = asPositiveInt(packed(firstPlayerValue(session, "UnitRace"), 3));
  const classId = asPositiveInt(packed(firstPlayerValue(session, "UnitClass"), 3));
  const faction = packed(firstPlayerValue(session, "UnitFactionGroup"), 1);
  if (raceId) player.raceId = raceId;
  if (classId) player.classId = classId;
  if (typeof faction === "string" && FACTIONS.has(faction)) player.faction = faction as PlayerContext["faction"];
  return player;
}

function readInterfaceVersion(session: SessionRecord): number | undefined {
  const value = firstPlayerValue(session, "GetBuildInfo");
  // PlayerIdentity stores the 4th GetBuildInfo return; accept the full packed tuple too.
  return asPositiveInt(value) ?? asPositiveInt(packed(value, 4));
}

function readLevels(session: SessionRecord): TimedValue<number>[] {
  const levels: TimedValue<number>[] = [];
  for (const entry of leafStream(session, "UnitLevel", "player") ?? []) {
    const level = asPositiveInt(entry.v);
    if (level && levels.at(-1)?.v !== level) levels.push({ t: entryTime(entry), v: level });
  }
  return levels;
}

/**
 * GetAllCompletedQuestIDs can read empty during a loading screen, which records a bulk removal of
 * the whole history followed seconds later by its re-add. Left in, long-done quests would look
 * not completed for those seconds. Drops the removal and the matching re-adds; ids that really
 * were removed (not re-added) and genuinely new adds keep their times.
 */
function collapseReadGaps(changes: CompletedTimeline["changes"], stats: DistillStats): CompletedTimeline["changes"] {
  for (let i = 0; i < changes.length; i++) {
    const removed = changes[i].remove ?? [];
    if (removed.length < DISTILL_PARAMS.completedReadGapMinQuests) continue;
    const pending = new Set(removed);
    const reAdds: Array<{ change: number; id: number }> = [];
    for (let j = i + 1; j < changes.length && changes[j].t - changes[i].t <= DISTILL_PARAMS.completedReadGapSeconds; j++) {
      for (const id of changes[j].add ?? []) {
        if (pending.delete(id)) reAdds.push({ change: j, id });
      }
    }
    if (reAdds.length < DISTILL_PARAMS.completedReadGapReAddedRatio * removed.length) continue;
    stats.completedReadGaps++;
    const restored = new Set(reAdds.map((reAdd) => reAdd.id));
    changes[i] = { t: changes[i].t, ...(changes[i].add ? { add: changes[i].add } : {}), remove: removed.filter((id) => !restored.has(id)) };
    for (const change of new Set(reAdds.map((reAdd) => reAdd.change))) {
      changes[change] = { ...changes[change], add: changes[change].add!.filter((id) => !restored.has(id)) };
    }
  }
  return changes
    .map((change) => ({
      t: change.t,
      ...(change.add?.length ? { add: change.add } : {}),
      ...(change.remove?.length ? { remove: change.remove } : {}),
    }))
    .filter((change) => change.add || change.remove);
}

function readCompleted(session: SessionRecord, stats: DistillStats): CompletedTimeline | undefined {
  const modern = session.functionsDelta?.["C_QuestLog.GetAllCompletedQuestIDs"];
  const legacy = session.functionsDelta?.["GetQuestsCompleted"];
  const stream = modern ?? legacy;
  if (!stream || !Array.isArray(stream.initial)) return undefined;
  const ids = (list: unknown) => (Array.isArray(list) ? list.filter((id): id is number => asPositiveInt(id) !== undefined) : []);
  const changes: CompletedTimeline["changes"] = [];
  for (const delta of Array.isArray(stream.delta) ? stream.delta : []) {
    const add = ids(delta.add);
    const remove = ids(delta.remove);
    if (add.length === 0 && remove.length === 0) continue;
    changes.push({ t: entryTime(delta), ...(add.length ? { add } : {}), ...(remove.length ? { remove } : {}) });
  }
  return { source: modern ? "modern" : "legacy", initial: ids(stream.initial).sort((a, b) => a - b), changes: collapseReadGaps(changes, stats) };
}

/** Quest log change points; the raw stream also changes when only the order changes. */
function readQuestLog(session: SessionRecord): TimedValue<number[]>[] {
  const log: TimedValue<number[]>[] = [];
  let previous = "";
  for (const entry of leafStream(session, "QuestLog") ?? []) {
    if (!Array.isArray(entry.v)) continue;
    const ids = [...new Set(entry.v.filter((id): id is number => asPositiveInt(id) !== undefined))].sort((a, b) => a - b);
    const signature = ids.join(",");
    if (log.length > 0 && signature === previous) continue;
    previous = signature;
    log.push({ t: entryTime(entry), v: ids });
  }
  return log;
}

/** Forever-only per-quest streams, keyed by quest id. Absent in traces older than the addon change. */
function readQuestInfo(session: SessionRecord): Record<string, QuestClientInfo> | undefined {
  const info: Record<string, QuestClientInfo> = {};
  const lastValue = (stream: unknown): unknown => {
    if (!Array.isArray(stream)) return undefined;
    for (let i = stream.length - 1; i >= 0; i--) if ((stream[i] as FunctionStreamEntry).v !== undefined) return (stream[i] as FunctionStreamEntry).v;
    return undefined;
  };
  const each = (name: string, apply: (target: QuestClientInfo, value: unknown) => void) => {
    const root = session.functions?.[name];
    if (!root || Array.isArray(root)) return;
    for (const [questId, stream] of Object.entries(root)) {
      if (!asPositiveInt(Number(questId))) continue;
      const value = lastValue(stream);
      if (value === undefined) continue;
      apply((info[questId] ??= {}), value);
    }
  };
  each("IsBreadcrumbQuest", (target, value) => {
    if (typeof value === "boolean") target.isBreadcrumb = value;
  });
  each("C_QuestLine.GetQuestLineInfo", (target, value) => {
    const questLineId = value && typeof value === "object" ? asPositiveInt((value as Record<string, unknown>).questLineID) : undefined;
    if (questLineId) target.questLineId = questLineId;
  });
  each("C_QuestInfoSystem.GetQuestClassification", (target, value) => {
    if (typeof value === "number") target.classification = value;
  });
  for (const [questId, value] of Object.entries(info)) if (Object.keys(value).length === 0) delete info[questId];
  return Object.keys(info).length > 0 ? info : undefined;
}

// ---------------------------------------------------------------------------
// Dialog steps: quest dialog / gossip events resolved to quest ids and givers
// ---------------------------------------------------------------------------

interface ListStep {
  kind: "gossip" | "greeting";
  tp: number;
  giver?: GiverRef;
  offer: OfferSnapshot;
  /** Legacy greeting titles, resolved after all submissions are read. */
  pending?: Omit<PendingGreeting, "offer">;
}

interface DialogStep {
  kind: "detail" | "progress" | "complete";
  tp: number;
  questId?: number;
  /** GetTitleText paired with the quest id, for greeting title resolution. */
  title?: string;
  /** Giver read from the dialog's own UnitGUID (questnpc/npc). */
  directGiver?: GiverRef;
  /** QUEST_DETAIL questStartItemID. */
  itemId?: number;
  /** A dialog closed (in an earlier frame) moments before this one opened. */
  afterClose?: boolean;
  /** tp of the first QUEST_FINISHED/GOSSIP_CLOSED after the dialog opened. */
  closeTp?: number;
  /**
   * Started from an item. Forever leaves questStartItemID at 0 and only puts an Item GUID, which
   * carries no item id, in questnpc/npc; such details get no giver rather than a guessed one.
   */
  fromItem?: boolean;
}

interface QuestStep {
  kind: "accepted" | "turnedIn" | "removed";
  tp: number;
  questId: number;
  replay?: boolean;
}

type Step = ListStep | DialogStep | QuestStep;

function giverFromGuid(guid: unknown): GiverRef | undefined {
  if (typeof guid !== "string") return undefined;
  // Only creatures, vehicles and game objects give quests; pets and players never do.
  const prefix = guid.slice(0, guid.indexOf("-"));
  if (prefix !== "Creature" && prefix !== "Vehicle" && prefix !== "GameObject") return undefined;
  const parsed = parseGuid(guid);
  const id = asPositiveInt(parsed.id);
  if (!id) return undefined;
  return { kind: parsed.kind === "object" ? "object" : "npc", id };
}

function nonEmptyTitle(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() !== "" ? value : undefined;
}

function sameGiver(a: GiverRef | undefined, b: GiverRef | undefined): boolean {
  return !!a && !!b && a.kind === b.kind && a.id === b.id;
}

/** Unknown givers are compatible with anything: used where matching too much is the safe side. */
function compatibleGiver(a: GiverRef | undefined, b: GiverRef | undefined): boolean {
  return !a || !b || sameGiver(a, b);
}

class SessionReader {
  readonly events: EventEntry[];
  /** For event i: tp of the next dialog open/close event, the end of the samples that describe event i's dialog. */
  private readonly nextDialogTp: number[];
  /** For event i: tp of the previous dialog open event. */
  private readonly prevOpenTp: number[];
  private readonly questIdStream?: FunctionStreamEntry[];
  private readonly titleStream?: FunctionStreamEntry[];

  constructor(readonly session: SessionRecord) {
    this.events = Array.isArray(session.events) ? session.events : [];
    this.nextDialogTp = new Array(this.events.length);
    this.prevOpenTp = new Array(this.events.length);
    let next = Infinity;
    for (let i = this.events.length - 1; i >= 0; i--) {
      this.nextDialogTp[i] = next;
      const name = this.events[i].e;
      if (DIALOG_OPEN.has(name) || DIALOG_CLOSE.has(name)) next = entryTime(this.events[i]);
    }
    let previous = -Infinity;
    for (let i = 0; i < this.events.length; i++) {
      this.prevOpenTp[i] = previous;
      if (DIALOG_OPEN.has(this.events[i].e)) previous = entryTime(this.events[i]);
    }
    this.questIdStream = leafStream(session, "GetQuestID");
    this.titleStream = leafStream(session, "GetTitleText");
  }

  /** tp of the first QUEST_FINISHED/GOSSIP_CLOSED after event i. */
  closeAfter(i: number): number | undefined {
    for (let j = i + 1; j < this.events.length; j++) if (DIALOG_CLOSE.has(this.events[j].e)) return entryTime(this.events[j]);
    return undefined;
  }

  /** Whether a QUEST_FINISHED/GOSSIP_CLOSED from an earlier frame happened within `seconds` before event i. */
  closedShortlyBefore(i: number, seconds: number): boolean {
    const event = this.events[i];
    for (let j = i - 1; j >= 0 && entryTime(event) - entryTime(this.events[j]) <= seconds; j--) {
      if (DIALOG_CLOSE.has(this.events[j].e) && this.events[j].t < event.t) return true;
    }
    return false;
  }

  /** The value a stream held once event i's dialog had been sampled. */
  dialogValue(i: number, name: string, ...params: Array<string | number>): unknown {
    return valueBefore(leafStream(this.session, name, ...params), this.nextDialogTp[i]);
  }

  dialogSampled(i: number, name: string, ...params: Array<string | number>): boolean {
    return sampledBefore(leafStream(this.session, name, ...params), this.nextDialogTp[i]);
  }

  /**
   * Gossip lists are stored change-only, so a GOSSIP_SHOW without its own sample either re-read
   * the same lists or failed to read them, and the two look alike. Inheriting is believable when
   * the earlier read was at the same giver, or when the lists are empty (every vendor and flight
   * master in a row); a non-empty list carried over from another giver is not.
   */
  gossipReadIsOwn(i: number, giver: GiverRef | undefined): boolean {
    const at = entryTime(this.events[i]);
    const streams = [leafStream(this.session, "C_GossipInfo.GetAvailableQuests"), leafStream(this.session, "C_GossipInfo.GetActiveQuests")];
    if (streams.some((stream) => entriesBetween(stream, at, this.nextDialogTp[i]).length > 0)) return true;
    const lists = streams.map((stream) => valueBefore(stream, at));
    if (lists.every((list) => !Array.isArray(list) || list.length === 0)) return true;
    let lastRead = -Infinity;
    for (const stream of streams) {
      const index = stream ? lastIndexBefore(stream, at) : -1;
      if (index >= 0) lastRead = Math.max(lastRead, entryTime(stream![index]));
    }
    // UnitInteraction samples the npc token just before the lists, in the same handler.
    return sameGiver(giver, giverFromGuid(valueAtOrBefore(leafStream(this.session, "UnitGUID", "npc"), lastRead)));
  }

  giverAt(i: number, tokens: readonly string[]): GiverRef | undefined {
    for (const token of tokens) {
      const giver = giverFromGuid(this.dialogValue(i, "UnitGUID", token));
      if (giver) return giver;
    }
    return undefined;
  }

  /**
   * GetQuestID for the dialog opened by event i. Prefers samples taken for the event itself;
   * falls back to samples earlier in the same frame, because the client can already be in the
   * new dialog while it is still dispatching the previous dialog's QUEST_FINISHED events (and an
   * auto-accept addon can close it again before QUEST_DETAIL reaches us).
   */
  dialogQuest(i: number): { questId: number; title?: string } | undefined {
    const event = this.events[i];
    const at = entryTime(event);
    const nonZero = (entry: FunctionStreamEntry) => asPositiveInt(entry.v) !== undefined;
    const ownSamples = entriesBetween(this.questIdStream, at, this.nextDialogTp[i]);
    const chosen =
      ownSamples.filter(nonZero).at(-1) ??
      entriesBetween(this.questIdStream, this.prevOpenTp[i], at)
        .filter((entry) => entry.t === event.t && nonZero(entry))
        .at(-1);
    if (chosen) return { questId: chosen.v as number, title: this.titleAt(entryTime(chosen)) };
    // The dialog's own reads saw no quest (it was already closed): an older value would be another dialog's.
    if (ownSamples.length > 0) return undefined;
    // Nothing new was sampled: the dialog still shows what an earlier sample of it read.
    const inEffect = asPositiveInt(valueBefore(this.questIdStream, at));
    return inEffect ? { questId: inEffect, title: this.titleAt(at, false) } : undefined;
  }

  /** GetTitleText from the same sample as a GetQuestID entry (both are read in one call). */
  private titleAt(tp: number, inclusive = true): string | undefined {
    return nonEmptyTitle(inclusive ? valueAtOrBefore(this.titleStream, tp) : valueBefore(this.titleStream, tp));
  }
}

function offeredFromGossip(entry: unknown): OfferedQuest | undefined {
  if (!entry || typeof entry !== "object") return undefined;
  const info = entry as Record<string, unknown>;
  const id = asPositiveInt(info.questID);
  if (!id) return undefined;
  const offered: OfferedQuest = { id };
  if (typeof info.isTrivial === "boolean") offered.trivial = info.isTrivial;
  if (typeof info.frequency === "number") offered.frequency = info.frequency;
  if (typeof info.repeatable === "boolean") offered.repeatable = info.repeatable;
  return offered;
}

function gossipStep(reader: SessionReader, i: number, stats: DistillStats, titles: TitleSink): ListStep {
  const event = reader.events[i];
  const tp = entryTime(event);
  const giver = reader.giverAt(i, ["npc", "questnpc"]);
  const rawAvailable = reader.dialogValue(i, "C_GossipInfo.GetAvailableQuests");
  const rawActive = reader.dialogValue(i, "C_GossipInfo.GetActiveQuests");
  const read = reader.dialogSampled(i, "C_GossipInfo.GetAvailableQuests") && Array.isArray(rawAvailable);
  if (!read) stats.gossipWithoutRead++;
  const inheritedFromOtherGiver = read && !reader.gossipReadIsOwn(i, giver);
  if (inheritedFromOtherGiver) stats.gossipInheritedFromOtherGiver++;

  // A list carried over from another giver describes that giver, not this one: keep nothing.
  const availableEntries: unknown[] = Array.isArray(rawAvailable) && !inheritedFromOtherGiver ? rawAvailable : [];
  const activeEntries: unknown[] = Array.isArray(rawActive) && !inheritedFromOtherGiver ? rawActive : [];
  const available = availableEntries.map(offeredFromGossip).filter((q): q is OfferedQuest => !!q);
  const active = activeEntries.map(offeredFromGossip).filter((q): q is OfferedQuest => !!q).map((q) => q.id);
  for (const entry of [...availableEntries, ...activeEntries]) {
    const info = entry as Record<string, unknown> | undefined;
    titles.add(info?.title, asPositiveInt(info?.questID), giver);
  }

  // The count API is sampled by a different tracker; disagreement means one read was stale.
  const count = reader.dialogValue(i, "C_GossipInfo.GetNumAvailableQuests");
  const countAgrees = typeof count !== "number" || count === available.length;
  // GOSSIP_SHOW and GOSSIP_CLOSED in one frame: an auto-quest addon handled the gossip at once.
  // A read that still saw quests happened before the close; an empty one may describe the closed
  // window instead of the giver.
  let closedSameFrame = false;
  for (let j = i + 1; j < reader.events.length && reader.events[j].t === event.t; j++) {
    if (reader.events[j].e === "GOSSIP_CLOSED") closedSameFrame = true;
  }
  const emptyAfterClose = closedSameFrame && available.length === 0 && active.length === 0;

  return {
    kind: "gossip",
    tp,
    giver,
    offer: {
      t: tp,
      source: "gossip",
      ...(giver ? { giver } : {}),
      available,
      active,
      listComplete: read && !inheritedFromOtherGiver && countAgrees && !emptyAfterClose,
    },
  };
}

/**
 * QUEST_GREETING lists. Newer addon builds record the quest id behind each index
 * (GetAvailableQuestInfo's 5th return, GetActiveQuestID); older builds, and clients whose
 * GetAvailableQuestInfo has no quest id, only have titles, which are resolved once every
 * submission's title pairings are known (titles.ts).
 */
function greetingStep(reader: SessionReader, i: number, stats: DistillStats, titles: TitleSink): ListStep {
  const tp = entryTime(reader.events[i]);
  const giver = reader.giverAt(i, ["npc", "questnpc"]);
  const numAvailable = reader.dialogValue(i, "GetNumAvailableQuests");
  const numActive = reader.dialogValue(i, "GetNumActiveQuests");
  if (typeof numAvailable !== "number") {
    stats.greetingWithoutCount++;
    return { kind: "greeting", tp, giver, offer: { t: tp, source: "greeting", ...(giver ? { giver } : {}), available: [], active: [], listComplete: false } };
  }

  const available: OfferedQuest[] = [];
  const availableTitles: string[] = [];
  for (let index = 1; index <= numAvailable; index++) {
    const info = reader.dialogValue(i, "GetAvailableQuestInfo", index);
    const title = nonEmptyTitle(reader.dialogValue(i, "GetAvailableTitle", index));
    const id = asPositiveInt(packed(info, 5));
    if (!id) {
      if (title) availableTitles.push(title);
      continue;
    }
    titles.add(title, id, giver);
    const offered: OfferedQuest = { id };
    if (typeof packed(info, 1) === "boolean") offered.trivial = packed(info, 1) as boolean;
    if (typeof packed(info, 2) === "number") offered.frequency = packed(info, 2) as number;
    if (typeof packed(info, 3) === "boolean") offered.repeatable = packed(info, 3) as boolean;
    available.push(offered);
  }

  const active: number[] = [];
  const activeTitles: string[] = [];
  for (let index = 1; index <= (typeof numActive === "number" ? numActive : 0); index++) {
    const id = asPositiveInt(reader.dialogValue(i, "GetActiveQuestID", index));
    const title = nonEmptyTitle(packed(reader.dialogValue(i, "GetActiveTitle", index), 1));
    if (id) titles.add(title, id, giver);
    if (id) active.push(id);
    else if (title) activeTitles.push(title);
  }

  const ids = available.map((quest) => quest.id);
  const offer: OfferSnapshot = {
    t: tp,
    source: "greeting",
    ...(giver ? { giver } : {}),
    available,
    active,
    listComplete: ids.length === numAvailable && new Set(ids).size === ids.length,
  };
  if (availableTitles.length === 0 && activeTitles.length === 0) return { kind: "greeting", tp, giver, offer };
  // Completeness is decided again once the titles are resolved.
  offer.listComplete = false;
  return { kind: "greeting", tp, giver, offer, pending: { available: availableTitles, active: activeTitles, expected: numAvailable } };
}

function dialogStep(reader: SessionReader, i: number, kind: DialogStep["kind"], stats: DistillStats): DialogStep {
  const quest = reader.dialogQuest(i);
  const step: DialogStep = { kind, tp: entryTime(reader.events[i]) };
  const closeTp = reader.closeAfter(i);
  if (closeTp !== undefined) step.closeTp = closeTp;
  if (quest) {
    step.questId = quest.questId;
    if (quest.title) step.title = quest.title;
  }
  if (kind === "detail") {
    if (reader.closedShortlyBefore(i, DISTILL_PARAMS.pushedDetailSeconds)) step.afterClose = true;
    const itemId = asPositiveInt(eventArg(reader.events[i], 1));
    if (itemId) step.itemId = itemId;
    const itemGuid = ["questnpc", "npc"].some((token) => String(reader.dialogValue(i, "UnitGUID", token)).startsWith("Item-"));
    if (itemId || itemGuid) {
      step.fromItem = true;
      stats.itemStartedDetails++;
      return step;
    }
  }
  const directGiver = reader.giverAt(i, ["questnpc", "npc"]);
  if (directGiver) step.directGiver = directGiver;
  return step;
}

/** QUEST_ACCEPTED is (logIndex, questID) on Era and (questID) on Forever. */
function acceptedQuestId(event: EventEntry): number | undefined {
  const n = typeof event.a?.n === "number" ? event.a.n : 0;
  return asPositiveInt(eventArg(event, n >= 2 ? 2 : 1));
}

// ---------------------------------------------------------------------------
// Title observations (only for resolving legacy greeting titles)
// ---------------------------------------------------------------------------

class TitleSink {
  private readonly seen = new Map<string, TitleObservation>();
  constructor(
    private readonly locale: string | undefined,
    private readonly faction: string | undefined,
  ) {}

  add(title: unknown, questId: number | undefined, giver?: GiverRef): void {
    const text = nonEmptyTitle(title);
    if (!this.locale || !text || !questId) return;
    const observation: TitleObservation = {
      locale: this.locale,
      ...(this.faction ? { faction: this.faction } : {}),
      title: text,
      questId,
      ...(giver ? { giver: giverKey(giver) } : {}),
    };
    this.seen.set(`${observation.giver ?? ""}\u0000${text}\u0000${questId}`, observation);
  }

  /** Quest log rows: C_QuestLog.GetInfo values (modern) and GetQuestLogTitle tuples (legacy). */
  addQuestLog(session: SessionRecord): void {
    const visit = (name: string, read: (value: unknown) => void) => {
      const root = session.functions?.[name];
      if (!root || Array.isArray(root)) return;
      for (const stream of Object.values(root)) if (Array.isArray(stream)) for (const entry of stream) read(entry.v);
    };
    visit("C_QuestLog.GetInfo", (value) => {
      if (!value || typeof value !== "object") return;
      const info = value as Record<string, unknown>;
      if (info.isHeader !== true) this.add(info.title, asPositiveInt(info.questID));
    });
    visit("GetQuestLogTitle", (value) => {
      if (packed(value, 4) !== true) this.add(packed(value, 1), asPositiveInt(packed(value, 8)));
    });
  }

  list(): TitleObservation[] {
    return [...this.seen.values()];
  }
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

function isList(step: Step): step is ListStep {
  return step.kind === "gossip" || step.kind === "greeting";
}

function isDialog(step: Step): step is DialogStep {
  return step.kind === "detail" || step.kind === "progress" || step.kind === "complete";
}

function stepGiver(step: Step): GiverRef | undefined {
  return isList(step) ? step.giver : isDialog(step) ? step.directGiver : undefined;
}

function listContains(list: ListStep, detail: DialogStep): boolean {
  if (detail.questId && list.offer.available.some((q) => q.id === detail.questId)) return true;
  return !!detail.title && !!list.pending?.available.includes(detail.title);
}

interface DetailContext {
  /** The gossip/greeting list the detail was picked from. */
  fromList?: ListStep;
  /** A gossip/greeting list from this giver was shown recently, so the detail was not opened directly. */
  listNearby: boolean;
  /** The quest turned in just before, whose follow-up this detail likely is. */
  afterTurnIn?: QuestStep;
  /** Some other dialog with a compatible giver was open moments before. */
  busy: boolean;
  /**
   * The detail came right after an accept or another dialog closing: the server pushed it (e.g. the
   * giver's next quest after an accept), so it says nothing about the giver's whole list.
   */
  pushed: boolean;
}

function detailContext(steps: Step[], index: number, turnInGivers: Map<QuestStep, GiverRef | undefined>): DetailContext {
  const detail = steps[index] as DialogStep;
  const context: DetailContext = { listNearby: false, busy: false, pushed: !!detail.afterClose };
  // The giver this visit is with: the detail's own, else the latest one seen before it.
  let anchor = detail.directGiver;
  let otherGiverSeen = false;
  for (let j = index - 1; j >= 0; j--) {
    const step = steps[j];
    const age = detail.tp - step.tp;
    if (age > DISTILL_PARAMS.listContextSeconds) break;
    const giver = stepGiver(step);
    anchor ??= giver;
    // Lists behind an interaction with a different known giver belong to an earlier visit.
    if (giver && !sameGiver(giver, anchor)) otherGiverSeen = true;
    if (isList(step) && !otherGiverSeen && compatibleGiver(step.giver, anchor)) {
      context.listNearby = true;
      if (!context.fromList && listContains(step, detail)) context.fromList = step;
    }
    if (age <= DISTILL_PARAMS.quietBeforeDetailSeconds && (isList(step) || isDialog(step)) && compatibleGiver(giver, detail.directGiver)) {
      context.busy = true;
    }
    if (step.kind === "accepted" && step.questId !== detail.questId && age <= DISTILL_PARAMS.pushedDetailSeconds) context.pushed = true;
    if (step.kind === "turnedIn" && age <= DISTILL_PARAMS.afterTurnInSeconds && !context.afterTurnIn) {
      if (compatibleGiver(turnInGivers.get(step), detail.directGiver)) context.afterTurnIn = step;
    }
  }
  return context;
}

/** The most recent earlier step matching `match` within the carry-over window. */
function findBack<T extends Step>(steps: Step[], index: number, match: (step: Step) => step is T): T | undefined {
  const tp = steps[index].tp;
  for (let j = index - 1; j >= 0 && tp - steps[j].tp <= DISTILL_PARAMS.carryGiverSeconds; j--) {
    if (match(steps[j])) return steps[j] as T;
  }
  return undefined;
}

/**
 * Fills dialog quest ids that GetQuestID never showed from the accept/turn-in that closes the
 * dialog. Clicking Accept closes the window and the server's QUEST_ACCEPTED follows within a
 * moment; an accept arriving long after the close answers something else (the detail was declined).
 */
function borrowQuestIds(steps: Step[], stats: DistillStats): void {
  for (let index = 0; index < steps.length; index++) {
    const step = steps[index];
    if (!isDialog(step) || step.questId) continue;
    const closing = step.kind === "detail" ? "accepted" : "turnedIn";
    for (let j = index + 1; j < steps.length && steps[j].tp - step.tp <= DISTILL_PARAMS.followUpSeconds; j++) {
      const next = steps[j];
      if (isList(next) || isDialog(next)) break;
      if (next.kind !== closing) continue;
      if (step.closeTp === undefined || next.tp - step.closeTp <= DISTILL_PARAMS.followUpAfterCloseSeconds) {
        step.questId = next.questId;
        stats.borrowedQuestIds++;
      }
      break;
    }
    if (!step.questId) {
      if (step.kind === "detail") stats.detailUnresolved++;
      else stats.progressUnresolved++;
    }
  }
}

function questEventsAndOffers(
  steps: Step[],
  completedAdds: Map<number, number[]>,
  stats: DistillStats,
): { questEvents: QuestEvent[]; offers: OfferSnapshot[]; pending: PendingGreeting[]; eventGiver: Map<Step, GiverRef | undefined> } {
  const questEvents: QuestEvent[] = [];
  const offers: OfferSnapshot[] = [];
  const pending: PendingGreeting[] = [];
  const turnInGivers = new Map<QuestStep, GiverRef | undefined>();
  const eventGiver = new Map<Step, GiverRef | undefined>();
  /** Offers that can claim completeness, with the index of the step they came from. */
  const offerSteps = new Map<OfferSnapshot, number>();
  const pendingByOffer = new Map<OfferSnapshot, PendingGreeting>();
  const push = (step: Step, kind: QuestEventKind, questId: number, giver?: GiverRef) => {
    eventGiver.set(step, giver);
    questEvents.push(giver ? { t: step.tp, kind, questId, giver } : { t: step.tp, kind, questId });
  };

  for (let index = 0; index < steps.length; index++) {
    const step = steps[index];
    if (isList(step)) {
      if (step.pending) {
        const entry: PendingGreeting = { offer: offers.length, ...step.pending };
        pending.push(entry);
        pendingByOffer.set(step.offer, entry);
      }
      offers.push(step.offer);
      offerSteps.set(step.offer, index);
      continue;
    }

    if (isDialog(step)) {
      if (!step.questId) continue;
      if (step.kind !== "detail") {
        // progress -> complete of one quest is a single visit; the giver carries over.
        const earlier = findBack(steps, index, (s): s is DialogStep => isDialog(s) && s.kind !== "detail" && s.questId === step.questId);
        push(step, step.kind, step.questId, step.directGiver ?? (earlier ? eventGiver.get(earlier) : undefined));
        continue;
      }

      if (step.fromItem) {
        const item: GiverRef | undefined = step.itemId ? { kind: "item", id: step.itemId } : undefined;
        push(step, "detail", step.questId, item);
        offers.push({ t: step.tp, source: "detail", ...(item ? { giver: item } : {}), available: [{ id: step.questId }], listComplete: false });
        continue;
      }

      const context = detailContext(steps, index, turnInGivers);
      const giver = step.directGiver ?? context.fromList?.giver ?? (context.afterTurnIn ? turnInGivers.get(context.afterTurnIn) : undefined);
      push(step, "detail", step.questId, giver);
      if (context.fromList) continue;
      // Opened by a fresh right-click on a giver with nothing else to say: the server only does
      // that when this is the giver's single available quest, so the one-quest list is complete.
      const standalone = !!step.directGiver && !context.listNearby && !context.afterTurnIn && !context.busy && !context.pushed;
      const offer: OfferSnapshot = { t: step.tp, source: "detail", ...(giver ? { giver } : {}), available: [{ id: step.questId }], listComplete: standalone };
      offers.push(offer);
      offerSteps.set(offer, index);
      continue;
    }

    if (step.kind === "accepted") {
      const detail = findBack(steps, index, (s): s is DialogStep => s.kind === "detail" && (s as DialogStep).questId === step.questId);
      push(step, "accepted", step.questId, detail ? eventGiver.get(detail) : undefined);
    } else if (step.kind === "turnedIn") {
      const dialog = findBack(steps, index, (s): s is DialogStep => isDialog(s) && s.kind !== "detail" && s.questId === step.questId);
      const giver = dialog ? eventGiver.get(dialog) : undefined;
      turnInGivers.set(step, giver);
      push(step, "turnedIn", step.questId, giver);
    } else {
      const kind = classifyRemoval(steps, index, completedAdds);
      if (kind) push(step, kind, step.questId);
    }
  }
  markInFlightAccepts(steps, offerSteps, pendingByOffer);
  markSubMenus(steps);
  return { questEvents, offers, pending, eventGiver };
}

/**
 * After an accept the server re-sends the giver's menu before QUEST_ACCEPTED (and the quest log
 * update) reaches the client, so the list already lacks the quest being accepted while the log
 * does not show it yet. Such a list cannot vouch for what was missing.
 */
function markInFlightAccepts(steps: Step[], offerSteps: Map<OfferSnapshot, number>, pendingByOffer: Map<OfferSnapshot, PendingGreeting>): void {
  for (const [offer, index] of offerSteps) {
    const pending = pendingByOffer.get(offer);
    if (!offer.listComplete && !pending) continue;
    const listed = new Set(offer.available.map((quest) => quest.id));
    for (let j = index + 1; j < steps.length && steps[j].tp - steps[index].tp <= DISTILL_PARAMS.inFlightAcceptSeconds; j++) {
      const next = steps[j];
      if (isList(next) || isDialog(next)) break;
      if (next.kind === "accepted" && !listed.has(next.questId)) {
        offer.listComplete = false;
        if (pending) pending.incomplete = true;
      }
    }
  }
}

/**
 * Picking a gossip option shows another gossip page, which lists no quests even though the giver
 * has some. An empty page right after a non-empty one from the same giver, with no quest event in
 * between, is such a sub-menu rather than an empty list.
 */
function markSubMenus(steps: Step[]): void {
  let previous: ListStep | undefined;
  let questActivity = false;
  for (const step of steps) {
    if (!isList(step)) {
      questActivity = true;
      continue;
    }
    const empty = step.offer.available.length === 0 && (step.offer.active?.length ?? 0) === 0 && !step.pending;
    const parentHadQuests = !!previous && (previous.offer.available.length > 0 || (previous.offer.active?.length ?? 0) > 0 || !!previous.pending);
    if (
      step.kind === "gossip" &&
      empty &&
      previous &&
      parentHadQuests &&
      !questActivity &&
      sameGiver(previous.giver, step.giver) &&
      step.tp - previous.tp <= DISTILL_PARAMS.subMenuSeconds
    ) {
      step.offer.listComplete = false;
      continue; // Further pages still belong to the same menu.
    }
    previous = step;
    questActivity = false;
  }
}

/**
 * QUEST_REMOVED: undefined when it is the quest's own turn-in (or completion), "removed" when
 * something else may have removed it (replay quests; another quest accepted/turned in moments
 * before or after, e.g. the server dropping a breadcrumb), otherwise "abandoned".
 */
function classifyRemoval(steps: Step[], index: number, completedAdds: Map<number, number[]>): "abandoned" | "removed" | undefined {
  const removal = steps[index] as QuestStep;
  const near = (tp: number, window: number) => Math.abs(tp - removal.tp) <= window;
  const turnInWindow = DISTILL_PARAMS.removalTurnInSeconds;
  if ((completedAdds.get(removal.questId) ?? []).some((tp) => near(tp, turnInWindow))) return undefined;
  let otherQuestActivity = false;
  for (const step of steps) {
    if (step.kind !== "turnedIn" && step.kind !== "accepted") continue;
    if (step.kind === "turnedIn" && step.questId === removal.questId && near(step.tp, turnInWindow)) return undefined;
    if (step.questId !== removal.questId && near(step.tp, DISTILL_PARAMS.removalOtherQuestSeconds)) otherQuestActivity = true;
  }
  return removal.replay || otherQuestActivity ? "removed" : "abandoned";
}

export function distillSession(session: SessionRecord): DistilledSession {
  const stats = emptyDistillStats();
  const reader = new SessionReader(session);
  const locale = firstPlayerValue(session, "GetLocale");
  const player = readPlayer(session);
  const titles = new TitleSink(typeof locale === "string" ? locale : undefined, player.faction);
  titles.addQuestLog(session);

  const steps: Step[] = [];
  reader.events.forEach((event, i) => {
    const tp = entryTime(event);
    switch (event.e) {
      case "GOSSIP_SHOW":
        steps.push(gossipStep(reader, i, stats, titles));
        break;
      case "QUEST_GREETING":
        steps.push(greetingStep(reader, i, stats, titles));
        break;
      case "QUEST_DETAIL":
        steps.push(dialogStep(reader, i, "detail", stats));
        break;
      case "QUEST_PROGRESS":
        steps.push(dialogStep(reader, i, "progress", stats));
        break;
      case "QUEST_COMPLETE":
        steps.push(dialogStep(reader, i, "complete", stats));
        break;
      case "QUEST_ACCEPTED": {
        const questId = acceptedQuestId(event);
        if (questId) steps.push({ kind: "accepted", tp, questId });
        break;
      }
      case "QUEST_TURNED_IN": {
        const questId = asPositiveInt(eventArg(event, 1));
        if (questId) steps.push({ kind: "turnedIn", tp, questId });
        break;
      }
      case "QUEST_REMOVED": {
        const questId = asPositiveInt(eventArg(event, 1));
        if (questId) steps.push({ kind: "removed", tp, questId, replay: eventArg(event, 2) === true });
        break;
      }
    }
  });
  borrowQuestIds(steps, stats);

  const completed = readCompleted(session, stats);
  const completedAdds = new Map<number, number[]>();
  for (const change of completed?.changes ?? []) for (const id of change.add ?? []) (completedAdds.get(id) ?? completedAdds.set(id, []).get(id)!).push(change.t);
  const { questEvents, offers, pending, eventGiver } = questEventsAndOffers(steps, completedAdds, stats);
  for (const step of steps) {
    if (isDialog(step) && step.questId) titles.add(step.title, step.questId, eventGiver.get(step));
  }

  const lastEventTp = reader.events.length > 0 ? entryTime(reader.events[reader.events.length - 1]) : 0;
  const recorded = [session.durationPrecise, session.duration].find((value): value is number => typeof value === "number");
  // A session recovered after a client restart gets its stop time from the new clock.
  const duration = recorded !== undefined && recorded >= lastEventTp && recorded <= lastEventTp + DURATION_SLACK_SECONDS ? recorded : lastEventTp;
  const interfaceVersion = readInterfaceVersion(session);
  const questInfo = readQuestInfo(session);
  const serverTime = leafStream(session, "GetServerTime")?.[0];

  return {
    key: sessionKey(session),
    eventCount: reader.events.length,
    saved: typeof session.name === "string",
    ...(serverTime && typeof serverTime.v === "number" ? { startServerTime: serverTime.v - entryTime(serverTime) } : {}),
    episode: {
      key: sessionKey(session),
      ...(typeof session.name === "string" && SESSION_NAME.test(session.name) ? { sessionName: session.name } : {}),
      ...(interfaceVersion ? { interfaceVersion } : {}),
      ...(typeof locale === "string" ? { locale } : {}),
      player,
      duration,
      levels: readLevels(session),
      ...(completed ? { completed } : {}),
      questLog: readQuestLog(session),
      questEvents,
      offers,
      ...(questInfo ? { questInfo } : {}),
    },
    pendingGreetings: pending,
    titles: titles.list(),
    stats,
  };
}
