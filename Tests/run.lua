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

-- WoW creates these controls from Widgets/Dialog.xml and Modules/Consent.xml. Tests exercise
-- the Lua bindings; XML loading and taint still need in-client validation.
local function NewConsentFrame()
  local function SetText(self, text) self.text = text end
  local function SetScript(self, name, callback) self[name] = callback end
  local function FontString(height)
    return {
      stringHeight = height,
      SetText = SetText,
      SetFontObject = function(self, font) self.font = font end,
      GetStringHeight = function(self) return self.stringHeight end,
    }
  end
  local function Button()
    return {
      textWidth = 24,
      Text = FontString(12),
      SetText = SetText,
      SetScript = SetScript,
      SetNormalFontObject = function(self, font) self.normalFont = font end,
      SetHighlightFontObject = function(self, font) self.highlightFont = font end,
      SetDisabledFontObject = function(self, font) self.disabledFont = font end,
      GetTextWidth = function(self) return self.textWidth end,
      GetFontString = function(self) return self.Text end,
      SetSize = function(self, width, height) self.width, self.height = width, height end,
    }
  end
  local function Texture()
    return {
      SetAtlas = function(self, atlas) self.atlas = atlas end,
      Show = function(self) self.shown = true end,
      Hide = function(self) self.shown = false end,
    }
  end
  return {
    shown = false,
    Background = Texture(),
    Border = Texture(),
    Text = FontString(100),
    AcceptButton = Button(),
    DeclineButton = Button(),
    SetBackdrop = function(self, backdrop) self.backdrop = backdrop end,
    SetWidth = function(self, width) self.width = width end,
    SetHeight = function(self, height) self.height = height end,
    GetWidth = function(self) return self.width end,
    GetHeight = function(self) return self.height end,
    GetEffectiveScale = function() return 1 end,
    ClearAllPoints = function() end,
    SetPoint = function(self, ...) self.point = { ... } end,
    Hide = function(self) self.shown = false end,
    Show = function(self)
      self.shown = true
      self.OnShow(self)
    end,
  }
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
  env.QuestieTrace = { schemaVersion = 13, settings = { dataCollectionConsent = true } }
  env.QuestieTraceCharacter = { sessions = {} }
  runtime.printedMessages = {}
  env.print = function(...)
    local parts = { ... }
    for i, v in ipairs(parts) do parts[i] = tostring(v) end
    runtime.printedMessages[#runtime.printedMessages + 1] = table.concat(parts, " ")
  end
  env.GetLocale = function() return "enUS" end
  env.GetTime = function() return runtime.now end
  env.GetTimePreciseSec = env.GetTime
  env.date = function() return "2024-01-01_12-00-00" end
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

  env.YES = "Yes"
  env.NO = "No"
  env.GameFontHighlight = {}
  env.GameFontNormal = {}
  env.GameFontDisable = {}
  env.BACKDROP_DIALOG_32_32 = {}
  env.UIParent = {
    GetRect = function() return 0, 0, 1920, 1080 end,
    GetEffectiveScale = function() return 1 end,
  }
  runtime.consentFrame = NewConsentFrame()

  LoadAddonFile(runtime, "Modules/globals.lua")
  LoadAddonFile(runtime, "Modules/Privacy.lua")
  LoadAddonFile(runtime, "Modules/Localization/l10n.lua")
  LoadAddonFile(runtime, "Modules/Localization/Translations/Consent.lua")
  LoadAddonFile(runtime, "Widgets/Dialog.lua")
  for key, value in pairs(env.QuestieTraceDialogMixin) do runtime.consentFrame[key] = value end
  runtime.consentFrame:OnLoad()
  LoadAddonFile(runtime, "Modules/Consent.lua")
  env.QuestieTraceCore.OnConsentFrameLoad(runtime.consentFrame)
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

--- Mock IsResting() for reminder tests
---@param runtime TestRuntime
---@param value boolean
local function MockIsResting(runtime, value)
  runtime.env.IsResting = function() return value end
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

local function TestGreetingQuestIDs()
  local runtime = NewRuntime({ "Modules/Trackers/QuestDialog.lua" })
  GreetingApis(runtime)
  runtime.env.GetActiveQuestID = function(index) return 100 + index end
  ---@param index number
  runtime.env.GetAvailableQuestInfo = function(index) return true, 1, false, false, 300 + index, false, false, nil end
  runtime.core.StartCapture("greeting ids")
  SendEvent(runtime, "QUEST_GREETING")
  AdvanceTo(runtime, 2)

  local functions = Session(runtime).functions
  assert(#functions.GetActiveQuestID[2] == 1 and functions.GetActiveQuestID[2][1].v == 102,
    "Active greeting quest IDs are recorded per index, change-only")
  local info = functions.GetAvailableQuestInfo[2]
  assert(#info == 1 and info[1].v.n == 8 and info[1].v[1] == true and info[1].v[5] == 302,
    "Available greeting info keeps the raw tuple per index, change-only")
end

---@class QuestFlagFixture
---@field runtime TestRuntime
---@field dialogQuestID number Returned by GetQuestID
---@field classification number
---@field questLineHidden boolean

---Runtime with the Forever-only quest-flag APIs mocked. Quest 200 is a
---breadcrumb, quest 300 has no quest line, and IsBreadcrumbQuest errors for
---quest 300. GetQuestLineInfo returns a fresh table on every call.
---@return QuestFlagFixture
local function QuestFlagRuntime()
  local runtime = NewRuntime({ "Modules/Trackers/QuestDialog.lua" })
  ---@type QuestFlagFixture
  local state = { runtime = runtime, dialogQuestID = 0, classification = 7, questLineHidden = false }
  local env = runtime.env
  env.GetQuestID = function() return state.dialogQuestID end
  ---@param questID number
  env.IsBreadcrumbQuest = function(questID)
    if questID == 300 then error("Breadcrumb data unavailable") end
    return questID == 200
  end
  env.C_QuestLine = {
    ---@param questID number
    GetQuestLineInfo = function(questID)
      if questID == 300 then return nil end
      return { questLineID = 7, questLineName = "Quest line", questID = questID, isHidden = state.questLineHidden }
    end,
  }
  env.C_QuestInfoSystem = { GetQuestClassification = function() return state.classification end }
  return state
end

local function TestQuestFlagsSampledForDialogQuests()
  local flags = QuestFlagRuntime()
  local runtime, env = flags.runtime, flags.runtime.env
  local gossipQuests, greetingCount = {}, 0
  env.C_GossipInfo = { GetAvailableQuests = function() return gossipQuests end }
  env.GetNumAvailableQuests = function() return greetingCount end
  env.GetAvailableQuestInfo = function() return false, 1, false, false, 400, false end
  runtime.core.StartCapture("quest flag sources")
  local functions = Session(runtime).functions
  assert(functions["C_QuestInfoSystem.GetQuestClassification"] == nil, "Quest ID 0 (no dialog quest) is not sampled")

  flags.dialogQuestID = 100
  SendEvent(runtime, "QUEST_DETAIL")
  flags.dialogQuestID, gossipQuests = 0, { { questID = 200 } }
  SendEvent(runtime, "GOSSIP_SHOW")
  gossipQuests, greetingCount = {}, 1
  SendEvent(runtime, "QUEST_GREETING")

  for _, key in ipairs({ "IsBreadcrumbQuest", "C_QuestLine.GetQuestLineInfo", "C_QuestInfoSystem.GetQuestClassification" }) do
    for _, questID in ipairs({ 100, 200, 400 }) do
      assert(#functions[key][questID] == 1, key .. " must be sampled for quest " .. questID)
    end
  end
  assert(functions.IsBreadcrumbQuest[200][1].v == true, "Flags are recorded under the quest's own ID")
end

local function TestQuestFlagsNilAndFailedCalls()
  local flags = QuestFlagRuntime()
  flags.dialogQuestID = 300
  flags.runtime.core.StartCapture("quest flag nil")
  AdvanceTo(flags.runtime, 2)

  local functions = Session(flags.runtime).functions
  local questLine = functions["C_QuestLine.GetQuestLineInfo"][300]
  assert(#questLine == 1 and questLine[1].v == nil, "A real nil return is recorded once")
  assert(functions.IsBreadcrumbQuest == nil, "A failed call records nothing")
  assert(functions["C_QuestInfoSystem.GetQuestClassification"][300][1].v == 7,
    "A failing flag API does not block the others")
end

local function TestQuestFlagsChangeOnly()
  local flags = QuestFlagRuntime()
  flags.dialogQuestID = 100
  flags.runtime.core.StartCapture("quest flag changes")
  AdvanceTo(flags.runtime, 2)

  local functions = Session(flags.runtime).functions
  local classification = functions["C_QuestInfoSystem.GetQuestClassification"][100]
  local questLine = functions["C_QuestLine.GetQuestLineInfo"][100]
  assert(#classification == 1, "An unchanged scalar flag is not re-appended")
  assert(#questLine == 1, "An identical (fresh) quest-line table is not re-appended")

  flags.classification, flags.questLineHidden = 2, true
  SendEvent(flags.runtime, "QUEST_DETAIL")
  assert(#classification == 2 and classification[2].v == 2, "A changed scalar flag is appended")
  assert(#questLine == 2 and questLine[2].v.isHidden == true, "A changed quest-line field is appended")
end

local function TestQuestDialogSanitizesTitlesAndQuestLineNames()
  local flags = QuestFlagRuntime()
  local env = flags.runtime.env
  env.UnitName = function(token) if token == "player" then return "Arthas" end end
  env.GetNumActiveQuests = function() return 1 end
  env.GetNumAvailableQuests = env.GetNumActiveQuests
  env.GetActiveTitle = function() return "Report to Arthas", false end
  env.GetAvailableTitle = function() return "Arthas needs help" end
  local apiInfo = { questLineID = 7, questLineName = "The Arthas line", questName = "Arthas rising", isHidden = false }
  env.C_QuestLine.GetQuestLineInfo = function() return apiInfo end
  flags.dialogQuestID = 100
  flags.runtime.core.StartCapture("quest dialog sanitization")

  local functions = Session(flags.runtime).functions
  local active = functions.GetActiveTitle[1][1].v
  assert(active.n == 2 and active[1] == "Report to <name>" and active[2] == false, "Active titles are sanitized")
  assert(functions.GetAvailableTitle[1][1].v == "<name> needs help", "Available titles are sanitized")
  local info = functions["C_QuestLine.GetQuestLineInfo"][100][1].v
  assert(info.questLineName == "The <name> line" and info.questName == "<name> rising", "Quest-line names are sanitized")
  assert(info.questLineID == 7 and info.isHidden == false, "Non-text quest-line fields stay raw")
  assert(info ~= apiInfo and apiInfo.questLineName == "The Arthas line", "The API's table is copied, not modified")
end

local function TestQuestFlagsAbsentWithoutApis()
  local runtime = NewRuntime({ "Modules/Trackers/QuestDialog.lua" })
  GreetingApis(runtime)
  local env = runtime.env
  local gossipReads = 0
  env.GetQuestID = function() return 100 end
  env.C_GossipInfo = {
    GetAvailableQuests = function()
      gossipReads = gossipReads + 1
      return { { questID = 200 } }
    end,
  }
  -- Older clients return no questID from GetAvailableQuestInfo.
  env.GetAvailableQuestInfo = function() return false, 1, false, false end
  runtime.core.StartCapture("no flag apis")
  SendEvent(runtime, "QUEST_GREETING")
  SendEvent(runtime, "GOSSIP_SHOW")
  AdvanceTo(runtime, 2)

  local functions = Session(runtime).functions
  assert(functions.GetAvailableQuestInfo[1][1].v.n == 4, "Short greeting info tuples are still recorded raw")
  assert(functions.GetActiveQuestID == nil, "An absent GetActiveQuestID creates no stream")
  for _, key in ipairs({ "IsBreadcrumbQuest", "C_QuestLine.GetQuestLineInfo", "C_QuestInfoSystem.GetQuestClassification" }) do
    assert(functions[key] == nil, key .. " must not be recorded when its API is absent")
  end
  assert(gossipReads == 0, "Without flag APIs the gossip list is not read for quest IDs")
end

local function TestSessionContract()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  MockIsResting(runtime, false)
  ---@type SessionRecord
  local legacy = {
    schemaVersion = 10, name = "legacy", startedAt = 0, startedAtPrecise = 0,
    events = {}, functions = { GetNumLootItems = { { t = 0, tp = 0, v = 0 } } }, functionsDelta = {},
  }
  local env = runtime.env
  local settings = env.QuestieTrace.settings
  env.QuestieTraceCharacter.sessions[1] = legacy
  SendEvent(runtime, "PLAYER_LOGIN")
  assert(env.QuestieTrace.settings == settings,
    "Recording contract must not reset existing settings")

  runtime.core.StartCapture("new capture")
  local current = Session(runtime)
  assert(current.schemaVersion == 13 and current.recordingContractVersion == 1,
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
--- session tables -- only the second return value (sourceSessions) may
--- reference them, and that must never be nested inside the payload table
--- itself.
local function TestExportPayloadNeverExposesSourceSessions()
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

  assert(payload.sourceSessions == nil, "The payload table must never carry the real session references")

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

  runtime.core.DeleteReportedSessions(sourceSessions)
  assert(#runtime.env.QuestieTraceCharacter.sessions == 0, "DeleteReportedSessions must remove the real source session")
end

local function TestExportPayloadExcludesDeletedSessions()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("s1")
  runtime.core.SaveCapture()

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "First build must include the freshly saved session")
  assert(payload.hasExportableData == true, "hasExportableData must be true when a session is included")

  runtime.core.DeleteReportedSessions(sourceSessions)

  local secondPayload = runtime.core.BuildExportPayload()
  assert(#secondPayload.sessions == 0, "A reported session must be deleted, not bundled again")
  assert(secondPayload.hasExportableData == false, "hasExportableData must be false once nothing new remains")
  assert(#runtime.env.QuestieTraceCharacter.sessions == 0, "Reported session must be permanently removed from SavedVariables")
end

local function TestExportPayloadIncludesOnlyNewSessionsAfterExport()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })
  runtime.core.StartCapture("s1")
  runtime.core.SaveCapture()
  local _, firstSourceSessions = runtime.core.BuildExportPayload()
  runtime.core.DeleteReportedSessions(firstSourceSessions)

  runtime.core.StartCapture("s2")
  runtime.core.SaveCapture()

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "Only the remaining saved session must be included")
  assert(sourceSessions[1].name == "s2", "The included session must be the new one, not the deleted one")
end

--- A pure size backstop for players who never report/export: unreported
--- sessions must still be capped so SavedVariables can't grow forever.
local function TestSaveCapturePrunesOldestSessionsPastCap()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })

  for i = 1, 12 do
    runtime.core.StartCapture("s" .. i)
    runtime.core.SaveCapture()
  end

  local sessions = runtime.env.QuestieTraceCharacter.sessions
  assert(#sessions == 10, "Sessions must be capped at 10 even though none were reported")
  assert(sessions[1].name == "s3", "The oldest sessions must be dropped first")
  assert(sessions[10].name == "s12", "The newest session must always be kept")
end

local function TestExportPayloadLiveSessionDeletedAndRestarted()
  local runtime = NewRuntime({ "Modules/Export/Export.lua" })

  -- Start a capture and add events
  runtime.core.StartCapture("live")
  local oldSession = Session(runtime)
  oldSession.events[1] = { t = 0, tp = 0, e = "TEST_EVENT", a = {} }

  local payload, sourceSessions = runtime.core.BuildExportPayload()
  assert(#payload.sessions == 1, "A live session with events must be included once")

  -- Report it: the live session must be discarded (never saved), and a fresh
  -- capture must start immediately (since consent is required for export).
  runtime.core.DeleteReportedSessions(sourceSessions)

  assert(#runtime.env.QuestieTraceCharacter.sessions == 0, "A reported live session must never be saved to sessions[]")

  local newSession = Session(runtime)
  assert(newSession ~= oldSession, "Fresh capture must be a different session object")
  assert(runtime.core.GetCaptureState() == "running", "After reporting, must be running a fresh capture")
  assert(#newSession.events == 0, "Fresh session must start empty")

  -- The old session must never resurface
  local secondPayload = runtime.core.BuildExportPayload()
  assert(#secondPayload.sessions == 0, "A reported live session must not be re-exportable after discard+restart")
end

local function TestExportSerializationRoundTrips()
  local runtime = { env = {}, core = {}, now = 0, timers = {}, frame = {} }
  local env = runtime.env
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestLog = {}
  env.SlashCmdList = {}
  env.QuestieTrace = { schemaVersion = 13, settings = { dataCollectionConsent = true } }
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

  local ok, text = runtime.core.BuildExportString()

  assert(ok)
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

local function TestLoginInitializesFreshCharacter()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  MockIsResting(runtime, false)
  local env = runtime.env
  env.QuestieTraceCharacter = nil
  local settings = env.QuestieTrace.settings

  SendEvent(runtime, "PLAYER_LOGIN")

  local session = Session(runtime)
  assert(env.QuestieTrace.settings == settings, "Login must preserve account settings and consent")
  assert(runtime.core.GetCaptureState() == "running", "Login must auto-start on a fresh character with account consent")
  assert(env.QuestieTraceCharacter.currentSession == session, "Login must link the live session to the character database")
  assert(session.events[1].e == "PLAYER_LOGIN", "PLAYER_LOGIN must be the first recorded event")
  assert(#env.QuestieTraceCharacter.sessions == 0, "Login must not save the new capture")
end

local function TestRecoverCurrentSessionOnLogin()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  MockIsResting(runtime, false)
  local env = runtime.env
  -- Simulate a leftover currentSession from a previous load (e.g. /reload without Save)
  local leftover = {
    schemaVersion = 10,
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

  -- Login finalizes the recovered session before starting a new capture.
  SendEvent(runtime, "PLAYER_LOGIN")

  -- The recovered session should now be in sessions[] (auto-finalized), not in capture.session
  assert(#env.QuestieTraceCharacter.sessions == 1, "Recovered session must be auto-finalized into sessions[]")
  local recovered = env.QuestieTraceCharacter.sessions[1]
  assert(recovered == leftover, "Recovered session must be the same table reference")
  assert(recovered.stoppedAt == 150, "Recovery must fill stoppedAt with current virtual time")
  assert(recovered.duration == 50, "Recovery must compute correct non-negative duration (150 - 100)")
  assert(recovered.durationPrecise == 50, "Recovery must compute correct durationPrecise")
  assert(runtime.core.GetCaptureState() == "running", "Login must start a new capture after recovery")
  local current = env.QuestieTraceCharacter.currentSession
  assert(current ~= nil and current ~= recovered, "Login must replace the recovered session with a new capture")
  assert(current.events[1].e == "PLAYER_LOGIN", "The new capture must record PLAYER_LOGIN first")
end

local function TestRecoverCurrentSessionDiscardedWhenConsentDeclined()
  local runtime = NewRuntime({})
  local env = runtime.env
  -- Simulate a leftover currentSession from a previous load
  local leftover = {
    schemaVersion = 10,
    recordingContractVersion = 1,
    name = "recovered",
    startedAt = 100,
    startedAtPrecise = 100,
    events = { { t = 101, e = "TEST_EVENT" } },
    functions = {},
    functionsDelta = {},
  }
  env.QuestieTraceCharacter.currentSession = leftover
  -- Consent explicitly declined
  env.QuestieTrace.settings.dataCollectionConsent = false

  runtime.now = 150
  SendEvent(runtime, "PLAYER_LOGIN")

  -- Session should be discarded, not saved
  assert(#env.QuestieTraceCharacter.sessions == 0, "Recovered session must not be saved when consent is declined")
  assert(env.QuestieTraceCharacter.currentSession == nil, "currentSession must be cleared")
  assert(runtime.core.GetCaptureState() == "idle", "State must be idle after discarding")
end

local function TestRecoverCurrentSessionDiscardedWhenConsentUndecided()
  local runtime = NewRuntime({})
  local env = runtime.env
  -- Simulate a leftover currentSession from a previous load
  local leftover = {
    schemaVersion = 10,
    recordingContractVersion = 1,
    name = "recovered",
    startedAt = 100,
    startedAtPrecise = 100,
    events = { { t = 101, e = "TEST_EVENT" } },
    functions = {},
    functionsDelta = {},
  }
  env.QuestieTraceCharacter.currentSession = leftover
  -- Consent not yet decided (nil)
  env.QuestieTrace.settings.dataCollectionConsent = nil

  runtime.now = 150
  SendEvent(runtime, "PLAYER_LOGIN")

  -- Session should be discarded, not saved
  assert(#env.QuestieTraceCharacter.sessions == 0, "Recovered session must not be saved when consent is undecided")
  assert(env.QuestieTraceCharacter.currentSession == nil, "currentSession must be cleared")
  assert(runtime.core.GetCaptureState() == "idle", "State must be idle after discarding")
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
  assert(runtime.core.IsShareDue() == false, "No saved sessions means nothing is shareable yet")

  -- A live, unsaved capture with no events yet is not in the export payload and must stay silent.
  runtime.core.StartCapture("live only")
  assert(runtime.core.IsShareDue() == false, "An unsaved live session with no events must not trigger a reminder")
end

local function TestShareReminderSuppressedAfterExportOpened()
  local runtime = NewRuntime(REMINDER_FILES)
  SaveSessions(runtime, 1)
  runtime.core.MarkExportOpened()
  assert(runtime.core.IsShareDue() == false, "Opening the export window must pause reminders")
end

--- Helper to set up a session with meaningful events
local function SetupShareableSession(runtime)
  runtime.core.StartCapture("test session")
  local session = runtime.env.QuestieTraceCharacter.currentSession
  session.events[#session.events + 1] = { t = 0, tp = 0, e = "QUEST_ACCEPTED", a = { n = 0 } }
  runtime.core.SaveCapture()
end

local function TestInnReminderNotDueBeforeOneHour()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  -- Player is in an inn from the start (simulate login in inn)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Wait 30 minutes - not enough
  AdvanceTo(runtime, 1800)
  assert(#messages == 0, "Must not remind before 1 hour of play, even in an inn")

  -- Wait until 1 hour - should fire now since we're in an inn
  AdvanceTo(runtime, 3600)
  assert(#messages == 1, "Must remind after 1 hour if already in an inn at login")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

local function TestInnReminderFiresWhenEnteringInnAfterOneHour()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  -- Player starts NOT in an inn
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play for 50 minutes, then enter inn - not enough
  AdvanceTo(runtime, 3000)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 0, "Must not remind if played less than 1 hour")

  -- Play for another 20 minutes (total 1h10m), then enter inn again
  AdvanceTo(runtime, 4200)
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "Must remind when entering inn after 1+ hour of play")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

local function TestInnReminderFiresWhenEnteringInnBeforeOneHourAndStaying()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  -- Player starts NOT in an inn
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play for 50 minutes, then enter inn
  AdvanceTo(runtime, 3000)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 0, "Must not remind immediately when entering at 50 minutes")

  -- Stay in inn for 10 more minutes (total 1 hour) - timer should fire
  AdvanceTo(runtime, 3600)
  assert(#messages == 1, "Must remind at 1-hour mark while still in inn")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

local function TestInnReminderFiresOnZoneChangeToInn()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play for 1 hour
  AdvanceTo(runtime, 3600)

  -- Zone change into an inn (simulated via PLAYER_UPDATE_RESTING)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "Must remind on zone change into inn after 1+ hour")
end

local function TestInnReminderResetsAfterFiring()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play 1 hour, enter inn -> reminder fires
  AdvanceTo(runtime, 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "First reminder must fire")

  -- Leave inn, play another hour, enter inn again -> second reminder
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  AdvanceTo(runtime, 7200)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 2, "Second reminder must fire after another hour of play")
end

local function TestInnReminderDoesNotFireWithoutShareableData()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)

  -- No sessions saved, no capture started (consent not granted)
  -- Simulate consent declined so no auto-start
  runtime.env.QuestieTrace.settings.dataCollectionConsent = false
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  AdvanceTo(runtime, 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 0, "Must not remind if there's no shareable data")
end

local function TestInnReminderPausedByExportWindow()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play 1 hour, enter inn -> reminder fires
  AdvanceTo(runtime, 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "First reminder must fire")

  -- Open export window (marks last reminder time)
  runtime.core.MarkExportOpened()

  -- Leave inn, re-enter immediately - should not fire again
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "Must not remind again immediately after export window opened")

  -- Play another hour, enter inn -> should fire again
  AdvanceTo(runtime, 7200)
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 2, "Must remind again after another hour of play post-export")
end

--- Test that leaving and re-entering an inn before the timer fires
--- cancels the old timer and schedules a fresh one (token validation).
--- Both timers target the same 1-hour mark, but only the latest executes.
local function TestInnReminderTokenInvalidatesStaleCallbacks()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play 50 minutes, enter inn -> schedules timer for remaining 10 minutes (fires at 3600)
  AdvanceTo(runtime, 3000)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Leave inn after 5 minutes (at 55 min total) - increments token
  AdvanceTo(runtime, 3300)
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Re-enter inn immediately (at 55 min) - new token, new timer also targets 3600
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Advance to 1-hour mark (3600) - BOTH timers fire, but token check allows only the fresh one
  AdvanceTo(runtime, 3600)
  assert(#messages == 1, "Only the fresh callback from re-entry must fire at 1-hour mark")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

--- Test multiple rapid inn re-entries before timer fires - only latest schedules.
--- All timers target the 1-hour mark; only the last entry's callback executes.
local function TestInnReminderMultipleReentriesBeforeTimer()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play 50 minutes
  AdvanceTo(runtime, 3000)

  -- Enter inn (entry 1) -> timer for 10 min remaining (fires at 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Leave (51 min total)
  AdvanceTo(runtime, 3060)
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Re-enter (entry 2) -> timer for 9 min remaining (fires at 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Leave (53 min total)
  AdvanceTo(runtime, 3180)
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Re-enter (entry 3) -> timer for 7 min remaining (fires at 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")

  -- Advance to 1-hour mark (3600) - all three timers fire, but only the last executes
  AdvanceTo(runtime, 3600)
  assert(#messages == 1, "Only the latest inn entry's timer must fire at 1-hour mark")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

--- Test the bug where re-entering an inn after a reminder already fired
--- incorrectly calculates remaining time from sessionStart instead of lastReminderAt.
--- This causes negative remaining time and no timer is scheduled, so the next
--- reminder never fires.
local function TestInnReminderReentryAfterReminderUsesLastReminderAtBaseline()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play 1 hour, enter inn -> reminder fires, lastReminderAt = 3600
  AdvanceTo(runtime, 3600)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "First reminder must fire at 1 hour")

  -- Leave inn at 1h30m (5400)
  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  AdvanceTo(runtime, 5400)

  -- Re-enter inn at 1h31m (5500) - only 1900s since last reminder
  -- Should schedule timer for 1700s remaining (to reach 3600s since last reminder)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 1, "Must not remind immediately on re-entry at 1h31m")

  -- Advance to 2 hours from session start (7200) = 3600s since last reminder
  -- Second reminder should fire
  AdvanceTo(runtime, 7200)
  assert(#messages == 2, "Second reminder must fire 1 hour after last reminder")
  assert(messages[2]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

--- Test that StartShareReminders() checks IsResting() immediately after registering
--- events, so that if the player is already in an inn (e.g., consent accepted while
--- in an inn), the inn logic runs and schedules the timer appropriately.
--- This covers the case where StartShareReminders is called from the consent dialog
--- Accept button while already in an inn, long after PLAYER_ENTERING_WORLD has fired.
local function TestInnReminderStartsWhenInInn()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  -- Disable consent so StartShareReminders is not auto-called on login
  runtime.env.QuestieTrace.settings.dataCollectionConsent = nil

  -- Simulate: PLAYER_LOGIN and PLAYER_ENTERING_WORLD already happened earlier
  -- Player is IN an inn at login time
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play for 50 minutes while in inn (no reminders started due to no consent)
  AdvanceTo(runtime, 3000)

  -- Player grants consent via dialog (calls StartShareReminders) while in inn
  -- This happens AFTER PLAYER_ENTERING_WORLD has already fired
  -- Do NOT send PLAYER_UPDATE_RESTING - player was already resting
  runtime.core.StartShareReminders()

  -- sessionStart is set to current time (3000) when StartShareReminders runs
  -- Timer should be scheduled for 1 hour from sessionStart (fires at 6600)
  assert(#messages == 0, "Must not remind immediately at 50 minutes")

  -- Advance to 1 hour after session start (6600) - timer should fire
  AdvanceTo(runtime, 6600)
  assert(#messages == 1, "Must remind 1 hour after session start while still in inn")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

--- Test that the timer callback re-arms itself if MarkExportOpened() updates
--- lastReminderAt while the timer is pending. Without re-arming, the reminder
--- would be lost because the timer fires, sees the updated lastReminderAt,
--- finds it's not due, and does nothing.
local function TestInnReminderTimerRearmsAfterExportWindowOpened()
  local runtime = NewRuntime(REMINDER_FILES)
  local messages = CaptureChat(runtime)
  SetupShareableSession(runtime)

  MockIsResting(runtime, false)
  SendEvent(runtime, "PLAYER_LOGIN")
  SendEvent(runtime, "PLAYER_ENTERING_WORLD")

  -- Play 50 minutes, enter inn -> schedules timer for remaining 10 minutes (fires at 3600)
  AdvanceTo(runtime, 3000)
  MockIsResting(runtime, true)
  SendEvent(runtime, "PLAYER_UPDATE_RESTING")
  assert(#messages == 0, "Must not remind immediately at 50 minutes")

  -- Open export window before timer fires (at 3500, 100s before 1-hour mark)
  -- This simulates user opening export window while timer is pending
  AdvanceTo(runtime, 3500)
  runtime.core.MarkExportOpened()

  -- After MarkExportOpened(), IsShareDue() must be false (lastReminderAt updated to now)
  assert(runtime.core.IsShareDue() == false, "IsShareDue must be false immediately after MarkExportOpened")

  -- Timer fires at 3600. It sees IsShareDue()==false and re-arms for 1 hour from MarkExportOpened (7100).
  AdvanceTo(runtime, 3600)
  assert(#messages == 0, "Must not remind at 3600 because export window was opened")

  -- Advance to 1 hour after export window (7100) - re-armed timer should fire
  AdvanceTo(runtime, 7100)
  assert(#messages == 1, "Must remind 1 hour after export window opened")
  assert(messages[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

--- Test that a fresh login (new runtime/addon load) resets sessionStart, so reminder
--- timing starts fresh. This ensures that if a player logs out and back in, the 1-hour
--- timer is based on the new session start, not the old one.
local function TestFreshLoginResetsSessionStart()
  -- First login session: play 50 minutes, enter inn
  local runtime1 = NewRuntime(REMINDER_FILES)
  local messages1 = CaptureChat(runtime1)
  SetupShareableSession(runtime1)

  MockIsResting(runtime1, false)
  SendEvent(runtime1, "PLAYER_LOGIN")
  SendEvent(runtime1, "PLAYER_ENTERING_WORLD")

  AdvanceTo(runtime1, 3000)
  MockIsResting(runtime1, true)
  SendEvent(runtime1, "PLAYER_UPDATE_RESTING")
  assert(#messages1 == 0, "Must not remind at 50 minutes in first session")

  -- Second login session (new addon load): simulate fresh login at virtual time 4000
  -- The new runtime gets a fresh InitReminderState call
  local runtime2 = NewRuntime(REMINDER_FILES)
  local messages2 = CaptureChat(runtime2)
  SetupShareableSession(runtime2)

  MockIsResting(runtime2, true) -- Player is in inn at login
  -- Set virtual time to 4000 to simulate 1000s passed since first login
  runtime2.now = 4000
  SendEvent(runtime2, "PLAYER_LOGIN")
  SendEvent(runtime2, "PLAYER_ENTERING_WORLD")

  -- sessionStart should be 4000 (current time), so timer fires at 7600
  AdvanceTo(runtime2, 7600)
  assert(#messages2 == 1, "Reminder must fire 1 hour after new sessionStart (at 7600)")
  assert(messages2[1]:find("|Hquestietrace:export|h", 1, true) ~= nil, "Reminder must have export link")
end

---@param runtime TestRuntime
---@param needle string
---@return boolean
local function WasPrinted(runtime, needle)
  for _, message in ipairs(runtime.printedMessages) do
    if message:find(needle, 1, true) then return true end
  end
  return false
end

local function TestConsentUndecidedShowsPromptAndDoesNotAutoStart()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  runtime.env.QuestieTrace.settings.dataCollectionConsent = nil

  SendEvent(runtime, "PLAYER_LOGIN")

  assert(runtime.consentFrame.shown, "Consent dialog must be shown on first login when undecided")
  assert(not WasPrinted(runtime, "gameplay data is being collected"), "No reminder message should be printed while undecided")
  assert(runtime.core.GetCaptureState() == "idle", "Capture must not auto-start while consent is undecided")
end

local function TestConsentDeclinedBlocksEverything()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  runtime.env.QuestieTrace.settings.dataCollectionConsent = false

  SendEvent(runtime, "PLAYER_LOGIN")

  assert(not runtime.consentFrame.shown, "Consent dialog must not be shown again once declined")
  assert(not WasPrinted(runtime, "gameplay data is being collected"), "No reminder message should be printed when consent is declined")
  assert(runtime.core.GetCaptureState() == "idle", "Capture must not auto-start when consent is declined")

  runtime.core.StartCapture("manual attempt")
  assert(runtime.core.GetCaptureState() == "idle", "Manual /qlt start must also be blocked when consent is declined")
end

local function TestConsentAcceptedPrintsReminderAndAllowsAutoStart()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  MockIsResting(runtime, false)
  runtime.env.QuestieTrace.settings.dataCollectionConsent = true

  SendEvent(runtime, "PLAYER_LOGIN")

  assert(not runtime.consentFrame.shown, "Consent dialog must not be shown once already accepted")
  assert(WasPrinted(runtime, "gameplay data is being collected"), "A reminder message must be printed every login once consented")
  assert(runtime.core.GetCaptureState() == "running", "Capture must auto-start when consent is granted")
end

local function TestDecliningConsentStopsAndDiscardsActiveCapture()
  local runtime = NewRuntime({})
  runtime.env.QuestieTrace.settings.dataCollectionConsent = true
  runtime.core.StartCapture("in progress")
  assert(runtime.core.GetCaptureState() == "running", "Precondition: capture must be running before declining")

  runtime.core.ShowConsentPrompt()
  runtime.consentFrame.DeclineButton:OnClick()

  assert(not runtime.consentFrame.shown, "No must close the consent dialog")
  assert(runtime.env.QuestieTraceCharacter.currentSession == nil, "Declining must remove the live SavedVariables reference")
  assert(runtime.env.QuestieTrace.settings.dataCollectionConsent == false, "Consent flag must be recorded as declined")
  assert(runtime.core.GetCaptureState() == "idle", "Declining consent must stop and discard any active capture")
  assert(#runtime.env.QuestieTraceCharacter.sessions == 0, "Declining consent must not save the discarded capture")
end

local function TestConsentAcceptImmediatelyStartsCapture()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  MockIsResting(runtime, false)
  runtime.env.QuestieTrace.settings.dataCollectionConsent = nil

  runtime.core.ShowConsentPrompt()
  runtime.consentFrame.AcceptButton:OnClick()

  assert(not runtime.consentFrame.shown, "Yes must close the consent dialog")
  assert(runtime.env.QuestieTrace.settings.dataCollectionConsent == true, "Consent flag must be set to true")
  assert(runtime.core.GetCaptureState() == "running", "Capture must start immediately when consent is accepted")
end

local function TestConsentPromptReopensWithoutChangingConsent()
  local runtime = NewRuntime({})
  runtime.env.QuestieTrace.settings.dataCollectionConsent = false
  local frame = runtime.consentFrame

  runtime.core.ShowConsentPrompt()
  assert(frame.shown, "The prompt must reopen even after consent was declined")
  assert(frame.AcceptButton.text == "Yes" and frame.DeclineButton.text == "No", "Both choices must be labeled")
  assert(frame.Text.text:find("Help improve Questie", 1, true), "The dialog must display the consent explanation")
  assert(frame.height == 162, "Dialog height must include wrapped text, button height, and original popup padding")
  frame:Hide()
  runtime.core.ShowConsentPrompt()

  assert(frame.shown, "The same dialog must be reusable")
  assert(runtime.env.QuestieTrace.settings.dataCollectionConsent == false, "Opening or hiding must not grant consent")
  assert(runtime.core.GetCaptureState() == "idle", "Opening or hiding must not start capture")
end

local function TestDialogUsesModernAssetsWhenAvailable()
  local runtime = NewRuntime({})
  local env, frame = runtime.env, runtime.consentFrame
  env.C_Texture = { GetAtlasInfo = function() return {} end }
  env.UserScaledFontGameHighlight = {}
  env.UserScaledFontGameNormal = {}
  env.UserScaledFontGameDisable = {}

  frame:OnLoad()

  assert(frame.Background.atlas == "UI-DialogBox-Background-Dark", "Use the original popup background")
  assert(frame.Border.atlas == "UI-DiamondDialogBox-Border", "Use the original popup border")
  assert(frame.Background.shown and frame.Border.shown and frame.backdrop == nil, "Do not draw a fallback behind modern art")
  assert(frame.Text.font == env.UserScaledFontGameHighlight, "Use the client's user-scaled body font")
  assert(frame.AcceptButton.normalFont == env.UserScaledFontGameNormal, "Use the client's user-scaled button font")
  assert(frame.DeclineButton.disabledFont == env.UserScaledFontGameDisable, "Use the client's disabled font")
end

local function TestDialogFallsBackWhenAnAtlasIsMissing()
  local runtime = NewRuntime({})
  local env, frame = runtime.env, runtime.consentFrame
  env.C_Texture = { GetAtlasInfo = function(atlas)
    if atlas == "UI-DialogBox-Background-Dark" then return {} end
  end }

  frame:OnLoad()

  assert(frame.backdrop == env.BACKDROP_DIALOG_32_32, "Older clients need the standard backdrop")
  assert(not frame.Background.shown and not frame.Border.shown, "Do not display incomplete modern art")
  assert(frame.Text.font == env.GameFontHighlight, "Older clients need the standard body font")
  assert(frame.AcceptButton.normalFont == env.GameFontNormal, "Older clients need the standard button font")
end

local function TestDialogFitsLongLabelsAndWrappedText()
  local runtime = NewRuntime({})
  local frame = runtime.consentFrame
  frame.Text.stringHeight = 200
  frame.AcceptButton.textWidth = 230
  frame.AcceptButton.Text.stringHeight = 24

  runtime.core.ShowConsentPrompt()

  assert(frame.AcceptButton.width == 250 and frame.DeclineButton.width == 250, "Both buttons must fit the longest label")
  assert(frame.AcceptButton.height == 32 and frame.DeclineButton.height == 32, "Buttons must fit taller fonts")
  assert(frame.width == 542, "Widen the dialog to fit both buttons, their gap, and side margins")
  assert(frame.height == 273, "Fit wrapped body text, buttons, and vertical spacing")
end

local function TestAcceptingConsentPreservesActiveCapture()
  local runtime = NewRuntime({ "Modules/Export/ExportReminder.lua" })
  MockIsResting(runtime, false)
  runtime.core.StartCapture("in progress")
  local session = Session(runtime)

  runtime.core.ShowConsentPrompt()
  runtime.consentFrame.AcceptButton:OnClick()

  assert(not runtime.consentFrame.shown, "Accepting an already granted consent must close the dialog")
  assert(Session(runtime) == session, "Accepting again must preserve the active capture")
end

---------------------------------------------------------------------------
-- Privacy tests
---------------------------------------------------------------------------

local function TestParseGUIDKindClassifiesPrefixes()
  local runtime = NewRuntime({})
  local Core = runtime.core

  assert(Core.ParseGUIDKind("Player-4618-0053656F") == "player", "Player- prefix must classify as player")
  assert(Core.ParseGUIDKind("Creature-0-6783-1-269-2980-00002C50D7") == "npc", "Creature- prefix must classify as npc")
  assert(Core.ParseGUIDKind("Pet-0-6783-1-269-2980-00002C50D7") == "npc", "Pet- prefix must classify as npc")
  assert(Core.ParseGUIDKind("Vehicle-0-6783-1-269-2980-00002C50D7") == "npc", "Vehicle- prefix must classify as npc")
  assert(Core.ParseGUIDKind("GameObject-0-6783-1-269-2980-00002C50D7") == "object", "GameObject- prefix must classify as object")
  assert(Core.ParseGUIDKind("Item-4-0-000012") == "item", "Item- prefix must classify as item")
  assert(Core.ParseGUIDKind("Nonsense-guid") == nil, "Unrecognized GUID prefixes must classify as nil")
  assert(Core.ParseGUIDKind(nil) == nil, "nil guid must classify as nil")

  assert(Core.IsPlayerGUID("Player-4618-0053656F") == true, "IsPlayerGUID must detect player GUIDs")
  assert(Core.IsPlayerGUID("Creature-0-6783-1-269-2980-00002C50D7") == false, "IsPlayerGUID must not flag npc GUIDs")
end

local function TestSanitizeTextRedactsLocalPlayerName()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function(token)
    if token == "player" then return "Cruxdruid" end
    return nil
  end

  local sanitized = runtime.core.SanitizeText("Greetings, Cruxdruid, and welcome to Camp Narache.")
  assert(sanitized == "Greetings, <name>, and welcome to Camp Narache.",
    "Local player's own name must be redacted: " .. tostring(sanitized))
  assert(not sanitized:find("Cruxdruid", 1, true), "Sanitized text must not contain the raw player name")
end

local function TestSanitizeTextRedactsGroupRosterNames()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function(token)
    if token == "player" then return "Cruxdruid" end
    if token == "party1" then return "Alice" end
    if token == "party2" then return "Bob" end
    return nil
  end
  runtime.env.IsInGroup = function() return true end
  runtime.env.IsInRaid = function() return false end

  local sanitized = runtime.core.SanitizeText("Alice and Bob and Cruxdruid enter the dungeon.")
  assert(sanitized == "<name> and <name> and <name> enter the dungeon.",
    "Party roster names must be redacted: " .. tostring(sanitized))
end

local function TestSanitizeTextLeavesUnrelatedTextAlone()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function() return nil end

  local text = "Your reputation with Timbermaw Hold has very slightly increased."
  assert(runtime.core.SanitizeText(text) == text, "Text without known player names must be left untouched")
  assert(runtime.core.SanitizeText(nil) == nil, "Non-string input must be returned unchanged")
  assert(runtime.core.SanitizeText("") == "", "Empty string must be returned unchanged")
end

local function TestSanitizeChatMsgArgsStripsNamesAndGuid()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function() return nil end

  -- CHAT_MSG_LOOT signature: text, playerName, languageName, channelName,
  -- playerName2, specialFlags, zoneChannelID, channelIndex, channelBaseName,
  -- languageID, lineID, guid, bnSenderID, isMobile, isSubtitle,
  -- hideSenderInLetterbox, supressRaidIcons
  local args = runtime.core.PackArgs(
    "Grimtotem receives loot: [Plainstrider Feather].", "Grimtotem", "", "",
    "", "", 0, 0, "", 0, 1, "Player-4618-0053656F", 0, false, false, false, false
  )

  local sanitized = runtime.core.SanitizeChatMsgArgs(args)

  assert(sanitized[1] == "<name> receives loot: [Plainstrider Feather].",
    "Message text must have the player's name redacted: " .. tostring(sanitized[1]))
  assert(sanitized[2] == nil, "playerName arg must be discarded")
  assert(sanitized[5] == nil, "playerName2 arg must be discarded")
  assert(sanitized[12] == nil, "Player guid arg must be discarded")
  assert(sanitized[11] == 1, "Unrelated args (lineID) must be preserved")
  assert(args[2] == "Grimtotem", "The original PackedArgs must not be mutated")
end

local function TestSanitizeChatMsgArgsPreservesNonPlayerGuid()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function() return nil end

  local args = runtime.core.PackArgs(
    "You loot 4 Copper", "Cruxdruid", "", "", "", "", 0, 0, "", 0, 1,
    "Creature-0-6783-1-269-2980-00002C50D7", 0, false, false, false, false
  )

  local sanitized = runtime.core.SanitizeChatMsgArgs(args)
  assert(sanitized[12] == "Creature-0-6783-1-269-2980-00002C50D7", "Non-player guid must be preserved")
end

local function TestChatMsgLootEventIsSanitizedInSession()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function() return nil end
  runtime.core.StartCapture("chat loot sanitize")

  runtime.frame.OnEvent(
    runtime.frame, "CHAT_MSG_LOOT",
    "Grimtotem receives loot: [Plainstrider Feather].", "Grimtotem", "", "",
    "", "", 0, 0, "", 0, 1, "Player-4618-0053656F", 0, false, false, false, false
  )

  local session = Session(runtime)
  local recorded = session.events[#session.events]
  assert(recorded.e == "CHAT_MSG_LOOT", "Precondition: last event must be CHAT_MSG_LOOT")
  assert(recorded.a[1] == "<name> receives loot: [Plainstrider Feather].",
    "Recorded event text must be sanitized: " .. tostring(recorded.a[1]))
  assert(recorded.a[2] == nil, "Recorded playerName must be scrubbed")
  assert(recorded.a[12] == nil, "Recorded guid must be scrubbed")
end

local function TestUnitInteractionSkipsPlayerTarget()
  local runtime = NewRuntime({ "Modules/Trackers/UnitInteraction.lua" })
  runtime.env.UnitGUID = function(token)
    if token == "target" then return "Player-4620-00572164" end
    return nil
  end
  runtime.env.UnitName = function(token)
    if token == "target" then return "SomeOtherPlayer", nil end
    return nil
  end

  runtime.core.StartCapture("unit interaction privacy")
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")

  local session = Session(runtime)
  local guidStream = session.functions.UnitGUID.target
  local nameStream = session.functions.UnitName.target
  assert(#guidStream == 0, "A player target's GUID must never be recorded")
  assert(#nameStream == 0, "A player target's name must never be recorded")
end

local function TestUnitInteractionRecordsNpcTarget()
  local runtime = NewRuntime({ "Modules/Trackers/UnitInteraction.lua" })
  runtime.env.UnitGUID = function(token)
    if token == "target" then return "Creature-0-6783-1-269-2980-00002C50D7" end
    return nil
  end
  runtime.env.UnitName = function(token)
    if token == "target" then return "Plainstrider", nil end
    return nil
  end

  runtime.core.StartCapture("unit interaction npc")
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")

  local session = Session(runtime)
  local guidStream = session.functions.UnitGUID.target
  local nameStream = session.functions.UnitName.target
  assert(#guidStream == 1 and guidStream[1].v == "Creature-0-6783-1-269-2980-00002C50D7",
    "NPC target GUID must still be recorded")
  assert(#nameStream == 1 and nameStream[1].v[1] == "Plainstrider", "NPC target name must still be recorded")
end

local function TestLootSkipsPlayerSourcedGuid()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function() return "Player-4620-00572164", 1 end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end

  runtime.core.StartCapture("loot privacy")
  SendEvent(runtime, "LOOT_READY")

  local session = Session(runtime)
  local sourceStream = session.functions.GetLootSourceInfo[1]
  assert(sourceStream == nil or #sourceStream == 0, "A player-sourced loot GUID must never be recorded")
end

local function TestLootRecordsNpcSourcedGuid()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function() return "Creature-0-6783-1-269-2980-00002C50D7", 1 end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end

  runtime.core.StartCapture("loot npc source")
  SendEvent(runtime, "LOOT_READY")

  local session = Session(runtime)
  local sourceStream = session.functions.GetLootSourceInfo[1]
  assert(sourceStream and #sourceStream == 1 and sourceStream[1].v[1] == "Creature-0-6783-1-269-2980-00002C50D7",
    "An npc-sourced loot GUID must be recorded")
end

local function TestLootKeepsUnchangedCorrelatedSamples()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function() return "Creature-0-6783-1-269-2980-00002C50D7", 1 end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end

  runtime.core.StartCapture("loot repeated samples")
  SendEvent(runtime, "LOOT_READY")
  AdvanceTo(runtime, 1)
  SendEvent(runtime, "LOOT_READY")

  local session = Session(runtime)
  assert(#session.functions.GetLootSourceInfo[1] == 2, "Every successful source probe must be retained for timestamp correlation")
  assert(#session.functions.GetLootSlotLink[1] == 2, "Every successful link probe must be retained for timestamp correlation")
end

local function TestLootAllowsMultiPairAllNpcSourceTuple()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function()
    return "Creature-0-6783-1-269-2980-00002C50D7", 1, "Creature-0-6783-1-269-2981-00002C50D7", 1
  end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end

  runtime.core.StartCapture("loot multi npc source")
  SendEvent(runtime, "LOOT_READY")

  local session = Session(runtime)
  local sourceStream = session.functions.GetLootSourceInfo[1]
  assert(sourceStream and #sourceStream == 1 and sourceStream[1].v.n == 4,
    "A multi-pair source tuple with only npc guids must be recorded in full")
end

local function TestLootSkipsMultiPairSourceTupleWithAnyPlayerGuid()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function()
    return "Creature-0-6783-1-269-2980-00002C50D7", 1, "Player-4620-00572164", 1
  end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end

  runtime.core.StartCapture("loot mixed source with player")
  SendEvent(runtime, "LOOT_READY")

  local session = Session(runtime)
  local sourceStream = session.functions.GetLootSourceInfo[1]
  assert(sourceStream == nil or #sourceStream == 0,
    "A source tuple must be discarded entirely if any entry is a player guid, even alongside an npc entry")
end

local function TestLootSkipsUnrecognizedGuidKind()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function() return "Corpse-0-6783-1-269-2980-00002C50D7", 1 end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end

  runtime.core.StartCapture("loot unrecognized guid kind")
  SendEvent(runtime, "LOOT_READY")

  local session = Session(runtime)
  local sourceStream = session.functions.GetLootSourceInfo[1]
  assert(sourceStream == nil or #sourceStream == 0,
    "An unrecognized (non npc/object/item) guid kind must be discarded, not just player guids")
end

local function TestUnitInteractionSkipsUnrecognizedGuidKind()
  local runtime = NewRuntime({ "Modules/Trackers/UnitInteraction.lua" })
  runtime.env.UnitGUID = function(token)
    if token == "target" then return "Corpse-0-6783-1-269-2980-00002C50D7" end
    return nil
  end
  runtime.env.UnitName = function(token)
    if token == "target" then return "SomeCorpse", nil end
    return nil
  end

  runtime.core.StartCapture("unit interaction unrecognized kind")
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")

  local session = Session(runtime)
  local guidStream = session.functions.UnitGUID.target
  local nameStream = session.functions.UnitName.target
  assert(#guidStream == 0, "An unrecognized guid kind must never be recorded")
  assert(#nameStream == 0, "The paired name for an unrecognized guid kind must never be recorded")
end

local function TestSanitizeTextEscapesSpecialCharactersInNames()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function(token)
    if token == "player" then return "Bri'ka-Nn" end
    return nil
  end

  local sanitized = runtime.core.SanitizeText("Greetings, Bri'ka-Nn, welcome home.")
  assert(sanitized == "Greetings, <name>, welcome home.",
    "Names with pattern-magic characters (apostrophe, hyphen) must be escaped and redacted: " .. tostring(sanitized))
end

local function TestChatMsgLootDispatchArgsAreSanitized()
  local runtime = NewRuntime({})
  runtime.env.UnitName = function() return nil end

  local capturedArgs
  runtime.core.RegisterTracker({
    events = { "CHAT_MSG_LOOT" },
    OnEvent = function(_, _, ...)
      capturedArgs = runtime.core.PackArgs(...)
    end,
  })

  runtime.core.StartCapture("dispatch sanitize")
  runtime.frame.OnEvent(
    runtime.frame, "CHAT_MSG_LOOT",
    "Grimtotem receives loot: [Plainstrider Feather].", "Grimtotem", "", "",
    "", "", 0, 0, "", 0, 1, "Player-4618-0053656F", 0, false, false, false, false
  )

  assert(capturedArgs, "The dummy tracker callback must have been invoked")
  assert(capturedArgs[1] == "<name> receives loot: [Plainstrider Feather].",
    "Args dispatched to trackers must be sanitized, not just the recorded event: " .. tostring(capturedArgs[1]))
  assert(capturedArgs[2] == nil, "Dispatched playerName must be scrubbed")
  assert(capturedArgs[12] == nil, "Dispatched guid must be scrubbed")
end

local function TestUnitInteractionMouseover()
  local runtime = NewRuntime({ "Modules/Trackers/UnitInteraction.lua" })
  local env = runtime.env

  -- Mock WoW API for mouseover unit
  env.UnitExists = function(token) return token == "mouseover" end
  env.UnitGUID = function(token)
    if token == "mouseover" then return "Creature-0-1-1-1-12345-000001" end
    return nil
  end
  env.UnitName = function(token)
    if token == "mouseover" then return "Test NPC", "Realm" end
    return nil
  end

  runtime.core.StartCapture("mouseover test")
  SendEvent(runtime, "UPDATE_MOUSEOVER_UNIT")

  local session = Session(runtime)
  local functions = session.functions

  -- Verify UnitGUID["mouseover"] was recorded
  assert(functions.UnitGUID and functions.UnitGUID.mouseover, "UnitGUID[mouseover] stream must exist")
  assert(#functions.UnitGUID.mouseover >= 1, "UnitGUID[mouseover] must have at least one entry")
  assert(functions.UnitGUID.mouseover[1].v == "Creature-0-1-1-1-12345-000001", "UnitGUID[mouseover] value must match mocked GUID")

  -- Verify UnitName["mouseover"] was recorded
  assert(functions.UnitName and functions.UnitName.mouseover, "UnitName[mouseover] stream must exist")
  assert(#functions.UnitName.mouseover >= 1, "UnitName[mouseover] must have at least one entry")
  assert(functions.UnitName.mouseover[1].v[1] == "Test NPC" and functions.UnitName.mouseover[1].v[2] == "Realm",
    "UnitName[mouseover] value must match mocked name")
end

local function TestUnitInteractionMouseoverPlayerIsSkipped()
  local runtime = NewRuntime({ "Modules/Trackers/UnitInteraction.lua" })
  local env = runtime.env

  -- Mock WoW API for a mouseover unit that is another player
  env.UnitExists = function(token) return token == "mouseover" end
  env.UnitGUID = function(token)
    if token == "mouseover" then return "Player-4620-00572164" end
    return nil
  end
  env.UnitName = function(token)
    if token == "mouseover" then return "SomeOtherPlayer", "Realm" end
    return nil
  end

  runtime.core.StartCapture("mouseover player privacy")
  SendEvent(runtime, "UPDATE_MOUSEOVER_UNIT")

  local session = Session(runtime)
  local functions = session.functions

  -- Verify neither the player GUID nor its name leaked into the mouseover streams
  assert(functions.UnitGUID and functions.UnitGUID.mouseover, "UnitGUID[mouseover] stream must exist")
  assert(#functions.UnitGUID.mouseover == 0, "A mouseover player's GUID must never be recorded")
  assert(functions.UnitName and functions.UnitName.mouseover, "UnitName[mouseover] stream must exist")
  assert(#functions.UnitName.mouseover == 0, "A mouseover player's name must never be recorded")
end

local function TestUnitStateTracker()
  local runtime = NewRuntime({ "Modules/Trackers/UnitState.lua" })
  local env = runtime.env

  -- Mock WoW API for target and mouseover
  env.UnitExists = function(token) return token == "target" or token == "mouseover" end
  env.UnitIsPlayer = function(_token) return false end
  env.UnitLevel = function(token)
    if token == "target" then return 20 end
    if token == "mouseover" then return -1 end -- "??" skull boss
    return nil
  end
  env.UnitClassification = function(token)
    if token == "target" then return "elite" end
    if token == "mouseover" then return "worldboss" end
    return nil
  end
  env.UnitReaction = function(unit, token)
    if unit == "player" and token == "target" then return 5 end -- Neutral
    if unit == "player" and token == "mouseover" then return 8 end -- Exalted
    return nil
  end

  runtime.core.StartCapture("unitstate test")
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")
  SendEvent(runtime, "UPDATE_MOUSEOVER_UNIT")

  local session = Session(runtime)
  local functions = session.functions

  -- Verify UnitLevel["target"] recorded
  assert(functions.UnitLevel and functions.UnitLevel.target, "UnitLevel[target] stream must exist")
  assert(#functions.UnitLevel.target >= 1, "UnitLevel[target] must have entry")
  assert(functions.UnitLevel.target[1].v == 20, "UnitLevel[target] must be 20")

  -- Verify UnitLevel["mouseover"] recorded (-1 for "??")
  assert(functions.UnitLevel and functions.UnitLevel.mouseover, "UnitLevel[mouseover] stream must exist")
  assert(#functions.UnitLevel.mouseover >= 1, "UnitLevel[mouseover] must have entry")
  assert(functions.UnitLevel.mouseover[1].v == -1, "UnitLevel[mouseover] must be -1")

  -- Verify UnitClassification["target"] recorded
  assert(functions.UnitClassification and functions.UnitClassification.target, "UnitClassification[target] stream must exist")
  assert(#functions.UnitClassification.target >= 1, "UnitClassification[target] must have entry")
  assert(functions.UnitClassification.target[1].v == "elite", "UnitClassification[target] must be elite")

  -- Verify UnitClassification["mouseover"] recorded
  assert(functions.UnitClassification and functions.UnitClassification.mouseover, "UnitClassification[mouseover] stream must exist")
  assert(#functions.UnitClassification.mouseover >= 1, "UnitClassification[mouseover] must have entry")
  assert(functions.UnitClassification.mouseover[1].v == "worldboss", "UnitClassification[mouseover] must be worldboss")

  -- Verify UnitReaction["player"]["target"] recorded
  assert(functions.UnitReaction and functions.UnitReaction.player and functions.UnitReaction.player.target,
    "UnitReaction[player][target] stream must exist")
  assert(#functions.UnitReaction.player.target >= 1, "UnitReaction[player][target] must have entry")
  assert(functions.UnitReaction.player.target[1].v == 5, "UnitReaction[player][target] must be 5 (neutral)")

  -- Verify UnitReaction["player"]["mouseover"] recorded
  assert(functions.UnitReaction and functions.UnitReaction.player and functions.UnitReaction.player.mouseover,
    "UnitReaction[player][mouseover] stream must exist")
  assert(#functions.UnitReaction.player.mouseover >= 1, "UnitReaction[player][mouseover] must have entry")
  assert(functions.UnitReaction.player.mouseover[1].v == 8, "UnitReaction[player][mouseover] must be 8 (exalted)")

  -- Verify player units are NOT sampled (guard works)
  env.UnitIsPlayer = function(_token) return true end
  env.UnitLevel = function(_token) return 60 end
  env.UnitClassification = function(_token) return "normal" end
  env.UnitReaction = function(_unit, _token) return 5 end

  SendEvent(runtime, "PLAYER_TARGET_CHANGED")

  -- Should NOT have new entries (player guard)
  assert(#functions.UnitLevel.target == 1, "Player target must not be sampled")
  assert(#functions.UnitClassification.target == 1, "Player target must not be sampled")
  assert(#functions.UnitReaction.player.target == 1, "Player target must not be sampled for reaction")
end

-- Merchant tracker tests
local function TestMerchantClassicAPIRecordsItems()
  local runtime = NewRuntime({ "Modules/Trackers/Merchant.lua" })
  -- Classic API only
  runtime.env.GetMerchantNumItems = function() return 2 end
  runtime.env.GetMerchantItemInfo = function(index)
    if index == 1 then
      return "Minor Healing Potion", 133765, 5000, 1, -1, true, true, false, nil, nil
    elseif index == 2 then
      return "Linen Cloth", 2596, 100, 1, 20, true, true, false, nil, nil
    end
    return nil
  end

  runtime.core.StartCapture("merchant classic")
  SendEvent(runtime, "MERCHANT_SHOW")
  AdvanceTo(runtime, 1)
  SendEvent(runtime, "MERCHANT_CLOSED")

  local session = Session(runtime)
  -- Count stream only records changes, so 1 entry (value is always 2)
  local count = session.functions.GetMerchantNumItems and #session.functions.GetMerchantNumItems or 0
  assert(count == 1, "GetMerchantNumItems must record initial value (1 entry since value doesn't change), got " .. count)
  -- Legacy item streams record initial values
  local item1 = session.functions.GetMerchantItemInfo[1]
  local item2 = session.functions.GetMerchantItemInfo[2]
  assert(item1 and #item1 == 1 and item1[1].v[1] == "Minor Healing Potion", "First item name must be recorded")
  assert(item1[1].v[3] == 5000, "First item price must be recorded")
  assert(item1[1].v[5] == -1, "First item numAvailable (-1 unlimited) must be recorded")
  assert(item2 and #item2 == 1 and item2[1].v[1] == "Linen Cloth", "Second item name must be recorded")
  assert(item2[1].v[5] == 20, "Second item numAvailable (limited) must be recorded")
  -- Modern API stream should not exist when C_MerchantFrame is not available
  assert(next(session.functions["C_MerchantFrame.GetItemInfo"]) == nil,
    "Modern API stream should not exist when C_MerchantFrame is not available")
end

local function TestMerchantModernAPIRecordsItems()
  local runtime = NewRuntime({ "Modules/Trackers/Merchant.lua" })
  -- Both APIs available
  runtime.env.GetMerchantNumItems = function() return 2 end
  runtime.env.GetMerchantItemInfo = function(index)
    if index == 1 then
      return "Minor Healing Potion", 133765, 5000, 1, -1, true, true, false, nil, nil
    elseif index == 2 then
      return "Linen Cloth", 2596, 100, 1, 20, true, true, false, nil, nil
    end
    return nil
  end
  runtime.env.C_MerchantFrame = {
    GetItemInfo = function(index)
      if index == 1 then
        return { name = "Minor Healing Potion", texture = 133765, price = 5000, stackCount = 1,
                 numAvailable = -1, isPurchasable = true, isUsable = true, hasExtendedCost = false,
                 currencyID = nil, spellID = nil, isQuestStartItem = false }
      elseif index == 2 then
        return { name = "Linen Cloth", texture = 2596, price = 100, stackCount = 1,
                 numAvailable = 20, isPurchasable = true, isUsable = true, hasExtendedCost = false,
                 currencyID = nil, spellID = nil, isQuestStartItem = false }
      end
      return nil
    end,
  }

  runtime.core.StartCapture("merchant both")
  SendEvent(runtime, "MERCHANT_SHOW")
  AdvanceTo(runtime, 1)
  SendEvent(runtime, "MERCHANT_CLOSED")

  local session = Session(runtime)
  -- Count stream only records changes, so 1 entry (value is always 2)
  local count = session.functions.GetMerchantNumItems and #session.functions.GetMerchantNumItems or 0
  assert(count == 1, "GetMerchantNumItems must record initial value (1 entry since value doesn't change), got " .. count)
  -- Legacy item streams
  local legacy1 = session.functions.GetMerchantItemInfo[1]
  local legacy2 = session.functions.GetMerchantItemInfo[2]
  assert(legacy1 and #legacy1 == 1 and legacy1[1].v[1] == "Minor Healing Potion", "Legacy: First item name must be recorded")
  assert(legacy2 and #legacy2 == 1 and legacy2[1].v[1] == "Linen Cloth", "Legacy: Second item name must be recorded")
  -- Modern item streams
  local modern1 = session.functions["C_MerchantFrame.GetItemInfo"][1]
  local modern2 = session.functions["C_MerchantFrame.GetItemInfo"][2]
  assert(modern1 and #modern1 == 1 and modern1[1].v[1].name == "Minor Healing Potion",
    "Modern: First item name must be recorded")
  assert(modern1[1].v[1].price == 5000, "Modern: First item price must be recorded")
  assert(modern1[1].v[1].numAvailable == -1, "Modern: First item numAvailable must be recorded")
  assert(modern2 and #modern2 == 1 and modern2[1].v[1].name == "Linen Cloth",
    "Modern: Second item name must be recorded")
  assert(modern2[1].v[1].numAvailable == 20, "Modern: Second item numAvailable must be recorded")
end

local function TestMerchantCloseProbesKnownIndices()
  local runtime = NewRuntime({ "Modules/Trackers/Merchant.lua" })
  runtime.env.GetMerchantNumItems = function() return 2 end
  runtime.env.GetMerchantItemInfo = function(index)
    if index == 1 then return "Item One", 1, 100, 1, -1, true, true, false, nil, nil end
    if index == 2 then return "Item Two", 2, 200, 1, 5, true, true, false, nil, nil end
    return nil
  end

  runtime.core.StartCapture("merchant close probes")
  SendEvent(runtime, "MERCHANT_SHOW")
  -- Change item count to 1 (simulating item sold out)
  runtime.env.GetMerchantNumItems = function() return 1 end
  runtime.env.GetMerchantItemInfo = function(index)
    if index == 1 then return "Item One", 1, 100, 1, -1, true, true, false, nil, nil end
    return nil
  end
  SendEvent(runtime, "MERCHANT_CLOSED")

  local session = Session(runtime)
  local item1 = session.functions.GetMerchantItemInfo[1]
  local item2 = session.functions.GetMerchantItemInfo[2]
  -- Item 1: value doesn't change, so only 1 entry (initial)
  assert(item1 and #item1 == 1, "First item value unchanged, only initial sample recorded")
  -- Item 2: value changes from "Item Two" to nil at close, so 2 entries
  assert(item2 and #item2 == 2, "Second item must have initial and close samples (value changes to nil)")
  -- Close sample for item 2 should have nil (API returns nil for index > count)
  assert(item2[2].v == nil or (item2[2].v and item2[2].v[1] == nil),
    "Close sample for removed item must record the observed nil/error return")
end

local function TestMerchantRecordsChangedItems()
  local runtime = NewRuntime({ "Modules/Trackers/Merchant.lua" })
  runtime.env.GetMerchantNumItems = function() return 1 end
  runtime.env.GetMerchantItemInfo = function(index)
    if index == 1 then return "Minor Healing Potion", 133765, 5000, 1, -1, true, true, false, nil, nil end
    return nil
  end

  runtime.core.StartCapture("merchant changed items")
  SendEvent(runtime, "MERCHANT_SHOW")
  AdvanceTo(runtime, 1)
  -- Item changed
  runtime.env.GetMerchantItemInfo = function(index)
    if index == 1 then return "Major Healing Potion", 133766, 10000, 1, -1, true, true, false, nil, nil end
    return nil
  end
  SendEvent(runtime, "MERCHANT_SHOW") -- Simulate update
  AdvanceTo(runtime, 2)
  SendEvent(runtime, "MERCHANT_CLOSED")

  local session = Session(runtime)
  local item1 = session.functions.GetMerchantItemInfo[1]
  -- Item changes from Minor to Major at second SHOW, then no change at CLOSE
  -- So we get: initial (Minor), update (Major) = 2 entries
  assert(#item1 == 2, "Must record initial and update samples when item changes, got " .. #item1)
  assert(item1[1].v[1] == "Minor Healing Potion", "First sample must be initial item")
  assert(item1[2].v[1] == "Major Healing Potion", "Second sample must be updated item")
end

local function TestMerchantEmptyVendor()
  local runtime = NewRuntime({ "Modules/Trackers/Merchant.lua" })
  runtime.env.GetMerchantNumItems = function() return 0 end

  runtime.core.StartCapture("merchant empty")
  SendEvent(runtime, "MERCHANT_SHOW")
  AdvanceTo(runtime, 1)
  SendEvent(runtime, "MERCHANT_CLOSED")

  local session = Session(runtime)
  assert(session.functions.GetMerchantNumItems and #session.functions.GetMerchantNumItems >= 1,
    "GetMerchantNumItems must be sampled even when zero")
  assert(session.functions.GetMerchantNumItems[#session.functions.GetMerchantNumItems].v == 0,
    "Last sample must be 0 for empty vendor")
  -- No item streams should be created
  assert(next(session.functions.GetMerchantItemInfo) == nil,
    "No item streams should exist for empty vendor")
end

local function TestQuestPOTracker()
  local runtime = NewRuntime({ "Modules/Trackers/QuestPOI.lua" })
  local env = runtime.env

  -- Mock C_QuestLog.GetMapForQuestPOIs (returns starting map for POIs)
  -- Mock C_QuestLog.GetQuestsOnMap (returns POIs for a given map)
  env.C_QuestLog = {
    GetMapForQuestPOIs = function()
      return 1453 -- Stormwind
    end,
    ---@param uiMapID number
    GetQuestsOnMap = function(uiMapID)
      if uiMapID == 1453 then
        return {
          {
            questID = 12345,
            mapID = 1453,
            x = 0.5,
            y = 0.5,
            isQuestStart = true,
            isDaily = false,
            isCombatAllyQuest = false,
            isMeta = false,
            inProgress = true,
            isMapIndicatorQuest = false,
            numObjectives = 2,
            childDepth = 0,
          },
        }
      end
      return {}
    end
  }

  runtime.core.StartCapture("quest poi test")
  local session = Session(runtime)
  local functions = session.functions

  -- Verify C_QuestLog.GetQuestsOnMap stream exists
  assert(functions["C_QuestLog.GetQuestsOnMap"], "C_QuestLog.GetQuestsOnMap stream must exist")
  assert(functions["C_QuestLog.GetQuestsOnMap"][1453], "C_QuestLog.GetQuestsOnMap[1453] stream must exist")
  assert(#functions["C_QuestLog.GetQuestsOnMap"][1453] >= 1, "C_QuestLog.GetQuestsOnMap[1453] must have entry")

  local pois = functions["C_QuestLog.GetQuestsOnMap"][1453][1].v
  assert(#pois == 1, "Must have 1 POI")
  assert(pois[1].questID == 12345, "POI questID must match")
  assert(pois[1].x == 0.5, "POI x must match")
  assert(pois[1].y == 0.5, "POI y must match")
  assert(pois[1].isQuestStart == true, "POI isQuestStart must be true")
end

local function TestQuestPOTrackerQuestPOIUpdate()
  local runtime = NewRuntime({ "Modules/Trackers/QuestPOI.lua" })
  local env = runtime.env

  -- Track whether POI data should be "updated"
  local useUpdatedPOI = false

  env.C_QuestLog = {
    GetMapForQuestPOIs = function()
      return 1453 -- Stormwind
    end,
    ---@param uiMapID number
    GetQuestsOnMap = function(uiMapID)
      if uiMapID == 1453 then
        if useUpdatedPOI then
          return {
            {
              questID = 67890,
              questTagType = 1,
              numObjectives = 3,
              mapID = 1453,
              x = 0.7,
              y = 0.8,
              isQuestStart = false,
              isDaily = true,
              isCombatAllyQuest = false,
              isMeta = false,
              inProgress = false,
              isMapIndicatorQuest = true,
              childDepth = 0,
            },
          }
        end
        return {
          {
            questID = 12345,
            questTagType = 0,
            numObjectives = 2,
            mapID = 1453,
            x = 0.5,
            y = 0.5,
            isQuestStart = true,
            isDaily = false,
            isCombatAllyQuest = false,
            isMeta = false,
            inProgress = true,
            isMapIndicatorQuest = false,
            childDepth = 0,
          },
        }
      end
      return {}
    end
  }

  runtime.core.StartCapture("quest poi update test")
  local session = Session(runtime)
  local functions = session.functions

  -- Verify initial POI recorded
  assert(functions["C_QuestLog.GetQuestsOnMap"][1453], "Initial stream must exist")
  assert(#functions["C_QuestLog.GetQuestsOnMap"][1453] == 1, "Must have 1 entry initially")
  assert(functions["C_QuestLog.GetQuestsOnMap"][1453][1].v[1].questID == 12345, "Initial POI must match")

  -- Now change the POI data and send QUEST_POI_UPDATE
  useUpdatedPOI = true
  SendEvent(runtime, "QUEST_POI_UPDATE")

  -- Re-fetch session after event
  session = Session(runtime)
  functions = session.functions

  -- Verify updated POI was recorded as a new entry
  assert(functions["C_QuestLog.GetQuestsOnMap"][1453], "Stream must still exist")
  assert(#functions["C_QuestLog.GetQuestsOnMap"][1453] == 2, "Must have 2 entries after update, got " .. #functions["C_QuestLog.GetQuestsOnMap"][1453])

  local updatedPOI = functions["C_QuestLog.GetQuestsOnMap"][1453][2].v
  assert(#updatedPOI == 1, "Must have 1 updated POI")
  assert(updatedPOI[1].questID == 67890, "Updated POI questID must match, got " .. updatedPOI[1].questID)
  assert(updatedPOI[1].x == 0.7, "Updated POI x must match")
  assert(updatedPOI[1].y == 0.8, "Updated POI y must match")
  assert(updatedPOI[1].isQuestStart == false, "Updated POI isQuestStart must be false")
  assert(updatedPOI[1].isDaily == true, "Updated POI isDaily must be true")
  assert(updatedPOI[1].isMapIndicatorQuest == true, "Updated POI isMapIndicatorQuest must be true")
end

-- These tests model restriction flags; native secret-value enforcement is verified in the game client.
local function TestRestrictedUnitIdentityDoesNotInterruptCapture()
  local runtime = NewRuntime({ "Modules/Trackers/UnitInteraction.lua" })
  local restrictedGuid = "Creature-0-1-1-1-123-000001"
  local npcGuid = "Creature-0-1-1-1-456-000002"
  local targetGuid, restricted = restrictedGuid, true
  runtime.env.issecretvalue = function(value) return restricted and value == restrictedGuid end
  runtime.env.UnitGUID = function(token)
    if token == "target" then return targetGuid end
    if token == "npc" then return npcGuid end
    if token == "questnpc" then return "Player-1-000001" end
  end
  runtime.env.UnitName = function(token)
    if token == "target" then
      assert(not restricted, "A restricted unit's name must not be sampled")
      if targetGuid then return "Creature" end
    elseif token == "npc" then return "Quest giver" end
  end
  runtime.env.C_GossipInfo = {
    GetAvailableQuests = function() return {{questID = 123}} end,
    GetActiveQuests = function() return {} end,
  }
  runtime.core.StartCapture("restricted identity")
  SendEvent(runtime, "GOSSIP_SHOW")
  local functions = Session(runtime).functions
  assert(#functions.UnitGUID.target == 0 and #functions.UnitName.target == 0, "Restricted identity must not be recorded")
  assert(#functions.UnitGUID.questnpc == 0, "Readable player identity must still be filtered")
  assert(functions.UnitGUID.npc[1].v == npcGuid, "Other readable units must still be recorded")
  assert(functions["C_GossipInfo.GetAvailableQuests"][1].v[1].questID == 123, "Gossip recording must continue")
  restricted = false
  AdvanceTo(runtime, 1)
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")
  assert(#functions.UnitGUID.target == 1 and functions.UnitGUID.target[1].v == restrictedGuid, "Readable identity must resume recording")
  assert(functions.UnitName.target[1].v[1] == "Creature", "Readable name must resume recording")
  restricted = true
  AdvanceTo(runtime, 2)
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")
  assert(#functions.UnitGUID.target == 1 and #functions.UnitName.target == 1, "A restriction must not invent a nil observation")
  targetGuid, restricted = nil, false
  AdvanceTo(runtime, 3)
  SendEvent(runtime, "PLAYER_TARGET_CHANGED")
  assert(#functions.UnitGUID.target == 2 and functions.UnitGUID.target[2].v == nil, "A real nil observation must still be recorded")
end

local function TestRestrictedLootSourcePreservesOtherLootData()
  local runtime = NewRuntime({ "Modules/Trackers/Loot.lua" })
  local guid = "Creature-0-1-1-1-123-000001"
  local restricted = true
  runtime.env.issecretvalue = function(value) return restricted and value == guid end
  runtime.env.GetNumLootItems = function() return 1 end
  runtime.env.GetLootSlotInfo = function() return "icon", "Item", 1, 0, 1, false, false, 0, true end
  runtime.env.GetLootSourceInfo = function() return guid, 1 end
  runtime.env.GetLootSlotLink = function() return "itemlink" end
  runtime.env.GetLootSlotType = function() return 1 end
  runtime.core.StartCapture("restricted loot source")
  SendEvent(runtime, "LOOT_READY")
  local functions = Session(runtime).functions
  assert(#functions.GetLootSourceInfo[1] == 0, "Restricted loot source must be discarded")
  assert(functions.GetLootSlotLink[1][1].v == "itemlink", "The item link must remain usable")
  assert(functions.GetLootSlotInfo[1][1].v[2] == "Item", "Other item information must remain usable")
  restricted = false
  AdvanceTo(runtime, 1)
  SendEvent(runtime, "LOOT_READY")
  assert(functions.GetLootSourceInfo[1][1].v[1] == guid, "Readable source must resume recording")
end

local function TestGameObjectTrackerRecordsObjectName()
  local runtime = NewRuntime({ "Modules/Trackers/GameObject.lua" })
  runtime.env.TooltipDataProcessor = {
    AddTooltipPostCall = function(tooltipType, callback)
      -- Simulate the callback being called with Object tooltip data
      if tooltipType == 4 then -- Enum.TooltipDataType.Object
        callback(nil, {
          type = 4, -- Enum.TooltipDataType.Object
          -- Note: Object tooltips don't include GUID in practice
          guid = nil,
          lines = {
            { type = 2, leftText = "Copper Vein", rightText = "" }, -- Enum.TooltipDataLineType.UnitName
            { type = 1, leftText = "Requires Mining (1)", rightText = "" },
          },
        })
      end
    end,
  }
  runtime.env.Enum = {
    TooltipDataType = { Object = 4 },
    TooltipDataLineType = { UnitName = 2 },
  }

  runtime.core.StartCapture("game object test")

  local session = Session(runtime)
  local functions = session.functions

  -- Verify GameObjectName stream recorded
  assert(functions.GameObjectName, "GameObjectName stream must exist")
  assert(#functions.GameObjectName >= 1, "GameObjectName must have entry")
  assert(functions.GameObjectName[1].v == "Copper Vein", "GameObjectName must be 'Copper Vein'")

  -- GameObjectGUID stream should NOT exist (tooltips don't have GUID)
  assert(functions.GameObjectGUID == nil, "GameObjectGUID stream must not exist")
end

local function TestGameObjectTrackerSkipsUnreadableUnitNameLine()
  local secretMarker = {}
  local runtime = NewRuntime({ "Modules/Trackers/GameObject.lua" })
  -- Simulate WoW's issecretvalue for tainted data
  runtime.env.issecretvalue = function(value) return value == secretMarker end

  runtime.env.TooltipDataProcessor = {
    AddTooltipPostCall = function(tooltipType, callback)
      if tooltipType == 4 then -- Enum.TooltipDataType.Object
        callback(nil, {
          type = 4,
          lines = {
            -- The UnitName line is secret/tainted - name must stay unset
            { type = 2, leftText = secretMarker, rightText = "" },
            -- A readable description line must not be used as a fallback
            { type = 1, leftText = "Requires Mining (1)", rightText = "" },
          },
        })
      end
    end,
  }
  runtime.env.Enum = {
    TooltipDataType = { Object = 4 },
    TooltipDataLineType = { UnitName = 2 },
  }

  runtime.core.StartCapture("game object unreadable unit name")

  local session = Session(runtime)
  local functions = session.functions

  -- No name must be recorded: the UnitName line exists but is unreadable,
  -- and the description line must never be used as a stand-in for the name.
  assert(functions.GameObjectName == nil or #functions.GameObjectName == 0,
    "GameObjectName must not record when the UnitName line is unreadable")
end

local function TestGameObjectTrackerSkipsWhenNoUnitNameLine()
  local runtime = NewRuntime({ "Modules/Trackers/GameObject.lua" })
  runtime.env.TooltipDataProcessor = {
    AddTooltipPostCall = function(tooltipType, callback)
      if tooltipType == 4 then -- Enum.TooltipDataType.Object
        callback(nil, {
          type = 4,
          lines = {
            -- No line is tagged UnitName (not a real-world case, but defensive).
            { type = 1, leftText = "Rusty Chest", rightText = "" },
            { type = 1, leftText = "Locked", rightText = "" },
          },
        })
      end
    end,
  }
  runtime.env.Enum = {
    TooltipDataType = { Object = 4 },
    TooltipDataLineType = { UnitName = 2 },
  }

  runtime.core.StartCapture("game object no unit name line")

  local session = Session(runtime)
  local functions = session.functions

  assert(functions.GameObjectName == nil or #functions.GameObjectName == 0,
    "GameObjectName must not record without a UnitName line")
end

---@type { name: string, run: fun() }[]
local tests = {
  { name = "restricted unit identity does not interrupt capture", run = TestRestrictedUnitIdentityDoesNotInterruptCapture },
  { name = "restricted loot source preserves other loot data", run = TestRestrictedLootSourcePreservesOtherLootData },
  { name = "greeting retries unsettled titles", run = function() TestGreetingRetry("stale") end },
  { name = "greeting retries failed calls", run = function() TestGreetingRetry("error") end },
  { name = "greeting close cancels delayed samples", run = TestGreetingClose },
  { name = "greeting capture restart resets probes", run = TestGreetingRestart },
  { name = "greeting records quest ids per index", run = TestGreetingQuestIDs },
  { name = "quest flags sampled for detail, gossip and greeting quests", run = TestQuestFlagsSampledForDialogQuests },
  { name = "quest flags record real nil once and skip failed calls", run = TestQuestFlagsNilAndFailedCalls },
  { name = "quest flags append only on change, including tables", run = TestQuestFlagsChangeOnly },
  { name = "quest dialog sanitizes titles and quest-line names", run = TestQuestDialogSanitizesTitlesAndQuestLineNames },
  { name = "quest flags absent without their APIs", run = TestQuestFlagsAbsentWithoutApis },
  { name = "session contract preserves legacy saves", run = TestSessionContract },
  { name = "spellbook preserves observed tuple arity", run = TestSpellBookArity },
  { name = "export scrubs player identity", run = TestExportScrubsPlayerIdentity },
  { name = "export payload never exposes sourceSessions", run = TestExportPayloadNeverExposesSourceSessions },
  { name = "export excludes deleted (reported) sessions", run = TestExportPayloadExcludesDeletedSessions },
  { name = "export includes only new sessions after export", run = TestExportPayloadIncludesOnlyNewSessionsAfterExport },
  { name = "SaveCapture prunes oldest sessions past the cap", run = TestSaveCapturePrunesOldestSessionsPastCap },
  { name = "export deletes and restarts live session after report", run = TestExportPayloadLiveSessionDeletedAndRestarted },
  { name = "export serialization round-trips", run = TestExportSerializationRoundTrips },
  { name = "currentSession linked on StartCapture", run = TestCurrentSessionLinkedOnStartCapture },
  { name = "SaveCapture clears currentSession", run = TestSaveCaptureClearsCurrentSession },
  { name = "fresh character login initializes SavedVariables", run = TestLoginInitializesFreshCharacter },
  { name = "recover currentSession on login before auto-start", run = TestRecoverCurrentSessionOnLogin },
  { name = "recover currentSession discarded when consent declined", run = TestRecoverCurrentSessionDiscardedWhenConsentDeclined },
  { name = "recover currentSession discarded when consent undecided", run = TestRecoverCurrentSessionDiscardedWhenConsentUndecided },
  { name = "consent undecided shows prompt and does not auto-start", run = TestConsentUndecidedShowsPromptAndDoesNotAutoStart },
  { name = "consent declined blocks auto-start and manual start", run = TestConsentDeclinedBlocksEverything },
  { name = "consent accepted prints reminder and allows auto-start", run = TestConsentAcceptedPrintsReminderAndAllowsAutoStart },
  { name = "consent accept immediately starts capture", run = TestConsentAcceptImmediatelyStartsCapture },
  { name = "declining consent stops and discards an active capture", run = TestDecliningConsentStopsAndDiscardsActiveCapture },
  { name = "consent prompt reopens without changing consent", run = TestConsentPromptReopensWithoutChangingConsent },
  { name = "accepting consent preserves an active capture", run = TestAcceptingConsentPreservesActiveCapture },
  { name = "dialog uses modern assets when available", run = TestDialogUsesModernAssetsWhenAvailable },
  { name = "dialog falls back when an atlas is missing", run = TestDialogFallsBackWhenAnAtlasIsMissing },
  { name = "dialog fits long labels and wrapped text", run = TestDialogFitsLongLabelsAndWrappedText },
  { name = "share reminder silent without saved sessions", run = TestShareReminderNotDueWithoutSavedSessions },
  { name = "share reminder paused by opening export", run = TestShareReminderSuppressedAfterExportOpened },
  { name = "inn reminder not due before 1 hour of play", run = TestInnReminderNotDueBeforeOneHour },
  { name = "inn reminder fires when entering inn after 1 hour", run = TestInnReminderFiresWhenEnteringInnAfterOneHour },
  { name = "inn reminder fires when entering inn before 1 hour and staying", run = TestInnReminderFiresWhenEnteringInnBeforeOneHourAndStaying },
  { name = "inn reminder fires on zone change to inn", run = TestInnReminderFiresOnZoneChangeToInn },
  { name = "inn reminder resets after firing", run = TestInnReminderResetsAfterFiring },
  { name = "inn reminder does not fire without shareable data", run = TestInnReminderDoesNotFireWithoutShareableData },
  { name = "inn reminder paused by export window", run = TestInnReminderPausedByExportWindow },
  { name = "inn reminder token invalidates stale callbacks", run = TestInnReminderTokenInvalidatesStaleCallbacks },
  { name = "inn reminder multiple reentries before timer", run = TestInnReminderMultipleReentriesBeforeTimer },
  { name = "inn reminder reentry after reminder uses lastReminderAt baseline", run = TestInnReminderReentryAfterReminderUsesLastReminderAtBaseline },
  { name = "inn reminder starts reminders when StartShareReminders called while in inn", run = TestInnReminderStartsWhenInInn },
  { name = "inn reminder timer re-arms after export window opened", run = TestInnReminderTimerRearmsAfterExportWindowOpened },
  { name = "fresh login resets sessionStart", run = TestFreshLoginResetsSessionStart },
  { name = "ParseGUIDKind classifies GUID prefixes", run = TestParseGUIDKindClassifiesPrefixes },
  { name = "SanitizeText redacts local player's own name", run = TestSanitizeTextRedactsLocalPlayerName },
  { name = "SanitizeText redacts group roster names", run = TestSanitizeTextRedactsGroupRosterNames },
  { name = "SanitizeText leaves unrelated text alone", run = TestSanitizeTextLeavesUnrelatedTextAlone },
  { name = "SanitizeChatMsgArgs strips names and player guid", run = TestSanitizeChatMsgArgsStripsNamesAndGuid },
  { name = "SanitizeChatMsgArgs preserves non-player guid", run = TestSanitizeChatMsgArgsPreservesNonPlayerGuid },
  { name = "CHAT_MSG_LOOT event is sanitized in session", run = TestChatMsgLootEventIsSanitizedInSession },
  { name = "UnitInteraction skips a player target's guid/name", run = TestUnitInteractionSkipsPlayerTarget },
  { name = "UnitInteraction still records an npc target's guid/name", run = TestUnitInteractionRecordsNpcTarget },
  { name = "Loot skips a player-sourced loot guid", run = TestLootSkipsPlayerSourcedGuid },
  { name = "Loot still records an npc-sourced loot guid", run = TestLootRecordsNpcSourcedGuid },
  { name = "Loot retains repeated source and link probes for timestamp correlation", run = TestLootKeepsUnchangedCorrelatedSamples },
  { name = "Loot allows a multi-pair all-npc source tuple", run = TestLootAllowsMultiPairAllNpcSourceTuple },
  { name = "Loot skips a multi-pair source tuple with any player guid", run = TestLootSkipsMultiPairSourceTupleWithAnyPlayerGuid },
  { name = "Loot skips an unrecognized (non npc/object/item) guid kind", run = TestLootSkipsUnrecognizedGuidKind },
  { name = "UnitInteraction skips an unrecognized guid kind", run = TestUnitInteractionSkipsUnrecognizedGuidKind },
  { name = "SanitizeText escapes pattern-magic characters in names", run = TestSanitizeTextEscapesSpecialCharactersInNames },
  { name = "CHAT_MSG_LOOT args dispatched to trackers are sanitized", run = TestChatMsgLootDispatchArgsAreSanitized },
  { name = "unit interaction mouseover records GUID and name", run = TestUnitInteractionMouseover },
  { name = "UnitInteraction skips a mouseover player's guid/name", run = TestUnitInteractionMouseoverPlayerIsSkipped },
  { name = "unit state tracker records level and classification", run = TestUnitStateTracker },
  { name = "Merchant classic API records items", run = TestMerchantClassicAPIRecordsItems },
  { name = "Merchant modern API records items", run = TestMerchantModernAPIRecordsItems },
  { name = "Merchant close probes known indices", run = TestMerchantCloseProbesKnownIndices },
  { name = "Merchant records changed items", run = TestMerchantRecordsChangedItems },
  { name = "Merchant empty vendor", run = TestMerchantEmptyVendor },
  { name = "quest poi tracker records quest log pois", run = TestQuestPOTracker },
  { name = "quest poi tracker handles QUEST_POI_UPDATE", run = TestQuestPOTrackerQuestPOIUpdate },
  { name = "game object tracker records object name", run = TestGameObjectTrackerRecordsObjectName },
  { name = "game object tracker skips unreadable UnitName line", run = TestGameObjectTrackerSkipsUnreadableUnitNameLine },
  { name = "game object tracker skips when no UnitName line", run = TestGameObjectTrackerSkipsWhenNoUnitNameLine },
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
