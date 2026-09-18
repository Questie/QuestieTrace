QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

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

--- Normalize UnitName return values into a flat list of name parts.
--- Handles both patterns:
---   - UnitName("player") -> "First Last" (single return, full name)
---   - UnitName("partyX") -> "First", "Last" (two returns)
--- If Blizzard ever makes UnitName("player") return two values like party units,
--- this captures both. Also splits the first return by spaces as a fallback.
---@param token string
---@return string[] parts
local function GetUnitNameParts(token)
  if type(UnitName) ~= "function" then return {} end
  local first, second = UnitName(token)
  local parts = {}
  if type(first) == "string" and first ~= "" then
    parts[#parts + 1] = first
    for part in first:gmatch("%S+") do
      parts[#parts + 1] = part
    end
  end
  if type(second) == "string" and second ~= "" then
    parts[#parts + 1] = second
  end
  return parts
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

  -- Local player
  for _, part in ipairs(GetUnitNameParts("player")) do
    AddName(part)
  end

  -- Party/raid members
  if type(IsInRaid) == "function" and IsInRaid() then
    for i = 1, 40 do
      for _, part in ipairs(GetUnitNameParts("raid" .. i)) do
        AddName(part)
      end
    end
  elseif type(IsInGroup) == "function" and IsInGroup() then
    for i = 1, 4 do
      for _, part in ipairs(GetUnitNameParts("party" .. i)) do
        AddName(part)
      end
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

  local guidKind = Core.ParseGUIDKind(out[CHAT_MSG_GUID_ARG_INDEX])
  if guidKind ~= "npc" and guidKind ~= "object" and guidKind ~= "item" then
    out[CHAT_MSG_GUID_ARG_INDEX] = nil
  end

  return out
end
