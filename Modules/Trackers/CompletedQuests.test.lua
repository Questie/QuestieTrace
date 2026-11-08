---@param env table<string, any>
---@return QuestieTraceCore, table
local function LoadCompletedQuestsTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.C_Timer = env.C_Timer or { After = function(_delay, _callback) end }
  env.GetTime = function() return 0 end
  env.GetTimePreciseSec = function() return 0 end
  env.wipe = function(t)
    for k in pairs(t) do t[k] = nil end
    return t
  end

  local chunkGlobals = assert(loadfile("Modules/globals.lua"))
  setfenv(chunkGlobals, env)
  chunkGlobals()

  local Core = env.QuestieTraceCore

  ---@type table[]
  local trackers = {}
  local realRegisterTracker = Core.RegisterTracker
  Core.RegisterTracker = function(tracker)
    trackers[#trackers + 1] = tracker
    return realRegisterTracker(tracker)
  end

  local chunk = assert(loadfile("Modules/Trackers/CompletedQuests.lua"))
  setfenv(chunk, env)
  chunk()

  return Core, trackers[1]
end

---@return table
local function NewCapture()
  return {
    active = true,
    token = 1,
    startedAt = 0,
    startedAtPrecise = 0,
    session = { functionsDelta = {} },
  }
end

describe("CompletedQuests tracker", function()
  ---@type table<string, any>
  local env
  ---@type table
  local tracker

  before_each(function()
    env = {}
  end)

  it("should record C_QuestLog.GetAllCompletedQuestIDs as its own independent delta stream", function()
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return { 200, 100 } end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.same({ 100, 200 }, stream.initial)
    assert.is_nil(capture.session.functionsDelta["GetQuestsCompleted"])
  end)

  it("should record the legacy GetQuestsCompleted as its own independent delta stream", function()
    env.GetQuestsCompleted = function(target)
      target[100] = true
      target[200] = true
      return target
    end

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    local stream = capture.session.functionsDelta["GetQuestsCompleted"]
    assert.are.same({ 100, 200 }, stream.initial)
    assert.is_nil(capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"])
  end)

  it("should record both streams independently when both APIs exist", function()
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return { 100 } end,
    }
    env.GetQuestsCompleted = function(target)
      target[200] = true
      return target
    end

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.same({ 100 }, capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"].initial)
    assert.are.same({ 200 }, capture.session.functionsDelta["GetQuestsCompleted"].initial)
  end)

  it("should append add-deltas to the modern stream without touching the legacy stream", function()
    local completedIds = { 100 }
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return completedIds end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    completedIds = { 100, 200 }
    tracker.OnEvent(capture, "QUEST_TURNED_IN")

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(1, #stream.delta)
    assert.are.same({ 200 }, stream.delta[1].add)
  end)

  it("should not record a delta when the completed quest count is unchanged", function()
    local completedIds = { 100, 200 }
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return completedIds end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    completedIds = { 200, 100 }
    tracker.OnEvent(capture, "QUEST_LOG_UPDATE")

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(0, #stream.delta)
  end)

  it("should record a swapped quest when the count is unchanged on QUEST_TURNED_IN", function()
    local completedIds = { 100, 200 }
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return completedIds end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    completedIds = { 300, 100 }
    tracker.OnEvent(capture, "QUEST_TURNED_IN")

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.same({ 300 }, stream.delta[1].add)
    assert.are.same({ 200 }, stream.delta[1].remove)
  end)

  it("should record a same-count replacement that only shows up in a follow-up sample", function()
    ---@type function[]
    local timers = {}
    env.C_Timer = {
      After = function(_delay, callback) timers[#timers + 1] = callback end,
    }
    local completedIds = { 100, 200 }
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return completedIds end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    tracker.OnEvent(capture, "QUEST_TURNED_IN")
    completedIds = { 300, 100 }
    while #timers > 0 do
      table.remove(timers, 1)()
    end

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(1, #stream.delta)
    assert.are.same({ 300 }, stream.delta[1].add)
    assert.are.same({ 200 }, stream.delta[1].remove)
  end)

  ---Set up a tracker driven by a virtual clock; timers fire in chronological order.
  ---@return table capture
  ---@return fun(delta: number) advance
  ---@return fun(event: string) send
  ---@return number[] sampleTimes
  local function SetUpClock()
    local now = 0
    ---@type { at: number, callback: function }[]
    local timers = {}
    ---@type number[]
    local sampleTimes = {}
    env.C_Timer = {
      After = function(delay, callback) timers[#timers + 1] = { at = now + delay, callback = callback } end,
    }
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function()
        sampleTimes[#sampleTimes + 1] = now
        return env.completedIds
      end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    env.GetTime = function() return now end
    local capture = NewCapture()
    tracker.Init(capture)

    local function Advance(to)
      while true do
        local nextIndex
        for index, timer in ipairs(timers) do
          if timer.at <= to and ((not nextIndex) or timer.at < timers[nextIndex].at) then
            nextIndex = index
          end
        end
        if (not nextIndex) then
          break
        end
        local timer = table.remove(timers, nextIndex)
        now = timer.at
        timer.callback()
      end
      now = to
    end

    Advance(0) -- finish the Init frame
    for i = #sampleTimes, 1, -1 do sampleTimes[i] = nil end -- ignore the Init samples

    return capture, function(delta) Advance(now + delta) end, function(event) tracker.OnEvent(capture, event) end, sampleTimes
  end

  it("should record completed quests that load shortly after Init", function()
    env.completedIds = { 100 }
    local capture, advance = SetUpClock()

    env.completedIds = { 100, 200 }
    advance(2)

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(1, #stream.delta)
    assert.are.same({ 200 }, stream.delta[1].add)
  end)

  it("should end the restarted follow-up chain at the last delay once samples settle", function()
    env.completedIds = { 100, 200 }
    local capture, advance, send, sampleTimes = SetUpClock()

    send("QUEST_TURNED_IN")
    advance(0.9)
    send("QUEST_LOG_UPDATE")
    advance(2.1)

    -- Restarted at 0.9: immediate sample, 3 quiet follow-ups (1.0, 1.25, 1.45), then the last delay (1.9)
    local lastSample = sampleTimes[#sampleTimes]
    assert.is_true(math.abs(lastSample - 1.9) < 1e-9)
    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(0, #stream.delta)
  end)

  it("should keep comparing full sets for a replacement that shows up late in a restarted chain", function()
    env.completedIds = { 100, 200 }
    local capture, advance, send = SetUpClock()

    send("QUEST_TURNED_IN")
    advance(0.9)
    send("QUEST_LOG_UPDATE")
    advance(0.5)
    send("QUEST_LOG_UPDATE") -- 1.4s: the chain from 0.9 is still active and restarts

    -- Visible only after the first window (1.5s) has passed, at the 1.95s sample
    advance(0.4)
    env.completedIds = { 300, 100 }
    advance(1.2)

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(1, #stream.delta)
    assert.are.same({ 300 }, stream.delta[1].add)
    assert.are.same({ 200 }, stream.delta[1].remove)
  end)

  it("should skip full comparison for a quest log update outside any turn-in window", function()
    local completedIds = { 100, 200 }
    env.C_QuestLog = {
      GetAllCompletedQuestIDs = function() return completedIds end,
    }

    tracker = select(2, LoadCompletedQuestsTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    completedIds = { 300, 100 }
    tracker.OnEvent(capture, "QUEST_LOG_UPDATE")

    local stream = capture.session.functionsDelta["C_QuestLog.GetAllCompletedQuestIDs"]
    assert.are.equal(0, #stream.delta)
  end)
end)
