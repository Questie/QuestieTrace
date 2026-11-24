---@param env table<string, any>
---@return QuestieTraceCore, table
local function LoadSkillLinesTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.GetTime = function() return 0 end
  env.GetTimePreciseSec = function() return 0 end

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

  local chunk = assert(loadfile("Modules/Trackers/SkillLines.lua"))
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

describe("SkillLines tracker", function()
  ---@type table<string, any>
  local env
  ---@type table
  local tracker

  before_each(function()
    env = {}
  end)

  it("should record GetNumSkillLines and GetSkillLineInfo when both exist", function()
    env.GetNumSkillLines = function() return 1 end
    env.GetSkillLineInfo = function(index)
      if index ~= 1 then return nil end
      return "First Aid", 0, 0, 150
    end

    tracker = select(2, LoadSkillLinesTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.equal(1, capture.session.functions["GetNumSkillLines"][1].v)
    assert.are.equal("First Aid", capture.session.functions["GetSkillLineInfo"][1][1].v[1])
  end)

  it("should not create GetNumSkillLines/GetSkillLineInfo streams when the APIs don't exist", function()
    tracker = select(2, LoadSkillLinesTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.is_nil(capture.session.functions["GetNumSkillLines"])
    assert.is_nil(capture.session.functions["GetSkillLineInfo"])
  end)

  it("should record GetProfessions/GetProfessionInfo independently of GetNumSkillLines", function()
    env.GetProfessions = function() return 1, nil, nil, nil, nil end
    env.GetProfessionInfo = function(index)
      if index ~= 1 then return nil end
      return "Alchemy", "icon", 150, 300
    end

    tracker = select(2, LoadSkillLinesTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.equal(1, capture.session.functions["GetProfessions"][1].v[1])
    assert.are.equal("Alchemy", capture.session.functions["GetProfessionInfo"][1][1].v[1])
    assert.is_nil(capture.session.functions["GetNumSkillLines"])
  end)

  it("should record C_TradeSkillUI trade-skill lines whenever the API exists, not only as a fallback", function()
    env.GetNumSkillLines = function() return 0 end
    env.C_TradeSkillUI = {
      GetAllProfessionTradeSkillLines = function() return { 171 } end,
      GetTradeSkillLineInfoByID = function(id)
        if id ~= 171 then return nil end
        return { professionName = "Alchemy", skillLevel = 150 }
      end,
    }

    tracker = select(2, LoadSkillLinesTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    local lineStream = capture.session.functions["C_TradeSkillUI.GetAllProfessionTradeSkillLines"]
    assert.are.same({ 171 }, lineStream[1].v)

    local infoStream = capture.session.functions["C_TradeSkillUI.GetTradeSkillLineInfoByID"]
    assert.are.equal("Alchemy", infoStream[171][1].v.professionName)
  end)

  describe("on a client with C_SkillInfo", function()
    ---@type (table|false)[] The skill window's rows; false for a row without data
    local rows
    ---@type table<number, table> Rows under a collapsed header, by skill ID
    local collapsedRows
    ---@type table[]
    local timers
    ---@type string[] Errors passed to the client's error handler
    local errors

    ---@param skillID number
    ---@param rank number
    ---@return table
    local function Skill(skillID, rank)
      return {
        skillID = skillID, name = "Testplayer's skill", description = "Testplayer learned this", isHeader = false, isCollapsed = false,
        rank = rank, maxRank = 150, modifier = 5, tempPoints = 0, parentSkillLineID = 0, skillLineCategoryID = 9,
      }
    end

    ---@return table<number, table[]>
    local function Streams(capture)
      return capture.session.functions["C_SkillInfo.GetSkillLineInfoByID"]
    end

    ---Run every timer due until `target`.
    ---@param target number
    local function AdvanceTo(target)
      while timers[1] and timers[1].at <= target do
        local timer = table.remove(timers, 1)
        env.now = timer.at
        timer.callback()
      end
      env.now = target
    end

    ---@return table capture
    local function StartCapture()
      tracker = select(2, LoadSkillLinesTracker(env))
      -- Two clocks that run apart, both running before the capture starts.
      env.GetTime = function() return 100 + env.now end
      env.GetTimePreciseSec = function() return 5000 + env.now + 0.25 end
      local capture = NewCapture()
      capture.startedAt, capture.startedAtPrecise = 100, 5000
      tracker.Init(capture)
      return capture
    end

    ---Count the samples taken from now on.
    ---@return fun(): number
    local function CountSamples()
      local samples = 0
      local getNumSkillLines = env.C_SkillInfo.GetNumSkillLines
      env.C_SkillInfo.GetNumSkillLines = function()
        samples = samples + 1
        return getNumSkillLines()
      end
      return function() return samples end
    end

    before_each(function()
      env.now = 0
      errors = {}
      env.geterrorhandler = function() return function(err) errors[#errors + 1] = err end end
      timers = {}
      env.C_Timer = { After = function(delay, callback) timers[#timers + 1] = { at = env.now + delay, callback = callback } end }
      collapsedRows = {}
      rows = { { skillID = 9, name = "Secondary Skills", isHeader = true }, Skill(129, 44), Skill(762, 75) }
      env.C_SkillInfo = {
        GetNumSkillLines = function() return #rows end,
        GetSkillLineInfo = function(index) return rows[index] end,
        GetSkillLineInfoByID = function(skillID) return collapsedRows[skillID] end,
      }
    end)

    it("should record the changing numbers of each skill by its id, and no text", function()
      local capture = StartCapture()

      assert.are.same({
        [129] = { { t = 0, tp = 0.25, v = { rank = 44, maxRank = 150, modifier = 5, tempPoints = 0 } } },
        [762] = { { t = 0, tp = 0.25, v = { rank = 75, maxRank = 150, modifier = 5, tempPoints = 0 } } },
      }, Streams(capture))
    end)

    it("should record a skill again only when one of its numbers changed", function()
      local capture = StartCapture()

      for second, field in ipairs({ "rank", "maxRank", "modifier", "tempPoints" }) do
        env.now = second * 10
        tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
        assert.are.equal(second, #Streams(capture)[129], "Nothing changed yet")
        rows[2][field] = rows[2][field] + 1
        env.now = second * 10 + 5
        tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
        assert.are.equal(second + 1, #Streams(capture)[129], field .. " changed")
        assert.are.equal(rows[2][field], Streams(capture)[129][second + 1].v[field])
      end
      assert.are.equal(1, #Streams(capture)[762])
    end)

    it("should keep following a skill under a collapsed header, and record an unlearned one as gone", function()
      local capture = StartCapture()

      collapsedRows[129] = Skill(129, 60)
      rows[3], rows[2] = nil, nil
      env.now = 5
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")

      assert.are.equal(60, Streams(capture)[129][2].v.rank)
      assert.are.same({ t = 5, tp = 5.25 }, Streams(capture)[762][2])

      env.now = 10
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      assert.are.equal(2, #Streams(capture)[762])

      rows[2] = Skill(762, 1)
      env.now = 15
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      assert.are.equal(1, Streams(capture)[762][3].v.rank, "Learned again")
    end)

    it("should skip a row the client has no data for", function()
      rows[2] = false
      local capture = StartCapture()

      assert.is_nil(Streams(capture)[129])
      assert.are.equal(75, Streams(capture)[762][1].v.rank)
      assert.are.same({}, errors)
    end)

    it("should not use the old functions another addon provides, nor open the player's skill headers", function()
      env.GetNumSkillLines = function() error("must not be called") end
      env.GetSkillLineInfo = env.GetNumSkillLines
      env.ExpandSkillHeader = env.GetNumSkillLines
      env.C_SkillInfo.ExpandSkillHeader = env.GetNumSkillLines

      local capture = StartCapture()

      assert.are.equal(44, Streams(capture)[129][1].v.rank)
      assert.is_nil(capture.session.functions["GetNumSkillLines"])
      assert.is_nil(capture.session.functions["GetSkillLineInfo"])
    end)

    it("should report a failing client function and keep the capture running", function()
      env.C_SkillInfo.GetNumSkillLines = function() error("not available yet") end
      env.GetProfessions = function() return 1 end

      local capture = StartCapture()

      assert.are.same({}, Streams(capture))
      assert.are.equal(1, #errors)
      assert.is_truthy(errors[1]:find("not available yet", 1, true))
      assert.are.equal(1, capture.session.functions["GetProfessions"][1].v[1])
    end)

    it("should sample a burst of events right away and once more 0.1 seconds later", function()
      local capture = StartCapture()
      local samples = CountSamples()

      env.now = 0.05
      for _ = 1, 4101 do tracker.OnEvent(capture, "SKILL_LINES_CHANGED") end
      assert.are.equal(1, samples(), "The first event samples at once, however soon after the capture started")

      rows[2].rank = 45
      AdvanceTo(0.12)
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      AdvanceTo(0.14)
      assert.are.equal(1, samples())
      AdvanceTo(0.2)
      assert.are.equal(2, samples())
      local changed = Streams(capture)[129][2]
      assert.are.equal(45, changed.v.rank)
      -- 0.1 seconds after the first sample.
      assert.is_true(math.abs(changed.t - 0.15) < 1e-6)

      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      assert.are.equal(2, samples(), "0.05 seconds after the last sample")
      AdvanceTo(0.26)
      assert.are.equal(3, samples())
      AdvanceTo(60)
      assert.are.equal(3, samples(), "No events, no samples")
    end)

    it("should wait only for the rest of the 0.1 seconds", function()
      local capture = StartCapture()
      local samples = CountSamples()

      env.now = 10
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      env.now = 10.075
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      AdvanceTo(10.11)
      assert.are.equal(2, samples())
    end)

    it("should drop a pending sample when the capture has stopped, or another one has started", function()
      local capture = StartCapture()
      env.now = 10
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      rows[2].rank = 50

      capture.active = false
      AdvanceTo(20)
      assert.are.equal(1, #Streams(capture)[129])

      -- QuestieTrace reuses one capture table: a restart sets it active again with a new token.
      capture.active = true
      capture.token = 2
      tracker.Init(capture)
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      capture.token = 3
      tracker.Init(capture)
      local samples = CountSamples()
      AdvanceTo(30)
      assert.are.equal(0, samples())

      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      assert.are.equal(1, samples())
      capture.token = 4
      tracker.Init(capture)
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")
      assert.are.equal(3, samples(), "A new capture samples its first event at once")
    end)
  end)

  describe("on a client with the old functions", function()
    ---@type number
    local headersExpanded
    ---@type table[]
    local timers
    ---@type table
    local capture
    ---@type boolean
    local isCapturing

    before_each(function()
      isCapturing = false
      headersExpanded, timers, env.now = 0, {}, 0
      env.C_Timer = { After = function(delay, callback) timers[#timers + 1] = { at = env.now + delay, callback = callback } end }
      env.rank = 150
      env.GetNumSkillLines = function() return 1 end
      env.GetSkillLineInfo = function(index) if index == 1 then return "First Aid", 0, 0, env.rank end end
      -- Like the client: expanding headers fires SKILL_LINES_CHANGED while the call runs.
      env.ExpandSkillHeader = function(index)
        assert.are.equal(0, index)
        headersExpanded = headersExpanded + 1
        if isCapturing then tracker.OnEvent(capture, "SKILL_LINES_CHANGED") end
      end

      tracker = select(2, LoadSkillLinesTracker(env))
      env.GetTime = function() return env.now end
      capture = NewCapture()
      tracker.Init(capture)
      isCapturing = true
    end)

    it("should expand the headers and sample on an event, without sampling again for its own event", function()
      assert.are.equal(1, headersExpanded)

      env.now, env.rank = 10, 151
      tracker.OnEvent(capture, "SKILL_LINES_CHANGED")

      assert.are.equal(2, headersExpanded)
      assert.are.equal(0, #timers)
      local stream = capture.session.functions["GetSkillLineInfo"][1]
      assert.are.equal(2, #stream)
      assert.are.equal(151, stream[2].v[4])
    end)

    it("should sample a burst of events once, and once more 0.1 seconds later", function()
      env.now = 10
      for _ = 1, 50 do tracker.OnEvent(capture, "SKILL_LINES_CHANGED") end
      assert.are.equal(2, headersExpanded)
      assert.are.equal(1, #timers)

      env.now, env.rank = 10.1, 151
      timers[1].callback()
      assert.are.equal(3, headersExpanded)
      assert.are.equal(1, #timers)
      assert.are.equal(151, capture.session.functions["GetSkillLineInfo"][1][2].v[4])
    end)
  end)

  it("should keep recording the old functions on clients without C_SkillInfo's skill list", function()
    env.C_SkillInfo = {}
    env.GetNumSkillLines = function() return 1 end
    env.GetSkillLineInfo = function(index) if index == 1 then return "First Aid", 0, 0, 150 end end

    tracker = select(2, LoadSkillLinesTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.equal("First Aid", capture.session.functions["GetSkillLineInfo"][1][1].v[1])
    assert.is_nil(capture.session.functions["C_SkillInfo.GetSkillLineInfoByID"])
  end)
end)
