---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- UnitLevel(unit)           -> number (or -1 for "??")
-- UnitClassification(unit)  -> string ("normal", "elite", "rare", "rareelite", "worldboss")
-- UnitReaction(unit, unit)  -> number 1..8 (or nil)
---------------------------------------------------------------------------

---@type string[]
local TOKENS = { "target", "mouseover" }

-- Stream references (set during Init)
---@type table<string, FunctionStreamEntry[]>?
local levelStreams
---@type table<string, FunctionStreamEntry[]>?
local classificationStreams
---@type table<string, FunctionStreamEntry[]>?
local reactionStreams

---Safely call a function and return the first result.
---@param fn function?
---@param ... any
---@return boolean ok
---@return any value
local function SafeScalarCall(fn, ...)
  if type(fn) ~= "function" then return false, nil end
  local ok, value = pcall(fn, ...)
  if not ok then return false, nil end
  return true, value
end

---Sample UnitLevel, UnitClassification, and UnitReaction for all tokens.
---@param t number Session-relative GetTime()
---@param tp number Session-relative GetTimePreciseSec()
local function SampleAll(t, tp)
  if not levelStreams or not classificationStreams or not reactionStreams then return end

  for _, token in ipairs(TOKENS) do
    -- Guard: only sample non-player creatures
    if UnitExists(token) and not UnitIsPlayer(token) then
      -- UnitLevel (scalar number). -1 indicates "??" (unknown/skull).
      local levelOk, level = SafeScalarCall(UnitLevel, token)
      if levelOk and level ~= nil then
        ---@type FunctionStreamEntry[]
        local levelStream = levelStreams[token]
        ---@type FunctionStreamEntry?
        local prevLevel = levelStream[#levelStream]
        if not prevLevel or prevLevel.v ~= level then
          levelStream[#levelStream + 1] = { t = t, tp = tp, v = level }
        end
      end

      -- UnitClassification (scalar string). Returns nil for non-existent units.
      local classOk, class = SafeScalarCall(UnitClassification, token)
      if classOk and class ~= nil then
        ---@type FunctionStreamEntry[]
        local classStream = classificationStreams[token]
        ---@type FunctionStreamEntry?
        local prevClass = classStream[#classStream]
        if not prevClass or prevClass.v ~= class then
          classStream[#classStream + 1] = { t = t, tp = tp, v = class }
        end
      end

      -- UnitReaction (scalar number 1..8). Native argument order: UnitReaction("player", token).
      local reactionOk, reaction = SafeScalarCall(UnitReaction, "player", token)
      if reactionOk and reaction ~= nil then
        ---@type FunctionStreamEntry[]
        local reactionStream = reactionStreams[token]
        ---@type FunctionStreamEntry?
        local prevReaction = reactionStream[#reactionStream]
        if not prevReaction or prevReaction.v ~= reaction then
          reactionStream[#reactionStream + 1] = { t = t, tp = tp, v = reaction }
        end
      end
    end
  end
end

Core.RegisterTracker({
  events = {
    "PLAYER_TARGET_CHANGED",
    "UPDATE_MOUSEOVER_UNIT",
  },

  ---@param capture CaptureState
  Init = function(capture)
    ---@type table<string, FunctionStreamEntry[]|table<string|number, FunctionStreamEntry[]>>
    local functions = capture.session.functions
    if not functions["UnitLevel"] then functions["UnitLevel"] = {} end
    if not functions["UnitClassification"] then functions["UnitClassification"] = {} end
    if not functions["UnitReaction"] then functions["UnitReaction"] = {} end
    if not functions["UnitReaction"]["player"] then functions["UnitReaction"]["player"] = {} end

    levelStreams = {}
    classificationStreams = {}
    reactionStreams = {}
    for _, token in ipairs(TOKENS) do
      functions["UnitLevel"][token] = {}
      functions["UnitClassification"][token] = {}
      functions["UnitReaction"]["player"][token] = {}
      levelStreams[token] = functions["UnitLevel"][token]
      classificationStreams[token] = functions["UnitClassification"][token]
      reactionStreams[token] = functions["UnitReaction"]["player"][token]
    end

    -- Initial sample at t=0
    SampleAll(0, 0)
  end,

  ---@param capture CaptureState
  ---@param _event string
  OnEvent = function(capture, _event)
    ---@type number
    local t  = GetTime()          - capture.startedAt
    ---@type number
    local tp = GetTimePreciseSec() - capture.startedAtPrecise
    SampleAll(t, tp)
  end,
})