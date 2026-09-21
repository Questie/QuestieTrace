// Questie Classic Era DB field schema.
//
// Mirrors the `*Keys` tables from the real Questie Classic DB files vendored at
// `tools/classic-data/{classicNpcDB,classicQuestDB,classicItemDB,classicObjectDB}.lua`
// (field name, 1-based positional index, and type comment - copied verbatim).
//
// `default` values are NOT hand-authored. They were mechanically derived by sampling
// every row of the real `*Data` tables in those files and measuring how often each
// field is `nil`:
//   - a field that is *never* `nil` in the real data gets a type-appropriate non-nil
//     default (`""` for string, `0` for int/bitmask), since Questie code presumably
//     treats that field as always-present.
//   - a field that is *ever* `nil` in the real data (regardless of type) defaults to
//     `nil`, matching Questie's own convention of using `nil` for "no data" on
//     optional fields.
//
// See `tools/trace-analyzer/README.md` (extract section) for how to re-derive these
// if the upstream Questie DB shape changes.

export type QuestieFieldType = "string" | "int" | "bitmask" | "table";

export interface QuestieFieldSchema {
  /** 1-based position in the Questie positional data row. */
  index: number;
  type: QuestieFieldType;
  /** Value written by the Lua writer when no observation exists for this field. */
  default: unknown;
  /** Verbatim (trimmed) comment from the real `*Keys` table. */
  comment: string;
}

export type QuestieEntitySchema = Record<string, QuestieFieldSchema>;

export const npcKeys: QuestieEntitySchema = {
  name: { index: 1, type: "string", default: "", comment: "string" },
  minLevelHealth: { index: 2, type: "int", default: 0, comment: "int" },
  maxLevelHealth: { index: 3, type: "int", default: 0, comment: "int" },
  minLevel: { index: 4, type: "int", default: 0, comment: "int" },
  maxLevel: { index: 5, type: "int", default: 0, comment: "int" },
  rank: {
    index: 6,
    type: "int",
    default: 0,
    comment: "int, see https://github.com/cmangos/issues/wiki/creature_template#rank",
  },
  spawns: {
    index: 7,
    type: "table",
    default: null,
    comment: "table {[zoneID(int)] = {coordPair(floatVector2D),...},...}",
  },
  waypoints: {
    index: 8,
    type: "table",
    default: null,
    comment: "table {[zoneID(int)] = {coordPair(floatVector2D),...},...}",
  },
  zoneID: { index: 9, type: "int", default: 0, comment: "guess as to where this NPC is most common" },
  questStarts: { index: 10, type: "table", default: null, comment: "table {questID(int),...}" },
  questEnds: { index: 11, type: "table", default: null, comment: "table {questID(int),...}" },
  factionID: {
    index: 12,
    type: "int",
    default: 0,
    comment: "int, see https://github.com/cmangos/issues/wiki/FactionTemplate.dbc",
  },
  friendlyToFaction: {
    index: 13,
    type: "string",
    default: null,
    comment:
      'string, Contains "A" and/or "H" depending on NPC being friendly towards those factions. nil if hostile to both.',
  },
  subName: {
    index: 14,
    type: "string",
    default: null,
    comment: 'string, The title or function of the NPC, e.g. "Weapon Vendor"',
  },
  npcFlags: {
    index: 15,
    type: "bitmask",
    default: 0,
    comment:
      "int, Bitmask containing various flags about the NPCs function (Vendor, Trainer, Flight Master, etc.).",
  },
};

export const questKeys: QuestieEntitySchema = {
  name: { index: 1, type: "string", default: "", comment: "string" },
  startedBy: { index: 2, type: "table", default: null, comment: "table {creatureStart, objectStart, itemStart}" },
  finishedBy: { index: 3, type: "table", default: null, comment: "table {creatureEnd, objectEnd}" },
  requiredLevel: { index: 4, type: "int", default: 0, comment: "int" },
  questLevel: { index: 5, type: "int", default: 0, comment: "int" },
  requiredRaces: { index: 6, type: "bitmask", default: 0, comment: "bitmask" },
  requiredClasses: { index: 7, type: "bitmask", default: null, comment: "bitmask" },
  objectivesText: {
    index: 8,
    type: "table",
    default: null,
    comment: "table: {string,...}, Description of the quest. Auto-complete if nil.",
  },
  triggerEnd: {
    index: 9,
    type: "table",
    default: null,
    comment: "table: {text, {[zoneID] = {coordPair,...},...}}",
  },
  objectives: {
    index: 10,
    type: "table",
    default: null,
    comment:
      "table {creatureObjective, objectObjective, itemObjective, reputationObjective, killCreditObjective, spellObjective}",
  },
  sourceItemId: { index: 11, type: "int", default: null, comment: "int, item provided by quest starter" },
  preQuestGroup: { index: 12, type: "table", default: null, comment: "table: {quest(int)}" },
  preQuestSingle: { index: 13, type: "table", default: null, comment: "table: {quest(int)}" },
  childQuests: { index: 14, type: "table", default: null, comment: "table: {quest(int)}" },
  inGroupWith: { index: 15, type: "table", default: null, comment: "table: {quest(int)}" },
  exclusiveTo: { index: 16, type: "table", default: null, comment: "table: {quest(int)}" },
  zoneOrSort: {
    index: 17,
    type: "int",
    default: 0,
    comment: "int, >0: AreaTable.dbc ID; <0: QuestSort.dbc ID",
  },
  requiredSkill: { index: 18, type: "table", default: null, comment: "table: {skill(int), value(int)}" },
  requiredMinRep: { index: 19, type: "table", default: null, comment: "table: {faction(int), value(int)}" },
  requiredMaxRep: { index: 20, type: "table", default: null, comment: "table: {faction(int), value(int)}" },
  requiredSourceItems: {
    index: 21,
    type: "table",
    default: null,
    comment: "table: {item(int), ...} Items that are not an objective but still needed for the quest.",
  },
  nextQuestInChain: {
    index: 22,
    type: "int",
    default: null,
    comment: "int: if this quest is active/finished, the current quest is not available anymore",
  },
  questFlags: {
    index: 23,
    type: "bitmask",
    default: null,
    comment: "bitmask: see https://github.com/cmangos/issues/wiki/Quest_template#questflags",
  },
  specialFlags: {
    index: 24,
    type: "bitmask",
    default: null,
    comment:
      "bitmask: 1 = Repeatable, 2 = Needs event, 4 = Monthly reset (req. 1). See https://github.com/cmangos/issues/wiki/Quest_template#specialflags",
  },
  parentQuest: {
    index: 25,
    type: "int",
    default: null,
    comment:
      "int, the ID of the parent quest that needs to be active for the current one to be available. See also 'childQuests' (field 14)",
  },
  reputationReward: {
    index: 26,
    type: "table",
    default: null,
    comment: "table: {{faction(int), value(int)},...}, a list of reputation rewarded upon quest completion",
  },
  breadcrumbForQuestId: {
    index: 27,
    type: "int",
    default: null,
    comment: "int: quest ID for the quest this optional breadcrumb quest leads to",
  },
  breadcrumbs: {
    index: 28,
    type: "table",
    default: null,
    comment: "table: {questID(int), ...} quest IDs of the breadcrumbs that lead to this quest",
  },
  extraObjectives: {
    index: 29,
    type: "table",
    default: null,
    comment:
      "table: {{spawnlist, iconFile, text, objectiveIndex (optional), {{dbReferenceType, id}, ...} (optional)},...}, a list of hidden special objectives for a quest. Similar to requiredSourceItems",
  },
  requiredSpell: {
    index: 30,
    type: "int",
    default: null,
    comment: "int: quest is only available if character has this spellID",
  },
  requiredSpecialization: {
    index: 31,
    type: "int",
    default: null,
    comment:
      "int: quest is only available if character meets the spec requirements. Use QuestieProfessions.specializationKeys for having a spec, or QuestieProfessions.professionKeys to indicate having the profession with no spec.",
  },
  requiredMaxLevel: {
    index: 32,
    type: "int",
    default: null,
    comment: "int: the maximum level at which the quest is still available",
  },
  availableUntilCompleted: {
    index: 33,
    type: "int",
    default: null,
    comment: "int: the current quest is available until this quest is turned in",
  },
  availableStartingWith: {
    index: 34,
    type: "int",
    default: null,
    comment:
      "int: the ID of the quest that needs to be in quest log OR turned in for the current one to be available.",
  },
  requiredRanks: {
    index: 35,
    type: "table",
    default: null,
    comment: "table: {{skill(int), value(int)}}. Table of professions and ranks to be checked with OR logic",
  },
  disabledByQuest: {
    index: 36,
    type: "int",
    default: null,
    comment: "int: quest that, if in player's quest log, makes current quest unavailable for the duration",
  },
};

export const itemKeys: QuestieEntitySchema = {
  name: { index: 1, type: "string", default: "", comment: "string" },
  npcDrops: { index: 2, type: "table", default: null, comment: "table or nil, NPC IDs" },
  objectDrops: { index: 3, type: "table", default: null, comment: "table or nil, object IDs" },
  itemDrops: { index: 4, type: "table", default: null, comment: "table or nil, item IDs" },
  startQuest: {
    index: 5,
    type: "int",
    default: null,
    comment: "int or nil, ID of the quest started by this item",
  },
  questRewards: { index: 6, type: "table", default: null, comment: "table or nil, quest IDs" },
  flags: {
    index: 7,
    type: "int",
    default: null,
    comment: "int or nil, see: https://github.com/cmangos/issues/wiki/Item_template#flags",
  },
  foodType: {
    index: 8,
    type: "int",
    default: null,
    comment: "int or nil, see https://github.com/cmangos/issues/wiki/Item_template#foodtype",
  },
  itemLevel: { index: 9, type: "int", default: 0, comment: "int, the level of this item" },
  requiredLevel: {
    index: 10,
    type: "int",
    default: 0,
    comment: "int, the level required to equip/use this item",
  },
  ammoType: { index: 11, type: "int", default: 0, comment: "int," },
  class: { index: 12, type: "int", default: 0, comment: "int," },
  subClass: { index: 13, type: "int", default: 0, comment: "int," },
  vendors: { index: 14, type: "table", default: null, comment: "table or nil, NPC IDs" },
  relatedQuests: {
    index: 15,
    type: "table",
    default: null,
    comment: "table or nil, IDs of quests that are related to this item",
  },
};

export const objectKeys: QuestieEntitySchema = {
  name: { index: 1, type: "string", default: "", comment: "string" },
  questStarts: { index: 2, type: "table", default: null, comment: "table {questID(int),...}" },
  questEnds: { index: 3, type: "table", default: null, comment: "table {questID(int),...}" },
  spawns: {
    index: 4,
    type: "table",
    default: null,
    comment: "table {[zoneID(int)] = {coordPair(floatVector2D),...},...}",
  },
  zoneID: { index: 5, type: "int", default: 0, comment: "guess as to where this object is most common" },
  factionID: {
    index: 6,
    type: "int",
    default: null,
    comment: "faction restriction mask (same as spawndb factionid)",
  },
  waypoints: {
    index: 7,
    type: "table",
    default: null,
    comment: "waypoints for objects on ships/zeppelins/etc",
  },
};
