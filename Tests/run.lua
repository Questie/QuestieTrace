-- Run from the addon root with: lua5.1 Tests/run.lua
-- Real addon code runs in isolated globals; no game or SavedVariables files are used.

---@class TestTimer
---@field at number
---@field callback fun()

---@class TestRuntime
---@field env table<string, any>
---@field core QuestieTraceCore
---@field now number
---@field timers TestTimer[]
---@field frame table<string, any>

---@param runtime TestRuntime
---@param path string
local function LoadAddonFile(runtime, path)
  local chunk = assert(loadfile(path))
  setfenv(chunk, runtime.env)
  chunk("QuestieTrace", runtime.env.QuestLog)
end

---@param trackerFiles string[]
---@return TestRuntime
local function NewRuntime(trackerFiles)
  ---@type TestRuntime
  local runtime = { env = {}, core = {}, now = 0, timers = {}, frame = {} }
  local env = runtime.env
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestLog = {}
  env.SlashCmdList = {}
  env.QuestieTrace = { schemaVersion = 9, settings = { maxSessions = 7, autoStart = false } }
  env.QuestieTraceCharacter = { sessions = {} }
  env.print = function() end
  env.GetLocale = function() return "enUS" end
  env.GetTime = function() return runtime.now end
  env.GetTimePreciseSec = env.GetTime
  env.C_Timer = {
    ---@param delay number
    ---@param callback fun()
    After = function(delay, callback)
      runtime.timers[#runtime.timers + 1] = { at = runtime.now + delay, callback = callback }
    end,
  }
  ---@type fun(frame: table, event: string)
  runtime.frame.RegisterEvent = function() end
  ---@param frame table<string, any>
  ---@param name string
  ---@param callback function
  runtime.frame.SetScript = function(frame, name, callback) frame[name] = callback end
  env.CreateFrame = function() return runtime.frame end

  LoadAddonFile(runtime, "Modules/globals.lua")
  for _, path in ipairs(trackerFiles) do LoadAddonFile(runtime, path) end
  LoadAddonFile(runtime, "QuestieTrace.lua")
  runtime.core = env.QuestieTraceCore
  return runtime
end

---@param runtime TestRuntime
---@param event string
local function SendEvent(runtime, event)
  runtime.frame.OnEvent(runtime.frame, event)
end

---@param runtime TestRuntime
---@param target number
local function AdvanceTo(runtime, target)
  while true do
    local nextIndex
    for index, timer in ipairs(runtime.timers) do
      if timer.at <= target and (not nextIndex or timer.at < runtime.timers[nextIndex].at) then
        nextIndex = index
      end
    end
    if not nextIndex then break end
    local timer = table.remove(runtime.timers, nextIndex)
    runtime.now = timer.at
    timer.callback()
  end
  runtime.now = target
end

---@param runtime TestRuntime
---@return SessionRecord
local function Session(runtime)
  return assert(runtime.core.GetDiagnosticSession())
end

---@class GreetingFixture
---@field count number
---@field phase "live"|"stale"|"error"|"settled"
---@field staleCalls number
---@field totalCalls number

---@param runtime TestRuntime
---@return GreetingFixture
local function GreetingApis(runtime)
  ---@type GreetingFixture
  local state = { count = 2, phase = "live", staleCalls = 0, totalCalls = 0 }
  local env = runtime.env
  env.GetNumActiveQuests = function() return state.count end
  env.GetNumAvailableQuests = env.GetNumActiveQuests
  ---@param index number
  ---@return string? title
  local function Title(index)
    state.totalCalls = state.totalCalls + 1
    if index == 1 then return "First quest" end
    state.staleCalls = state.staleCalls + 1
    if state.phase == "error" then error("Greeting data unavailable") end
    if state.phase == "settled" then return nil end
    return "Second quest"
  end
  env.GetAvailableTitle = Title
  ---@param index number
  ---@return string? title
  ---@return boolean complete
  env.GetActiveTitle = function(index) return Title(index), false end
  return state
end

---@param phase "stale"|"error"
local function TestGreetingRetry(phase)
  local runtime = NewRuntime({ "Modules/Trackers/QuestDialog.lua" })
  local state = GreetingApis(runtime)
  runtime.core.StartCapture("greeting retry")
  AdvanceTo(runtime, 1)

  state.count, state.phase = 1, phase
  SendEvent(runtime, "QUEST_GREETING")
  local functions = Session(runtime).functions
  local active = functions.GetActiveTitle[2]
  local available = functions.GetAvailableTitle[2]
  assert(#active == 1 and #available == 1, "Shrink must not fabricate an inactive value")

  state.phase = "settled"
  AdvanceTo(runtime, 2)
  assert(#active == 2 and active[2].v.n == 2 and active[2].v[1] == nil and active[2].v[2] == false,
    "Delayed samples must observe the removed active title, preserving nil and false")
  assert(#available == 2 and available[2].v == nil,
    "Delayed samples must observe the removed available title")
end

local function TestGreetingClose()
  local runtime = NewRuntime({ "Modules/Trackers/QuestDialog.lua" })
  local state = GreetingApis(runtime)
  runtime.core.StartCapture("close cancellation")
  state.count, state.phase = 0, "stale"
  SendEvent(runtime, "GOSSIP_CLOSED")
  local callsAtClose = state.totalCalls
  state.phase = "settled"
  AdvanceTo(runtime, 2)
  assert(state.totalCalls == callsAtClose, "Close must cancel pending open samples")
  local available = Session(runtime).functions.GetAvailableTitle[2]
  assert(#available == 1 and available[1].v == "Second quest", "Close must not synthesize nil")

  state.count = 1
  SendEvent(runtime, "QUEST_GREETING")
  assert(#available == 2 and available[2].v == nil, "Reopening must still probe known stale indices")
end

local function TestGreetingRestart()
  local runtime = NewRuntime({ "Modules/Trackers/QuestDialog.lua" })
  local state = GreetingApis(runtime)
  runtime.core.StartCapture("old capture")
  local oldSession = Session(runtime)
  AdvanceTo(runtime, 0.05)
  runtime.core.StopCapture()
  state.count = 1
  local oldCalls = state.staleCalls
  runtime.core.StartCapture("new capture")
  local callsAtRestart = state.totalCalls
  -- Old capture timers start at 0.10; new capture timers start at 0.15.
  AdvanceTo(runtime, 0.11)
  assert(state.totalCalls == callsAtRestart, "Old timers must not sample the new capture's valid indices")
  AdvanceTo(runtime, 0.16)
  assert(state.totalCalls == callsAtRestart + 2, "The new capture's own timer must still sample both APIs")
  AdvanceTo(runtime, 2)
  assert(state.staleCalls == oldCalls, "New capture must reset known indices")
  assert(Session(runtime).functions.GetAvailableTitle[2] == nil, "Old indices must not leak into new capture")
  assert(#oldSession.functions.GetAvailableTitle[2] == 1, "Stopped capture must not receive delayed writes")
end

local function TestSessionContract()
  local runtime = NewRuntime({})
  ---@type SessionRecord
  local legacy = {
    schemaVersion = 9, name = "legacy", startedAt = 0, startedAtPrecise = 0,
    events = {}, functions = { GetNumLootItems = { { t = 0, tp = 0, v = 0 } } }, functionsDelta = {},
  }
  local env = runtime.env
  local settings = env.QuestieTrace.settings
  env.QuestieTraceCharacter.sessions[1] = legacy
  SendEvent(runtime, "VARIABLES_LOADED")
  assert(env.QuestieTrace.settings == settings and settings.maxSessions == 7 and settings.autoStart == false,
    "Recording contract must not reset existing settings")

  runtime.core.StartCapture("new capture")
  local current = Session(runtime)
  assert(current.schemaVersion == 9 and current.recordingContractVersion == 1,
    "New captures need contract provenance without a storage schema bump")
  runtime.core.SaveCapture()
  assert(env.QuestieTraceCharacter.sessions[2] == current and current.recordingContractVersion == 1,
    "Saving must retain the recording contract")
  assert(env.QuestieTraceCharacter.sessions[1] == legacy and legacy.recordingContractVersion == nil,
    "Legacy sessions must survive unchanged and unmarked")
end

local function TestSpellBookArity()
  local runtime = NewRuntime({ "Modules/Trackers/SpellBook.lua" })
  local arity = 2
  ---@param slot number
  ---@param bookType string
  ---@return any ...
  runtime.env.GetSpellBookItemName = function(slot, bookType)
    assert(bookType == "spell")
    if slot > 1 then return nil end
    if arity == 2 then return "Fireball", nil end
    if arity == 3 then return "Fireball", nil, 133 end
    return "Fireball", nil, 133, nil
  end
  runtime.core.StartCapture("spell arity")
  local session = Session(runtime)
  local names = session.functions.GetSpellBookItemName[1]
  assert(names[1].v.n == 2 and names[1].v[2] == nil, "Do not pad a two-value return to three")
  arity = 3
  SendEvent(runtime, "SPELLS_CHANGED")
  assert(names[2].v.n == 3 and names[2].v[3] == 133, "Preserve the spell ID return")
  assert(session.functions.SpellBook[2].v[1] == 133, "Synthetic spell membership still uses the third return")
  assert(session.functionsDelta.PlayerKnownSpells.delta[1].add[1] == 133, "Known-spell delta must still update")
  arity = 4
  SendEvent(runtime, "SPELLS_CHANGED")
  assert(names[3].v.n == 4 and names[3].v[4] == nil, "Preserve extra trailing nil returns")
end

local function TestExportScrubsPlayerIdentity()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("export test")
  local session = Session(runtime)
  session.functions.UnitName = {
    player = { { t = 0, tp = 0, v = { "Hero", "Realm", n = 2 } } },
    questnpc = { { t = 0, tp = 0, v = { "Some NPC", nil, n = 2 } } },
  }
  session.functions.UnitGUID = {
    player = { { t = 0, tp = 0, v = "Player-1-000001" } },
    npc = { { t = 0, tp = 0, v = "Creature-0-1-1-1-123-000001" } },
  }
  runtime.core.SaveCapture()

  local payload = runtime.core.BuildExportPayload()
  local exported = payload.sessions[1]
  assert(exported.functions.UnitName.player == nil, "Player name must be scrubbed from export")
  assert(exported.functions.UnitName.questnpc ~= nil, "NPC name must remain in export")
  assert(exported.functions.UnitGUID.player == nil, "Player GUID must be scrubbed from export")
  assert(exported.functions.UnitGUID.npc ~= nil, "NPC GUID must remain in export")

  local savedSession = runtime.env.QuestieTraceCharacter.sessions[1]
  assert(savedSession.functions.UnitName.player ~= nil, "BuildExportPayload must not mutate the saved session")
end

--- The payload handed to the encoder (i.e. what Core.BuildExportString
--- actually serializes) must never contain the real, unscrubbed source
--- session tables or bookkeeping fields like exportedAt -- only the second
--- return value (sourceSessions) may reference them, and that must never be
--- nested inside the payload table itself.
local function TestExportPayloadNeverExposesSourceSessionsOrExportedAt()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("s1")
  local session = Session(runtime)
  session.functions.UnitName = {
    player = { { t = 0, tp = 0, v = { "Hero", "Realm", n = 2 } } },
  }
  session.functions.UnitGUID = {
    player = { { t = 0, tp = 0, v = "Player-1-000001" } },
  }
  runtime.core.SaveCapture()

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  runtime.core.MarkSessionsExported(sourceSessions)

  assert(payload.sourceSessions == nil, "The payload table must never carry the real session references")
  assert(payload.sessions[1].exportedAt == nil, "exportedAt must be stripped from the exported copy")
  assert(sourceSessions[1].exportedAt ~= nil, "MarkSessionsExported must still stamp the real source session")

  -- Simulate what the encoder actually sees: only `payload` is ever passed to
  -- Core.EncodeExportPayload()/serialization, never `sourceSessions`.
  local sawPlayerGuid = false
  local function ScanForPlayerGuid(value)
    if type(value) ~= "table" then return end
    for _, v in pairs(value) do
      if v == "Player-1-000001" then sawPlayerGuid = true end
      ScanForPlayerGuid(v)
    end
  end
  ScanForPlayerGuid(payload)
  assert(not sawPlayerGuid, "The scrubbed player GUID must not be reachable from the payload passed to the encoder")
end

local function TestExportPayloadExcludesAlreadyExportedSessions()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("s1")
  runtime.core.SaveCapture()

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "First build must include the freshly saved session")
  assert(payload.hasExportableData == true, "hasExportableData must be true when a session is included")

  runtime.core.MarkSessionsExported(sourceSessions)

  local secondPayload = runtime.core.BuildExportPayload()
  assert(#secondPayload.sessions == 0, "A session already marked exported must not be bundled again")
  assert(secondPayload.hasExportableData == false, "hasExportableData must be false once nothing new remains")

  local forcedPayload = runtime.core.BuildExportPayload(true)
  assert(#forcedPayload.sessions == 1, "includeAlreadyExported = true must force previously-exported sessions back in")
end

local function TestExportPayloadIncludesOnlyNewSessionsAfterExport()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("s1")
  runtime.core.SaveCapture()
  local _, firstSourceSessions = runtime.core.BuildExportPayload()
  runtime.core.MarkSessionsExported(firstSourceSessions)

  runtime.core.StartCapture("s2")
  runtime.core.SaveCapture()

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "Only the newly saved session must be included")
  assert(sourceSessions[1].name == "s2", "The included session must be the new one, not the already-exported one")
end

local function TestExportPayloadLiveSessionExportedOnce()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("live")
  local session = Session(runtime)
  session.events[1] = { t = 0, tp = 0, e = "TEST_EVENT", a = {} }

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "A live session with events must be included once")

  runtime.core.MarkSessionsExported(sourceSessions)

  local secondPayload = runtime.core.BuildExportPayload()
  assert(#secondPayload.sessions == 0, "The live session must not be re-bundled after being marked exported")

  -- Saving it afterwards must not resurrect it into future payloads either,
  -- since the exportedAt marker carries over onto the saved record.
  runtime.core.SaveCapture()
  local thirdPayload = runtime.core.BuildExportPayload()
  assert(#thirdPayload.sessions == 0, "A saved session that was already exported while live must stay excluded")
end

local function TestExportFinalizesAndRestartsLiveSession()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.env.QuestieTrace.settings.autoStart = true

  -- Start a capture and add events
  runtime.core.StartCapture("test session")
  local oldSession = Session(runtime)
  oldSession.events[1] = { t = 0, tp = 0, e = "TEST_EVENT", a = {} }

  -- Export it (marks it exported)
  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "Live session with events should be in export payload")
  runtime.core.MarkSessionsExported(sourceSessions)

  -- Finalize it (what happens after ShowExportWindow marks sessions exported)
  runtime.core.FinalizeLiveSessionIfExported()

  -- The old session should now be saved
  assert(#runtime.env.QuestieTraceCharacter.sessions == 1, "Finalized session should be saved")
  local savedSession = runtime.env.QuestieTraceCharacter.sessions[1]
  assert(savedSession == oldSession, "Saved session must be the same table reference")
  assert(savedSession.exportedAt ~= nil, "Saved session must retain exportedAt")

  -- A fresh capture should now be running
  local newSession = Session(runtime)
  assert(newSession ~= oldSession, "Fresh capture must be a different session object")
  assert(runtime.core.GetCaptureState() == "running", "After finalize, must be running a fresh capture")
  assert(#newSession.events == 0, "Fresh session must start empty")

  -- The old session should not be re-exported
  local secondPayload = runtime.core.BuildExportPayload()
  assert(#secondPayload.sessions == 0, "Old exported session must not be re-exported after finalize+restart")
end

local function TestPruneRemovesExportedSessionsFirst()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  local maxSessions = runtime.env.QuestieTrace.settings.maxSessions
  for i = 1, maxSessions do
    runtime.core.StartCapture("s" .. i)
    runtime.core.SaveCapture()
  end

  local sessions = runtime.env.QuestieTraceCharacter.sessions
  -- Mark the 3rd and 5th sessions (not the oldest) as already exported.
  sessions[3].exportedAt = 1
  sessions[5].exportedAt = 1

  -- Saving one more session pushes the list over the cap and triggers pruning.
  runtime.core.StartCapture("overflow")
  runtime.core.SaveCapture()

  assert(#sessions == maxSessions, "Pruning must still cap the session list")

  local names = {}
  for i = 1, #sessions do names[sessions[i].name] = true end
  assert(names["s3"] == nil, "An already-exported session must be pruned before never-exported ones")
  assert(names["s1"] == true, "Never-exported sessions must be kept over already-exported ones")
  assert(names["overflow"] == true, "The newly saved session must be present")
end

local function TestExportSerializationRoundTrips()
  local runtime = { env = {}, core = {}, now = 0, timers = {}, frame = {} }
  local env = runtime.env
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestLog = {}
  env.SlashCmdList = {}
  env.QuestieTrace = { schemaVersion = 9, settings = { maxSessions = 7, autoStart = false } }
  env.QuestieTraceCharacter = { sessions = {} }
  env.print = function() end
  env.GetTime = function() return runtime.now end
  env.GetTimePreciseSec = env.GetTime
  env.C_Timer = {
    After = function(delay, callback)
      runtime.timers[#runtime.timers + 1] = { at = runtime.now + delay, callback = callback }
    end,
  }
  runtime.frame.RegisterEvent = function() end
  runtime.frame.SetScript = function(frame, name, callback) frame[name] = callback end
  env.CreateFrame = function() return runtime.frame end

  -- Initialize QuestieTraceCore BEFORE loading any addon files
  env.QuestieTraceCore = {}

  -- Mock C_EncodingUtil BEFORE loading Export.lua
  env.C_EncodingUtil = {
    SerializeCBOR = function(_value)
      -- Mock: return a fake CBOR string (just a marker)
      return "CBOR_ENCODED_DATA"
    end,
    DeserializeCBOR = function(source)
      if source == "CBOR_ENCODED_DATA" then
        return { exportVersion = 1, sessions = { { functions = { GetZoneText = { { t = 0, tp = 0, v = "Dun Morogh" } } } } } }
      end
      return nil
    end,
    CompressString = function(_source, _method, _level)
      -- Mock: return a fake compressed string
      return "DEFLATE_COMPRESSED_DATA"
    end,
    DecompressString = function(source, _method)
      if source == "DEFLATE_COMPRESSED_DATA" then
        return "CBOR_ENCODED_DATA"
      end
      return nil
    end,
  }

  -- Mock Enum compression methods
  env.Enum = {
    CompressionMethod = {
      Deflate = 1,
    },
    CompressionLevel = {
      Default = 1,
    },
  }

  -- Mock LibDeflate BEFORE loading Export.lua
  env.LibDeflate_Instance = {
    EncodeForPrint = function(_self, source)
      return "PRINT:" .. tostring(source)
    end,
    DecodeForPrint = function(_self, source)
      if type(source) == "string" and source:sub(1, 6) == "PRINT:" then
        return source:sub(7)
      end
      return nil
    end,
  }
  env.LibStub = function(name, _optional)
    if name == "LibDeflate" then
      return env.LibDeflate_Instance
    end
    return nil
  end

  -- Now load addon files with mocks in place
  LoadAddonFile(runtime, "Modules/globals.lua")

  LoadAddonFile(runtime, "Modules/Export/Encoding.lua")
  LoadAddonFile(runtime, "Modules/Export/Export.lua")
  LoadAddonFile(runtime, "QuestieTrace.lua")
  runtime.core = env.QuestieTraceCore

  runtime.core.StartCapture("serialize test")
  local session = assert(runtime.core.GetDiagnosticSession())
  session.functions.GetZoneText = { { t = 0, tp = 0, v = "Dun Morogh" } }
  runtime.core.SaveCapture()

  local text = runtime.core.BuildExportString()
  -- text should be a compressed/encoded string, not raw Lua
  assert(type(text) == "string" and #text > 0, "Export string must be non-empty")
  -- Verify it starts with the version marker, followed by the print-encoding marker
  assert(text:match("^!QuestieTrace:%d+!"), "Export must start with the version marker")
  assert(text:find("PRINT:", 1, true), "Export must be print-encoded")
end

local function TestCurrentSessionLinkedOnStartCapture()
  local runtime = NewRuntime({})
  runtime.core.StartCapture("link test")
  local session = Session(runtime)
  assert(runtime.env.QuestieTraceCharacter.currentSession == session, "StartCapture must link currentSession to the same table as capture.session")
end

local function TestSaveCaptureClearsCurrentSession()
  local runtime = NewRuntime({})
  runtime.core.StartCapture("save test")
  runtime.core.SaveCapture()
  assert(runtime.env.QuestieTraceCharacter.currentSession == nil, "SaveCapture must clear currentSession")
  assert(runtime.env.QuestieTraceCharacter.sessions[1] ~= nil, "Session must be moved to sessions array")
end


local function TestRecoverCurrentSessionOnVariablesLoaded()
  local runtime = NewRuntime({})
  local env = runtime.env
  -- Simulate a leftover currentSession from a previous load (e.g. /reload without Save)
  local leftover = {
    schemaVersion = 9,
    recordingContractVersion = 1,
    name = "recovered",
    startedAt = 100,
    startedAtPrecise = 100,
    events = { { t = 101, e = "TEST_EVENT" } },
    functions = {},
    functionsDelta = {},
    -- stoppedAt/duration intentionally missing to test recovery fills them
  }
  env.QuestieTraceCharacter.currentSession = leftover

  -- Set virtual time to a value > startedAt so duration is non-negative
  runtime.now = 150

  -- Trigger VARIABLES_LOADED which calls EnsureSavedVariables and auto-finalizes the recovered session
  SendEvent(runtime, "VARIABLES_LOADED")

  -- The recovered session should now be in sessions[] (auto-finalized), not in capture.session
  assert(#env.QuestieTraceCharacter.sessions == 1, "Recovered session must be auto-finalized into sessions[]")
  local recovered = env.QuestieTraceCharacter.sessions[1]
  assert(recovered == leftover, "Recovered session must be the same table reference")
  assert(recovered.stoppedAt == 150, "Recovery must fill stoppedAt with current virtual time")
  assert(recovered.duration == 50, "Recovery must compute correct non-negative duration (150 - 100)")
  assert(recovered.durationPrecise == 50, "Recovery must compute correct durationPrecise")
  assert(runtime.core.GetCaptureState() == "idle", "After recovery finalization, state must be idle")
  assert(env.QuestieTraceCharacter.currentSession == nil, "currentSession must be cleared after finalization")
end

local function TestAutoStartAfterRecoveryStartsFreshCapture()
  local runtime = NewRuntime({})
  local env = runtime.env
  -- Simulate a leftover currentSession from a previous load
  local leftover = {
    schemaVersion = 9,
    recordingContractVersion = 1,
    name = "recovered",
    startedAt = 100,
    startedAtPrecise = 100,
    events = { { t = 101, e = "TEST_EVENT" } },
    functions = {},
    functionsDelta = {},
  }
  env.QuestieTraceCharacter.currentSession = leftover
  -- Enable autoStart
  env.QuestieTrace.settings.autoStart = true

  -- Trigger VARIABLES_LOADED (recovery auto-finalizes)
  SendEvent(runtime, "VARIABLES_LOADED")

  -- Recovered session should be saved
  assert(#env.QuestieTraceCharacter.sessions == 1, "Recovered session must be saved")
  local recoveredSession = env.QuestieTraceCharacter.sessions[1]
  assert(recoveredSession == leftover, "Recovered session in array must be same reference")
  assert(#recoveredSession.events == 1, "Recovered session events must be preserved")

  -- Trigger PLAYER_LOGIN (auto-start logic should start a fresh capture)
  SendEvent(runtime, "PLAYER_LOGIN")

  -- A fresh capture should now be running, different from the recovered session
  local currentSession = runtime.core.GetDiagnosticSession()
  assert(currentSession ~= leftover, "Auto-start must create a fresh capture, not reuse the recovered one")
  assert(runtime.core.GetCaptureState() == "running", "After auto-start on login, state must be running")
  assert(#env.QuestieTraceCharacter.sessions == 1, "Sessions array must still have only the recovered one (not the new capture yet)")
end

---@type string[] Files a share-reminder runtime needs on top of globals.lua.
local REMINDER_FILES = {
  "Modules/Localization/l10n.lua",
  "Modules/Localization/Translations/ExportReminder.lua",
  "Modules/Export/ExportReminder.lua",
}

---@param runtime TestRuntime
---@return string[] messages Captured chat output, appended to as reminders fire.
local function CaptureChat(runtime)
  ---@type string[]
  local messages = {}
  runtime.env.DEFAULT_CHAT_FRAME = {
    ---@param _ table
    ---@param message string
    AddMessage = function(_, message) messages[#messages + 1] = message end,
  }
  return messages
end

--- Save `count` sessions back to back through the real capture lifecycle.
---@param runtime TestRuntime
---@param count number
local function SaveSessions(runtime, count)
  for i = 1, count do
    runtime.core.StartCapture("session " .. i)
    runtime.core.SaveCapture()
  end
end

local function TestShareReminderNotDueWithoutSavedSessions()
  local runtime = NewRuntime(REMINDER_FILES)
  SendEvent(runtime, "VARIABLES_LOADED")
  assert(runtime.core.IsShareDue() == false, "No saved sessions means nothing is shareable yet")

  -- A live, unsaved capture is not in the export payload and must stay silent.
  runtime.core.StartCapture("live only")
  assert(runtime.core.IsShareDue() == false, "An unsaved live session must not trigger a reminder")
end

local function TestShareReminderDueAfterSave()
  local runtime = NewRuntime(REMINDER_FILES)
  SendEvent(runtime, "VARIABLES_LOADED")
  SaveSessions(runtime, 1)
  assert(runtime.core.IsShareDue() == true, "A saved session must make sharing due")
end

local function TestShareReminderSuppressedAfterExportOpened()
  local runtime = NewRuntime(REMINDER_FILES)
  SendEvent(runtime, "VARIABLES_LOADED")
  SaveSessions(runtime, 1)
  runtime.core.MarkExportOpened()
  assert(runtime.core.IsShareDue() == false, "Opening the export window must pause reminders")
end

local function TestShareReminderResumesAfterNewSave()
  local runtime = NewRuntime(REMINDER_FILES)
  SendEvent(runtime, "VARIABLES_LOADED")
  SaveSessions(runtime, 1)
  runtime.core.MarkExportOpened()
  SaveSessions(runtime, 1)
  assert(runtime.core.IsShareDue() == true, "New data after an export must re-arm the reminder")
end

local function TestShareReminderSurvivesSessionPruning()
  local runtime = NewRuntime(REMINDER_FILES)
  SendEvent(runtime, "VARIABLES_LOADED")
  local env = runtime.env
  ---@type number
  local maxSessions = env.QuestieTrace.settings.maxSessions

  -- Fill to the prune cap, then acknowledge, then save more. #sessions stops
  -- growing here, so only the monotonic counter can still detect new data.
  SaveSessions(runtime, maxSessions)
  runtime.core.MarkExportOpened()
  SaveSessions(runtime, 2)

  assert(#env.QuestieTraceCharacter.sessions == maxSessions, "Pruning must still cap the session list")
  assert(env.QuestieTraceCharacter.savedSessionCounter == maxSessions + 2,
    "The saved-session counter must keep counting past the prune cap")
  assert(runtime.core.IsShareDue() == true, "Pruning must not permanently suppress reminders")
end

local function TestShareReminderFiresOnLoginAndAtThirtyMinutes()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SendEvent(runtime, "VARIABLES_LOADED")
  SaveSessions(runtime, 1)

  SendEvent(runtime, "PLAYER_LOGIN")
  AdvanceTo(runtime, 9)
  assert(#messages == 0, "No reminder before the login delay elapses")

  AdvanceTo(runtime, 10)
  assert(#messages == 1, "The login reminder must fire once the delay elapses")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil,
    "The reminder must carry the clickable export hyperlink")

  AdvanceTo(runtime, 1810)
  assert(#messages == 2, "The reminder must repeat 30 minutes later")

  -- Opening the export window silences the next tick, without stopping the loop.
  runtime.core.MarkExportOpened()
  AdvanceTo(runtime, 3610)
  assert(#messages == 2, "An acknowledged reminder must stay silent")

  SaveSessions(runtime, 1)
  AdvanceTo(runtime, 5410)
  assert(#messages == 3, "The still-running loop must fire again once new data is saved")
end

---@type { name: string, run: fun() }[]
local tests = {
  { name = "greeting retries unsettled titles", run = function() TestGreetingRetry("stale") end },
  { name = "greeting retries failed calls", run = function() TestGreetingRetry("error") end },
  { name = "greeting close cancels delayed samples", run = TestGreetingClose },
   { name = "greeting capture restart resets probes", run = TestGreetingRestart },
   { name = "session contract preserves legacy saves", run = TestSessionContract },
   { name = "spellbook preserves observed tuple arity", run = TestSpellBookArity },
   { name = "export scrubs player identity", run = TestExportScrubsPlayerIdentity },
   { name = "export payload never exposes sourceSessions or exportedAt", run = TestExportPayloadNeverExposesSourceSessionsOrExportedAt },
   { name = "export excludes already-exported sessions", run = TestExportPayloadExcludesAlreadyExportedSessions },
   { name = "export includes only new sessions after export", run = TestExportPayloadIncludesOnlyNewSessionsAfterExport },
   { name = "export live session is exported only once", run = TestExportPayloadLiveSessionExportedOnce },
   { name = "export finalizes and restarts live session after export", run = TestExportFinalizesAndRestartsLiveSession },
   { name = "prune removes exported sessions first", run = TestPruneRemovesExportedSessionsFirst },
   { name = "export serialization round-trips", run = TestExportSerializationRoundTrips },
   { name = "currentSession linked on StartCapture", run = TestCurrentSessionLinkedOnStartCapture },
   { name = "SaveCapture clears currentSession", run = TestSaveCaptureClearsCurrentSession },
   { name = "recover currentSession on VARIABLES_LOADED and auto-finalize", run = TestRecoverCurrentSessionOnVariablesLoaded },
   { name = "autoStart after recovery starts fresh capture", run = TestAutoStartAfterRecoveryStartsFreshCapture },
   { name = "share reminder silent without saved sessions", run = TestShareReminderNotDueWithoutSavedSessions },
   { name = "share reminder due after a save", run = TestShareReminderDueAfterSave },
   { name = "share reminder paused by opening export", run = TestShareReminderSuppressedAfterExportOpened },
   { name = "share reminder resumes after new save", run = TestShareReminderResumesAfterNewSave },
   { name = "share reminder survives session pruning", run = TestShareReminderSurvivesSessionPruning },
   { name = "share reminder fires on login and every 30 minutes", run = TestShareReminderFiresOnLoginAndAtThirtyMinutes },

}

local failures = 0
for _, test in ipairs(tests) do
  local ok, err = pcall(test.run)
  print((ok and "PASS " or "FAIL ") .. test.name)
  if not ok then
    failures = failures + 1
    print(err)
  end
end
assert(failures == 0, failures .. " test(s) failed")
print(#tests .. " tests passed")
