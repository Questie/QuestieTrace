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
---@type string Custom chat hyperlink type owned by this addon.
local LINK_TYPE = "questietrace"
---@type string Hyperlink option identifying the export action.
local LINK_ACTION = "export"

---@class ReminderState
---@field sessionCounterAtExport number Value of savedSessionCounter when the export window was last opened.

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
    return { sessionCounterAtExport = 0 }
  end

  if type(characterDb.reminder) ~= "table" then
    characterDb.reminder = {}
  end

  ---@type ReminderState
  local reminder = characterDb.reminder
  if type(reminder.sessionCounterAtExport) ~= "number" then
    reminder.sessionCounterAtExport = 0
  end

  return reminder
end

--- Read the monotonic saved-session counter.
---
--- This counter only ever increases, unlike #sessions which is capped by
--- PruneSessionsIfNeeded(). Using the session count here would permanently
--- suppress reminders for any character sitting at maxSessions.
---@return number counter
local function GetSavedSessionCounter()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" then return 0 end

  if type(characterDb.savedSessionCounter) == "number" then
    return characterDb.savedSessionCounter
  end

  return type(characterDb.sessions) == "table" and #characterDb.sessions or 0
end

--- Count all saved sessions, regardless of whether Core.BuildExportPayload()
--- would actually include them (it skips ones already marked exported via
--- `exportedAt`). This is only used to gate whether *any* saved data exists
--- at all, not whether new/unexported data exists -- that's what
--- `savedSessionCounter` vs `reminder.sessionCounterAtExport` is for below.
---@return number count
local function GetSavedSessionCount()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" or type(characterDb.sessions) ~= "table" then
    return 0
  end
  return #characterDb.sessions
end

--- Does the live (unsaved) session hold any events Core.BuildExportPayload()
--- would actually include? Mirrors the eligibility check in
--- Core.BuildExportPayload() (has events, not already exported) so a player
--- who has been playing for hours without saving still gets prompted, and
--- doesn't lose that data to a crash before /qlt save runs.
---@return boolean hasEvents
local function HasUnexportedLiveSessionEvents()
  ---@type table?
  local characterDb = QuestieTraceCharacter
  if type(characterDb) ~= "table" then return false end

  ---@type SessionRecord?
  local currentSession = characterDb.currentSession
  if type(currentSession) ~= "table" or type(currentSession.events) ~= "table" then
    return false
  end

  return #currentSession.events > 0 and not currentSession.exportedAt
end

---------------------------------------------------------------------------
-- Eligibility
---------------------------------------------------------------------------

--- Is there saved data the player has not been prompted about since their
--- last visit to the export window?
---
--- Sessions in QuestieTraceCharacter.sessions are gated by the saved-session
--- counter watermark. The live (unsaved) session is checked separately: it
--- has no counter of its own, so it's due whenever it holds unexported
--- events, regardless of whether anything has been saved yet.
---
--- Extension point: a trace-size rule belongs here as a further condition.
---@return boolean due
function Core.IsShareDue()
  if HasUnexportedLiveSessionEvents() then
    return true
  end

  if GetSavedSessionCount() == 0 then
    return false
  end
  return GetSavedSessionCounter() > GetReminderState().sessionCounterAtExport
end

--- Record that the player opened the export window, pausing reminders until
--- another session is saved. Called from Core.ShowExportWindow().
function Core.MarkExportOpened()
  GetReminderState().sessionCounterAtExport = GetSavedSessionCounter()
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
--- unknown link type and we hide it again. hooksecurefunc is a documented,
--- taint-safe post-hook mechanism (it never introduces taint into the
--- hooked function itself); SetItemRef is never replaced outright, which
--- would risk taint.
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

-- Defer registration to PLAYER_LOGIN to ensure LinkUtil and chat frames are
-- reliably available. RegisterLinkHandler() is idempotent so it's safe to
-- call multiple times.
local loginFrame = CreateFrame("Frame")
loginFrame:RegisterEvent("PLAYER_LOGIN")
loginFrame:SetScript("OnEvent", function(_, event)
  if event == "PLAYER_LOGIN" then
    RegisterLinkHandler()
  end
end)
