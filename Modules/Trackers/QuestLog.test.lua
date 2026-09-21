---@param env table<string, any>
---@return QuestieTraceCore, table
local function LoadQuestLogTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.C_Timer = { After = function(_delay, _callback) end }
  env.GetTime = function() return 0 end
  env.GetTimePreciseSec = function() return 0 end

  local chunkGlobals = assert(loadfile("Modules/globals.lua"))
  setfenv(chunkGlobals, env)
  chunkGlobals()

  local Core = env.QuestieTraceCore
  Core.SanitizeText = function(s) return s end

  -- Minimal C_QuestLog mock: quest-log rows are supplied per-test via
  -- env.questLogRows, a plain array of { title, isHeader, questID } indexed
  -- like the real quest log. Each raw API below is probed independently by
  -- the tracker, mirroring how the real client exposes them.
  env.C_QuestLog = {
    GetInfo = function(questLogIndex)
      local row = env.questLogRows[questLogIndex]
      if not row then return nil end
      return {
        title = row.title,
        isHeader = row.isHeader or false,
        questID = row.questID,
      }
    end,
    GetLogIndexForQuestID = function(questID)
      for index, row in ipairs(env.questLogRows) do
        if row.questID == questID then return index end
      end
      return nil
    end,
  }

  ---@type table[]
  local trackers = {}
  local realRegisterTracker = Core.RegisterTracker
  Core.RegisterTracker = function(tracker)
    trackers[#trackers + 1] = tracker
    return realRegisterTracker(tracker)
  end

  local chunk = assert(loadfile("Modules/Trackers/QuestLog.lua"))
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
    session = { functions = {} },
  }
end

describe("QuestLog tracker", function()
  ---@type table<string, any>
  local env
  ---@type table
  local tracker

  before_each(function()
    env = { questLogRows = {} }
    tracker = select(2, LoadQuestLogTracker(env))
  end)

  it("should record the preceding header title as QuestLogZone for each quest under it", function()
    env.questLogRows = {
      { title = "Westfall", isHeader = true },
      { title = "A Quest", questID = 100 },
      { title = "Another Quest", questID = 101 },
      { title = "Loch Modan", isHeader = true },
      { title = "Third Quest", questID = 200 },
    }

    local capture = NewCapture()
    tracker.Init(capture)

    local zone = capture.session.functions["QuestLogZone"]
    assert.are.equal("Westfall", zone[100][1].v)
    assert.are.equal("Westfall", zone[101][1].v)
    assert.are.equal("Loch Modan", zone[200][1].v)
  end)

  it("should not include header rows themselves in the QuestLog quest-ID stream", function()
    env.questLogRows = {
      { title = "Westfall", isHeader = true },
      { title = "A Quest", questID = 100 },
    }

    local capture = NewCapture()
    tracker.Init(capture)

    local questLogStream = capture.session.functions["QuestLog"]
    assert.are.same({ 100 }, questLogStream[1].v)
  end)

  it("should not append a duplicate QuestLogZone entry when the header hasn't changed between samples", function()
    env.questLogRows = {
      { title = "Westfall", isHeader = true },
      { title = "A Quest", questID = 100 },
    }

    local capture = NewCapture()
    tracker.Init(capture)
    tracker.OnEvent(capture, "QUEST_LOG_UPDATE")

    local zone = capture.session.functions["QuestLogZone"]
    assert.are.equal(1, #zone[100])
  end)

  it("should record C_QuestLog.GetInfo as its own raw stream keyed by questID", function()
    env.questLogRows = {
      { title = "A Quest", questID = 100 },
    }

    local capture = NewCapture()
    tracker.Init(capture)

    local infoStream = capture.session.functions["C_QuestLog.GetInfo"]
    assert.is_not_nil(infoStream[100], "C_QuestLog.GetInfo stream should exist for quest 100")
    assert.are.equal("A Quest", infoStream[100][1].v.title)
  end)

  it("should record C_QuestLog.IsComplete and C_QuestLog.IsFailed as independent raw streams", function()
    env.questLogRows = {
      { title = "Failed Quest", questID = 100 },
    }
    env.C_QuestLog.IsComplete = function(questID) return questID == 100 and false end
    env.C_QuestLog.IsFailed = function(questID) return questID == 100 and true end

    local capture = NewCapture()
    tracker.Init(capture)

    local isCompleteStream = capture.session.functions["C_QuestLog.IsComplete"]
    local isFailedStream = capture.session.functions["C_QuestLog.IsFailed"]
    assert.is_false(isCompleteStream[100][1].v)
    assert.is_true(isFailedStream[100][1].v)
  end)

  it("should record C_QuestLog.GetQuestTagInfo as its own raw stream", function()
    env.questLogRows = {
      { title = "Group Quest", questID = 100 },
    }
    env.C_QuestLog.GetQuestTagInfo = function(questID)
      if questID ~= 100 then return nil end
      return { tagName = "Group" }
    end

    local capture = NewCapture()
    tracker.Init(capture)

    local tagStream = capture.session.functions["C_QuestLog.GetQuestTagInfo"]
    assert.are.equal("Group", tagStream[100][1].v.tagName)
  end)

  it("should not probe quest-specific APIs for header rows", function()
    env.questLogRows = {
      { title = "Zone Header", isHeader = true, questID = 0 },
    }
    env.C_QuestLog.IsComplete = function() error("should not be called for header rows") end
    env.C_QuestLog.IsFailed = function() error("should not be called for header rows") end

    local capture = NewCapture()
    tracker.Init(capture)

    -- Headers should not appear in the QuestLog stream (only valid quests with questID > 0)
    local questLogStream = capture.session.functions["QuestLog"]
    assert.are.same({}, questLogStream[1].v, "QuestLog array should be empty since header has questID = 0")
    assert.is_nil(capture.session.functions["C_QuestLog.IsComplete"])
  end)

  it("should record the legacy GetQuestLogTitle global as a raw tuple only when it exists", function()
    env.questLogRows = {
      { title = "Legacy Quest", questID = 100 },
    }
    env.GetQuestLogTitle = function(questLogIndex)
      local row = env.questLogRows[questLogIndex]
      if not row then return nil end
      return row.title, 3, nil, false, false, nil, 1, row.questID
    end

    local capture = NewCapture()
    tracker.Init(capture)

    local titleStream = capture.session.functions["GetQuestLogTitle"]
    assert.is_not_nil(titleStream[100], "legacy GetQuestLogTitle stream should exist for quest 100")
    local titleTuple = titleStream[100][1].v
    assert.are.equal("Legacy Quest", titleTuple[1])
    assert.are.equal(100, titleTuple[8])
  end)
end)
