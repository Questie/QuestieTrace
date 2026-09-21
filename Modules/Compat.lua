QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- Compat: wrappers for WoW APIs that differ or are missing across clients
---------------------------------------------------------------------------

---@class Compat
local Compat = {}
Core.Compat = Compat

---@class QuestLogTitleInfo
---@field title string
---@field level number
---@field questTag string?
---@field isHeader boolean
---@field isCollapsed boolean
---@field isComplete number?
---@field frequency number
---@field questID number
---@field startEvent boolean
---@field displayQuestID number
---@field isOnMap boolean
---@field hasLocalPOI boolean
---@field isTask boolean
---@field isBounty boolean
---@field isStory boolean
---@field isHidden boolean
---@field isScaling boolean

--- Compatibility wrapper for GetQuestLogTitle. Some clients (e.g. "WoW
--- Forever") do not expose the global GetQuestLogTitle and only provide
--- C_QuestLog.GetInfo/GetQuestTagInfo/IsComplete instead. Returns a single
--- table (like the modern C_ APIs) instead of a long tuple of return values.
---@param questLogIndex number
---@return QuestLogTitleInfo? info
function Compat.GetQuestLogTitle(questLogIndex)
  if C_QuestLog and C_QuestLog.GetInfo then
    local info = C_QuestLog.GetInfo(questLogIndex)
    if not info then
      return nil
    end

    local questTag
    local isComplete

    -- Only call quest-specific APIs for non-header entries with a valid questID
    if not info.isHeader and info.questID and info.questID > 0 then
      if C_QuestLog.GetQuestTagInfo then
        local tagInfo = C_QuestLog.GetQuestTagInfo(info.questID)
        questTag = tagInfo and tagInfo.tagName
      end

      if C_QuestLog.IsComplete then
        if C_QuestLog.IsComplete(info.questID) then
          isComplete = 1
        elseif C_QuestLog.IsFailed and C_QuestLog.IsFailed(info.questID) then
          isComplete = -1
        end
      end
    end

    return {
      title = info.title,
      level = info.level,
      questTag = questTag,
      isHeader = info.isHeader,
      isCollapsed = info.isCollapsed,
      isComplete = isComplete,
      frequency = info.frequency,
      questID = info.questID,
      startEvent = info.startEvent,
      displayQuestID = info.questID,
      isOnMap = info.isOnMap,
      hasLocalPOI = info.hasLocalPOI,
      isTask = info.isTask,
      isBounty = info.isBounty,
      isStory = info.isStory,
      isHidden = info.isHidden,
      isScaling = info.isScaling,
    }
  elseif GetQuestLogTitle then
    local title, level, questTag, isHeader, isCollapsed, isComplete, frequency, questID,
    startEvent, displayQuestID, isOnMap, hasLocalPOI, isTask, isBounty, isStory, isHidden, isScaling =
        GetQuestLogTitle(questLogIndex)
    if not title then
      return nil
    end

    return {
      title = title,
      level = level,
      questTag = questTag,
      isHeader = isHeader,
      isCollapsed = isCollapsed,
      isComplete = isComplete,
      frequency = frequency,
      questID = questID,
      startEvent = startEvent,
      displayQuestID = displayQuestID,
      isOnMap = isOnMap,
      hasLocalPOI = hasLocalPOI,
      isTask = isTask,
      isBounty = isBounty,
      isStory = isStory,
      isHidden = isHidden,
      isScaling = isScaling,
    }
  end

  Core.Error("Compat.GetQuestLogTitle: no available API (C_QuestLog.GetInfo / GetQuestLogTitle)")
  return nil
end

--- Compatibility wrapper for GetQuestLogIndexByID. Some clients (e.g. "WoW
--- Forever") do not expose the global GetQuestLogIndexByID and only provide
--- C_QuestLog.GetLogIndexForQuestID instead.
---@param questID number
---@return number? questLogIndex
function Compat.GetQuestLogIndexByID(questID)
  if C_QuestLog and C_QuestLog.GetLogIndexForQuestID then
    return C_QuestLog.GetLogIndexForQuestID(questID)
  elseif GetQuestLogIndexByID then
    return GetQuestLogIndexByID(questID)
  end

  Core.Error("Compat.GetQuestLogIndexByID: no available API (C_QuestLog.GetLogIndexForQuestID / GetQuestLogIndexByID)")
  return nil
end

--- Compatibility wrapper for GetQuestsCompleted. Some clients (e.g. "WoW
--- Forever") do not expose the global GetQuestsCompleted and only provide
--- C_QuestLog.GetAllCompletedQuestIDs (a plain array) instead. Always returns
--- a [questID] = true map, converting the array form when needed.
---@param target table<number, boolean>? Table to fill and return instead of allocating a new one.
---@return table<number, boolean>? questsCompleted
function Compat.GetQuestsCompleted(target)
  if C_QuestLog and C_QuestLog.GetAllCompletedQuestIDs then
    local completed = target or {}
    for _, questID in ipairs(C_QuestLog.GetAllCompletedQuestIDs()) do
      completed[questID] = true
    end
    return completed
  elseif GetQuestsCompleted then
    return GetQuestsCompleted(target)
  end

  Core.Error("Compat.GetQuestsCompleted: no available API (C_QuestLog.GetAllCompletedQuestIDs / GetQuestsCompleted)")
  return nil
end

--- Compatibility wrapper for GetQuestResetTime. Some clients (e.g. "WoW
--- Forever") do not expose the global GetQuestResetTime and only provide
--- C_DateAndTime.GetSecondsUntilDailyReset instead.
---@return number? secondsUntilReset
function Compat.GetQuestResetTime()
  if C_DateAndTime and C_DateAndTime.GetSecondsUntilDailyReset then
    return C_DateAndTime.GetSecondsUntilDailyReset()
  elseif GetQuestResetTime then
    return GetQuestResetTime()
  end

  Core.Error("Compat.GetQuestResetTime: no available API (C_DateAndTime.GetSecondsUntilDailyReset / GetQuestResetTime)")
  return nil
end

--- Compatibility wrapper for GetNumFactions. Some clients (e.g. "WoW
--- Forever") do not expose the global GetNumFactions and only provide
--- C_Reputation.GetNumFactions instead.
---@return number? numFactions
function Compat.GetNumFactions()
  if C_Reputation and C_Reputation.GetNumFactions then
    return C_Reputation.GetNumFactions()
  elseif GetNumFactions then
    return GetNumFactions()
  end

  Core.Error("Compat.GetNumFactions: no available API (C_Reputation.GetNumFactions / GetNumFactions)")
  return nil
end

--- Compatibility wrapper for GetFactionInfo. Some clients (e.g. "WoW
--- Forever") do not expose the global GetFactionInfo and only provide
--- C_Reputation.GetFactionDataByIndex (a table) instead. Returns the same
--- legacy tuple shape as the global so existing callers keep working
--- unchanged.
---@param index number
---@return string? name
---@return string? description
---@return number? reaction
---@return number? currentReactionThreshold
---@return number? nextReactionThreshold
---@return number? currentStanding
---@return boolean? atWarWith
---@return boolean? canToggleAtWar
---@return boolean? isHeader
---@return boolean? isCollapsed
---@return boolean? isHeaderWithRep
---@return boolean? isWatched
---@return boolean? isChild
---@return number? factionID
---@return boolean? hasBonusRepGain
---@return boolean? canSetInactive
function Compat.GetFactionInfo(index)
  if C_Reputation and C_Reputation.GetFactionDataByIndex then
    local d = C_Reputation.GetFactionDataByIndex(index)
    if not d then return nil end
    return d.name, d.description, d.reaction, d.currentReactionThreshold,
        d.nextReactionThreshold, d.currentStanding, d.atWarWith,
        d.canToggleAtWar, d.isHeader, d.isCollapsed, d.isHeaderWithRep,
        d.isWatched, d.isChild, d.factionID, d.hasBonusRepGain,
        d.canSetInactive
  elseif GetFactionInfo then
    return GetFactionInfo(index)
  end

  Core.Error("Compat.GetFactionInfo: no available API (C_Reputation.GetFactionDataByIndex / GetFactionInfo)")
  return nil
end

--- Compatibility wrapper for GetFactionInfoByID. Some clients (e.g. "WoW
--- Forever") do not expose the global GetFactionInfoByID and only provide
--- C_Reputation.GetFactionDataByID (a table) instead. Returns the same
--- legacy tuple shape as the global so existing callers keep working
--- unchanged.
---@param factionID number
---@return string? name
---@return string? description
---@return number? reaction
---@return number? currentReactionThreshold
---@return number? nextReactionThreshold
---@return number? currentStanding
---@return boolean? atWarWith
---@return boolean? canToggleAtWar
---@return boolean? isHeader
---@return boolean? isCollapsed
---@return boolean? isHeaderWithRep
---@return boolean? isWatched
---@return boolean? isChild
---@return number? factionID
---@return boolean? hasBonusRepGain
---@return boolean? canSetInactive
function Compat.GetFactionInfoByID(factionID)
  if C_Reputation and C_Reputation.GetFactionDataByID then
    local d = C_Reputation.GetFactionDataByID(factionID)
    if not d then return nil end
    return d.name, d.description, d.reaction, d.currentReactionThreshold,
        d.nextReactionThreshold, d.currentStanding, d.atWarWith,
        d.canToggleAtWar, d.isHeader, d.isCollapsed, d.isHeaderWithRep,
        d.isWatched, d.isChild, d.factionID, d.hasBonusRepGain,
        d.canSetInactive
  elseif GetFactionInfoByID then
    return GetFactionInfoByID(factionID)
  end

  Core.Error("Compat.GetFactionInfoByID: no available API (C_Reputation.GetFactionDataByID / GetFactionInfoByID)")
  return nil
end

-- The indexed skill-line API is gone on modern clients. GetNumSkillLines /
-- GetSkillLineInfo are used purely to learn which professions the player has
-- and at what rank, so rebuild that list from whichever modern source this
-- client provides and present it in the old shape.
local skillLines = {}

local function collectSkillLines()
  wipe(skillLines)

  if GetProfessions and GetProfessionInfo then
    local prof1, prof2, archaeology, fishing, cooking = GetProfessions()
    for _, index in ipairs({ prof1 or false, prof2 or false, archaeology or false,
                              fishing or false, cooking or false }) do
      if index then
        local name, _, rank = GetProfessionInfo(index)
        if name then
          skillLines[#skillLines + 1] = { name = name, rank = rank or 0 }
        end
      end
    end
  end

  if #skillLines == 0 and C_TradeSkillUI and C_TradeSkillUI.GetAllProfessionTradeSkillLines
      and C_TradeSkillUI.GetTradeSkillLineInfoByID then
    for _, skillLineID in ipairs(C_TradeSkillUI.GetAllProfessionTradeSkillLines()) do
      local info = C_TradeSkillUI.GetTradeSkillLineInfoByID(skillLineID)
      local name = info and (info.professionName or info.displayName)
      if name then
        skillLines[#skillLines + 1] = { name = name, rank = info.skillLevel or 0 }
      end
    end
  end

  return #skillLines
end

--- Compatibility wrapper for GetNumSkillLines. Some clients (e.g. "WoW
--- Forever") do not expose the global GetNumSkillLines and only provide
--- GetProfessions/GetProfessionInfo or C_TradeSkillUI instead. Rebuilds the
--- profession list from whichever of those is available and reports its size.
---@return number numSkillLines
function Compat.GetNumSkillLines()
  if GetNumSkillLines then
    return GetNumSkillLines()
  end
  return collectSkillLines()
end

--- Compatibility wrapper for GetSkillLineInfo. Some clients (e.g. "WoW
--- Forever") do not expose the global GetSkillLineInfo. Reads from the
--- profession list rebuilt by Compat.GetNumSkillLines and returns the
--- (name, isHeader, isExpanded, rank) subset the legacy global provided.
---@param index number
---@return string? skillName
---@return boolean? isHeader
---@return boolean? isExpanded
---@return number? skillRank
function Compat.GetSkillLineInfo(index)
  if GetSkillLineInfo then
    return GetSkillLineInfo(index)
  end
  local line = skillLines[index]
  if not line then return nil end
  return line.name, false, false, line.rank
end
