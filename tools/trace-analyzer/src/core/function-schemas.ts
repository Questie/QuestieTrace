// ============================================================
// Static return-value schemas for tracked WoW API functions.
//
// Maps function name → ordered list of return field names.
// Used by the UI to render labeled values instead of raw tuples.
//
// Source: Documentation/WoW-API + warcraft.wiki.gg
// Only functions tracked by QuestieTrace are included.
// ============================================================

export interface ReturnField {
  /** Display name for this return value position */
  name: string;
  /** Lua type hint (for future formatting) */
  type: "string" | "number" | "boolean" | "table" | "unknown";
}

/**
 * Lookup a function's return schema.
 * Returns undefined for single-return or custom-stream functions
 * (where labeling adds no value).
 */
export function getSchema(functionName: string): ReturnField[] | undefined {
  return FUNCTION_SCHEMAS[functionName];
}

// ---------------------------------------------------------------------------
// Schemas — only multi-return functions need entries here.
// Single-return functions (GetZoneText, UnitLevel, etc.) are omitted
// because their value is self-explanatory.
//
// Schemas are keyed by function name, not by parameter path. Nested streams such
// as GetQuestLogRewardInfo[rewardIndex][questId] still share one return tuple
// shape for every leaf stream.
// ---------------------------------------------------------------------------

const FUNCTION_SCHEMAS: Record<string, ReturnField[]> = {
  // -- Loot.lua ---------------------------------------------------------------

  "GetLootSlotInfo": [
    { name: "lootIcon", type: "string" },
    { name: "lootName", type: "string" },
    { name: "lootQuantity", type: "number" },
    { name: "currencyID", type: "number" },
    { name: "lootQuality", type: "number" },
    { name: "locked", type: "boolean" },
    { name: "isQuestItem", type: "boolean" },
    { name: "questID", type: "number" },
    { name: "isActive", type: "boolean" },
  ],

  "GetLootSourceInfo": [
    { name: "guid", type: "string" },
    { name: "quantity", type: "number" },
  ],

  // -- QuestDialog.lua --------------------------------------------------------

  "GetActiveTitle": [
    { name: "title", type: "string" },
    { name: "isComplete", type: "boolean" },
  ],

  // -- QuestLog.lua -----------------------------------------------------------
  //
  // Quest-log title/tag/completion data is split across several independent
  // raw streams; there is no synthesized composite value. This schema only
  // labels the legacy `GetQuestLogTitle` global's raw tuple (only present in
  // captures from clients that expose that global). Modern clients instead
  // populate `C_QuestLog.GetInfo`/`C_QuestLog.GetQuestTagInfo`/
  // `C_QuestLog.IsComplete`/`C_QuestLog.IsFailed`, which are plain tables/
  // scalars rendered without a tuple schema.

  "GetQuestLogTitle": [
    { name: "title", type: "string" },
    { name: "level", type: "number" },
    { name: "suggestedGroup", type: "number" },
    { name: "isHeader", type: "boolean" },
    { name: "isCollapsed", type: "boolean" },
    { name: "isComplete", type: "number" },       // 1=done, -1=failed, nil=in progress
    { name: "frequency", type: "number" },         // 1=normal, 2=daily, 3=weekly
    { name: "questID", type: "number" },
    { name: "startEvent", type: "boolean" },
    { name: "displayQuestID", type: "boolean" },
    { name: "isOnMap", type: "boolean" },
    { name: "hasLocalPOI", type: "boolean" },
    { name: "isTask", type: "boolean" },
    { name: "isBounty", type: "boolean" },
    { name: "isStory", type: "boolean" },
    { name: "isHidden", type: "boolean" },
    { name: "isScaling", type: "boolean" },
  ],

  "GetQuestLogQuestText": [
    { name: "questDescription", type: "string" },
    { name: "questObjectives", type: "string" },
  ],

  "GetQuestTagInfo": [
    { name: "tagID", type: "number" },
    { name: "tagName", type: "string" },
    { name: "worldQuestType", type: "number" },
    { name: "rarity", type: "number" },
    { name: "isElite", type: "boolean" },
    { name: "tradeskillLineIndex", type: "number" },
    { name: "displayTimeLeft", type: "unknown" },
  ],

  "GetQuestLogRewardInfo": [
    { name: "itemName", type: "string" },
    { name: "itemTexture", type: "unknown" },
    { name: "numItems", type: "number" },
    { name: "quality", type: "number" },
    { name: "isUsable", type: "boolean" },
    { name: "itemID", type: "number" },
    { name: "itemLevel", type: "number" },
  ],

  // -- Reputation.lua ---------------------------------------------------------
  //
  // Only present when the legacy GetFactionInfoByID global exists on the
  // capturing client, independently of whether C_Reputation.GetFactionDataByID
  // also exists (a client exposing both records both streams).
  // C_Reputation.GetFactionDataByID stores a plain table, rendered without a
  // tuple schema.

  "GetFactionInfoByID": [
    { name: "name", type: "string" },
    { name: "description", type: "string" },
    { name: "standingID", type: "number" },        // 4=Neutral, 5=Friendly, 6=Honored...
    { name: "barMin", type: "number" },
    { name: "barMax", type: "number" },
    { name: "barValue", type: "number" },
    { name: "atWarWith", type: "boolean" },
    { name: "canToggleAtWar", type: "boolean" },
    { name: "isHeader", type: "boolean" },
    { name: "isCollapsed", type: "boolean" },
    { name: "hasRep", type: "boolean" },
    { name: "isWatched", type: "boolean" },
    { name: "isChild", type: "boolean" },
    { name: "factionID", type: "number" },
    { name: "hasBonusRepGain", type: "boolean" },
    { name: "canSetInactive", type: "boolean" },
  ],

  // -- SkillLines.lua --------------------------------------------------------
  //
  // Only present when the legacy GetNumSkillLines/GetSkillLineInfo globals
  // exist; there is no synthesized fallback derived from professions or
  // C_TradeSkillUI on clients that lack them. C_TradeSkillUI.* streams are
  // tracked independently and store plain tables, rendered without a tuple
  // schema.

  "GetSkillLineInfo": [
    { name: "skillName", type: "string" },
    { name: "header", type: "number" },
    { name: "isExpanded", type: "number" },
    { name: "skillRank", type: "number" },
    { name: "numTempPoints", type: "number" },
    { name: "skillModifier", type: "number" },
    { name: "skillMaxRank", type: "number" },
    { name: "isAbandonable", type: "number" },
    { name: "stepCost", type: "number" },
    { name: "rankCost", type: "number" },
    { name: "minLevel", type: "number" },
    { name: "skillCostType", type: "number" },
    { name: "skillDescription", type: "string" },
  ],

  "GetProfessions": [
    { name: "prof1", type: "number" },
    { name: "prof2", type: "number" },
    { name: "archaeology", type: "number" },
    { name: "fishing", type: "number" },
    { name: "cooking", type: "number" },
  ],

  "GetProfessionInfo": [
    { name: "name", type: "string" },
    { name: "icon", type: "string" },
    { name: "skillLevel", type: "number" },
    { name: "maxSkillLevel", type: "number" },
    { name: "numAbilities", type: "number" },
    { name: "spelloffset", type: "number" },
    { name: "skillLine", type: "number" },
    { name: "skillModifier", type: "number" },
    { name: "specializationIndex", type: "number" },
    { name: "specializationOffset", type: "number" },
  ],

  // -- Position.lua --------------------------------------------------------------

  "IsInInstance": [
    { name: "inInstance", type: "boolean" },
    { name: "instanceType", type: "string" },    // "none"|"party"|"raid"|"pvp"|"arena"
  ],

  "GetInstanceInfo": [
    { name: "name", type: "string" },
    { name: "instanceType", type: "string" },
    { name: "difficultyID", type: "number" },
    { name: "difficultyName", type: "string" },
    { name: "maxPlayers", type: "number" },
    { name: "dynamicDifficulty", type: "number" },
    { name: "isDynamic", type: "boolean" },
    { name: "instanceID", type: "number" },
    { name: "instanceGroupSize", type: "number" },
    { name: "LfgDungeonID", type: "number" },
  ],

  // -- PlayerIdentity.lua -----------------------------------------------------

  "UnitRace": [
    { name: "localizedRaceName", type: "string" },
    { name: "englishRaceName", type: "string" },
    { name: "raceID", type: "number" },
  ],

  "UnitClass": [
    { name: "className", type: "string" },
    { name: "classFilename", type: "string" },
    { name: "classID", type: "number" },
  ],

  "UnitClassBase": [
    { name: "classFilename", type: "string" },
    { name: "classID", type: "number" },
  ],

  "UnitFactionGroup": [
    { name: "englishFaction", type: "string" },   // "Alliance"|"Horde"|"Neutral"
    { name: "localizedFaction", type: "string" },
  ],

  // -- UnitInteraction.lua -----------------------------------------------------

  "UnitName": [
    { name: "name", type: "string" },
    { name: "realm", type: "string" },
  ],

  // -- SpellBook.lua ----------------------------------------------------------

  "GetSpellBookItemName": [
    { name: "spellName", type: "string" },
    { name: "spellSubName", type: "string" },
    { name: "spellID", type: "number" },
  ],

  "GetSpellBookItemInfo": [
    { name: "spellType", type: "string" },
    { name: "id", type: "number" },
  ],
};
