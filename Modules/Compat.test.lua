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

  describe("GetQuestLogIndexByID", function()
    it("should use C_QuestLog.GetLogIndexForQuestID when available", function()
      env.C_QuestLog = {
        GetLogIndexForQuestID = function(questID)
          if questID ~= 42 then return nil end
          return 3
        end,
      }

      local questLogIndex = Core.Compat.GetQuestLogIndexByID(42)

      assert.are.equal(3, questLogIndex)
    end)

    it("should fall back to the global GetQuestLogIndexByID when C_QuestLog.GetLogIndexForQuestID is unavailable", function()
      env.GetQuestLogIndexByID = function(questID)
        if questID ~= 42 then return nil end
        return 5
      end

      local questLogIndex = Core.Compat.GetQuestLogIndexByID(42)

      assert.are.equal(5, questLogIndex)
    end)

    it("should return nil and report an error when neither API is available", function()
      local errorSpy = spy.new(function() end)
      Core.Error = errorSpy

      local questLogIndex = Core.Compat.GetQuestLogIndexByID(42)

      assert.is_nil(questLogIndex)
      assert.spy(errorSpy).was.called()
    end)
  end)

  describe("GetQuestsCompleted", function()
    it("should use C_QuestLog.GetAllCompletedQuestIDs when available", function()
      env.C_QuestLog = {
        GetAllCompletedQuestIDs = function() return { 1, 2, 3 } end,
      }

      local completed = Core.Compat.GetQuestsCompleted()

      assert.is_true(completed[1])
      assert.is_true(completed[2])
      assert.is_true(completed[3])
      assert.is_nil(completed[4])
    end)

    it("should fill and return the given target table", function()
      env.C_QuestLog = {
        GetAllCompletedQuestIDs = function() return { 7 } end,
      }
      local target = {}

      local completed = Core.Compat.GetQuestsCompleted(target)

      assert.are.equal(target, completed)
      assert.is_true(completed[7])
    end)

    it("should fall back to the global GetQuestsCompleted when C_QuestLog.GetAllCompletedQuestIDs is unavailable", function()
      env.GetQuestsCompleted = function(target)
        target = target or {}
        target[9] = true
        return target
      end

      local completed = Core.Compat.GetQuestsCompleted()

      assert.is_true(completed[9])
    end)

    it("should return nil and report an error when neither API is available", function()
      local errorSpy = spy.new(function() end)
      Core.Error = errorSpy

      local completed = Core.Compat.GetQuestsCompleted()

      assert.is_nil(completed)
      assert.spy(errorSpy).was.called()
    end)
  end)

  describe("GetQuestResetTime", function()
    it("should use C_DateAndTime.GetSecondsUntilDailyReset when available", function()
      env.C_DateAndTime = {
        GetSecondsUntilDailyReset = function() return 12345 end,
      }

      assert.are.equal(12345, Core.Compat.GetQuestResetTime())
    end)

    it("should fall back to the global GetQuestResetTime when C_DateAndTime.GetSecondsUntilDailyReset is unavailable", function()
      env.GetQuestResetTime = function() return 6789 end

      assert.are.equal(6789, Core.Compat.GetQuestResetTime())
    end)

    it("should return nil and report an error when neither API is available", function()
      local errorSpy = spy.new(function() end)
      Core.Error = errorSpy

      local secondsUntilReset = Core.Compat.GetQuestResetTime()

      assert.is_nil(secondsUntilReset)
      assert.spy(errorSpy).was.called()
    end)
  end)
end)
