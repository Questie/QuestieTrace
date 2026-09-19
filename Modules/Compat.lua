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
    if C_QuestLog.GetQuestTagInfo then
      local tagInfo = C_QuestLog.GetQuestTagInfo(info.questID)
      questTag = tagInfo and tagInfo.tagName
    end

    local isComplete
    if C_QuestLog.IsComplete and C_QuestLog.IsComplete(info.questID) then
      isComplete = 1
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
