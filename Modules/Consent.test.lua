---@param env table<string, any>
---@return QuestieTraceCore
local function LoadConsentModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.QuestieTrace = { schemaVersion = 13, settings = {} }
  env.QuestieTraceCharacter = { sessions = {}, currentSession = nil }
  env.GetLocale = function() return "enUS" end

  local chunk

  -- Load globals.lua first (dependencies)
  chunk = assert(loadfile("Modules/globals.lua"))
  setfenv(chunk, env)
  chunk()

  -- Load Privacy.lua
  chunk = assert(loadfile("Modules/Privacy.lua"))
  setfenv(chunk, env)
  chunk()

  -- Load localization
  chunk = assert(loadfile("Modules/Localization/l10n.lua"))
  setfenv(chunk, env)
  chunk()

  chunk = assert(loadfile("Modules/Localization/Translations/Consent.lua"))
  setfenv(chunk, env)
  chunk()

  -- Load Consent.lua (no UI dependencies)
  chunk = assert(loadfile("Modules/Consent.lua"))
  setfenv(chunk, env)
  chunk()

  -- Add mocks for Core functions used by Consent.lua but defined elsewhere
  env.QuestieTraceCore.GetCaptureState = function() return "idle" end
  env.QuestieTraceCore.StartCapture = function() end
  env.QuestieTraceCore.DiscardCapture = function() end
  env.QuestieTraceCore.StartShareReminders = function() end
  env.QuestieTraceCore.Print = function() end
  env.QuestieTraceCore.ShowConsentPrompt = function() end

  return env.QuestieTraceCore
end

describe("Consent", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadConsentModule(env)
  end)

  describe("HandleConsentOnLogin", function()
    it("should show consent prompt when consent is nil (undecided)", function()
      env.QuestieTrace.settings.dataCollectionConsent = nil

      local promptShown = false
      Core.ShowConsentPrompt = function() promptShown = true end

      Core.HandleConsentOnLogin()

      assert.is_true(promptShown)
    end)

    it("should not start capture when consent is nil", function()
      env.QuestieTrace.settings.dataCollectionConsent = nil

      local captureStarted = false
      Core.StartCapture = function() captureStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_false(captureStarted)
    end)

    it("should print reminder and start capture when consent is true", function()
      env.QuestieTrace.settings.dataCollectionConsent = true

      local reminderPrinted = false
      local captureStarted = false
      Core.PrintConsentReminder = function() reminderPrinted = true end
      Core.StartCapture = function() captureStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_true(reminderPrinted)
      assert.is_true(captureStarted)
    end)

    it("should do nothing when consent is false (declined)", function()
      env.QuestieTrace.settings.dataCollectionConsent = false

      local reminderPrinted = false
      local captureStarted = false
      local promptShown = false
      Core.PrintConsentReminder = function() reminderPrinted = true end
      Core.StartCapture = function() captureStarted = true end
      Core.ShowConsentPrompt = function() promptShown = true end

      Core.HandleConsentOnLogin()

      assert.is_false(reminderPrinted)
      assert.is_false(captureStarted)
      assert.is_false(promptShown)
    end)

    it("should not start capture if already running", function()
      env.QuestieTrace.settings.dataCollectionConsent = true

      local captureStarted = false
      Core.GetCaptureState = function() return "running" end
      Core.StartCapture = function() captureStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_false(captureStarted)
    end)

    it("should not start capture if stopped_unsaved", function()
      env.QuestieTrace.settings.dataCollectionConsent = true

      local captureStarted = false
      Core.GetCaptureState = function() return "stopped_unsaved" end
      Core.StartCapture = function() captureStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_false(captureStarted)
    end)

    it("should start share reminders when consent is true", function()
      env.QuestieTrace.settings.dataCollectionConsent = true

      local remindersStarted = false
      Core.StartShareReminders = function() remindersStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_true(remindersStarted)
    end)

    it("should not start share reminders when consent is false", function()
      env.QuestieTrace.settings.dataCollectionConsent = false

      local remindersStarted = false
      Core.StartShareReminders = function() remindersStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_false(remindersStarted)
    end)

    it("should not start share reminders when consent is nil", function()
      env.QuestieTrace.settings.dataCollectionConsent = nil

      local remindersStarted = false
      Core.StartShareReminders = function() remindersStarted = true end

      Core.HandleConsentOnLogin()

      assert.is_false(remindersStarted)
    end)
  end)
end)