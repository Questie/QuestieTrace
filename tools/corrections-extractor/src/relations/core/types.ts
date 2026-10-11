// Shared contracts for quest-relationship inference (see ../README.md).
//
// Data flows through three files, each owned by one area:
//
//   trace-data submissions --ingest--> episodes.jsonl (QuestEpisode per line)
//   QuestieDB sources      --catalog-> catalog.json (QuestCatalog) + groundtruth.json (GroundTruth)
//   episodes + catalog     --signals-> candidates/<signal>.json (CandidateFile)
//
// These types are the seams between areas. Change them only in core, and keep
// every producer and consumer in sync when you do.

// ---------------------------------------------------------------------------
// Relationship fields (the Questie questKeys we infer)
// ---------------------------------------------------------------------------

/**
 * Questie quest relationship fields, as evaluated by Questie's `QuestieDB.IsDoable`
 * (Questie-Forever/Database/QuestieDB.lua). Semantics for quest Q:
 *
 * - preQuestSingle: Q is available once ANY listed quest is completed.
 * - preQuestGroup: Q is available once ALL listed quests are completed (an entry also counts as
 *   done when one of its own exclusiveTo siblings is completed). Ignored when preQuestSingle is set.
 * - exclusiveTo: Q is hidden while ANY listed quest is completed or in the quest log.
 *   One-directional in Questie, so both sides must be emitted.
 * - nextQuestInChain: Q is hidden once that quest is completed or in the quest log.
 * - parentQuest: Q is only available while that quest is in the quest log.
 * - breadcrumbForQuestId: on breadcrumb Q, Q is hidden once its target is completed or in the log.
 * - breadcrumbs: on target Q, Q is hidden while any listed breadcrumb is in the quest log.
 * - availableUntilCompleted: Q is hidden once that quest is completed.
 * - availableStartingWith: Q is only available once that quest is completed or in the quest log.
 * - disabledByQuest: Q is hidden while that quest is in the quest log.
 *
 * Scalar fields (nextQuestInChain, parentQuest, breadcrumbForQuestId, availableUntilCompleted,
 * availableStartingWith, disabledByQuest) are still modelled as one candidate per target.
 */
export const RELATION_FIELDS = [
  "preQuestSingle",
  "preQuestGroup",
  "exclusiveTo",
  "nextQuestInChain",
  "parentQuest",
  "breadcrumbForQuestId",
  "breadcrumbs",
  "availableUntilCompleted",
  "availableStartingWith",
  "disabledByQuest",
] as const;

export type RelationField = (typeof RELATION_FIELDS)[number];

/** Fields Questie stores as a single quest id rather than a list. */
export const SCALAR_RELATION_FIELDS: ReadonlySet<RelationField> = new Set<RelationField>([
  "nextQuestInChain",
  "parentQuest",
  "breadcrumbForQuestId",
  "availableUntilCompleted",
  "availableStartingWith",
  "disabledByQuest",
]);

/**
 * Relationship values for one quest. Scalar fields are normalized to one-element arrays, and
 * negative preQuestGroup entries (Questie: "this quest, without the exclusiveTo substitute")
 * are normalized to their absolute value.
 */
export type RelationSet = Partial<Record<RelationField, number[]>>;

// ---------------------------------------------------------------------------
// Episodes (produced by ingest/, consumed by signals/)
// ---------------------------------------------------------------------------

/** Seconds since the start of the recording session. Never compare across episodes. */
export type SessionTime = number;

export interface TimedValue<T> {
  t: SessionTime;
  v: T;
}

/** The entity a quest dialog or offer list belongs to. Players are never givers. */
export interface GiverRef {
  kind: "npc" | "object" | "item";
  id: number;
}

/** Character context. Pseudonymous only: never names, GUIDs, realms or guilds. */
export interface PlayerContext {
  /** Blizzard ChrRaces id (UnitRace 3rd return). Questie race bit = 2^(raceId - 1). */
  raceId?: number;
  /** Blizzard ChrClasses id (UnitClass 3rd return). Questie class bit = 2^(classId - 1). */
  classId?: number;
  faction?: "Alliance" | "Horde" | "Neutral";
}

/**
 * The character's completed-quest set over the session: `initial` is the full completion history
 * when the capture started, `changes` are the deltas observed afterwards. Removals happen for
 * daily/weekly resets.
 */
export interface CompletedTimeline {
  /** Which API the data came from. Prefer "modern" (C_QuestLog.GetAllCompletedQuestIDs). */
  source: "modern" | "legacy";
  initial: number[];
  changes: Array<{ t: SessionTime; add?: number[]; remove?: number[] }>;
}

/**
 * - detail: the quest's accept dialog was shown (QUEST_DETAIL).
 * - accepted / turnedIn: QUEST_ACCEPTED / QUEST_TURNED_IN.
 * - progress / complete: the turn-in dialog pages (QUEST_PROGRESS / QUEST_COMPLETE).
 * - abandoned: removed from the log without a turn-in and without becoming completed.
 * - removed: removed from the log, but ingest could not tell why.
 */
export type QuestEventKind = "detail" | "accepted" | "progress" | "complete" | "turnedIn" | "abandoned" | "removed";

export interface QuestEvent {
  t: SessionTime;
  kind: QuestEventKind;
  questId: number;
  /** Who the dialog belonged to, when ingest could resolve it. */
  giver?: GiverRef;
}

export interface OfferedQuest {
  id: number;
  /** Gray/low-level quest. The client still lists these; only the color changes. */
  trivial?: boolean;
  /** 1 normal, 2 daily, 3 weekly. */
  frequency?: number;
  repeatable?: boolean;
}

/**
 * The quests a giver offered at one moment. Servers omit unavailable quests entirely
 * (they are not shown grayed out), so a complete list is evidence about what was NOT available.
 */
export interface OfferSnapshot {
  t: SessionTime;
  /**
   * - gossip: C_GossipInfo.GetAvailableQuests/GetActiveQuests at GOSSIP_SHOW.
   * - greeting: the QUEST_GREETING frame (GetAvailableQuestInfo / GetAvailableTitle per index).
   * - detail: a QUEST_DETAIL that was not reached by picking from a gossip/greeting list.
   */
  source: "gossip" | "greeting" | "detail";
  giver?: GiverRef;
  available: OfferedQuest[];
  /** Greeting titles that ingest could not map to a quest id (legacy traces record titles only). */
  unresolvedTitles?: string[];
  /** Quests the giver showed as in progress / ready to turn in. */
  active?: number[];
  /**
   * True when `available` is believed to be the giver's ENTIRE available list at `t`
   * (successful gossip read, fully resolved greeting, or a standalone detail dialog).
   * Only complete lists may be used as evidence that a quest was not offered.
   */
  listComplete: boolean;
}

/** Client-side facts about a quest that newer traces record directly (Forever-only APIs). */
export interface QuestClientInfo {
  /** Raw IsBreadcrumbQuest(questID) result. Undocumented API: treat as a hint, verify against behaviour. */
  isBreadcrumb?: boolean;
  /** C_QuestLine.GetQuestLineInfo(questID).questLineID */
  questLineId?: number;
  /** C_QuestInfoSystem.GetQuestClassification(questID) (Enum.QuestClassification, 6 = Questline). */
  classification?: number;
}

/** One deduplicated recording session, reduced to what quest-relationship inference needs. */
export interface QuestEpisode {
  /** Stable, unique, pseudonymous id of this session. */
  key: string;
  /** Pseudonymous contributor id from the trace-data submission. */
  contributorId: string;
  /** Pseudonymous id grouping the sessions ingest believes belong to the same character. */
  characterKey: string;
  /** Trace-data submission ids that contained this session (duplicates are merged). */
  submissionIds: string[];
  /** Session name as recorded (a local "YYYY-MM-DD_HH-MM-SS" stamp). Absent for about a third of sessions. */
  sessionName?: string;
  /**
   * 0-based position of this session among its character's sessions, oldest first. Ordered by
   * progress (size of the completed history at capture start), then capture start time, then
   * session name. Use this, not sessionName, to order a character's sessions: in-progress exports
   * carry no name.
   */
  sessionOrder: number;
  /** GetBuildInfo interface version. Forever builds start with 16. Undefined for old traces. */
  interfaceVersion?: number;
  locale?: string;
  player: PlayerContext;
  duration: SessionTime;
  /** Player level change points; the first entry is the level at (or near) t = 0. */
  levels: TimedValue<number>[];
  completed?: CompletedTimeline;
  /** Active quest log change points (quest ids, no headers). */
  questLog: TimedValue<number[]>[];
  /** Quest dialog and log events in time order. */
  questEvents: QuestEvent[];
  /** Giver offer lists in time order. */
  offers: OfferSnapshot[];
  /** Per-quest client facts, keyed by quest id. Absent in traces older than the addon change. */
  questInfo?: Record<string, QuestClientInfo>;
}

// ---------------------------------------------------------------------------
// Catalog and ground truth (produced by catalog/, consumed by signals/ and scoring)
// ---------------------------------------------------------------------------

/**
 * Non-relationship facts about a quest from QuestieDB, as Forever sees them (Classic base,
 * then Forever legacy, generated, traces and authored corrections). Signals use these to gate
 * evidence: a quest the character could not take for level/race/class/skill reasons says
 * nothing about relationships.
 */
export interface CatalogQuest {
  id: number;
  name?: string;
  questLevel?: number;
  requiredLevel?: number;
  /** Questie bitmask; undefined or 0 means every race. Uses bits above 32: test with core/eligibility.ts. */
  requiredRaces?: number;
  /** Questie bitmask; undefined or 0 means every class. */
  requiredClasses?: number;
  /** [skillId, value] */
  requiredSkill?: [number, number];
  /** [factionId, value] */
  requiredMinRep?: [number, number];
  requiredMaxRep?: [number, number];
  /** Spell the character must know; negative means the character must NOT know it. */
  requiredSpell?: number;
  /** Bitmask: 1 = repeatable, 2 = needs event, 4 = monthly. */
  specialFlags?: number;
  starters: { npcs: number[]; objects: number[]; items: number[] };
  finishers: { npcs: number[]; objects: number[] };
}

export interface QuestCatalog {
  generatedAt: string;
  /** Source files in layering order. */
  sources: string[];
  quests: Record<string, CatalogQuest>;
}

/**
 * Reference relationship data to score signals against. Signals must NOT read this as input.
 *
 * - authoredForever: hand-authored relations for Forever-new quests (QuestieDB foreverQuestFixes).
 * - inheritedClassic: every quest Forever inherits from Era (data/Forever rows plus anything the
 *   Forever/legacy/ corrections touch, whatever its id), with relations as Forever loads them ({} when
 *   none). Mostly server-derived; useful because traces cover Classic content too.
 */
export interface GroundTruth {
  generatedAt: string;
  sources: string[];
  tiers: {
    authoredForever: Record<string, RelationSet>;
    inheritedClassic: Record<string, RelationSet>;
  };
}

// ---------------------------------------------------------------------------
// Candidates (produced by signals/ and sources/, consumed by combine/ and scoring)
// ---------------------------------------------------------------------------

export interface Evidence {
  /** QuestEpisode.key, or a source reference such as "wowhead:quest/92579". */
  ref: string;
  t?: SessionTime;
  /** One human-readable line a reviewer can check, e.g. "turned in 91741 at npc 1234, 92124 offered 0.8s later". */
  text: string;
}

/**
 * One claimed relationship edge: `questId`'s `field` contains (or, for scalar fields, equals) `target`.
 * A list value such as preQuestSingle = {A, B} is two candidates.
 */
export interface RelationCandidate {
  questId: number;
  field: RelationField;
  target: number;
  /** Signal-specific confidence in [0, 1]. */
  score: number;
  /** Independent characters whose observations support the edge. */
  support: number;
  /** Independent characters whose observations contradict the edge. */
  contradict: number;
  /** A few concrete examples (keep it to about 5). */
  evidence: Evidence[];
  note?: string;
}

export interface CandidateFile {
  /** Signal name, matching the file name: candidates/<signal>.json */
  signal: string;
  generatedAt: string;
  /** Episodes (or source records) the signal looked at. */
  inputCount: number;
  /** Tunables the signal ran with, for reproducibility. */
  params: Record<string, unknown>;
  /** Quests the signal had enough data to judge, whether or not it produced candidates for them. Used for recall. */
  coveredQuestIds: number[];
  candidates: RelationCandidate[];
}
