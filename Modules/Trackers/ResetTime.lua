---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- GetServerTime()     -> number serverTime
-- GetQuestResetTime() -> number nextReset
---------------------------------------------------------------------------

---@type table<string, FunctionStream>
local functions
---@type table<string, any>
local previousValues

---@class ResetTimeStreamDef
---@field key string Stream key stored in `session.functions`.
---@field fn function Zero-argument WoW API sampled defensively with pcall.

---@type ResetTimeStreamDef[]
local streams

---Append a scalar value to a stream if it changed.
---@param stream FunctionStreamEntry[]
---@param t number
---@param tp number
---@param key string
---@param value any
local function AppendIfChanged(stream, t, tp, key, value)
  local previous = previousValues[key]
  ---@type boolean
  local changed

  if previous == nil and value == nil then
    changed = #stream == 0
  elseif previous == nil or value == nil then
    changed = true
  else
    changed = value ~= previous
  end

  if changed then
    stream[#stream + 1] = { t = t, tp = tp, v = value }
    previousValues[key] = value
  end
end

---Create stream definitions for APIs available on this client.
---These APIs are client/era dependent, so unavailable functions simply do not
---create streams. That keeps missing API support distinct from a captured nil.
---@return ResetTimeStreamDef[]
local function BuildStreams()
  ---@type ResetTimeStreamDef[]
  local defs = {}

  if type(GetServerTime) == "function" then
    functions["GetServerTime"] = {}
    defs[#defs + 1] = { key = "GetServerTime", fn = GetServerTime }
  end
  if type(GetQuestResetTime) == "function" or (C_DateAndTime and type(C_DateAndTime.GetSecondsUntilDailyReset) == "function") then
    functions["GetQuestResetTime"] = {}
    defs[#defs + 1] = { key = "GetQuestResetTime", fn = Core.Compat.GetQuestResetTime }
  end

  return defs
end

---Sample all available reset-time streams.
---These are low-frequency snapshots, not a per-second clock trace. Replay
---consumers that need continuously increasing server time or countdown reset
---semantics should derive them from the nearest sampled value and replay time.
---@param capture CaptureState
local function SampleResetTime(capture)
  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  for i = 1, #streams do
    local def = streams[i]
    local ok, value = pcall(def.fn)
    if ok then
      local stream = functions[def.key]
      ---@cast stream FunctionStreamEntry[]
      AppendIfChanged(stream, t, tp, def.key, value)
    end
  end
end

Core.RegisterTracker({
  events = {
    "PLAYER_LOGIN",
    "PLAYER_ENTERING_WORLD",
    "PLAYER_LOGOUT",
  },

  ---@param capture CaptureState
  Init = function(capture)
    functions = capture.session.functions
    previousValues = {}
    streams = BuildStreams()

    SampleResetTime(capture)
  end,

  ---@param capture CaptureState
  OnEvent = function(capture)
    SampleResetTime(capture)
  end,
})
