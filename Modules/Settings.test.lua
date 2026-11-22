---@param env table<string, any>
---@return QuestieTraceCore
local function LoadSettingsModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.QuestieTrace = { settings = {} }
  env.GetLocale = function() return "enUS" end

  for _, path in ipairs({
    "Modules/globals.lua",
    "Modules/Privacy.lua",
    "Modules/Localization/l10n.lua",
    "Modules/Localization/Translations/Consent.lua",
  }) do
    local chunk = assert(loadfile(path))
    setfenv(chunk, env)
    chunk()
  end

  return env.QuestieTraceCore
end

---@param env table<string, any>
local function MockSettingsApi(env)
  env.registered = { categories = {}, settings = {}, checkboxes = {}, addOnCategories = {} }
  env.Settings = {
    VarType = { Boolean = "boolean" },
    RegisterVerticalLayoutCategory = function(name)
      local category = { name = name }
      env.registered.categories[#env.registered.categories + 1] = category
      return category
    end,
    RegisterProxySetting = function(category, variable, variableType, name, defaultValue, getValue, setValue)
      local setting = { category = category, variable = variable, variableType = variableType, name = name, defaultValue = defaultValue, getValue = getValue, setValue = setValue }
      env.registered.settings[#env.registered.settings + 1] = setting
      return setting
    end,
    CreateCheckbox = function(category, setting, tooltip)
      env.registered.checkboxes[#env.registered.checkboxes + 1] = { category = category, setting = setting, tooltip = tooltip }
    end,
    RegisterAddOnCategory = function(category)
      env.registered.addOnCategories[#env.registered.addOnCategories + 1] = category
    end,
  }
end

---@param env table<string, any>
local function LoadSettingsFile(env)
  local chunk = assert(loadfile("Modules/Settings.lua"))
  setfenv(chunk, env)
  chunk()
end

describe("Settings", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadSettingsModule(env)
    MockSettingsApi(env)
  end)

  it("should register one addon category with a single consent checkbox", function()
    LoadSettingsFile(env)
    Core.RegisterSettingsPanel()

    assert.are.equal(1, #env.registered.categories)
    assert.are.equal("QuestieTrace", env.registered.categories[1].name)
    assert.are.equal(env.registered.categories[1], env.registered.addOnCategories[1])
    assert.are.equal(1, #env.registered.settings)
    assert.are.equal(1, #env.registered.checkboxes)
    assert.are.equal("boolean", env.registered.settings[1].variableType)
    assert.are.equal("Allow data collection", env.registered.settings[1].name)
  end)

  it("should show the checkbox as checked only when consent was granted", function()
    LoadSettingsFile(env)
    Core.RegisterSettingsPanel()
    local setting = env.registered.settings[1]

    env.QuestieTrace.settings.dataCollectionConsent = nil
    assert.is_false(setting.getValue())
    env.QuestieTrace.settings.dataCollectionConsent = false
    assert.is_false(setting.getValue())
    env.QuestieTrace.settings.dataCollectionConsent = true
    assert.is_true(setting.getValue())
  end)

  it("should grant consent when the checkbox is checked", function()
    LoadSettingsFile(env)
    Core.RegisterSettingsPanel()
    local granted
    Core.SetConsent = function(value) granted = value end

    env.registered.settings[1].setValue(true)

    assert.is_true(granted)
  end)

  it("should revoke consent when the checkbox is unchecked", function()
    LoadSettingsFile(env)
    Core.RegisterSettingsPanel()
    env.QuestieTrace.settings.dataCollectionConsent = true
    local granted
    Core.SetConsent = function(value) granted = value end

    env.registered.settings[1].setValue(false)

    assert.is_false(granted)
  end)

  it("should not change consent when the checkbox is set to its current value", function()
    LoadSettingsFile(env)
    Core.RegisterSettingsPanel()
    env.QuestieTrace.settings.dataCollectionConsent = true
    local called = false
    Core.SetConsent = function() called = true end

    env.registered.settings[1].setValue(true)

    assert.is_false(called)
  end)

  it("should register nothing on clients without the Settings API", function()
    env.Settings = nil

    LoadSettingsFile(env)
    Core.RegisterSettingsPanel()

    assert.are.equal(0, #env.registered.categories)
  end)
end)
