---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- UnitLevel(unit)              -> number level
-- C_QuestLog.GetTrivialRange() -> number range       -- Forever
-- GetQuestGreenRange()         -> number greenRange  -- the other clients
--   How many levels below the player a quest may be before it is trivial.
---------------------------------------------------------------------------

---@type FunctionStreamEntry[]? Shortcut to functions["UnitLevel"]["player"]
local stream
---@type (fun(): number)? The client's range function, nil if it has none
local getRange
---@type FunctionStreamEntry[] The stream of getRange
local rangeStream

Core.RegisterTracker({
  events = { "PLAYER_LEVEL_UP", "PLAYER_ENTERING_WORLD", "SPELLS_CHANGED" },

  ---@param capture CaptureState
  Init = function(capture)
    ---@type number
    local level = UnitLevel("player")
    ---@type table<string, FunctionStreamEntry[]|table<string|number, FunctionStreamEntry[]>>
    local functions = capture.session.functions
    functions["UnitLevel"] = {
      ["player"] = { { t = 0, tp = 0, v = level } },
    }
    stream = functions["UnitLevel"]["player"]

    -- Forever has no GetQuestGreenRange: a global of that name there is another addon's.
    local rangeKey = "GetQuestGreenRange"
    getRange = GetQuestGreenRange
    if C_QuestLog and C_QuestLog.GetTrivialRange then
      rangeKey = "C_QuestLog.GetTrivialRange"
      getRange = C_QuestLog.GetTrivialRange
    end
    if getRange then
      rangeStream = {}
      functions[rangeKey] = rangeStream
      Core.SampleScalar(rangeStream, 0, 0, getRange)
    end
  end,

  ---@param capture CaptureState
  OnEvent = function(capture)
    if not stream then return end
    ---@type number
    local t  = GetTime() - capture.startedAt
    ---@type number
    local tp = GetTimePreciseSec() - capture.startedAtPrecise

    ---@type number
    local level = UnitLevel("player")
    ---@type FunctionStreamEntry?
    local prevLevel = stream[#stream]
    if not prevLevel or prevLevel.v ~= level then
      stream[#stream + 1] = { t = t, tp = tp, v = level }
    end

    if getRange then
      Core.SampleScalar(rangeStream, t, tp, getRange)
    end
  end,
})
