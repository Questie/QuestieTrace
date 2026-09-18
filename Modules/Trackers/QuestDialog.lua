---@type QuestieTraceCore
local Core = QuestieTraceCore
---@type fun(...): PackedArgs
local PackArgs = Core.PackArgs
---@type fun(t1: any, t2: any, ignore_mt: boolean?, visited: table?): boolean
local DeepCompare = Core.DeepCompare

local C_After = C_Timer.After

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- C_GossipInfo.GetOptions()         -> GossipOptionUIInfo[]
--
-- GetGossipAvailableQuests() -> repeated legacy 7-tuples:
--   title, questLevel, isTrivial, frequency, repeatable, isLegendary, isIgnored
-- GetGossipActiveQuests() -> repeated legacy 6-tuples:
--   title, questLevel, isTrivial, isComplete, isLegendary, isIgnored
--
-- GetActiveTitle(index)    -> title, isComplete
-- GetAvailableTitle(index) -> title
---------------------------------------------------------------------------

---@type number[]
local SAMPLE_DELAYS = { 0, 0.10, 0.35, 0.55, 0.75, 1.00 }

---@class QuestDialogStreamDef
---@field key string
---@field getter fun(): boolean, any

---@type table<string, FunctionStream>
local functions
---@type table<string, any>
local previousValues
---@type table<string, boolean>
local availableFlatStreams
---@type number
local maxActiveTitleCount = 0
---@type number
local maxAvailableTitleCount = 0
---@type number
local delayedSampleToken = 0 -- Invalidates pending delayed samples when dialog/gossip state changes.

---Return whether a nested table function exists.
---@param namespace table?
---@param functionName string
---@return boolean
local function HasMethod(namespace, functionName)
  return type(namespace) == "table" and type(namespace[functionName]) == "function"
end

---Call a function with pcall and return all results packed, without the pcall status.
---@param fn function?
---@return boolean ok
---@return PackedArgs? results
local function SafePackedCall(fn)
  if type(fn) ~= "function" then return false, nil end

  ---@type PackedArgs
  local packed = PackArgs(pcall(fn))
  if not packed[1] then return false, nil end

  ---@type PackedArgs
  local results = { n = packed.n - 1 }
  for i = 2, packed.n do
    results[i - 1] = packed[i]
  end
  return true, results
end

---Call a function with pcall and return its first result.
---@param fn function?
---@return boolean ok
---@return any value
local function SafeScalarCall(fn)
  local ok, results = SafePackedCall(fn)
  if not ok or not results then return false, nil end
  return true, results[1]
end

---Call an indexed function with pcall and return all results packed.
---@param fn function?
---@param index number
---@return boolean ok
---@return PackedArgs? results
local function SafePackedIndexCall(fn, index)
  if type(fn) ~= "function" then return false, nil end

  ---@type PackedArgs
  local packed = PackArgs(pcall(fn, index))
  if not packed[1] then return false, nil end

  ---@type PackedArgs
  local results = { n = packed.n - 1 }
  for i = 2, packed.n do
    results[i - 1] = packed[i]
  end
  return true, results
end

---Call an indexed function with pcall and return its first result.
---@param fn function?
---@param index number
---@return boolean ok
---@return any value
local function SafeScalarIndexCall(fn, index)
  local ok, results = SafePackedIndexCall(fn, index)
  if not ok or not results then return false, nil end
  return true, results[1]
end

---Append a value to a stream if it differs from the previous value.
---@param stream FunctionStreamEntry[]
---@param t number
---@param tp number
---@param key string
---@param value any
local function AppendIfChanged(stream, t, tp, key, value)
  local previous = previousValues[key]
  local changed

  if previous == nil and value == nil then
    changed = #stream == 0
  elseif previous == nil or value == nil then
    changed = true
  elseif type(value) == "table" then
    changed = not DeepCompare(value, previous)
  else
    changed = value ~= previous
  end

  if changed then
    stream[#stream + 1] = { t = t, tp = tp, v = value }
    previousValues[key] = value
  end
end

---Get or create a parameterized function stream.
---@param functionName string
---@param key number
---@return FunctionStreamEntry[]
local function GetOrCreateIndexStream(functionName, key)
  if not functions[functionName] then
    functions[functionName] = {}
  end

  local streams = functions[functionName]
  ---@cast streams table<number, FunctionStreamEntry[]>
  if not streams[key] then
    streams[key] = {}
  end
  return streams[key]
end

---Create a stream only when its getter exists in this client.
---Dialog/gossip APIs vary across Classic flavors; absent APIs intentionally do
---not create empty streams, so replay can distinguish unavailable from nil.
---@param key string
local function EnsureFlatStream(key)
  functions[key] = {}
  availableFlatStreams[key] = true
end

---Function keys whose returned string is free-form text that can embed a
---player's own name (e.g. NPC greetings like "Greetings, <name>"). These are
---run through Core.SanitizeText before being stored; see AGENTS.md/CLAUDE.md
---"Privacy" sections.
---@type table<string, boolean>
local SANITIZE_TEXT_FUNCTIONS = {
  ["C_GossipInfo.GetText"] = true,
  GetGreetingText = true,
  GetQuestText = true,
  GetObjectiveText = true,
  GetProgressText = true,
  GetRewardText = true,
}

---Sample one parameterless stream.
---@param t number
---@param tp number
---@param def QuestDialogStreamDef
local function SampleFlatStream(t, tp, def)
  if not availableFlatStreams[def.key] then return end

  local ok, value = def.getter()
  if not ok then return end

  if SANITIZE_TEXT_FUNCTIONS[def.key] then
    value = Core.SanitizeText(value)
  end

  local stream = functions[def.key]
  ---@cast stream FunctionStreamEntry[]
  AppendIfChanged(stream, t, tp, def.key, value)
end

---@return QuestDialogStreamDef[]
local function BuildFlatStreamDefs()
  ---@type QuestDialogStreamDef[]
  local defs = {}

  if HasMethod(C_GossipInfo, "GetNumAvailableQuests") then
    EnsureFlatStream("C_GossipInfo.GetNumAvailableQuests")
    defs[#defs + 1] = {
      key = "C_GossipInfo.GetNumAvailableQuests",
      getter = function() return SafeScalarCall(C_GossipInfo.GetNumAvailableQuests) end
    }
  end
  if HasMethod(C_GossipInfo, "GetNumActiveQuests") then
    EnsureFlatStream("C_GossipInfo.GetNumActiveQuests")
    defs[#defs + 1] = {
      key = "C_GossipInfo.GetNumActiveQuests",
      getter = function() return SafeScalarCall(C_GossipInfo.GetNumActiveQuests) end
    }
  end
  if HasMethod(C_GossipInfo, "GetText") then
    EnsureFlatStream("C_GossipInfo.GetText")
    defs[#defs + 1] = {
      key = "C_GossipInfo.GetText",
      getter = function() return SafeScalarCall(C_GossipInfo.GetText) end
    }
  end
  if HasMethod(C_GossipInfo, "GetOptions") then
    EnsureFlatStream("C_GossipInfo.GetOptions")
    defs[#defs + 1] = {
      key = "C_GossipInfo.GetOptions",
      getter = function() return SafeScalarCall(C_GossipInfo.GetOptions) end
    }
  end

  ---@type table<string, boolean>
  local scalarFunctions = {
    GetNumGossipAvailableQuests = true,
    GetNumGossipActiveQuests = true,
    GetGreetingText = true,
    GetNumActiveQuests = true,
    GetNumAvailableQuests = true,
    GetQuestID = true,
    GetTitleText = true,
    GetQuestText = true,
    GetObjectiveText = true,
    GetProgressText = true,
    GetRewardText = true,
    GetRewardXP = true,
    IsQuestCompletable = true,
    GetNumQuestChoices = true,
  }

  for functionName in pairs(scalarFunctions) do
    local fn = _G[functionName]
    if type(fn) == "function" then
      EnsureFlatStream(functionName)
      defs[#defs + 1] = { key = functionName, getter = function() return SafeScalarCall(fn) end }
    end
  end

  ---@type table<string, boolean>
  local packedFunctions = {
    GetGossipAvailableQuests = true,
    GetGossipActiveQuests = true,
  }

  for functionName in pairs(packedFunctions) do
    local fn = _G[functionName]
    if type(fn) == "function" then
      EnsureFlatStream(functionName)
      defs[#defs + 1] = { key = functionName, getter = function() return SafePackedCall(fn) end }
    end
  end

  return defs
end

---@type QuestDialogStreamDef[]
local flatStreamDefs

---Return whether an event closes transient dialog/gossip state.
---@param event string
---@return boolean
local function IsCloseEvent(event)
  return event == "GOSSIP_CLOSED" or event == "QUEST_FINISHED"
end

---Sample indexed greeting title APIs.
---Keep the highest counts for this capture so delayed samples retry stale
---indices even when the first shrink probe errors or returns unsettled data.
---Failed calls are skipped; raw streams never receive invented inactive values.
---@param t number
---@param tp number
local function SampleTitleStreams(t, tp)
  if type(GetNumActiveQuests) == "function" and type(GetActiveTitle) == "function" then
    local ok, count = SafeScalarCall(GetNumActiveQuests)
    if ok and type(count) == "number" then
      maxActiveTitleCount = math.max(maxActiveTitleCount, count)
      for index = 1, maxActiveTitleCount do
        local titleOk, titleData = SafePackedIndexCall(GetActiveTitle, index)
        if titleOk then
          local stream = GetOrCreateIndexStream("GetActiveTitle", index)
          AppendIfChanged(stream, t, tp, "GetActiveTitle:" .. index, titleData)
        end
      end
    end
  end

  if type(GetNumAvailableQuests) == "function" and type(GetAvailableTitle) == "function" then
    local ok, count = SafeScalarCall(GetNumAvailableQuests)
    if ok and type(count) == "number" then
      maxAvailableTitleCount = math.max(maxAvailableTitleCount, count)
      for index = 1, maxAvailableTitleCount do
        local titleOk, title = SafeScalarIndexCall(GetAvailableTitle, index)
        if titleOk then
          local stream = GetOrCreateIndexStream("GetAvailableTitle", index)
          AppendIfChanged(stream, t, tp, "GetAvailableTitle:" .. index, title)
        end
      end
    end
  end
end

---Sample all quest-dialog streams.
---@param capture CaptureState
---@return number t Session-relative GetTime()
---@return number tp Session-relative GetTimePreciseSec()
local function SampleQuestDialog(capture)
  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  for i = 1, #flatStreamDefs do
    SampleFlatStream(t, tp, flatStreamDefs[i])
  end
  SampleTitleStreams(t, tp)

  return t, tp
end

---Schedule staggered re-samples to catch dialog state settling after open/update events.
---Quest and gossip frames may expose incomplete values in the event call stack.
---Close events cancel these pending delayed reads and perform a single observed
---close-state sample instead of writing synthetic reset values.
---@param capture CaptureState
local function ScheduleDelayedSamples(capture)
  delayedSampleToken = delayedSampleToken + 1
  local sampleToken = delayedSampleToken
  local token = capture.token
  for i = 1, #SAMPLE_DELAYS do
    local delay = SAMPLE_DELAYS[i]
    if delay > 0 then
      C_After(delay, function()
        if not capture.active or capture.token ~= token or delayedSampleToken ~= sampleToken then return end
        SampleQuestDialog(capture)
      end)
    end
  end
end

Core.RegisterTracker({
  events = {
    "QUEST_DETAIL",
    "QUEST_PROGRESS",
    "QUEST_COMPLETE",
    "QUEST_FINISHED",
    "QUEST_GREETING",
    "QUEST_ACCEPT_CONFIRM",
    "GOSSIP_SHOW",
    "GOSSIP_CLOSED",
  },

  ---@param capture CaptureState
  Init = function(capture)
    functions = capture.session.functions
    previousValues = {}
    availableFlatStreams = {}
    maxActiveTitleCount = 0
    maxAvailableTitleCount = 0
    delayedSampleToken = 0
    flatStreamDefs = BuildFlatStreamDefs()

    SampleQuestDialog(capture)
    ScheduleDelayedSamples(capture)
  end,

  ---@param capture CaptureState
  ---@param event string
  OnEvent = function(capture, event)
    if IsCloseEvent(event) then
      -- Close boundaries are causal sampling points for mutable dialog/gossip
      -- APIs. Cancel pending open/update delayed reads, sample the APIs once,
      -- and record only the observed return values.
      delayedSampleToken = delayedSampleToken + 1
      SampleQuestDialog(capture)
      return
    end

    SampleQuestDialog(capture)
    ScheduleDelayedSamples(capture)
  end,
})
