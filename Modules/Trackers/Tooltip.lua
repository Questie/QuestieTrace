---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- Tooltips (Forever)
---------------------------------------------------------------------------
-- What the client shows in the tooltips of NPCs, world objects and quests. Besides names and titles, the
-- tooltip of an NPC or object lists the quests it counts for: a QuestTitle line with the quest ID, then the
-- QuestObjective lines. In a group each member's objectives follow a QuestPlayer line, so it also shows
-- quests the player is not on.
--
-- Tooltip (synthetic) -> functions["Tooltip"][kind][key], v = { guid = string?, lines = TooltipTraceLine[] }
--   "unit"          [NPC ID]             NPC under the mouse
--   "object"        [object ID or name]  by name while Forever gives object tooltips no GUID
--   "quest"         [quest ID]           quest links
--   "questProgress" [quest ID]           a quest's progress in the group
--   An entry is written for every different set of lines, the first time it is seen. Progress, levels and
--   other numbers are stored, but do not make a set different, so refreshes and other spawns add nothing.
-- TooltipPartyProgress (synthetic) -> functions["TooltipPartyProgress"][questID][who][objective], how far a
--   group member is, whenever that changes. The player's own is in C_QuestLog.GetQuestObjectives. Objectives
--   with the same text are told apart by " (2)" and so on.
-- TooltipDropped (synthetic) -> how many different sets and values the limits below kept out, as one entry.
--
-- A QuestPlayer line has `who` instead of text: "self", "party1".."party4" ("raid1".. in a raid), or "other".
-- The objectives below it are that member's. A slot is a place in the group: it can be someone else after
-- GROUP_ROSTER_UPDATE. A line the client hides is { hidden = true }: what follows may belong to another line.
--
-- Privacy: lines that are a name by type (QuestPlayer, UnitOwner) keep no text, their GUIDs are only compared.
-- Every other text goes through SanitizeText. Players, pets and player-controlled units are not recorded.
---------------------------------------------------------------------------

---@class TooltipData
---@field id number? Quest ID of a quest's tooltip

---@class TooltipTraceLine
---@field type number? Enum.TooltipDataLineType
---@field text string?
---@field id number? Quest ID of a QuestTitle line
---@field onQuest boolean? Whether the player is on the quest of a QuestTitle line
---@field who string? Of a QuestPlayer line
---@field have number? Progress of a QuestObjective line
---@field need number?
---@field done boolean?
---@field hidden boolean? The client did not let the addon read the line's type

---@type number Entities of each kind with entries, and entries of each
local MAX_ENTITIES, MAX_VERSIONS = 1000, 8
---@type number Lines stored of one tooltip
local MAX_LINES = 40
---@type number Entries of one party progress stream
local MAX_PROGRESS = 50

---@type CaptureState?
local captureRef
---@class TooltipEntity
---@field seen table<string, boolean>? Signatures stored or counted as left out
---@field versions number?
---@field full boolean? Past MAX_ENTITIES

---@type table<string, table<string|number, TooltipEntity>> [kind][key] What is stored of each entity
local known = {}
---@type table<string, number>
local entityCounts = {}
---@type FunctionStreamEntry?
local droppedEntry
---@type table<string, number> Last party progress by quest, slot and unsanitized objective. Compared first, as
---sanitizing a text is too slow to do on every hover.
local lastProgress = {}
---@type boolean TooltipDataProcessor hooks cannot be removed, so they are installed once.
local hooksInstalled = false

---@param value any
---@param valueType "number"|"string"|"boolean"|"table"
---@return any value nil when it is restricted (secret) or of another type
local function Readable(value, valueType)
  if issecretvalue and issecretvalue(value) then return nil end
  if type(value) ~= valueType then return nil end
  return value
end

---@param parent table
---@param key string|number
---@return table child
local function GetOrCreate(parent, key)
  local child = parent[key]
  if not child then
    child = {}
    parent[key] = child
  end
  return child
end

---@param capture CaptureState
---@param value any
---@return FunctionStreamEntry
local function NewEntry(capture, value)
  return { t = GetTime() - capture.startedAt, tp = GetTimePreciseSec() - capture.startedAtPrecise, v = value }
end

---@param capture CaptureState
local function CountDropped(capture)
  if not droppedEntry then
    droppedEntry = NewEntry(capture, 0)
    capture.session.functions["TooltipDropped"] = { droppedEntry }
  end
  droppedEntry.v = droppedEntry.v + 1
end

---@param guid any
---@return string who Group slot of a QuestPlayer line's GUID
local function GetMember(guid)
  local memberGUID = Readable(guid, "string")
  if not memberGUID then return "other" end
  if memberGUID == Readable(UnitGUID("player"), "string") then return "self" end
  local prefix, count = "party", 4
  if IsInRaid and IsInRaid() then prefix, count = "raid", 40 end
  for i = 1, count do
    if memberGUID == Readable(UnitGUID(prefix .. i), "string") then return prefix .. i end
  end
  return "other"
end

---@param text string
---@return string
local function FoldNumbers(text)
  return (text:gsub("%d[%d.,]*", "#"))
end

---An objective without how far it is, which keeps other numbers ("Nest 1 searched") that tell objectives apart.
---@param text string
---@return string
local function FoldProgress(text)
  text = text:gsub("%d[%d.,]*/%d[%d.,]*", "#/#")
  text = text:gsub("%d[%d.,]*%%", "#%%")
  return text
end

---The text of a line, without names the line type gives away.
---@param line table
---@param lineType number
---@return string?
local function GetLineText(line, lineType)
  local lineTypes = Enum.TooltipDataLineType
  if lineType == lineTypes.QuestPlayer or lineType == lineTypes.UnitOwner then return nil end
  return Readable(line.leftText, "string")
end

---What tells one version of a tooltip from another, without its numbers. Read before anything is stored.
---@param lines table
---@return string
local function GetSignature(lines)
  local lineTypes = Enum.TooltipDataLineType
  local parts = {}
  for _, rawLine in ipairs(lines) do
    local line = Readable(rawLine, "table")
    local lineType = line and Readable(line.type, "number")
    if not lineType then
      parts[#parts + 1] = "?"
    elseif lineType == lineTypes.QuestPlayer then
      parts[#parts + 1] = "P" .. GetMember(line.guid)
    else
      local text = GetLineText(line, lineType)
      local folded = text and (lineType == lineTypes.QuestObjective and FoldProgress(text) or FoldNumbers(text)) or ""
      parts[#parts + 1] = lineType .. "\1" .. tostring(Readable(line.id, "number")) .. "\1" .. folded ..
        "\1" .. tostring(Readable(line.numRequired, "number")) .. "\1" .. tostring(Readable(line.completed, "boolean"))
    end
    if #parts >= MAX_LINES then break end
  end
  return table.concat(parts, "\2")
end

---@param lines table
---@return TooltipTraceLine[]
local function BuildLines(lines)
  local lineTypes = Enum.TooltipDataLineType
  local built = {}
  for _, rawLine in ipairs(lines) do
    local line = Readable(rawLine, "table")
    local lineType = line and Readable(line.type, "number")
    if not lineType then
      built[#built + 1] = { hidden = true }
    elseif lineType == lineTypes.QuestPlayer then
      built[#built + 1] = { type = lineType, who = GetMember(line.guid) }
    else
      local text = GetLineText(line, lineType)
      local questID = lineType == lineTypes.QuestTitle and Readable(line.id, "number") or nil
      local onQuest
      if questID then onQuest = C_QuestLog.IsOnQuest(questID) == true end
      built[#built + 1] = {
        type = lineType,
        text = text and Core.SanitizeText(text) or nil,
        id = questID,
        onQuest = onQuest,
        have = Readable(line.numFulfilled, "number"),
        need = Readable(line.numRequired, "number"),
        done = Readable(line.completed, "boolean"),
      }
    end
    if #built >= MAX_LINES then break end
  end
  return built
end

---Record how far group members are with the objectives a tooltip lists.
---@param capture CaptureState
---@param questID number
---@param who string
---@param text string
---@param have number
---@param sameText table<string, number>
local function RecordMemberProgress(capture, questID, who, text, have, sameText)
  local folded = FoldProgress(text)
  local count = (sameText[folded] or 0) + 1
  sameText[folded] = count
  local suffix = count > 1 and (" (" .. count .. ")") or ""
  local progressKey = questID .. "\1" .. who .. "\1" .. folded .. suffix
  if lastProgress[progressKey] == have then return end
  lastProgress[progressKey] = have

  local quests = GetOrCreate(capture.session.functions, "TooltipPartyProgress")
  local stream = GetOrCreate(GetOrCreate(GetOrCreate(quests, questID), who), FoldProgress(Core.SanitizeText(text)) .. suffix)
  if #stream >= MAX_PROGRESS then
    CountDropped(capture)
  else
    stream[#stream + 1] = NewEntry(capture, have)
  end
end

---@param capture CaptureState
---@param lines table
---@param defaultQuestID number? The quest of a tooltip that can list one without a title line
local function RecordPartyProgress(capture, lines, defaultQuestID)
  local lineTypes = Enum.TooltipDataLineType
  local questID, who = defaultQuestID, nil
  ---@type table<string, number> How often an objective's text came up below the current member
  local sameText = {}
  for _, rawLine in ipairs(lines) do
    local line = Readable(rawLine, "table")
    local lineType = line and Readable(line.type, "number")
    if line and lineType == lineTypes.QuestTitle then
      questID, who, sameText = Readable(line.id, "number"), nil, {}
    elseif line and lineType == lineTypes.QuestPlayer then
      who, sameText = GetMember(line.guid), {}
    elseif line and lineType == lineTypes.QuestObjective then
      local text, have = Readable(line.leftText, "string"), Readable(line.numFulfilled, "number")
      if questID and text and have and who and who ~= "self" and who ~= "other" then
        RecordMemberProgress(capture, questID, who, text, have, sameText)
      end
    else
      -- Anything else, or a line that cannot be read, ends the member's part.
      questID, who, sameText = defaultQuestID, nil, {}
    end
  end
end

---@param kind "unit"|"object"|"quest"|"questProgress"
---@param key string|number
---@param guid string?
---@param tooltipData TooltipData
---@param defaultQuestID number? The quest of a tooltip that can list one without a title line
local function StoreTooltip(kind, key, guid, tooltipData, defaultQuestID)
  local capture = captureRef
  local lines = Readable(tooltipData.lines, "table")
  if not capture or not lines or lines[1] == nil then return end

  if IsInGroup and IsInGroup() then RecordPartyProgress(capture, lines, defaultQuestID) end

  local signature = GetSignature(lines)
  local entity = GetOrCreate(GetOrCreate(known, kind), key)
  if not entity.seen then
    entity.seen, entity.versions = {}, 0
    -- An entity past the limit is kept in memory without entries, so it is counted once.
    entity.full = (entityCounts[kind] or 0) >= MAX_ENTITIES
    if not entity.full then entityCounts[kind] = (entityCounts[kind] or 0) + 1 end
  end
  -- A tooltip can flip between a few sets of lines, such as an NPC's alive and dead. Each is stored once.
  if entity.seen[signature] then return end
  entity.seen[signature] = true
  if entity.full or entity.versions >= MAX_VERSIONS then return CountDropped(capture) end
  entity.versions = entity.versions + 1

  local stream = GetOrCreate(GetOrCreate(GetOrCreate(capture.session.functions, "Tooltip"), kind), key)
  stream[#stream + 1] = NewEntry(capture, { guid = guid, lines = BuildLines(lines) })
end

---@param guid string
---@return number? id The NPC or object ID of a GUID
local function GetEntityID(guid)
  return tonumber(guid:match("^%a+%-%d+%-%d+%-%d+%-%d+%-(%d+)%-"))
end

---@type table<string, fun(tooltipData: TooltipData)> By the name of their Enum.TooltipDataType. Not registered for all
---types: the client rebuilds the minimap's tooltip every frame.
local HANDLERS = {
  Unit = function(tooltipData)
    local guid = Core.GetNpcTooltipGUID(tooltipData)
    local npcID = guid and GetEntityID(guid)
    if npcID then StoreTooltip("unit", npcID, guid, tooltipData) end
  end,

  Object = function(tooltipData)
    local guid = Core.ParseGUIDKind(tooltipData.guid) == "object" and tooltipData.guid or nil
    local key = guid and GetEntityID(guid) or Core.GetObjectTooltipName(tooltipData)
    if key then StoreTooltip("object", key, guid, tooltipData) end
  end,

  Quest = function(tooltipData)
    local questID = Readable(tooltipData.id, "number")
    if questID then StoreTooltip("quest", questID, nil, tooltipData) end
  end,

  QuestPartyProgress = function(tooltipData)
    local questID = Readable(tooltipData.id, "number")
    if questID then StoreTooltip("questProgress", questID, nil, tooltipData, questID) end
  end,
}

local function InstallHooks()
  if hooksInstalled then return end
  local tooltipTypes = Enum and Enum.TooltipDataType
  if not TooltipDataProcessor or not TooltipDataProcessor.AddTooltipPostCall or not tooltipTypes or not Enum.TooltipDataLineType then return end
  if not C_QuestLog or not C_QuestLog.IsOnQuest then return end
  for name, handler in pairs(HANDLERS) do
    if tooltipTypes[name] then
      TooltipDataProcessor.AddTooltipPostCall(tooltipTypes[name], function(_, tooltipData)
        local data = captureRef and Readable(tooltipData, "table")
        if data then handler(data) end
      end)
    end
  end
  hooksInstalled = true
end

Core.RegisterTracker({
  events = { "GROUP_ROSTER_UPDATE" },

  -- A slot can be someone else now, whose progress has to be recorded even if it is the same.
  OnEvent = function()
    lastProgress = {}
  end,

  ---@param capture CaptureState
  Init = function(capture)
    captureRef = capture
    known, entityCounts, droppedEntry, lastProgress = {}, {}, nil, {}
    InstallHooks()
  end,

  OnCaptureStopped = function()
    captureRef = nil
    known, entityCounts, droppedEntry, lastProgress = {}, {}, nil, {}
  end,
})
