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
  env.GetTime = function() return 100 end
  env.QuestieTraceCore.FinalizeLiveSessionIfExported = function() end
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

  it("should not mark sessions exported when the codec is unavailable", function()
    Core.EncodeExportPayload = function() return nil end

    Core.ShowExportWindow()

    assert.is_nil(session.exportedAt)
  end)

  it("should not finalize the live session when the codec is unavailable", function()
    Core.EncodeExportPayload = function() return nil end
    local finalizeCalls = 0
    Core.FinalizeLiveSessionIfExported = function() finalizeCalls = finalizeCalls + 1 end

    Core.ShowExportWindow()

    assert.equal(0, finalizeCalls)
  end)

  it("should mark sessions exported once the codec succeeds", function()
    Core.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end

    Core.ShowExportWindow()

    assert.equal(100, session.exportedAt)
  end)
end)
