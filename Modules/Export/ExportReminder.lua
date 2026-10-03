QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

---------------------------------------------------------------------------
-- Share reminder (chat notification pointing at the export window)
---------------------------------------------------------------------------
-- Eligibility and scheduling only. The export window lives in ExportUI.lua
-- and the payload in Export.lua; this module never reads or writes session
-- data, only the small per-character reminder watermark.
---------------------------------------------------------------------------

local C_After = C_Timer.After

---@type string
local ADDON_NAME = "QuestieTrace"
---@type number Seconds to wait after login before the first check, so the
--- message is not buried in login and addon load spam.
local LOGIN_DELAY = 10
---@type number Seconds between reminder checks while playing.
local REMINDER_INTERVAL = 1800
---@type number Minimum age in seconds before a live (unsaved) session counts as
--- shareable. Auto-start records login events immediately, so without this a
--- freshly logged-in character would be reminded 10 seconds after login.
local LIVE_SESSION_MIN_AGE = 900
---@type string Custom chat hyperlink type owned by this addon.
local LINK_TYPE = "questietrace"
---@type string Hyperlink option identifying the export action.
local LINK_ACTION = "export"
---@type number Minimum number of meaningful (non-noise) events required for a
--- live session to be considered shareable.
local LIVE_SESSION_MIN_MEANINGFUL_EVENTS = 1

---@type table<string, boolean> Event names that DO count as meaningful gameplay
--- (quest, loot, combat xp/rep, NPC interaction, skill/level, zone changes,
--- currency, trade skills).
local MEANINGFUL_EVENTS = {
  -- Quest events
  QUEST_LOG_UPDATE = true,
  QUEST_ACCEPTED = true,
  QUEST_TURNED_IN = true,
  QUEST_GREETING = true,
  QUEST_DETAIL = true,
  QUEST_PROGRESS = true,
  QUEST_COMPLETE = true,
  -- Loot events
  CHAT_MSG_LOOT = true,
  LOOT_READY = true,
  LOOT_CLOSED = true,
  -- Combat XP/reputation
  CHAT_MSG_COMBAT_XP_GAIN = true,
  CHAT_MSG_COMBAT_FACTION_CHANGE = true,
  UPDATE_FACTION = true,
  -- NPC interaction
  GOSSIP_SHOW = true,
  GOSSIP_CLOSED = true,
  -- Skill/Level
  CHAT_MSG_SKILL = true,
  PLAYER_LEVEL_UP = true,
  NEW_RECIPE_LEARNED = true,
  -- Zone changes
  ZONE_CHANGED = true,
  ZONE_CHANGED_NEW_AREA = true,
  ZONE_CHANGED_INDOORS = true,
  -- Currency
  CURRENCY_DISPLAY_UPDATE = true,
  -- Trade skills
  CHAT_MSG_TRADESKILLS = true,
}

--- Check if a session has at least the minimum number of meaningful (non-noise)
--- events. Only meaningful events count; noise events like login, UI, map,
--- group, achievement, item, and combat state changes are ignored.
---@param session SessionRecord
---@return boolean hasMeaningful
local function HasMeaningfulEvents(session)
  if type(session.events) ~= "table" then return false end
  local count = 0
  for _, eventRecord in ipairs(session.events) do
    if type(eventRecord) == "table" and type(eventRecord.e) == "string" then
      local eventName = eventRecord.e
      if MEANINGFUL_EVENTS[eventName] then
        count = count + 1
        if count >= LIVE_SESSION_MIN_MEANINGFUL_EVENTS then
          return true
        end
      end
    end
  end
  return false
end

---@class ReminderState
---@field lastExportAt number Timestamp (GetTime()) when the export window was last opened.

---@type boolean Set once the repeating check has been scheduled for this load.
local scheduled = false
---@type boolean Set once a hyperlink handler has been installed for this load.
local linkHandlerRegistered = false

---------------------------------------------------------------------------
-- Per-character state
---------------------------------------------------------------------------

--- Read the per-character reminder state, initializing missing fields.
---
--- Returns a detached table when SavedVariables are not ready yet so callers
--- never fail; in practice EnsureSavedVariables runs on VARIABLES_LOADED,
--- well before the first check.
---@return ReminderState
local function GetReminderState()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" then
    return { lastExportAt = 0 }
  end

  if type(characterDb.reminder) ~= "table" then
    characterDb.reminder = {}
  end

  ---@type ReminderState
  local reminder = characterDb.reminder
  if type(reminder.lastExportAt) ~= "number" or reminder.lastExportAt > GetTime() then
    reminder.lastExportAt = 0
  end

  return reminder
end

--- Count all saved sessions.
---@return number count
local function GetSavedSessionCount()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" or type(characterDb.sessions) ~= "table" then
    return 0
  end
  return #characterDb.sessions
end

--- Has the live (unsaved) session been running long enough, with events, to
--- be worth sharing? A player who has been playing for hours without saving
--- should still get prompted, and shouldn't lose that data to a crash before
--- /qlt save runs. Sessions younger than LIVE_SESSION_MIN_AGE only hold login
--- noise and must not trigger a reminder.
---@return boolean shareable
local function IsLiveSessionShareable()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" then return false end

  ---@type SessionRecord?
  local currentSession = characterDb.currentSession
  if type(currentSession) ~= "table" or type(currentSession.events) ~= "table" then
    return false
  end

  if #currentSession.events == 0 or type(currentSession.startedAt) ~= "number" then
    return false
  end

  if GetTime() - currentSession.startedAt < LIVE_SESSION_MIN_AGE then
    return false
  end

  if not HasMeaningfulEvents(currentSession) then
    return false
  end

  return true
end

--- Check if any saved session was completed after the last export window open
--- (or if never exported, any saved session with meaningful events) and
--- contains meaningful events.
---@return boolean hasNewSavedData
local function HasNewSavedData()
  local reminder = GetReminderState()
  local lastExportAt = reminder.lastExportAt

  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" or type(characterDb.sessions) ~= "table" then
    return false
  end

  for _, session in ipairs(characterDb.sessions) do
    if type(session) == "table"
        and type(session.stoppedAt) == "number"
        and HasMeaningfulEvents(session) then
      if lastExportAt == 0 or session.stoppedAt > lastExportAt then
        return true
      end
    end
  end
  return false
end

---------------------------------------------------------------------------
-- Eligibility
---------------------------------------------------------------------------

--- Is there saved data the player has not been prompted about since their
--- last visit to the export window?
---
--- The live (unsaved) session is checked separately: it's due once it holds
--- meaningful events and has run for at least LIVE_SESSION_MIN_AGE.
---
--- Saved sessions are checked by timestamp: any session with stoppedAt >
--- lastExportAt and meaningful events is considered new.
---
--- Extension point: a trace-size rule belongs here as a further condition.
---@return boolean due
function Core.IsShareDue()
  if IsLiveSessionShareable() then
    return true
  end

  if GetSavedSessionCount() == 0 then
    return false
  end

  return HasNewSavedData()
end

--- Record that the player opened the export window, pausing reminders until
--- another session with meaningful data is saved. Called from Core.ShowExportWindow().
function Core.MarkExportOpened()
  GetReminderState().lastExportAt = GetTime()
end

---------------------------------------------------------------------------
-- Chat message
---------------------------------------------------------------------------

--- Build the clickable chat hyperlink that opens the export window.
---@return string link
local function BuildExportLink()
  return string.format(
    "|cFF33FF99|H%s:%s|h[%s]|h|r",
    LINK_TYPE,
    LINK_ACTION,
    l10n("Click here to open the export window")
  )
end

--- Print the reminder to the default chat frame.
local function ShowReminder()
  ---@type table?
  local chatFrame = DEFAULT_CHAT_FRAME
  if type(chatFrame) ~= "table" then return end

  chatFrame:AddMessage(string.format(
    "|cFFFFD100%s|r: %s",
    ADDON_NAME,
    l10n("It is time to share your trace data again. Each submission only covers what happened so far, so please keep submitting regularly. %s", BuildExportLink())
  ))
end

---------------------------------------------------------------------------
-- Scheduling
---------------------------------------------------------------------------

--- Run one eligibility check and schedule the next one.
---
--- The tick reschedules unconditionally: data that is not shareable now can
--- become shareable later in the same play session.
local function Tick()
  if Core.IsShareDue() then
    ShowReminder()
  end
  C_After(REMINDER_INTERVAL, Tick)
end

--- Start the login and repeating share reminders. Idempotent within one load.
function Core.StartShareReminders()
  if scheduled then return end
  scheduled = true
  C_After(LOGIN_DELAY, Tick)
end

---------------------------------------------------------------------------
-- Hyperlink handling
---------------------------------------------------------------------------

--- Open the export window if `link` is this addon's export hyperlink.
---@param link any The hyperlink body, e.g. "questietrace:export"
---@return boolean handled
local function HandleExportLink(link)
  if type(link) ~= "string" then return false end

  ---@type string?, string?
  local linkType, action = link:match("^([^:]+):([^:|]+)")
  if linkType ~= LINK_TYPE or action ~= LINK_ACTION then
    return false
  end

  Core.ShowExportWindow()

  return true
end

--- Install the questietrace: hyperlink handler.
---
--- LinkUtil is preferred: the click is consumed, so SetItemRef never falls
--- through to ItemRefTooltip. Clients without LinkUtil fall back to a secure
--- hook, where the stock handler has already opened an empty tooltip for our
--- unknown link type and we hide it again. SetItemRef is never replaced,
--- which would risk taint.
local function RegisterLinkHandler()
  if linkHandlerRegistered then return end

  if type(LinkUtil) == "table"
      and type(LinkUtil.RegisterLinkHandler) == "function"
      and type(LinkUtil.IsLinkHandlerRegistered) == "function"
      and (not LinkUtil.IsLinkHandlerRegistered(LINK_TYPE)) then
    LinkUtil.RegisterLinkHandler(LINK_TYPE, function(link)
      HandleExportLink(link)
    end)
    linkHandlerRegistered = true
    return
  end

  if type(hooksecurefunc) == "function" then
    hooksecurefunc("SetItemRef", function(link)
      if HandleExportLink(link) and type(ItemRefTooltip) == "table" then
        ItemRefTooltip:Hide()
      end
    end)
    linkHandlerRegistered = true
  end
end

RegisterLinkHandler()
