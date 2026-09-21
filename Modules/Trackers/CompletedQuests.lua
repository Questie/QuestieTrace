---@type QuestieTraceCore
local Core = QuestieTraceCore

local C_After = C_Timer.After

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- C_QuestLog.GetAllCompletedQuestIDs() -> number[] questIDs (raw array)
-- GetQuestsCompleted(table?) -> table questsCompleted  -- keyed by questID -> true
--                                (legacy global)
--
-- Each API is tracked as its own independent DeltaStream (initial set +
-- add/remove deltas over time), only when that real API actually exists.
-- There is no merged/normalized "whichever API is available" value: a
-- client exposing both would get both streams recorded independently.
---------------------------------------------------------------------------

---@type number[]
local SAMPLE_DELAYS = { 0, 0.10, 0.35, 0.55, 0.75, 1.00 }

---@type table<string, DeltaStream>
local functionsDelta  -- capture.session.functionsDelta
---@type table<number, boolean>
local currentModernSet -- [questId] = true -- last C_QuestLog.GetAllCompletedQuestIDs set
---@type table<number, boolean>
local currentLegacySet -- [questId] = true -- last legacy GetQuestsCompleted set
---@type table<number, boolean>
local legacyCompletedScratch = {}

---------------------------------------------------------------------------
-- API helpers
---------------------------------------------------------------------------

--- Get completed quest IDs from C_QuestLog.GetAllCompletedQuestIDs, sorted.
---@return number[]? ids Sorted array of completed quest IDs, or nil if the API doesn't exist
local function GetModernCompletedQuestIds()
  if not (C_QuestLog and C_QuestLog.GetAllCompletedQuestIDs) then return nil end

  ---@type number[]
  local ids = {}
  for _, questID in ipairs(C_QuestLog.GetAllCompletedQuestIDs()) do
    ids[#ids + 1] = questID
  end
  table.sort(ids)
  return ids
end

--- Get completed quest IDs from the legacy GetQuestsCompleted global, sorted.
---@return number[]? ids Sorted array of completed quest IDs, or nil if the API doesn't exist
local function GetLegacyCompletedQuestIds()
  if not GetQuestsCompleted then return nil end

  wipe(legacyCompletedScratch)
  ---@type table<number, boolean>?
  local completed = GetQuestsCompleted(legacyCompletedScratch)

  ---@type number[]
  local ids = {}
  if type(completed) == "table" then
    for questId, isCompleted in pairs(completed) do
      if isCompleted == true then
        ids[#ids + 1] = questId
      end
    end
  end
  table.sort(ids)
  return ids
end

---------------------------------------------------------------------------
-- Sampling
---------------------------------------------------------------------------

--- Compute add/remove deltas for one completed-quest source against its
--- previous set, and append a delta entry to its own independent stream.
---@param streamKey string
---@param currentSet table<number, boolean>
---@param ids number[]? Sorted completed quest IDs from this source, or nil if unavailable
---@param t number
---@param tp number
---@return table<number, boolean> newSet
local function SampleCompletedSource(streamKey, currentSet, ids, t, tp)
  if ids == nil then return currentSet end

  ---@type table<number, boolean>
  local newSet = {}
  for i = 1, #ids do newSet[ids[i]] = true end

  ---@type number[], number[]
  local added, removed = {}, {}
  for questId in pairs(newSet) do
    if not currentSet[questId] then
      added[#added + 1] = questId
    end
  end
  for questId in pairs(currentSet) do
    if not newSet[questId] then
      removed[#removed + 1] = questId
    end
  end

  if #added > 0 or #removed > 0 then
    table.sort(added)
    table.sort(removed)

    ---@type DeltaStreamEntry
    local delta = { t = t, tp = tp }
    if #added > 0 then delta.add = added end
    if #removed > 0 then delta.remove = removed end

    ---@type DeltaStream
    local stream = functionsDelta[streamKey]
    stream.delta[#stream.delta + 1] = delta
  end

  return newSet
end

--- Sample completed quests from every available source and record delta
--- changes independently for each.
---@param capture CaptureState
local function SampleCompleted(capture)
  ---@type number
  local t  = GetTime()          - capture.startedAt
  ---@type number
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  currentModernSet = SampleCompletedSource(
    "C_QuestLog.GetAllCompletedQuestIDs", currentModernSet, GetModernCompletedQuestIds(), t, tp
  )
  currentLegacySet = SampleCompletedSource(
    "GetQuestsCompleted", currentLegacySet, GetLegacyCompletedQuestIds(), t, tp
  )
end

--- Schedule staggered re-samples to catch server lag.
---@param capture CaptureState
local function ScheduleDelayedSamples(capture)
  ---@type number
  local token = capture.token
  for i = 1, #SAMPLE_DELAYS do
    ---@type number
    local delay = SAMPLE_DELAYS[i]
    if delay > 0 then
      C_After(delay, function()
        if not capture.active or capture.token ~= token then return end
        SampleCompleted(capture)
      end)
    end
  end
end

---------------------------------------------------------------------------
-- Tracker registration
---------------------------------------------------------------------------

Core.RegisterTracker({
  events = {
    "QUEST_LOG_UPDATE",
    "QUEST_ACCEPTED",
    "QUEST_REMOVED",
    "QUEST_TURNED_IN",
    "QUEST_AUTOCOMPLETE",
    "QUEST_WATCH_UPDATE",
    "UNIT_QUEST_LOG_CHANGED",
    "QUEST_POI_UPDATE",
    "QUEST_ITEM_UPDATE",
    "QUEST_LOG_CRITERIA_UPDATE",
    "QUEST_DATA_LOAD_RESULT",
    "QUEST_WATCH_LIST_CHANGED",
    "QUESTLINE_UPDATE",
    "TASK_PROGRESS_UPDATE",
    "PLAYER_ENTERING_WORLD",
    "SPELLS_CHANGED",
  },

  ---@param capture CaptureState
  Init = function(capture)
    functionsDelta = capture.session.functionsDelta
    currentModernSet = {}
    currentLegacySet = {}

    local modernIds = GetModernCompletedQuestIds()
    if modernIds then
      functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"] = { t = 0, tp = 0, initial = modernIds, delta = {} }
      for i = 1, #modernIds do currentModernSet[modernIds[i]] = true end
    end

    local legacyIds = GetLegacyCompletedQuestIds()
    if legacyIds then
      functionsDelta["GetQuestsCompleted"] = { t = 0, tp = 0, initial = legacyIds, delta = {} }
      for i = 1, #legacyIds do currentLegacySet[legacyIds[i]] = true end
    end

    -- Schedule delayed re-samples for initial capture
    ScheduleDelayedSamples(capture)
  end,

  ---@param capture CaptureState
  OnEvent = function(capture)
    SampleCompleted(capture)
    ScheduleDelayedSamples(capture)
  end,
})
