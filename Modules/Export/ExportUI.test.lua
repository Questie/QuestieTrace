--- A generic UI-widget stub: any method call is a no-op returning the stub
--- itself (for chaining), and any field access yields another stub (e.g.
--- frame.editBox). Good enough to exercise ExportUI.lua's control flow
--- without reimplementing the WoW widget API.
---@return table stub
local function NewWidgetStub()
  local stub
  stub = setmetatable({}, {
    __index = function(_, _key)
      return function() return stub end
    end,
  })
  return stub
end

---@param env table<string, any>
---@param l10n (fun(key: string, ...: any): string)? Defaults to returning the key unchanged.
---@return QuestieTraceCore
local function LoadExportUIModules(env, l10n)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = { l10n = l10n or function(s) return s end }
  env.CreateFrame = function() return NewWidgetStub() end
  env.GameTooltip = NewWidgetStub()
  env.GetTime = function() return 100 end
  env.QuestieTrace = { settings = { dataCollectionConsent = true } }
  env.QuestieTraceCore.DiscardCapture = function() end
  env.QuestieTraceCore.StartCapture = function() end
  env.QuestieTraceCore.MarkExportOpened = function() end
  env.QuestieTraceCore.DecodeExportPayload = function() return { functions = {} } end

  local exportChunk = assert(loadfile("Modules/Export/Export.lua"))
  setfenv(exportChunk, env)
  exportChunk()

  local uiChunk = assert(loadfile("Modules/Export/ExportUI.lua"))
  setfenv(uiChunk, env)
  uiChunk()

  return env.QuestieTraceCore
end

describe("ExportUI.ShowExportWindow encoding state transition", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core
  ---@type table
  local session

  before_each(function()
    env = {}
    Core = LoadExportUIModules(env)
    session = { encoded = "SESSION" }
    env.QuestieTraceCharacter = { sessions = { session } }
  end)

  it("should not delete sessions when the codec is unavailable", function()
    Core.EncodeExportPayload = function() return nil end

    Core.ShowExportWindow()

    assert.equal(1, #env.QuestieTraceCharacter.sessions)
  end)

  it("should not delete sessions when the codec is unavailable even after confirming", function()
    Core.EncodeExportPayload = function() return nil end

    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.equal(1, #env.QuestieTraceCharacter.sessions)
  end)

  it("should not delete sessions just by opening the window", function()
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end

    Core.ShowExportWindow()

    assert.equal(1, #env.QuestieTraceCharacter.sessions)
  end)

  it("should delete the reported session once the player confirms they reported it", function()
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end

    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.equal(0, #env.QuestieTraceCharacter.sessions)
  end)

  it("should not error on a second confirm without reopening", function()
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end

    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.has_no.errors(function()
      Core.ConfirmExportReported()
    end)
    assert.equal(0, #env.QuestieTraceCharacter.sessions)
  end)

  it("should not error when confirming without ever opening the window", function()
    assert.has_no.errors(function()
      Core.ConfirmExportReported()
    end)
    assert.equal(1, #env.QuestieTraceCharacter.sessions)
  end)

  it("should not delete sessions when reopening into a codec failure after a prior pending confirm", function()
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end
    Core.ShowExportWindow()

    Core.EncodeExportPayload = function() return nil end
    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.equal(1, #env.QuestieTraceCharacter.sessions)
  end)

  it("should not error when reopening with nothing left to export after a prior pending confirm", function()
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end
    Core.ShowExportWindow()

    env.QuestieTraceCharacter.sessions = {}
    Core.ShowExportWindow()

    assert.has_no.errors(function()
      Core.ConfirmExportReported()
    end)
  end)
end)

describe("ExportUI.ConfirmExportReported live session handling", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportUIModules(env)
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end
  end)

  it("should discard and restart the live session once reported", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = { functions = {}, events = { { t = 0, e = "PLAYER_LOGIN" } } },
    }
    local saveCalls, discardCalls, startCalls = 0, 0, 0
    Core.SaveCapture = function() saveCalls = saveCalls + 1 end
    Core.DiscardCapture = function()
      discardCalls = discardCalls + 1
      env.QuestieTraceCharacter.currentSession = nil
    end
    Core.StartCapture = function() startCalls = startCalls + 1 end

    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.equal(1, saveCalls)
    assert.equal(1, discardCalls)
    assert.equal(2, startCalls)
  end)
end)

describe("ExportUI export edit box read-only handling", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core
  ---@type table
  local exportEditBox
  ---@type table[]
  local editBoxes
  local setTextCalls

  before_each(function()
    env = {}
    Core = LoadExportUIModules(env)
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end
    env.QuestieTraceCharacter = { sessions = { { encoded = "SESSION" } } }
    setTextCalls = 0
    editBoxes = {}
    exportEditBox = nil

    env.CreateFrame = function(frameType)
      local widget = NewWidgetStub()
      if frameType == "EditBox" then
        local scripts = {}
        widget = setmetatable({
          scripts = scripts,
          SetScript = function(_, name, handler) scripts[name] = handler end,
          SetMultiLine = function(self, multiLine) self.multiLine = multiLine end,
          SetMaxLetters = function(self, maxLetters) self.maxLetters = maxLetters end,
          SetText = function(self, text)
            setTextCalls = setTextCalls + 1
            self.text = text
          end,
        }, getmetatable(widget))
        table.insert(editBoxes, widget)
      end
      return widget
    end
  end)

  it("should restore the export text when the user edits it", function()
    Core.ShowExportWindow()
    exportEditBox = editBoxes[2]
    assert.is_not_nil(exportEditBox)
    assert.is_not_nil(exportEditBox.originalText)

    exportEditBox.text = "tampered"

    exportEditBox.scripts.OnTextChanged(exportEditBox, true)

    assert.equal(exportEditBox.originalText, exportEditBox.text)
  end)

  it("should not rewrite the export text on programmatic changes", function()
    Core.ShowExportWindow()
    exportEditBox = editBoxes[2]
    assert.is_not_nil(exportEditBox)
    assert.is_not_nil(exportEditBox.originalText)
    local callsAfterShow = setTextCalls

    exportEditBox.scripts.OnTextChanged(exportEditBox, false)

    assert.equal(callsAfterShow, setTextCalls)
  end)

  it("should not rewrite the export text when the cursor moves", function()
    Core.ShowExportWindow()
    exportEditBox = editBoxes[2]
    assert.is_not_nil(exportEditBox)
    assert.is_not_nil(exportEditBox.originalText)

    assert.is_nil(exportEditBox.scripts.OnCursorChanged)
  end)

  it("should show the export text in a single-line edit box", function()
    Core.ShowExportWindow()
    exportEditBox = editBoxes[2]
    assert.is_not_nil(exportEditBox)
    assert.is_not_nil(exportEditBox.originalText)

    assert.is_not_true(exportEditBox.multiLine)
  end)

  it("should not limit the length of the export text", function()
    Core.ShowExportWindow()
    exportEditBox = editBoxes[2]
    assert.is_not_nil(exportEditBox)
    assert.is_not_nil(exportEditBox.originalText)

    assert.equal(0, exportEditBox.maxLetters)
  end)
end)

--- A widget that records what the export window does to it: text, whether
--- it is shown, its texture and its anchor points. Child font strings and
--- textures are new recorders, so each one can be inspected on its own. Any
--- other method is a no-op returning the recorder.
---@return table recorder
local function NewWidgetRecorder()
  local recorder
  recorder = setmetatable({
    points = {},
    SetText = function(self, text) self.text = text end,
    Show = function(self) self.shown = true end,
    Hide = function(self) self.shown = false end,
    SetTexture = function(self, texture) self.texture = texture end,
    SetPoint = function(self, ...) self.points[#self.points + 1] = { ... } end,
    CreateFontString = function() return NewWidgetRecorder() end,
    CreateTexture = function() return NewWidgetRecorder() end,
  }, {
    __index = function()
      return function() return recorder end
    end,
  })
  return recorder
end

---@param count integer
---@return table[] events
local function MakeEvents(count)
  local events = {}
  for i = 1, count do
    events[i] = { t = i, e = "PLAYER_LOGIN" }
  end
  return events
end

--- Remove color codes (|cAARRGGBB ... |r), leaving the text the player reads.
---@param text string?
---@return string?
local function StripColors(text)
  if not text then return nil end
  return (text:gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|r", ""))
end

describe("ExportUI export statistics", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core
  ---@type table?
  local exportFrame

  --- The window's statistic rows; fails the test if the window was never built.
  ---@return table
  local function Rows()
    return assert(exportFrame, "export frame was not created").stats
  end

  --- A statistic's text as the player reads it.
  ---@param key string Row key in ExportStatRows.
  ---@return string?
  local function StatText(key)
    return StripColors(Rows()[key].text.text)
  end

  ---@return boolean?
  local function StatsShown()
    return assert(exportFrame, "export frame was not created").statsFrame.shown
  end

  before_each(function()
    env = {}
    -- Format like the real l10n, so the count actually reaches the text.
    Core = LoadExportUIModules(env, function(s, ...)
      if select("#", ...) > 0 then
        return s:format(...)
      end
      return s
    end)
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end
    Core.DecodeExportPayload = function() return { functions = {}, events = MakeEvents(3) } end
    Core.SaveCapture = function() end
    env.BreakUpLargeNumbers = nil
    env.SecondsToTime = nil
    env.QuestieTraceCharacter = { sessions = { { encoded = "SESSION" } } }

    -- The default stub returns itself from every method, so every font string
    -- would be the frame itself. Frames (the window and its stats container)
    -- are recorders instead; edit boxes and buttons stay plain stubs.
    exportFrame = nil
    env.CreateFrame = function(frameType, name)
      if frameType ~= "Frame" then
        return NewWidgetStub()
      end
      local frame = NewWidgetRecorder()
      if name == "QuestieTraceExportFrame" then
        exportFrame = frame
      end
      return frame
    end
  end)

  it("should show the event count of the exported sessions", function()
    Core.ShowExportWindow()

    assert.equal("Events captured: 3", StatText("events"))
    assert.is_true(StatsShown())
  end)

  it("should count saved sessions and the live session together", function()
    env.QuestieTraceCharacter.currentSession = { functions = {}, events = MakeEvents(2) }

    Core.ShowExportWindow()

    assert.equal("Events captured: 5", StatText("events"))
    assert.is_true(StatsShown())
  end)

  it("should hide the statistics when there is nothing to export", function()
    env.QuestieTraceCharacter.sessions = {}

    Core.ShowExportWindow()

    assert.is_false(StatsShown())
  end)

  it("should hide the statistics when the codec is unavailable", function()
    Core.EncodeExportPayload = function() return nil end

    Core.ShowExportWindow()

    assert.is_false(StatsShown())
  end)

  it("should hide the statistics when reopening with nothing left to export", function()
    Core.ShowExportWindow()
    assert.is_true(StatsShown())

    env.QuestieTraceCharacter.sessions = {}
    Core.ShowExportWindow()

    assert.is_false(StatsShown())
  end)

  it("should format the count with BreakUpLargeNumbers when it exists", function()
    Core.DecodeExportPayload = function() return { functions = {}, events = MakeEvents(1234) } end
    env.BreakUpLargeNumbers = function(n) return n == 1234 and "1,234" or tostring(n) end

    Core.ShowExportWindow()

    assert.equal("Events captured: 1,234", StatText("events"))
  end)

  it("should show the plain count when BreakUpLargeNumbers does not exist", function()
    Core.DecodeExportPayload = function() return { functions = {}, events = MakeEvents(1234) } end

    Core.ShowExportWindow()

    assert.equal("Events captured: 1234", StatText("events"))
  end)

  it("should show time recorded and quest counts", function()
    Core.DecodeExportPayload = function()
      return {
        functions = {},
        duration = 3725,
        events = {
          { t = 1, e = "QUEST_ACCEPTED" },
          { t = 2, e = "QUEST_ACCEPTED" },
          { t = 3, e = "QUEST_TURNED_IN" },
        },
      }
    end

    Core.ShowExportWindow()

    assert.equal("Events captured: 3", StatText("events"))
    assert.equal("Time recorded: 1:02:05", StatText("time"))
    assert.equal("Quests accepted: 2", StatText("questsAccepted"))
    assert.equal("Quests turned in: 1", StatText("questsTurnedIn"))
  end)

  it("should show the value in gold after a white label", function()
    Core.ShowExportWindow()

    assert.equal("Events captured: |cffffd1003|r", Rows().events.text.text)
  end)

  it("should lay out events and time on the left and quests on the right", function()
    Core.ShowExportWindow()

    local expected = {
      events = { icon = "Interface\\Icons\\INV_Misc_Note_02", x = 0, y = 0 },
      time = { icon = "Interface\\Icons\\INV_Misc_PocketWatch_01", x = 0, y = -22 },
      questsAccepted = { icon = "Interface\\GossipFrame\\AvailableQuestIcon", x = 210, y = 0 },
      questsTurnedIn = { icon = "Interface\\GossipFrame\\ActiveQuestIcon", x = 210, y = -22 },
    }
    for key, want in pairs(expected) do
      local icon = Rows()[key].icon
      assert.equal(want.icon, icon.texture, key .. " icon")
      local point = icon.points[1]
      assert.equal("TOPLEFT", point[1], key .. " icon anchor")
      assert.equal(want.x, point[4], key .. " column")
      assert.equal(want.y, point[5], key .. " row")
    end
  end)

  it("should center each statistic's text vertically on its icon", function()
    Core.ShowExportWindow()

    for key, row in pairs(Rows()) do
      local point = row.text.points[1]
      assert.equal("LEFT", point[1], key .. " text point")
      assert.equal(row.icon, point[2], key .. " anchored to its own icon")
      assert.equal("RIGHT", point[3], key .. " icon point")
      assert.equal(0, point[5], key .. " vertical offset")
    end
  end)

  it("should format time recorded with SecondsToTime when it exists", function()
    env.SecondsToTime = function(seconds) return seconds == 3 and "3 Sec" or "?" end

    Core.ShowExportWindow()

    assert.equal("Time recorded: 3 Sec", StatText("time"))
  end)

  it("should fall back to the plain time when SecondsToTime returns nothing", function()
    Core.DecodeExportPayload = function() return { functions = {}, duration = 0, events = MakeEvents(1) } end
    env.SecondsToTime = function() return "" end

    Core.ShowExportWindow()

    assert.equal("Time recorded: 0:00:00", StatText("time"))
  end)
end)
