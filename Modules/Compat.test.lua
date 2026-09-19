---@param env table<string, any>
---@return QuestieTraceCore
local function LoadCompatModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}

  local chunkGlobals = assert(loadfile("Modules/globals.lua"))
  setfenv(chunkGlobals, env)
  chunkGlobals()

  local chunk = assert(loadfile("Modules/Compat.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

describe("Compat", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadCompatModule(env)
  end)

  describe("GetQuestLogTitle", function()
    it("should use C_QuestLog.GetInfo when available", function()
      env.C_QuestLog = {
        GetInfo = function(questLogIndex)
          if questLogIndex ~= 1 then return nil end
          return {
            title = "A Quest",
            level = 5,
            isHeader = false,
            isCollapsed = false,
            frequency = 1,
            questID = 42,
            startEvent = false,
            isOnMap = true,
            hasLocalPOI = false,
            isTask = false,
            isBounty = false,
            isStory = false,
            isHidden = false,
            isScaling = false,
          }
        end,
        GetQuestTagInfo = function(questID)
          if questID ~= 42 then return nil end
          return { tagName = "Group" }
        end,
        IsComplete = function(questID)
          return questID == 42
        end,
      }

      local info = Core.Compat.GetQuestLogTitle(1)

      assert.are.equal("A Quest", info.title)
      assert.are.equal(5, info.level)
      assert.are.equal("Group", info.questTag)
      assert.is_false(info.isHeader)
      assert.is_false(info.isCollapsed)
      assert.are.equal(1, info.isComplete)
      assert.are.equal(1, info.frequency)
      assert.are.equal(42, info.questID)
    end)

    it("should return nil when C_QuestLog.GetInfo has no entry at the index", function()
      env.C_QuestLog = {
        GetInfo = function() return nil end,
      }

      local info = Core.Compat.GetQuestLogTitle(99)
      assert.is_nil(info)
    end)

    it("should fall back to the global GetQuestLogTitle when C_QuestLog.GetInfo is unavailable", function()
      env.GetQuestLogTitle = function(_questLogIndex)
        return "Legacy Quest", 3, nil, false, false, nil, 1, 7
      end

      local info = Core.Compat.GetQuestLogTitle(2)
      assert.are.equal("Legacy Quest", info.title)
      assert.are.equal(3, info.level)
      assert.are.equal(7, info.questID)
    end)

    it("should return nil and report an error when neither API is available", function()
      local errorSpy = spy.new(function() end)
      Core.Error = errorSpy

      local info = Core.Compat.GetQuestLogTitle(1)

      assert.is_nil(info)
      assert.spy(errorSpy).was.called()
    end)
  end)
end)
