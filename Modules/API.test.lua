---@param env table<string, any>
---@return QuestieTraceCore
local function LoadApiModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.QuestieTrace = { schemaVersion = 18, settings = {} }

  local chunk = assert(loadfile("Modules/API.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

describe("API v1", function()
  local env, Core, v1, calls, state

  before_each(function()
    env = {}
    Core = LoadApiModule(env)
    v1 = env.QuestieTraceAPI.v1
    calls = {}
    state = "idle"
    Core.IsReady = function() return true end
    Core.GetCaptureState = function() return state end
    Core.SaveCapture = function() calls[#calls + 1] = "save" end
    Core.StartCapture = function() calls[#calls + 1] = "start"; state = "running" end
    Core.StopCapture = function() calls[#calls + 1] = "stop"; state = "stopped_unsaved" end
    Core.ShowConsentPrompt = function() calls[#calls + 1] = "prompt" end
    Core.ShowExportWindow = function() calls[#calls + 1] = "export" end
  end)

  describe("GetConsentState", function()
    it("should map the tri-state setting", function()
      assert.are.equal("unset", v1.GetConsentState())
      env.QuestieTrace.settings.dataCollectionConsent = false
      assert.are.equal("declined", v1.GetConsentState())
      env.QuestieTrace.settings.dataCollectionConsent = true
      assert.are.equal("granted", v1.GetConsentState())
    end)

    it("should report unset when the saved variables are missing", function()
      env.QuestieTrace = nil
      assert.are.equal("unset", v1.GetConsentState())
    end)
  end)

  describe("StartCapture", function()
    it("should not start when consent is unset", function()
      assert.is_false(v1.StartCapture())
      assert.are.same({}, calls)
    end)

    it("should not start when consent was declined", function()
      env.QuestieTrace.settings.dataCollectionConsent = false
      assert.is_false(v1.StartCapture())
      assert.are.same({}, calls)
    end)

    it("should save the previous session and start when consent is granted", function()
      env.QuestieTrace.settings.dataCollectionConsent = true
      assert.is_true(v1.StartCapture())
      assert.are.same({ "save", "start" }, calls)
    end)

    it("should not restart a running capture", function()
      env.QuestieTrace.settings.dataCollectionConsent = true
      state = "running"
      assert.is_true(v1.StartCapture())
      assert.are.same({}, calls)
    end)

    it("should not start before QuestieTrace is initialized", function()
      env.QuestieTrace.settings.dataCollectionConsent = true
      Core.IsReady = function() return false end
      assert.is_false(v1.StartCapture())
      assert.are.same({}, calls)
    end)

    it("should report false when core refused to start", function()
      env.QuestieTrace.settings.dataCollectionConsent = true
      Core.StartCapture = function() end
      assert.is_false(v1.StartCapture())
    end)
  end)

  describe("ShowExportWindow", function()
    it("should open the export window", function()
      v1.ShowExportWindow()
      assert.are.same({ "export" }, calls)
    end)
  end)
end)
