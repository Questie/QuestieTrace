---@type QuestieTraceCore
local Core = QuestieTraceCore
---@type fun(t1: any, t2: any, ignore_mt: boolean?, visited: table?): boolean
local DeepCompare = Core.DeepCompare

local C_After = C_Timer.After

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- C_Trainer.GetTrainerType() -> number trainerType
--   Forever. Enum.TrainerType: 0 general, 2 tradeskills, 3 pet.
-- IsTradeskillTrainer() -> boolean isTradeskillTrainer
--   Recorded on clients without C_Trainer.GetTrainerType.
-- GetTrainerServiceTypeFilter(serviceType) -> boolean isListed
--   "available", "unavailable" or "used": the client leaves services of a
--   type that is switched off out of the list, and so does the recording.
--
-- TrainerServices[npcID][service] (synthetic) -> TrainerService
--   What a trainer teaches. The service key is its name and required level
--   ("Fireball:6"): the list index names another service at the next trainer
--   and after something was learned, and the rank text can arrive late.
--   Change-only, so the same trainer visited again adds only what changed.
---------------------------------------------------------------------------

---@class TrainerService
---@field name string
---@field rank string? The text behind the name, e.g. "Rank 2"
---@field type string? "available", "unavailable" or "used", for this player
---@field level number? Required level
---@field cost number Copper; training points for pet skills on Forever
---@field skill string? Name of the required skill
---@field skillRank number? Required rank of that skill
---@field spellID number? The spell its tooltip shows (Forever)
---@field itemID number? The item its tooltip (Forever) or link shows

---@type string[]
local SERVICE_TYPES = { "available", "unavailable", "used" }

--- Seconds between an event and the sample it asks for, so a burst of updates
--- costs one sample.
---@type number
local SAMPLE_DELAY = 0.1

---@class TrainerWindow
---@field capture CaptureState
---@field isSamplePending boolean

---@type TrainerWindow? The open window, if any. Each opening gets its own table.
local openWindow

---@type (fun(): number)? Only on Forever, where GetTrainerServiceInfo also answers in another order
local getTrainerType
---@type (fun(index: number): { type: number, id: number }?)?
local getServiceTooltip
---@type FunctionStreamEntry[]
local kindStream
---@type table<string, FunctionStreamEntry[]>
local filterStreams
---@type table<number, table<string, FunctionStreamEntry[]>>
local serviceStreams

---@param stream FunctionStreamEntry[]
---@param t number
---@param tp number
---@param value any
local function AppendIfChanged(stream, t, tp, value)
  local previous = stream[#stream]
  if not previous or not DeepCompare(previous.v, value) then
    stream[#stream + 1] = { t = t, tp = tp, v = value }
  end
end

---The trainer, as an NPC ID. nil for a window without an NPC (hunters train
---their pet in this window, too).
---@return number? npcID
local function GetTrainerNpcID()
  -- Privacy: only world NPCs, never players or their pets.
  if not Core.IsNpcUnit("npc") then return nil end
  return tonumber(UnitGUID("npc"):match("^%a+%-%d+%-%d+%-%d+%-%d+%-(%d+)%-"))
end

---@param index number
---@return string? name nil for headers, and while the client has not loaded it
---@return string? rank
---@return string? serviceType
---@return number? level
local function ReadServiceInfo(index)
  if getTrainerType then
    local name, serviceType, _, level, rank = GetTrainerServiceInfo(index)
    return name, rank, serviceType, level
  end
  local name, rank, serviceType = GetTrainerServiceInfo(index)
  if serviceType == "header" then return nil end
  return name, rank, serviceType, GetTrainerServiceLevelReq(index)
end

---@param index number
---@return TrainerService? service Without IDs
local function ReadService(index)
  local name, rank, serviceType, level = ReadServiceInfo(index)
  if not name then return nil end

  local skill, skillRank = GetTrainerServiceSkillReq(index)
  return {
    name = name,
    rank = rank ~= "" and rank or nil,
    type = serviceType,
    level = level,
    cost = GetTrainerServiceCost(index),
    skill = skill,
    skillRank = skill and skillRank,
  }
end

---What a service's tooltip or link shows: the spell it teaches or the item its
---recipe makes.
---@param index number
---@return number? spellID
---@return number? itemID
local function ReadServiceIDs(index)
  if getServiceTooltip then
    local tooltip = getServiceTooltip(index)
    if tooltip and tooltip.type == Enum.TooltipDataType.Spell then return tooltip.id, nil end
    if tooltip and tooltip.type == Enum.TooltipDataType.Item then return nil, tooltip.id end
    return nil, nil
  end
  local link = GetTrainerServiceItemLink(index)
  return nil, link and tonumber(link:match("item:(%d+)"))
end

---@param streams table<string, FunctionStreamEntry[]> The trainer's services
---@param key string
---@param index number
---@param service TrainerService
---@param t number
---@param tp number
local function AppendService(streams, key, index, service, t, tp)
  local stream = streams[key] or {}
  streams[key] = stream

  -- An ID does not change, and a tooltip is costly to build.
  local previous = stream[#stream]
  if previous then
    service.spellID, service.itemID = previous.v.spellID, previous.v.itemID
  end
  if not service.spellID and not service.itemID then
    service.spellID, service.itemID = ReadServiceIDs(index)
  end
  AppendIfChanged(stream, t, tp, service)
end

---@param npcID number
---@param t number
---@param tp number
local function SampleServices(npcID, t, tp)
  ---@type table<string, boolean>
  local seenServices = {}
  for index = 1, GetNumTrainerServices() do
    local service = ReadService(index)
    if service then
      local key = ("%s:%s"):format(service.name, tostring(service.level))
      -- Of two services with one key only the first is recorded: both would
      -- append to the same stream on every sample.
      if not seenServices[key] then
        seenServices[key] = true
        serviceStreams[npcID] = serviceStreams[npcID] or {}
        AppendService(serviceStreams[npcID], key, index, service, t, tp)
      end
    end
  end
end

---@param capture CaptureState
local function SampleTrainer(capture)
  local npcID = GetTrainerNpcID()
  if not npcID then return end

  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise
  AppendIfChanged(kindStream, t, tp, (getTrainerType or IsTradeskillTrainer)())
  for _, serviceType in ipairs(SERVICE_TYPES) do
    AppendIfChanged(filterStreams[serviceType], t, tp, GetTrainerServiceTypeFilter(serviceType))
  end
  SampleServices(npcID, t, tp)
end

---Sample the open window a moment later, once however often this is asked.
local function RequestSample()
  local window = openWindow
  if not window or window.isSamplePending then return end

  window.isSamplePending = true
  C_After(SAMPLE_DELAY, function()
    if openWindow ~= window then return end
    window.isSamplePending = false
    SampleTrainer(window.capture)
  end)
end

---------------------------------------------------------------------------
-- Private update frame
-- Blizzard's trainer window redraws its list on these events: when services
-- arrive, are learned or filtered, and when their names arrive. They are
-- sampling triggers only and stay out of the recorded event stream.
---------------------------------------------------------------------------

---@type Frame
local updateFrame = CreateFrame("Frame")
updateFrame:RegisterEvent("TRAINER_UPDATE")
updateFrame:RegisterEvent("TRAINER_SERVICE_INFO_NAME_UPDATE")
updateFrame:SetScript("OnEvent", RequestSample)

Core.RegisterTracker({
  events = { "TRAINER_SHOW", "TRAINER_CLOSED" },

  ---@param capture CaptureState
  Init = function(capture)
    ---@diagnostic disable-next-line: undefined-global -- newer than Documentation/WoW-API
    local trainerApi = C_Trainer
    getTrainerType = type(trainerApi) == "table" and trainerApi.GetTrainerType or nil
    getServiceTooltip = type(C_TooltipInfo) == "table" and C_TooltipInfo.GetTrainerService or nil

    kindStream = {}
    filterStreams = {}
    for _, serviceType in ipairs(SERVICE_TYPES) do
      filterStreams[serviceType] = {}
    end
    serviceStreams = {}

    local functions = capture.session.functions
    functions[getTrainerType and "C_Trainer.GetTrainerType" or "IsTradeskillTrainer"] = kindStream
    functions["GetTrainerServiceTypeFilter"] = filterStreams
    functions["TrainerServices"] = serviceStreams
  end,

  ---@param capture CaptureState
  ---@param event string
  OnEvent = function(capture, event)
    if event == "TRAINER_SHOW" then
      openWindow = { capture = capture, isSamplePending = false }
      RequestSample()
    elseif event == "TRAINER_CLOSED" then
      openWindow = nil
    end
  end,

  OnCaptureStopped = function()
    openWindow = nil
  end,
})
