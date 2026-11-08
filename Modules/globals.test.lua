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

  describe("CreateSampleScheduler", function()
    ---@type { delay: number, callback: function }[]
    local timers
    ---@type table
    local capture
    ---@type number
    local sampleCount
    ---@type boolean[]
    local changes

    ---@return fun(capture: table)
    local function NewScheduler()
      return Core.CreateSampleScheduler(function()
        sampleCount = sampleCount + 1
        return table.remove(changes, 1) or false
      end)
    end

    ---Run all currently queued timers with the given delay, in order.
    ---@param delay number?
    local function RunTimers(delay)
      local pending = timers
      timers = {}
      for _, timer in ipairs(pending) do
        if delay == nil or timer.delay == delay then
          timer.callback()
        else
          timers[#timers + 1] = timer
        end
      end
    end

    before_each(function()
      timers = {}
      sampleCount = 0
      changes = {}
      capture = { active = true, token = 1 }
      env.C_Timer = {
        After = function(delay, callback)
          timers[#timers + 1] = { delay = delay, callback = callback }
        end,
      }
    end)

    it("should sample immediately on the first request", function()
      NewScheduler()(capture)

      assert.are.equal(1, sampleCount)
    end)

    it("should sample once more on the next frame for a burst of requests in one frame", function()
      local request = NewScheduler()

      request(capture)
      request(capture)
      request(capture)
      assert.are.equal(1, sampleCount)

      RunTimers(0)
      assert.are.equal(2, sampleCount)
    end)

    it("should keep sampling on later frames after a sample throws", function()
      local shouldThrow = true
      local request = Core.CreateSampleScheduler(function()
        sampleCount = sampleCount + 1
        if shouldThrow then
          error("boom")
        end
        return false
      end)

      assert.has_error(function() request(capture) end)
      shouldThrow = false
      RunTimers(0)
      request(capture)

      assert.are.equal(2, sampleCount)
    end)

    it("should continue the follow-up chain after a follow-up sample throws", function()
      local request = Core.CreateSampleScheduler(function()
        sampleCount = sampleCount + 1
        if sampleCount == 2 then
          error("boom")
        end
        return true
      end)

      request(capture)
      RunTimers(0)
      assert.has_error(function() RunTimers() end)
      RunTimers()

      assert.are.equal(3, sampleCount)
    end)

    it("should not sample on the next frame when only one request happened", function()
      NewScheduler()(capture)

      RunTimers(0)

      assert.are.equal(1, sampleCount)
    end)

    it("should keep only one follow-up chain for a burst of requests", function()
      local request = NewScheduler()
      request(capture)
      request(capture)
      request(capture)
      RunTimers(0)
      sampleCount = 0

      RunTimers()

      assert.are.equal(1, sampleCount)
    end)

    it("should stop following up after three samples without changes", function()
      NewScheduler()(capture)
      RunTimers(0)
      sampleCount = 0

      for _ = 1, 10 do RunTimers() end

      assert.are.equal(3, sampleCount)
    end)

    it("should keep following up while samples record changes, up to the last delay", function()
      NewScheduler()(capture)
      RunTimers(0)
      sampleCount = 0
      changes = { true, true, true, true, true }

      for _ = 1, 10 do RunTimers() end

      assert.are.equal(#Core.RESAMPLE_DELAYS, sampleCount)
    end)

    it("should schedule follow-ups at the configured offsets", function()
      NewScheduler()(capture)
      RunTimers(0)
      changes = { true, true, true, true, true }

      local elapsed = 0
      local offsets = {}
      while #timers > 0 do
        elapsed = elapsed + timers[1].delay
        offsets[#offsets + 1] = elapsed
        RunTimers()
      end

      for i, expected in ipairs(Core.RESAMPLE_DELAYS) do
        assert.is_true(math.abs(offsets[i] - expected) < 1e-9)
      end
    end)

    it("should restart the follow-up chain when a request arrives during it", function()
      local request = NewScheduler()
      request(capture)
      RunTimers(0)
      RunTimers()
      request(capture)
      RunTimers(0)
      sampleCount = 0

      for _ = 1, 10 do RunTimers() end

      assert.are.equal(3, sampleCount)
    end)

    it("should sample immediately for a restarted capture in the same frame", function()
      local request = NewScheduler()
      request(capture)
      local restarted = { active = true, token = 2 }

      request(restarted)

      assert.are.equal(2, sampleCount)
    end)

    it("should take the next-frame sample for a restarted capture", function()
      local request = NewScheduler()
      request(capture)
      local restarted = { active = true, token = 2 }
      request(restarted)
      request(restarted)

      RunTimers(0)

      assert.are.equal(3, sampleCount)
    end)

    it("should not sample after the capture stopped", function()
      NewScheduler()(capture)
      capture.active = false
      sampleCount = 0

      for _ = 1, 10 do RunTimers() end

      assert.are.equal(0, sampleCount)
    end)
  end)
end)
