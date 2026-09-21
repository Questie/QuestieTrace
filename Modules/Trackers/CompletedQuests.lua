---@type QuestieTraceCore
local Core = QuestieTraceCore

local C_After = C_Timer.After

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- GetQuestsCompleted(table?) -> table questsCompleted  -- keyed by questID -> true
--                                (from either C_QuestLog.GetAllCompletedQuestIDs()
--                                or the legacy GetQuestsCompleted() global,
--                                whichever the client exposes)
--
-- Stored as DeltaStream: initial set + add/remove deltas over time. This is
-- a derived synthetic stream (a completed-quest set), not a raw single-API
-- return value.
---------------------------------------------------------------------------

---@type number[]
local SAMPLE_DELAYS = { 0, 0.10, 0.35, 0.55, 0.75, 1.00 }

---@type table<string, DeltaStream>
local functionsDelta  -- capture.session.functionsDelta
---@type table<number, boolean>
local currentSet      -- [questId] = true -- current known completed set
---@type table<number, boolean>
local completedQuestScratch = {}

---------------------------------------------------------------------------
-- API helpers (logic preserved from existing implementation)
---------------------------------------------------------------------------

--- Get all completed quest IDs sorted in ascending order.
---@return number[] ids Sorted array of completed quest IDs
local function GetCompletedQuestIds()
  ---@type number[]
  local ids = {}

  ---@type table<number, boolean>?
  local completed

  if C_QuestLog and C_QuestLog.GetAllCompletedQuestIDs then
    wipe(completedQuestScratch)
    completed = completedQuestScratch
    for _, questID in ipairs(C_QuestLog.GetAllCompletedQuestIDs()) do
      completed[questID] = true
    end
  elseif GetQuestsCompleted then
    completed = GetQuestsCompleted(wipe(completedQuestScratch))
  end

  if type(completed) ~= "table" then return ids end

  for questId, isCompleted in pairs(completed) do
    if isCompleted == true then
      ids[#ids + 1] = questId
    end
  end
  table.sort(ids)
  return ids
end

---------------------------------------------------------------------------
-- Sampling
---------------------------------------------------------------------------

--- Sample completed quests and record delta changes.
---@param capture CaptureState
local function SampleCompleted(capture)
  ---@type number[]
  local ids = GetCompletedQuestIds()

  -- Build new set
  ---@type table<number, boolean>
  local newSet = {}
  for i = 1, #ids do newSet[ids[i]] = true end

  -- Compute diff
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

    ---@type number
    local t  = GetTime()          - capture.startedAt
    ---@type number
    local tp = GetTimePreciseSec() - capture.startedAtPrecise

    ---@type DeltaStreamEntry
    local delta = { t = t, tp = tp }
    if #added > 0 then delta.add = added end
    if #removed > 0 then delta.remove = removed end

    ---@type DeltaStream
    local stream = functionsDelta["GetQuestsCompleted"]
    stream.delta[#stream.delta + 1] = delta
  end

  currentSet = newSet
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

    -- Capture initial set
    ---@type number[]
    local ids = GetCompletedQuestIds()
    currentSet = {}
    for i = 1, #ids do currentSet[ids[i]] = true end

    functionsDelta["GetQuestsCompleted"] = {
      t = 0,
      tp = 0,
      initial = ids,
      delta = {},
    }

    -- Schedule delayed re-samples for initial capture
    ScheduleDelayedSamples(capture)
  end,

  ---@param capture CaptureState
  OnEvent = function(capture)
    SampleCompleted(capture)
    ScheduleDelayedSamples(capture)
  end,
})
