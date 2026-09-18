QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- Shared type definitions (used across all trackers)
---------------------------------------------------------------------------

---@class FunctionStreamEntry
---@field t number Relative time from GetTime()
---@field tp number Relative time from GetTimePreciseSec()
---@field v any The value at this point in time

---A stored API stream leaf or nested parameter map.
---Parameterless APIs store the leaf array directly. Parameterized APIs nest by
---native argument order until the final value is a FunctionStreamEntry[] leaf;
---for example `GetQuestLogRewardInfo(index, questID)` is stored as
---`functions["GetQuestLogRewardInfo"][index][questID]`.
---@alias FunctionStream FunctionStreamEntry[]|table<string|number, FunctionStream>

---@class EventRecord
---@field t number Relative time from GetTime()
---@field tp number Relative time from GetTimePreciseSec()
---@field e string Event name
---@field a PackedArgs Event arguments

---@class PackedArgs
---@field n number Number of arguments
---@field [number] any Positional arguments

---@class DeltaStreamEntry
---@field t number Relative time from GetTime()
---@field tp number Relative time from GetTimePreciseSec()
---@field add number[]? Added IDs
---@field remove number[]? Removed IDs

---@class DeltaStream
---@field t number Initial timestamp
---@field tp number Initial precise timestamp
---@field initial number[] Initial set of IDs
---@field delta DeltaStreamEntry[] Ordered delta entries

---@class SessionRecord
---@field schemaVersion number
---@field recordingContractVersion number? 1 = observed raw API streams; absent = legacy/unknown.
---@field name string?
---@field startedAt number
---@field startedAtPrecise number
---@field stoppedAt number?
---@field stoppedAtPrecise number?
---@field duration number?
---@field durationPrecise number?
---@field events EventRecord[]
---@field functions table<string, FunctionStream>
---@field functionsDelta table<string, DeltaStream>
---@field exportedAt number? GetTime() when this session was last included in a shown export payload; absent = never exported.

---@class CaptureState
---@field active boolean
---@field token number
---@field startedAt number?
---@field startedAtPrecise number?
---@field session SessionRecord?

---@class TrackerDef
---@field events string[]?
---@field Init fun(capture: CaptureState)?
---@field OnEvent fun(capture: CaptureState, event: string, ...)?
---@field OnCaptureStopped fun(capture: CaptureState)?

---@class DumpDef
---@field key string
---@field helpText string?
---@field slashCommands string[]?
---@field events string[]?
---@field Run fun(event: string?, ...): (boolean?, string?)

---@class StatusData
---@field captureState "running"|"stopped_unsaved"|"idle"
---@field isRunning boolean
---@field sessionName string
---@field eventCount number
---@field canSave boolean

---@class EventCategory
---@field name string
---@field events string[]

---@class MapRectData
---@field minX number
---@field maxX number
---@field minY number
---@field maxY number

---@class QuestieTraceMapEntry
---@field name string
---@field parentMapID number
---@field mapType number
---@field children number[]
---@field rect MapRectData?

---@class MapHierarchyDumpData
---@field schemaVersion number
---@field capturedAt string
---@field fallbackWorldMapID number
---@field rootSeeds number[]
---@field topUiMapIDs number[]
---@field mapsWithChildren number[]
---@field maps table<number, QuestieTraceMapEntry>
---@field rectOnMap table<number, table<number, MapRectData>>

---@class QuestieTraceDumpsData
---@field schemaVersion number
---@field dumps table<string, any>

---------------------------------------------------------------------------
-- Deep compare
---------------------------------------------------------------------------

--- Deep compare two values (ignores "timestamp" keys for backwards compat).
---@param t1 any
---@param t2 any
---@param ignore_mt boolean?
---@param visited table<table, table>? Internal cycle-detection table
---@return boolean equal
function Core.DeepCompare(t1, t2, ignore_mt, visited)
  if t1 == t2 then return true end

  local type1, type2 = type(t1), type(t2)
  if type1 ~= type2 then return false end
  if type1 ~= "table" then return false end

  visited = visited or {}
  if visited[t1] and visited[t1] == t2 then return true end
  visited[t1] = t2

  if not ignore_mt then
    local mt1, mt2 = getmetatable(t1), getmetatable(t2)
    if mt1 or mt2 then
      if not Core.DeepCompare(mt1, mt2, ignore_mt, visited) then
        return false
      end
    end
  end

  for key, value in pairs(t1) do
    if t2[key] == nil or not Core.DeepCompare(value, t2[key], ignore_mt, visited) then
      return false
    end
  end

  for key in pairs(t2) do
    if t1[key] == nil then
      return false
    end
  end

  return true
end

---------------------------------------------------------------------------
-- Shared helpers used by multiple trackers
---------------------------------------------------------------------------

--- Pack varargs into a table with n field.
---@param ... any The arguments to pack
---@return PackedArgs
function Core.PackArgs(...)
  local tbl = { ... }
  ---@cast tbl PackedArgs
  tbl.n = select("#", ...)
  return tbl
end

--- Copy a packed-args table (table with n field).
---@param args PackedArgs?
---@return PackedArgs
function Core.CopyPacked(args)
  if type(args) ~= "table" then
    return { n = 0 }
  end
  local out = {}
  local n = args.n or #args
  for i = 1, n do
    out[i] = args[i]
  end
  out.n = n
  return out
end

--- Round a number to the given decimal places.
---@param num number?
---@param decimals number?
---@return number?
function Core.Round(num, decimals)
  if type(num) ~= "number" then return num end
  local mult = 10 ^ (decimals or 0)
  return math.floor(num * mult + 0.5) / mult
end

---------------------------------------------------------------------------
-- Privacy: GUID classification, text sanitization, chat message scrubbing
---------------------------------------------------------------------------
-- See the "Privacy" sections in AGENTS.md / CLAUDE.md for the rules this
-- code implements. No player names, guild names, or player GUIDs may ever
-- be persisted to a session. Classification here is manual and
-- deterministic (GUID prefix matching, known-name substitution) rather
-- than relying on client flags like `issecretvalue()`, which are not a
-- reliable privacy guard on their own.

---@type table<number, {[1]: string, [2]: "player"|"npc"|"object"|"item"}>
local GUID_KIND_PATTERNS = {
  { "^Player%-", "player" },
  { "^Creature%-", "npc" },
  { "^Pet%-", "npc" },
  { "^Vehicle%-", "npc" },
  { "^GameObject%-", "object" },
  { "^Item%-", "item" },
}

--- Classify a WoW GUID by its prefix.
---@param guid any
---@return "player"|"npc"|"object"|"item"|nil kind
function Core.ParseGUIDKind(guid)
  if type(guid) ~= "string" then return nil end
  for i = 1, #GUID_KIND_PATTERNS do
    ---@type string, "player"|"npc"|"object"|"item"
    local pattern, kind = GUID_KIND_PATTERNS[i][1], GUID_KIND_PATTERNS[i][2]
    if guid:match(pattern) then return kind end
  end
  return nil
end

--- Whether a GUID belongs to a player character. Used to discard any
--- observation (UnitGUID, GetLootSourceInfo, chat message guid args, ...)
--- before it is ever stored.
---@param guid any
---@return boolean
function Core.IsPlayerGUID(guid)
  return Core.ParseGUIDKind(guid) == "player"
end

--- Escape Lua pattern-magic characters so a literal string can be used as a gsub pattern.
---@param str string
---@return string
local function EscapePattern(str)
  return (str:gsub("[%(%)%.%%%+%-%*%?%[%]%^%$]", "%%%1"))
end

--- Collect the set of player names currently known to the client: the
--- local player and, if grouped, the player's raid/party members. Built
--- fresh on every call and never persisted -- it exists purely as an
--- in-memory sanitization dictionary for Core.SanitizeText.
---@return table<string, boolean>
function Core.GetPrivacyNameSet()
  ---@type table<string, boolean>
  local names = {}

  ---@param name any
  local function AddName(name)
    if type(name) == "string" and name ~= "" then
      names[name] = true
    end
  end

  if type(UnitName) == "function" then
    AddName(UnitName("player"))
  end

  if type(IsInRaid) == "function" and IsInRaid() then
    for i = 1, 40 do
      AddName(UnitName("raid" .. i))
    end
  elseif type(IsInGroup) == "function" and IsInGroup() then
    for i = 1, 4 do
      AddName(UnitName("party" .. i))
    end
  end

  return names
end

--- Redact any known player name from a free-text WoW API return value.
--- Used for quest/gossip text and chat message text before storage.
---@param text any
---@return any sanitized Unchanged unless it is a non-empty string.
function Core.SanitizeText(text)
  if type(text) ~= "string" or text == "" then return text end

  local sanitized = text
  for name in pairs(Core.GetPrivacyNameSet()) do
    sanitized = sanitized:gsub(EscapePattern(name), "<name>")
  end
  return sanitized
end

---@type table<number, boolean>
local CHAT_MSG_NAME_ARG_INDICES = { [2] = true, [5] = true }
---@type number
local CHAT_MSG_GUID_ARG_INDEX = 12

--- Scrub the argument shape shared by every `CHAT_MSG_*` event QuestieTrace
--- records: `text, playerName, languageName, channelName, playerName2,
--- specialFlags, zoneChannelID, channelIndex, channelBaseName, languageID,
--- lineID, guid, bnSenderID, isMobile, isSubtitle, hideSenderInLetterbox,
--- supressRaidIcons`.
---
--- The literal `playerName`/`playerName2` values are used to redact any
--- occurrence of that name from `text` (this catches names even when the
--- subject isn't grouped with the local player, e.g. a duel/trade request),
--- then `Core.SanitizeText` is applied as a second pass for the local
--- player/roster. The name args and any player GUID are then discarded.
--- The input is never mutated; a new PackedArgs is returned.
---@param args PackedArgs
---@return PackedArgs sanitized
function Core.SanitizeChatMsgArgs(args)
  local out = Core.CopyPacked(args)

  local text = out[1]
  if type(text) == "string" and text ~= "" then
    for index in pairs(CHAT_MSG_NAME_ARG_INDICES) do
      local name = out[index]
      if type(name) == "string" and name ~= "" then
        text = text:gsub(EscapePattern(name), "<name>")
      end
    end
    text = Core.SanitizeText(text)
  end
  out[1] = text

  for index in pairs(CHAT_MSG_NAME_ARG_INDICES) do
    out[index] = nil
  end

  if Core.IsPlayerGUID(out[CHAT_MSG_GUID_ARG_INDEX]) then
    out[CHAT_MSG_GUID_ARG_INDEX] = nil
  end

  return out
end

---------------------------------------------------------------------------
-- Tracker registration
---------------------------------------------------------------------------

---@type table<string, fun(capture: CaptureState, event: string, ...)[]>
Core._trackerCallbacks = {}
---@type TrackerDef[]
Core._trackers = {}

--- Register a tracker with the event routing system.
---@param tracker TrackerDef
function Core.RegisterTracker(tracker)
  Core._trackers[#Core._trackers + 1] = tracker
  if tracker.events then
    for _, event in ipairs(tracker.events) do
      if not Core._trackerCallbacks[event] then
        Core._trackerCallbacks[event] = {}
      end
      local cbs = Core._trackerCallbacks[event]
      cbs[#cbs + 1] = tracker.OnEvent
    end
  end
end

---------------------------------------------------------------------------
-- Dump registration
---------------------------------------------------------------------------

---@type DumpDef[]
Core._dumps = {}
---@type table<string, DumpDef>
Core._dumpsBySlash = {}
---@type table<string, DumpDef[]>
Core._dumpCallbacks = {}

--- Register a dump provider with optional slash command and event triggers.
---@param dump DumpDef
function Core.RegisterDump(dump)
  if type(dump) ~= "table" or type(dump.key) ~= "string" or type(dump.Run) ~= "function" then
    return
  end

  Core._dumps[#Core._dumps + 1] = dump

  if type(dump.slashCommands) == "table" then
    for i = 1, #dump.slashCommands do
      local slash = string.lower(tostring(dump.slashCommands[i] or ""))
      if slash ~= "" then
        Core._dumpsBySlash[slash] = dump
      end
    end
  end

  if type(dump.events) == "table" then
    for i = 1, #dump.events do
      local event = dump.events[i]
      if type(event) == "string" and event ~= "" then
        if not Core._dumpCallbacks[event] then
          Core._dumpCallbacks[event] = {}
        end
        local callbacks = Core._dumpCallbacks[event]
        callbacks[#callbacks + 1] = dump
      end
    end
  end
end

--- Run dumps registered for a specific event.
---@param event string
---@param ... any
function Core.RunDumpsForEvent(event, ...)
  local callbacks = Core._dumpCallbacks[event]
  if not callbacks then return end
  for i = 1, #callbacks do
    local dump = callbacks[i]
    local ok, err = pcall(dump.Run, event, ...)
    if not ok then
      Core.Debug("Dump failed:", dump.key, err)
    elseif err then
      Core.Debug("Dump warning:", dump.key, err)
    end
  end
end

--- Run a dump via slash command.
---@param action string
---@param ... any
---@return boolean handled
function Core.RunDumpBySlash(action, ...)
  local dump = Core._dumpsBySlash[string.lower(action or "")]
  if not dump then return false end
  local ok, err = pcall(dump.Run, "SLASH", ...)
  if not ok then
    Core.Debug("Dump failed:", dump.key, err)
  elseif err then
    Core.Debug("Dump warning:", dump.key, err)
  end
  return true
end

--- Get additional help lines from registered dump providers.
---@return string[] lines
function Core.GetDumpHelpLines()
  local lines = {}
  for i = 1, #Core._dumps do
    local help = Core._dumps[i].helpText
    if type(help) == "string" and help ~= "" then
      lines[#lines + 1] = help
    end
  end
  table.sort(lines)
  return lines
end

--- Print a debug message if debug mode is enabled.
---@param ... any Values to print
function Core.Debug(...)
  if QuestieTrace and QuestieTrace.settings and QuestieTrace.settings.debug then
    Core.Print("DEBUG:", ...)
  end
end

--- Print a message with colored addon name prefix.
--- Usage: Core.Print("message") or Core.Print("msg1", "msg2", ...)
function Core.Print(...)
  local parts = { ... }
  for i, v in ipairs(parts) do
    parts[i] = tostring(v)
  end
  local message = table.concat(parts, " ")
  print("|cFFFFD100QuestieTrace|r: " .. message)
end
