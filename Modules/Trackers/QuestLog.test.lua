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

  -- Minimal Compat mock: quest-log rows are supplied per-test via env.questLogRows,
  -- a plain array of { title, isHeader, questID } indexed like the real quest log.
  Core.Compat = {
    GetQuestLogTitle = function(questLogIndex)
      local row = env.questLogRows[questLogIndex]
      if not row then return nil end
      return {
        title = row.title,
        isHeader = row.isHeader or false,
        questID = row.questID,
      }
    end,
    GetQuestLogIndexByID = function(questID)
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

  it("should map C_QuestLog.IsFailed to isComplete = -1", function()
    env.questLogRows = {
      { title = "Failed Quest", questID = 100, isComplete = -1 },
    }

    -- Override the mocked Compat.GetQuestLogTitle to support isComplete
    local Core = env.QuestieTraceCore
    Core.Compat.GetQuestLogTitle = function(questLogIndex)
      local row = env.questLogRows[questLogIndex]
      if not row then return nil end
      return {
        title = row.title,
        isHeader = row.isHeader or false,
        questID = row.questID,
        level = 0,
        frequency = 0,
        startEvent = false,
        isOnMap = false,
        hasLocalPOI = false,
        isTask = false,
        isBounty = false,
        isStory = false,
        isHidden = false,
        isScaling = false,
        isComplete = row.isComplete,
      }
    end

    local capture = NewCapture()
    tracker.Init(capture)

    -- Retrieve GetQuestLogTitle stream for quest 100
    local titleStream = capture.session.functions["GetQuestLogTitle"]
    assert.is_not_nil(titleStream[100], "GetQuestLogTitle stream should exist for quest 100")
    assert.is_not_nil(titleStream[100][1], "GetQuestLogTitle should have at least one entry")
    local titleData = titleStream[100][1].v
    assert.are.equal(-1, titleData.isComplete)
  end)

  it("should skip quest-specific APIs for header rows and return nil isComplete", function()
    env.questLogRows = {
      { title = "Zone Header", isHeader = true, questID = 0 },
    }

    -- Override the mocked Compat.GetQuestLogTitle to support headers
    local Core = env.QuestieTraceCore
    Core.Compat.GetQuestLogTitle = function(questLogIndex)
      local row = env.questLogRows[questLogIndex]
      if not row then return nil end
      return {
        title = row.title,
        isHeader = row.isHeader or false,
        questID = row.questID,
        level = 0,
        frequency = 0,
        startEvent = false,
        isOnMap = false,
        hasLocalPOI = false,
        isTask = false,
        isBounty = false,
        isStory = false,
        isHidden = false,
        isScaling = false,
        isComplete = nil,  -- Should be nil for headers since no quest-specific APIs are called
      }
    end

    local capture = NewCapture()
    tracker.Init(capture)

    -- Headers should not appear in the QuestLog stream (only valid quests with questID > 0)
    local questLogStream = capture.session.functions["QuestLog"]
    assert.is_not_nil(questLogStream, "QuestLog stream should exist")
    -- The QuestLog stream at [1] contains the array of quest IDs
    assert.are.same({}, questLogStream[1].v, "QuestLog array should be empty since header has questID = 0")
  end)
end)
