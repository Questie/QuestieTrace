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
---@return QuestieTraceCore
local function LoadExportUIModules(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = { l10n = function(s) return s end }
  env.CreateFrame = function() return NewWidgetStub() end
  env.GameTooltip = NewWidgetStub()
  env.GetTime = function() return 100 end
  env.QuestieTrace = { settings = { autoStart = false } }
  env.QuestieTraceCore.DiscardCapture = function() end
  env.QuestieTraceCore.StartCapture = function() end
  env.QuestieTraceCore.MarkExportOpened = function() end

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
    session = { functions = {} }
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
    local discardCalls, startCalls = 0, 0
    Core.DiscardCapture = function()
      discardCalls = discardCalls + 1
      env.QuestieTraceCharacter.currentSession = nil
    end
    Core.StartCapture = function() startCalls = startCalls + 1 end
    env.QuestieTrace.settings.autoStart = true

    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.equal(1, discardCalls)
    assert.equal(1, startCalls)
  end)

  it("should not restart the live session when autoStart is disabled", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = { functions = {}, events = { { t = 0, e = "PLAYER_LOGIN" } } },
    }
    local startCalls = 0
    Core.DiscardCapture = function() env.QuestieTraceCharacter.currentSession = nil end
    Core.StartCapture = function() startCalls = startCalls + 1 end
    env.QuestieTrace.settings.autoStart = false

    Core.ShowExportWindow()
    Core.ConfirmExportReported()

    assert.equal(0, startCalls)
  end)
end)
