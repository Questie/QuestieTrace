---@type QuestieTraceCore
local Core = QuestieTraceCore
---@type fun(t1: any, t2: any, ignore_mt: boolean?, visited: table?): boolean
local DeepCompare = Core.DeepCompare
---@type fun(fmt: string, ...): string
local fmt = string.format

local C_After = C_Timer.After

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- C_QuestLog.GetQuestsOnMap(uiMapID) -> QuestPOIMapInfo[]
--   QuestPOIMapInfo:
--     childDepth        number?
--     questTagType      Enum.QuestTagType?
--     questID           number
--     numObjectives     number
--     mapID             number
--     x                 number
--     y                 number
--     isQuestStart      boolean
--     isDaily           boolean
--     isCombatAllyQuest boolean
--     isMeta            boolean
--     inProgress        boolean
--     isMapIndicatorQuest boolean
--
-- Streams:
--   C_QuestLog.GetQuestsOnMap[uiMapID] -> { t, tp, v = QuestPOIMapInfo[] }
---------------------------------------------------------------------------

---@type number[]
local SAMPLE_DELAYS = { 0, 0.10, 0.35, 0.55, 0.75, 1.00 }

---@type table<string, FunctionStream>
local functions   -- capture.session.functions

---------------------------------------------------------------------------
-- API helpers
---------------------------------------------------------------------------

---Sample POI data for a specific map ID from C_QuestLog.GetQuestsOnMap.
---Collects POIs and discovers additional mapIDs from their mapID field.
---@param capture CaptureState
---@param t number
---@param tp number
---@param uiMapID number
---@param processedMaps table<number, boolean> Maps already processed this cycle
local function SampleQuestLogPOIs(capture, t, tp, uiMapID, processedMaps)
  if not (C_QuestLog and C_QuestLog.GetQuestsOnMap) then return end
  if processedMaps[uiMapID] then return end
  processedMaps[uiMapID] = true

  local ok, pois = pcall(C_QuestLog.GetQuestsOnMap, uiMapID)
  if not ok or type(pois) ~= "table" then
    return
  end
  local stream = functions["C_QuestLog.GetQuestsOnMap"]
  if not stream then
    stream = {}
    functions["C_QuestLog.GetQuestsOnMap"] = stream
  end

  ---@cast stream table<number, FunctionStreamEntry[]>
  if not stream[uiMapID] then
    stream[uiMapID] = {}
  end
  local mapStream = stream[uiMapID]

  local prev = mapStream[#mapStream]
  local changed
  if not prev then
    changed = true
  elseif type(prev.v) ~= "table" or type(pois) ~= "table" then
    changed = true
  else
    changed = not DeepCompare(pois, prev.v)
  end

  if changed then
    mapStream[#mapStream + 1] = { t = t, tp = tp, v = pois }
    Core.Debug(fmt("[QuestPOI] Recorded change for mapID %d", uiMapID))
  end

  -- Discover additional maps from POI mapID fields
  for i = 1, #pois do
    local poi = pois[i]
    local poiMapID = poi.mapID
    if poiMapID and poiMapID > 0 and poiMapID ~= uiMapID then
      SampleQuestLogPOIs(capture, t, tp, poiMapID, processedMaps)
    end
  end
end

---Sample POIs starting from the current POI map and following mapID references.
---@param capture CaptureState
local function SampleAllPOIs(capture)
  if not (C_QuestLog and C_QuestLog.GetMapForQuestPOIs) then
    return
  end

  ---@type number
  local t  = GetTime()          - capture.startedAt
  ---@type number
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  -- 1. Get the current POI map
  local ok, startMapID = pcall(C_QuestLog.GetMapForQuestPOIs)
  if not ok or not startMapID or startMapID <= 0 then
    return
  end

  -- 2. Sample starting map and recursively follow mapID references
  local processedMaps = {}
  SampleQuestLogPOIs(capture, t, tp, startMapID, processedMaps)
end

---Schedule staggered re-samples to catch server lag.
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
        SampleAllPOIs(capture)
      end)
    end
  end
end

---------------------------------------------------------------------------
-- Tracker registration
---------------------------------------------------------------------------

Core.RegisterTracker({
  events = {
    "QUEST_POI_UPDATE",
    "QUEST_LOG_UPDATE",
    "PLAYER_ENTERING_WORLD",
    "ZONE_CHANGED_NEW_AREA",
  },

  ---@param capture CaptureState
  Init = function(capture)
    functions = capture.session.functions
    functions["C_QuestLog.GetQuestsOnMap"] = {}

    -- Immediate sample at t=0
    SampleAllPOIs(capture)

    -- Schedule delayed re-samples for initial capture
    ScheduleDelayedSamples(capture)
  end,

  ---@param capture CaptureState
  ---@param _event string
  OnEvent = function(capture, _event)
    -- Immediate sample in this callstack (the 0-delay entry)
    SampleAllPOIs(capture)

    -- Schedule staggered re-samples
    ScheduleDelayedSamples(capture)
  end,
})
