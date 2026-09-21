---@type QuestieTraceCore
local Core = QuestieTraceCore
---@type fun(...): PackedArgs
local PackArgs = Core.PackArgs
---@type fun(t1: any, t2: any, ignore_mt: boolean?, visited: table?): boolean
local DeepCompare = Core.DeepCompare

local Compat = Core.Compat

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- GetFactionInfoByID(factionID) -> string  name,
--                                  string  description,
--                                  number  standingID,      -- 4=Neutral, 5=Friendly, 6=Honored...
--                                  number  barMin,
--                                  number  barMax,
--                                  number  barValue,
--                                  boolean atWarWith,
--                                  boolean canToggleAtWar,
--                                  boolean isHeader,
--                                  boolean isCollapsed,
--                                  boolean hasRep,
--                                  boolean isWatched,
--                                  boolean isChild,
--                                  number  factionID,
--                                  boolean hasBonusRepGain,
--                                  boolean canSetInactive
--
-- GetNumFactions() -> number numberOfFactions
--
-- FactionOrder (custom stream) -> number[] factionIDs  -- ordered known faction IDs
---------------------------------------------------------------------------

---@type table<string, FunctionStreamEntry[]|table<string|number, FunctionStreamEntry[]>>
local functions       -- capture.session.functions
---@type number[]
local factionOrder    -- ordered array of known factionIDs
---@type table<number, PackedArgs>
local prevValues      -- [factionID] -> last packed tuple (for change detection)
---@type boolean
local collecting      -- recursion guard for ExpandFactionHeader

--- Discover all factionIDs by iterating the reputation panel.
--- Expands collapsed headers (side effect). Never re-collapses.
---@return number[] ids Ordered array of factionIDs
local function CollectFactionIDs()
  ---@type number[]
  local ids = {}
  ---@type number
  local numFactions = Compat.GetNumFactions() or 0
  if numFactions == 0 then return ids end

  collecting = true
  ---@type number
  local index = 1

  while index <= numFactions do
    local _, _, _, _, _, _,
      _, _, isHeader, isCollapsed, _,
      _, _, factionID =
      Compat.GetFactionInfo(index)

    if factionID then
      ids[#ids + 1] = factionID
    end

    if isHeader and isCollapsed and type(ExpandFactionHeader) == "function" then
      ExpandFactionHeader(index)
      numFactions = Compat.GetNumFactions() or numFactions
    end

    index = index + 1
  end

  collecting = false
  return ids
end

--- Sample all known factions. Appends {t,tp,v} only when value changed.
---@param capture CaptureState
local function SampleReputation(capture)
  ---@type number
  local t  = GetTime()          - capture.startedAt
  ---@type number
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  for i = 1, #factionOrder do
    ---@type number
    local factionID = factionOrder[i]
    ---@type PackedArgs
    local v = PackArgs(Compat.GetFactionInfoByID(factionID))

    ---@type PackedArgs?
    local prev = prevValues[factionID]
    if not prev or not DeepCompare(v, prev) then
      ---@type FunctionStreamEntry[]
      local stream = functions["GetFactionInfoByID"][factionID]
      if not stream then
        stream = {}
        functions["GetFactionInfoByID"][factionID] = stream
      end
      stream[#stream + 1] = { t = t, tp = tp, v = v }
      prevValues[factionID] = v
    end
  end
end

--- Full collection + sample. Updates FactionOrder if the set changed.
---@param capture CaptureState
local function CollectAndSample(capture)
  ---@type number[]
  local ids = CollectFactionIDs()

  -- Update FactionOrder stream if changed
  ---@type FunctionStreamEntry[]
  local orderStream = functions["FactionOrder"]
  ---@type FunctionStreamEntry?
  local prevOrder = orderStream[#orderStream]
  if not prevOrder or not DeepCompare(prevOrder.v, ids) then
    ---@type number
    local t  = GetTime()          - capture.startedAt
    ---@type number
    local tp = GetTimePreciseSec() - capture.startedAtPrecise
    ---@type number[]
    local copy = {}
    for i = 1, #ids do copy[i] = ids[i] end
    orderStream[#orderStream + 1] = { t = t, tp = tp, v = copy }
  end

  factionOrder = ids
  SampleReputation(capture)
end

Core.RegisterTracker({
  events = {
    "CHAT_MSG_COMBAT_FACTION_CHANGE",
    "UPDATE_FACTION",
    "QUEST_TURNED_IN",
    "PLAYER_ENTERING_WORLD",
    "SPELLS_CHANGED",
  },

  ---@param capture CaptureState
  Init = function(capture)
    functions = capture.session.functions
    functions["FactionOrder"] = {}
    functions["GetFactionInfoByID"] = {}
    prevValues = {}
    factionOrder = {}
    collecting = false

    CollectAndSample(capture)
  end,

  ---@param capture CaptureState
  ---@param event string
  OnEvent = function(capture, event)
    -- Recursion guard: ExpandFactionHeader fires UPDATE_FACTION
    if collecting then return end

    if event == "QUEST_TURNED_IN" then
      -- Quest rewards can reveal new factions
      CollectAndSample(capture)
    else
      -- CHAT_MSG_COMBAT_FACTION_CHANGE, UPDATE_FACTION: values changed, set is the same
      SampleReputation(capture)
    end
  end,
})
