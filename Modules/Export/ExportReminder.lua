QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type l10n
local l10n = Core.l10n

---------------------------------------------------------------------------
-- Export reminder (inn-based, no login/interval spam)
---------------------------------------------------------------------------
-- Reminds the player to export when they have been playing for at least
-- one hour AND then enter a resting area (Inn). A new timer starts after
-- each reminder. Login starts the timer; leaving and re-entering an inn
-- after 60+ minutes triggers the reminder.
---------------------------------------------------------------------------

local C_After = C_Timer.After

---@type string
local ADDON_NAME = "QuestieTrace"
---@type number Minimum play time in seconds before a reminder can fire (1 hour).
local MIN_PLAY_TIME = 3600
---@type string Custom chat hyperlink type owned by this addon.
local LINK_TYPE = "questietrace"
---@type string Hyperlink option identifying the export action.
local LINK_ACTION = "export"

---@class ReminderState
---@field sessionStart number GetTime() when the current play session began (capture started).
---@field lastReminderAt number GetTime() when the last reminder was shown (0 = never).

---@type boolean Set once the event frame has been registered for this load.
local frameRegistered = false

---------------------------------------------------------------------------
-- Per-character state
---------------------------------------------------------------------------

--- Read the per-character reminder state, initializing missing fields.
---@return ReminderState
local function GetReminderState()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" then
    return { sessionStart = 0, lastReminderAt = 0 }
  end

  if type(characterDb.reminder) ~= "table" then
    characterDb.reminder = {}
  end

  ---@type ReminderState
  local reminder = characterDb.reminder
  if type(reminder.sessionStart) ~= "number" or reminder.sessionStart > GetTime() then
    reminder.sessionStart = 0
  end
  if type(reminder.lastReminderAt) ~= "number" or reminder.lastReminderAt > GetTime() then
    reminder.lastReminderAt = 0
  end

  return reminder
end

---------------------------------------------------------------------------
-- Eligibility
---------------------------------------------------------------------------

--- Check if the player has been playing long enough for a reminder.
---@param sessionStart number
---@return boolean
local function HasPlayedLongEnough(sessionStart)
  return GetTime() - sessionStart >= MIN_PLAY_TIME
end

--- Event names that count as meaningful gameplay (not login/UI noise).
---@type table<string, boolean>
local MEANINGFUL_EVENTS = {
  QUEST_LOG_UPDATE = true, QUEST_ACCEPTED = true, QUEST_TURNED_IN = true,
  QUEST_GREETING = true, QUEST_DETAIL = true, QUEST_PROGRESS = true,
  QUEST_COMPLETE = true, CHAT_MSG_LOOT = true, LOOT_READY = true,
  LOOT_CLOSED = true, CHAT_MSG_COMBAT_XP_GAIN = true,
  CHAT_MSG_COMBAT_FACTION_CHANGE = true, UPDATE_FACTION = true,
  GOSSIP_SHOW = true, GOSSIP_CLOSED = true, CHAT_MSG_SKILL = true,
  PLAYER_LEVEL_UP = true, NEW_RECIPE_LEARNED = true, ZONE_CHANGED = true,
  ZONE_CHANGED_NEW_AREA = true, ZONE_CHANGED_INDOORS = true,
  CURRENCY_DISPLAY_UPDATE = true, CHAT_MSG_TRADESKILLS = true,
}

--- Check if a session has at least one meaningful event.
---@param session SessionRecord
---@return boolean
local function HasMeaningfulEvents(session)
  if type(session.events) ~= "table" then return false end
  for _, eventRecord in ipairs(session.events) do
    if type(eventRecord) == "table" and type(eventRecord.e) == "string" then
      if MEANINGFUL_EVENTS[eventRecord.e] then
        return true
      end
    end
  end
  return false
end

--- Check if there is shareable data (saved sessions or live session with meaningful events).
---@return boolean
local function HasShareableData()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" then return false end

  -- Check saved sessions (any saved session is shareable)
  if type(characterDb.sessions) == "table" and #characterDb.sessions > 0 then
    return true
  end

  -- Check live session for meaningful events
  ---@type SessionRecord?
  local currentSession = characterDb.currentSession
  if type(currentSession) == "table"
      and type(currentSession.events) == "table"
      and #currentSession.events > 0
      and type(currentSession.startedAt) == "number"
      and HasMeaningfulEvents(currentSession) then
    return true
  end

  return false
end

--- Is a reminder due right now?
---@return boolean due
function Core.IsShareDue()
  local reminder = GetReminderState()

  -- Must have played long enough since last reminder (or session start)
  local sinceLast = reminder.lastReminderAt == 0 and reminder.sessionStart or reminder.lastReminderAt
  if not HasPlayedLongEnough(sinceLast) then
    return false
  end

  -- Must have shareable data
  if not HasShareableData() then
    return false
  end

  return true
end

--- Record that the player opened the export window, pausing reminders until
--- another session with meaningful data is saved.
function Core.MarkExportOpened()
  GetReminderState().lastReminderAt = GetTime()
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
-- Resting state handling
---------------------------------------------------------------------------

---@type boolean True if the initial login was in a resting area (for timer scheduling).
local loggedInAtInn = false

--- Called when the player enters a resting area (IsResting() becomes true).
---@param isLogin boolean True if this is the initial login
local function OnEnterResting(isLogin)
  -- Check if reminder is due (uses lastReminderAt, not sessionStart)
  if Core.IsShareDue() then
    ShowReminder()
    GetReminderState().lastReminderAt = GetTime()
    return
  end

  -- Only schedule a timer if this is the initial login in an inn
  if isLogin then
    loggedInAtInn = true
    local reminder = GetReminderState()
    local remaining = MIN_PLAY_TIME - (GetTime() - reminder.sessionStart)
    if remaining > 0 then
      C_After(remaining, function()
        -- Only fire if still in a resting area and has shareable data
        if loggedInAtInn and IsResting() and HasShareableData() then
          if Core.IsShareDue() then
            ShowReminder()
            GetReminderState().lastReminderAt = GetTime()
          end
        end
      end)
    end
  end
end

--- Called when the player leaves a resting area (IsResting() becomes false).
local function OnLeaveResting()
  loggedInAtInn = false
end

--- Initialize the reminder state on login.
local function InitReminderState()
  local reminder = GetReminderState()
  reminder.sessionStart = GetTime()
  reminder.lastReminderAt = 0
end

---------------------------------------------------------------------------
-- Event frame
---------------------------------------------------------------------------

local eventFrame = CreateFrame("Frame")

---@param event string
local function OnEvent(_, event)
  if event == "PLAYER_ENTERING_WORLD" then
    -- Check immediately in case we're already in a resting area at login
    if IsResting() then
      OnEnterResting(true)
    end
  elseif event == "PLAYER_UPDATE_RESTING" then
    if IsResting() then
      OnEnterResting(false)
    else
      OnLeaveResting()
    end
  end
end

--- Start the export reminder system. Idempotent within one load.
function Core.StartShareReminders()
  if frameRegistered then return end
  frameRegistered = true

  InitReminderState()

  eventFrame:RegisterEvent("PLAYER_ENTERING_WORLD")
  eventFrame:RegisterEvent("PLAYER_UPDATE_RESTING")
  eventFrame:SetScript("OnEvent", OnEvent)
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
local linkHandlerRegistered = false

local function RegisterLinkHandler()
  if linkHandlerRegistered then return end
  linkHandlerRegistered = true

  if type(LinkUtil) == "table"
      and type(LinkUtil.RegisterLinkHandler) == "function"
      and type(LinkUtil.IsLinkHandlerRegistered) == "function"
      and (not LinkUtil.IsLinkHandlerRegistered(LINK_TYPE)) then
    LinkUtil.RegisterLinkHandler(LINK_TYPE, function(link)
      HandleExportLink(link)
    end)
    return
  end

  if type(hooksecurefunc) == "function" then
    hooksecurefunc("SetItemRef", function(link)
      if HandleExportLink(link) and type(ItemRefTooltip) == "table" then
        ItemRefTooltip:Hide()
      end
    end)
  end
end

RegisterLinkHandler()
