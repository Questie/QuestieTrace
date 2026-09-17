QuestieTraceCore = QuestieTraceCore or {}

---@class QuestieTraceCore
local Core = QuestieTraceCore

---@type string
local ADDON_NAME = "QuestieTrace"
---@type number
local SCHEMA_VERSION = 9
---@type number
local DEFAULT_MAX_SESSIONS = 20

---------------------------------------------------------------------------
-- Capture state
---------------------------------------------------------------------------

---@type CaptureState
local capture = {
  active = false,
  token = 0,
  startedAt = nil,        -- GetTime() baseline
  startedAtPrecise = nil, -- GetTimePreciseSec() baseline
  session = nil,          -- the session record being built
}

---------------------------------------------------------------------------
-- All events the addon registers for (union of all tracker events + extras)
---------------------------------------------------------------------------

---@type EventCategory[]
local TRACKED_EVENT_CATEGORIES = {
  {
    name = "quest_state",
    events = {
      "QUEST_LOG_UPDATE",
      "QUEST_ACCEPTED",
      "QUEST_REMOVED",
      "QUEST_TURNED_IN",
      "QUEST_WATCH_UPDATE",
      "UNIT_QUEST_LOG_CHANGED",
      "QUEST_AUTOCOMPLETE",
      "QUEST_POI_UPDATE",
      "QUEST_ITEM_UPDATE",
      "QUEST_LOG_CRITERIA_UPDATE",
      "QUEST_DATA_LOAD_RESULT",
      "QUEST_WATCH_LIST_CHANGED",
      "QUESTLINE_UPDATE",
      "TASK_PROGRESS_UPDATE",
    },
  },
  {
    name = "quest_dialog",
    events = {
      "QUEST_DETAIL",
      "QUEST_PROGRESS",
      "QUEST_COMPLETE",
      "QUEST_FINISHED",
      "QUEST_GREETING",
      "QUEST_ACCEPT_CONFIRM",
      "GOSSIP_SHOW",
      "GOSSIP_CLOSED",
    },
  },
  {
    name = "npc_interaction",
    events = {
      "MERCHANT_SHOW",
      "MERCHANT_CLOSED",
      "TRAINER_SHOW",
      "TRAINER_CLOSED",
      "MAIL_SHOW",
      "MAIL_CLOSED",
      "AUCTION_HOUSE_SHOW",
      "AUCTION_HOUSE_CLOSED",
      "BANKFRAME_OPENED",
      "BANKFRAME_CLOSED",
      "TAXIMAP_OPENED",
      "TAXIMAP_CLOSED",
      "GUILD_REGISTRAR_SHOW",
      "GUILD_REGISTRAR_CLOSED",
      "PET_STABLE_SHOW",
      "PET_STABLE_CLOSED",
      "BATTLEFIELDS_SHOW",
      "BATTLEFIELDS_CLOSED",
      "PETITION_SHOW",
      "PETITION_CLOSED",
      "GUILDBANKFRAME_OPENED",
      "GUILDBANKFRAME_CLOSED",
    },
  },
  {
    name = "initialization",
    events = {
      "ADDON_LOADED",
      "SPELLS_CHANGED",
      "PLAYER_LOGOUT",
      "PLAYER_LEAVING_WORLD",
      "LOADING_SCREEN_DISABLED",
    },
  },
  {
    name = "player_state",
    events = {
      "PLAYER_LOGIN",
      "PLAYER_ENTERING_WORLD",
      "PLAYER_ALIVE",
      "PLAYER_LEVEL_UP",
      "MODIFIER_STATE_CHANGED",
      "PLAYER_REGEN_DISABLED",
      "PLAYER_REGEN_ENABLED",
      "PLAYER_TARGET_CHANGED",
      "PLAYER_EQUIPMENT_CHANGED",
      "SKILL_LINES_CHANGED",
      "LOOT_OPENED",
      "LOOT_READY",
      "LOOT_CLOSED",
      "NEW_RECIPE_LEARNED",
      "UI_INFO_MESSAGE",
      "CURRENCY_DISPLAY_UPDATE",
    },
  },
  {
    name = "map_zone",
    events = {
      "ZONE_CHANGED",
      "ZONE_CHANGED_NEW_AREA",
      "ZONE_CHANGED_INDOORS",
      "MAP_EXPLORATION_UPDATED",
      "PLAYER_MAP_CHANGED",
      "WORLD_MAP_OPEN",
      "AREA_POIS_UPDATED",
      "NEW_WMO_CHUNK",
      "UPDATE_ALL_UI_WIDGETS",
    },
  },
  {
    name = "chat_system",
    events = {
      "CHAT_MSG_SYSTEM",
      "CHAT_MSG_LOOT",
      "CHAT_MSG_MONEY",
      "CHAT_MSG_SKILL",
      "CHAT_MSG_TRADESKILLS",
      "CHAT_MSG_COMBAT_FACTION_CHANGE",
      "CHAT_MSG_COMBAT_XP_GAIN",
    },
  },
  {
    name = "group_world",
    events = {
      "GROUP_JOINED",
      "GROUP_LEFT",
      "GROUP_ROSTER_UPDATE",
      "NAME_PLATE_UNIT_ADDED",
      "NAME_PLATE_UNIT_REMOVED",
      "ACHIEVEMENT_EARNED",
      "TRACKED_ACHIEVEMENT_LIST_CHANGED",
      "TRACKED_ACHIEVEMENT_UPDATE",
      "CRITERIA_UPDATE",
      "UPDATE_FACTION",
    },
  },
  {
    name = "inventory",
    events = {
      -- "BAG_UPDATE",
      -- "BAG_UPDATE_DELAYED",
      "ITEM_PUSH",
      "ITEM_LOCK_CHANGED",
      "ITEM_COUNT_CHANGED",
    },
  },
}

-- Build flat deduplicated event list
---@type string[]
local TRACKED_EVENTS = {}
do
  ---@type table<string, boolean>
  local seen = {}
  for _, category in ipairs(TRACKED_EVENT_CATEGORIES) do
    for _, event in ipairs(category.events) do
      if not seen[event] then
        seen[event] = true
        TRACKED_EVENTS[#TRACKED_EVENTS + 1] = event
      end
    end
  end
end
Core.TRACKED_EVENT_CATEGORIES = TRACKED_EVENT_CATEGORIES

---------------------------------------------------------------------------
-- Event filters (skip events that don't match criteria)
---------------------------------------------------------------------------

local EVENT_FILTERS = {
  ---@param addonName string
  ---@return boolean
  ADDON_LOADED = function(addonName)
    return addonName == ADDON_NAME
  end,
}

---------------------------------------------------------------------------
-- Helpers
---------------------------------------------------------------------------

--- Trim whitespace from both ends of a string.
---@param s any The value to trim (returns empty string for non-strings)
---@return string trimmed
local function Trim(s)
  if type(s) ~= "string" then return "" end
  return (s:gsub("^%s+", ""):gsub("%s+$", ""))
end

--- Create a session name from an override or generate one from the current date.
---@param override any The optional name override
---@return string name
local function CreateSessionName(override)
  local candidate = Trim(override)
  if candidate ~= "" then return candidate end
  ---@diagnostic disable-next-line: return-type-mismatch
  return date("%Y-%m-%d_%H-%M-%S")
end

---------------------------------------------------------------------------
-- SavedVariables
---------------------------------------------------------------------------

--- Ensure SavedVariables tables exist and match the current schema version.
local function EnsureSavedVariables()
  ---@type table?
  local globalDb = QuestieTrace
  if type(globalDb) ~= "table" or globalDb.schemaVersion ~= SCHEMA_VERSION then
    QuestieTrace = {
      schemaVersion = SCHEMA_VERSION,
      settings = {
        maxSessions = DEFAULT_MAX_SESSIONS,
      },
    }
  end

  QuestieTrace.settings = type(QuestieTrace.settings) == "table" and QuestieTrace.settings or {}
  if type(QuestieTrace.settings.maxSessions) ~= "number" or QuestieTrace.settings.maxSessions < 1 then
    QuestieTrace.settings.maxSessions = DEFAULT_MAX_SESSIONS
  end

  if QuestieTrace.settings.autoStart == nil then
    QuestieTrace.settings.autoStart = true
  end

  if QuestieTrace.settings.debug == nil then
    QuestieTrace.settings.debug = false
  end

  if type(QuestieTraceDumps) ~= "table" then
    QuestieTraceDumps = {
      schemaVersion = 1,
      dumps = {},
    }
  end
  if type(QuestieTraceDumps.dumps) ~= "table" then
    QuestieTraceDumps.dumps = {}
  end

  QuestieTraceCharacter = type(QuestieTraceCharacter) == "table" and QuestieTraceCharacter or {}
  QuestieTraceCharacter.sessions = type(QuestieTraceCharacter.sessions) == "table" and QuestieTraceCharacter.sessions or {}

  -- Share-reminder state. Lives on the per-character table, which is shape-checked
  -- rather than schema-gated, so no SCHEMA_VERSION bump is needed (a bump would
  -- reset every user's settings). savedSessionCounter is monotonic: it must not be
  -- replaced with #sessions, which PruneSessionsIfNeeded caps at maxSessions.
  if type(QuestieTraceCharacter.savedSessionCounter) ~= "number" then
    QuestieTraceCharacter.savedSessionCounter = #QuestieTraceCharacter.sessions
  end
  QuestieTraceCharacter.reminder = type(QuestieTraceCharacter.reminder) == "table" and QuestieTraceCharacter.reminder or {}
  if type(QuestieTraceCharacter.reminder.sessionCounterAtExport) ~= "number" then
    QuestieTraceCharacter.reminder.sessionCounterAtExport = 0
  end

  -- Recover an unsaved session left over from a previous load (e.g. /reload
  -- or logout without explicit Save). Auto-finalize it into sessions[]
  -- immediately so the normal PLAYER_LOGIN flow starts fresh.
  if QuestieTraceCharacter.currentSession then
    capture.session = QuestieTraceCharacter.currentSession
    if (not capture.session.stoppedAt) then
      capture.session.stoppedAt = GetTime()
      capture.session.stoppedAtPrecise = GetTimePreciseSec()
      capture.session.duration = capture.session.stoppedAt - capture.session.startedAt
      capture.session.durationPrecise = capture.session.stoppedAtPrecise - capture.session.startedAtPrecise
    end
    -- Finalize it into sessions[] right away
    Core.SaveCapture()
  end
end

--- Remove oldest sessions if the count exceeds the configured maximum.
---
--- Already-exported sessions are removed first (oldest-first among them),
--- since their data has already been shared. Only once those are exhausted
--- does pruning fall back to removing never-exported sessions, so a slow or
--- infrequent exporter doesn't lose data they haven't had a chance to share.
local function PruneSessionsIfNeeded()
  ---@type number
  local maxSessions = QuestieTrace.settings.maxSessions
  ---@type SessionRecord[]
  local sessions = QuestieTraceCharacter.sessions

  while #sessions > maxSessions do
    ---@type number?
    local removeIndex
    for i = 1, #sessions do
      if sessions[i].exportedAt then
        removeIndex = i
        break
      end
    end
    table.remove(sessions, removeIndex or 1)
  end
end

---------------------------------------------------------------------------
-- Status data (consumed by UI)
---------------------------------------------------------------------------

--- Get the current capture state as a string label.
---@return "running"|"stopped_unsaved"|"idle"
function Core.GetCaptureState()
  if capture.active then
    return "running"
  elseif capture.session then
    return "stopped_unsaved"
  else
    return "idle"
  end
end

--- Get status data for UI consumption.
---@return StatusData
function Core.GetStatusData()
  return {
    captureState = Core.GetCaptureState(),
    isRunning = capture.active,
    sessionName = capture.session and (capture.session.name or "(unnamed)") or "None",
    eventCount = capture.session and #capture.session.events or 0,
    canSave = capture.session ~= nil and not capture.active,
  }
end

--- Return the live or newest saved session for external diagnostics.
---
--- This is intended for read-only bridge/testing inspection. The returned
--- session table is the actual live/saved table, not a defensive copy; callers
--- MUST treat it as read-only by convention.
---@return SessionRecord? session The active/stopped unsaved session, newest saved session, or nil.
---@return "active"|"stopped_unsaved"|"saved"|"none" source Where the session came from.
function Core.GetDiagnosticSession()
  if capture.session then
    return capture.session, capture.active and "active" or "stopped_unsaved"
  end

  local characterDb = QuestieTraceCharacter
  if type(characterDb) == "table" and type(characterDb.sessions) == "table" then
    local session = characterDb.sessions[#characterDb.sessions]
    if type(session) == "table" then
      return session, "saved"
    end
  end

  return nil, "none"
end

---------------------------------------------------------------------------
-- Session lifecycle
---------------------------------------------------------------------------

--- Start a new capture session.
---@param sessionName string? Optional name for the session
function Core.StartCapture(sessionName)
  if capture.active then
    return
  end

  local settings = QuestieTrace and QuestieTrace.settings
  if not (settings and settings.dataCollectionConsent == true) then
    Core.Print(Core.l10n("Data collection is disabled. Use /qlt consent to change this."))
    return
  end

  capture.token = capture.token + 1
  capture.startedAt = GetTime()
  capture.startedAtPrecise = GetTimePreciseSec()

  capture.session = {
    schemaVersion = SCHEMA_VERSION,
    -- Mark new captures only; existing v9 sessions may contain synthetic resets.
    recordingContractVersion = 1,
    name = Trim(sessionName) ~= "" and Trim(sessionName) or nil,
    startedAt        = capture.startedAt,
    startedAtPrecise = capture.startedAtPrecise,
    events = {},
    functions = {},
    functionsDelta = {},
  }

  -- Link the in-memory session into SavedVariables so it survives /reload
  -- and logout without an explicit Save. Since trackers mutate the table
  -- in place, a single reference assignment is sufficient.
  QuestieTraceCharacter.currentSession = capture.session

  capture.active = true

  -- Initialize all registered trackers
  for i = 1, #Core._trackers do
    local tracker = Core._trackers[i]
    if tracker.Init then
      tracker.Init(capture)
    end
  end
end

--- Stop the active capture session.
function Core.StopCapture()
  if not capture.active or not capture.session then
    return
  end

  capture.session.stoppedAt        = GetTime()
  capture.session.stoppedAtPrecise = GetTimePreciseSec()
  capture.session.duration         = capture.session.stoppedAt - capture.session.startedAt
  capture.session.durationPrecise  = capture.session.stoppedAtPrecise - capture.session.startedAtPrecise

  capture.active = false

  -- Notify trackers that capture has stopped
  for i = 1, #Core._trackers do
    local tracker = Core._trackers[i]
    if tracker.OnCaptureStopped then
      tracker.OnCaptureStopped(capture)
    end
  end
end

--- Stop and discard the current capture without saving it to sessions[].
--- Used only when consent is revoked mid-capture (see Modules/Consent.lua),
--- since data collected up to that point must not be persisted once declined.
function Core.DiscardCapture()
  if capture.active then
    Core.StopCapture()
  end
  capture.session = nil
  QuestieTraceCharacter.currentSession = nil
end

--- Save the current capture session to SavedVariables.
---@param nameOverride string? Optional name override for the session
function Core.SaveCapture(nameOverride)
  if capture.active then
    Core.StopCapture()
  end

  if not capture.session then
    return
  end

  ---@type SessionRecord
  local session = capture.session
  session.name = CreateSessionName(nameOverride or session.name)

  -- Ensure stop timestamps exist
  if not session.stoppedAt then
    session.stoppedAt        = GetTime()
    session.stoppedAtPrecise = GetTimePreciseSec()
    session.duration         = session.stoppedAt - session.startedAt
    session.durationPrecise  = session.stoppedAtPrecise - session.startedAtPrecise
  end

  -- The session IS the record — events, functions, functionsDelta are already
  -- populated in-place by the trackers. No serialization step needed.
  QuestieTraceCharacter.sessions[#QuestieTraceCharacter.sessions + 1] = session
  QuestieTraceCharacter.lastSavedSession = session.name
  -- Monotonic across pruning; the share reminder compares it to its watermark.
  QuestieTraceCharacter.savedSessionCounter = (QuestieTraceCharacter.savedSessionCounter or 0) + 1

  PruneSessionsIfNeeded()

  capture.session = nil
  QuestieTraceCharacter.currentSession = nil
end

--- Finalize and restart a live session if it was just exported.
--- Called after an export to save the just-exported session and immediately
--- start a fresh one, so new events don't get added to an already-exported record.
function Core.FinalizeLiveSessionIfExported()
  if capture.session and capture.session.exportedAt then
    Core.SaveCapture()
    if QuestieTrace.settings.autoStart then
      Core.StartCapture()
    end
  end
end

---------------------------------------------------------------------------
-- Event processing
---------------------------------------------------------------------------

--- Process a tracked event: record it and dispatch to registered tracker callbacks.
---@param event string The event name
---@param ... any Event arguments
local function ProcessTrackedEvent(event, ...)
  if not capture.active or not capture.session then return end

  -- Record raw event
  ---@type number
  local t  = GetTime()          - capture.startedAt
  ---@type number
  local tp = GetTimePreciseSec() - capture.startedAtPrecise
  ---@type PackedArgs
  local packedArgs = Core.CopyPacked(Core.PackArgs(...))

  capture.session.events[#capture.session.events + 1] = {
    t = t,
    tp = tp,
    e = event,
    a = packedArgs,
  }

  -- Dispatch to registered tracker callbacks
  ---@type fun(capture: CaptureState, event: string, ...)[]?
  local callbacks = Core._trackerCallbacks[event]
  if callbacks then
    for i = 1, #callbacks do
      callbacks[i](capture, event, ...)
    end
  end
end

---------------------------------------------------------------------------
-- Slash commands
---------------------------------------------------------------------------

--- Print the current capture status to chat.
local function PrintStatus()
  ---@type StatusData
  local state = Core.GetStatusData()
  ---@type string
  local running = state.isRunning and "running" or "stopped"
  Core.Print("status:", running, "session:", state.sessionName, "events:", state.eventCount)
end

--- Print the available slash commands to chat.
local function PrintHelp()
  print("/qlt status - Show capture status")
  print("/qlt tracking - Toggle data collection on login")
  print("/qlt consent - Show the data collection consent prompt")
  print("/qlt debug - Toggle debug prints")
  print("/qlt export - Show the export window")
  print("/qlt export all - Re-show the export window including previously exported sessions (e.g. if a submission failed)")
  if Core.GetDumpHelpLines then
    local dumpHelpLines = Core.GetDumpHelpLines()
    for i = 1, #dumpHelpLines do
      print(dumpHelpLines[i])
    end
  end
end

---@param msg string? The slash command arguments
SlashCmdList["QUESTIETRACE"] = function(msg)
  ---@type string, string?
  local action, argument = strsplit(" ", msg or "", 2)
  action = string.lower(action or "")

  if action == "" or action == "help" then
    PrintHelp()
  elseif action == "status" then
    PrintStatus()
  elseif action == "tracking" then
    QuestieTrace.settings.autoStart = not QuestieTrace.settings.autoStart
    print(ADDON_NAME, "Tracking:", QuestieTrace.settings.autoStart and "enabled" or "disabled")

    -- Apply toggle immediately
    if QuestieTrace.settings.autoStart then
      -- Enable tracking: start a new capture if none is running
      if not capture.active and not capture.session then
        Core.StartCapture()
      end
    else
      -- Disable tracking: save and stop any active capture
      if capture.active or capture.session then
        Core.SaveCapture()
      end
    end
  elseif action == "consent" then
    Core.ShowConsentPrompt()
  elseif action == "debug" then
    QuestieTrace.settings.debug = not QuestieTrace.settings.debug
    Core.Print("Debug prints:", QuestieTrace.settings.debug and "enabled" or "disabled")
  elseif action == "export" then
    if string.lower(Trim(argument)) == "all" then
      Core.ShowExportWindow(true)
    else
      Core.ShowExportWindow()
    end
  elseif Core.RunDumpBySlash(action, argument) then
    -- handled by dump provider
  else
    Core.Print("Unknown command:", action)
    PrintHelp()
  end
end

SLASH_QUESTIETRACE1 = "/questietrace"
SLASH_QUESTIETRACE2 = "/qlt"

---------------------------------------------------------------------------
-- Bootstrap
---------------------------------------------------------------------------

--- Main event handler for the addon's event frame.
---@param _ Frame
---@param event string
---@param ... any
local function OnEvent(_, event, ...)
  -- 1. Initialization (unchanged)
  if event == "VARIABLES_LOADED" then
    EnsureSavedVariables()
    return
  end

  -- 2. Event filtering
  local filter = EVENT_FILTERS[event]
  if filter and not filter(...) then return end

  -- 3. Dump providers + auto-start on PLAYER_LOGIN
  --    StartCapture BEFORE ProcessTrackedEvent so PLAYER_LOGIN
  --    is recorded as the first event in the session.
  if Core.RunDumpsForEvent then
    Core.RunDumpsForEvent(event, ...)
  end
  if event == "PLAYER_LOGIN" then
    local settings = QuestieTrace and QuestieTrace.settings
    local consent = settings and settings.dataCollectionConsent
    if consent == nil then
      -- First login: ask for permission before collecting anything.
      if Core.ShowConsentPrompt then
        Core.ShowConsentPrompt()
      end
    elseif consent == true then
      if Core.PrintConsentReminder then
        Core.PrintConsentReminder()
      end
      -- Only auto-start if no capture is running AND no recovered session exists
      -- (capture.session would be set by EnsureSavedVariables recovery)
      if (not capture.active) and (not capture.session) then
        if settings.autoStart ~= false then
          Core.StartCapture()
        end
      end
      if Core.StartShareReminders then
        Core.StartShareReminders()
      end
    end
    -- consent == false: declined; do not prompt, message, or auto-start.
  end

  -- 4. Process the event (record + dispatch to trackers)
  ProcessTrackedEvent(event, ...)

  -- 5. Auto-save on PLAYER_LOGOUT
  --    ProcessTrackedEvent runs first so the event is recorded
  --    in the session before saving.
  if event == "PLAYER_LOGOUT" and capture.active then
    Core.SaveCapture()
  end
end

---@type Frame
local eventFrame = CreateFrame("Frame")
eventFrame:RegisterEvent("VARIABLES_LOADED")
for i = 1, #TRACKED_EVENTS do
  ---@type boolean
  local ok = pcall(eventFrame.RegisterEvent, eventFrame, TRACKED_EVENTS[i])
  if not ok then
    Core.Debug("Skipping unsupported event:", TRACKED_EVENTS[i])
  end
end
eventFrame:SetScript("OnEvent", OnEvent)
