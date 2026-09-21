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
end)
