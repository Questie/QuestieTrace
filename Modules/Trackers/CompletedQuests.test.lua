---@param env table<string, any>
---@return QuestieTraceCore, table
local function LoadCompletedQuestsTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.C_Timer = { After = function(_delay, _callback) end }
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
end)
