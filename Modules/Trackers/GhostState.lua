---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- UnitIsGhost("player") -> boolean isGhost
---------------------------------------------------------------------------

---@type FunctionStreamEntry[]? Shortcut to functions["UnitIsGhost"]["player"]
local stream

Core.RegisterTracker({
  events = {
    "PLAYER_DEAD",
    "PLAYER_ALIVE", -- also fires when the player releases the spirit and becomes a ghost
    "PLAYER_UNGHOST",
    "PLAYER_ENTERING_WORLD",
  },

  ---@param capture CaptureState
  Init = function(capture)
    stream = {}
    capture.session.functions["UnitIsGhost"] = { ["player"] = stream }
    Core.SampleScalar(stream, 0, 0, UnitIsGhost, "player")
  end,

  ---@param capture CaptureState
  OnEvent = function(capture)
    if not stream then return end
    local t = GetTime() - capture.startedAt
    local tp = GetTimePreciseSec() - capture.startedAtPrecise
    Core.SampleScalar(stream, t, tp, UnitIsGhost, "player")
  end,
})
