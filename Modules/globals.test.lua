---@param env table<string, any>
---@return QuestieTraceCore
local function LoadGlobalsModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}

  local chunk = assert(loadfile("Modules/globals.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

describe("globals", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadGlobalsModule(env)
  end)

  describe("Error", function()
    it("should print the message with an ERROR prefix", function()
      local printed

      env.print = function(message) printed = message end

      Core.Error("something went wrong")

      assert.is_not_nil(printed)
      assert.matches("ERROR:", printed, 1, true)
      assert.matches("something went wrong", printed, 1, true)
    end)

    it("should print regardless of debug settings", function()
      local printed

      env.print = function(message) printed = message end
      env.QuestieTrace = { settings = { debug = false } }

      Core.Error("still printed")

      assert.is_not_nil(printed)
    end)

    it("should join multiple values with spaces", function()
      local printed

      env.print = function(message) printed = message end

      Core.Error("API", "missing:", "GetQuestLogIndexByID")

      assert.matches("API missing: GetQuestLogIndexByID", printed, 1, true)
    end)
  end)
end)
