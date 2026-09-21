---@param env table<string, any>
---@return QuestieTraceCore, table
local function LoadReputationTracker(env)
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

  local chunk = assert(loadfile("Modules/Trackers/Reputation.lua"))
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

describe("Reputation tracker", function()
  ---@type table<string, any>
  local env
  ---@type table
  local tracker

  before_each(function()
    env = {}
  end)

  it("should record C_Reputation.GetFactionDataByID as a raw table when C_Reputation is available", function()
    env.C_Reputation = {
      GetNumFactions = function() return 1 end,
      GetFactionDataByIndex = function(index)
        if index ~= 1 then return nil end
        return { isHeader = false, isCollapsed = false, factionID = 42 }
      end,
      GetFactionDataByID = function(factionID)
        if factionID ~= 42 then return nil end
        return { name = "Stormwind", factionID = 42, reaction = 5 }
      end,
    }

    tracker = select(2, LoadReputationTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    local stream = capture.session.functions["C_Reputation.GetFactionDataByID"]
    assert.is_not_nil(stream[42], "raw C_Reputation.GetFactionDataByID stream should exist for factionID 42")
    assert.are.equal("Stormwind", stream[42][1].v.name)
    -- The legacy global isn't mocked in this environment, so its stream stays empty.
    assert.are.equal(0, #capture.session.functions["GetFactionInfoByID"])
  end)

  it("should record the legacy GetFactionInfoByID tuple only when C_Reputation is unavailable", function()
    env.GetNumFactions = function() return 1 end
    env.GetFactionInfo = function(index)
      if index ~= 1 then return nil end
      return "Stormwind", "desc", 5, 0, 100, 50, false, false, false, false, true, true, false, 42
    end
    env.GetFactionInfoByID = function(factionID)
      if factionID ~= 42 then return nil end
      return "Stormwind", "desc", 5, 0, 100, 50, false, false, false, false, true, true, false, 42
    end

    tracker = select(2, LoadReputationTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    local stream = capture.session.functions["GetFactionInfoByID"]
    assert.is_not_nil(stream[42], "legacy GetFactionInfoByID stream should exist for factionID 42")
    assert.are.equal("Stormwind", stream[42][1].v[1])
    assert.are.equal(0, #capture.session.functions["C_Reputation.GetFactionDataByID"])
  end)

  it("should record both C_Reputation.GetFactionDataByID and legacy GetFactionInfoByID independently when both APIs exist", function()
    env.C_Reputation = {
      GetNumFactions = function() return 1 end,
      GetFactionDataByIndex = function(index)
        if index ~= 1 then return nil end
        return { isHeader = false, isCollapsed = false, factionID = 42 }
      end,
      GetFactionDataByID = function(factionID)
        if factionID ~= 42 then return nil end
        return { name = "Stormwind (modern)", factionID = 42 }
      end,
    }
    env.GetFactionInfoByID = function(factionID)
      if factionID ~= 42 then return nil end
      return "Stormwind (legacy)", "desc", 5, 0, 100, 50, false, false, false, false, true, true, false, 42
    end

    tracker = select(2, LoadReputationTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    local modernStream = capture.session.functions["C_Reputation.GetFactionDataByID"]
    local legacyStream = capture.session.functions["GetFactionInfoByID"]
    assert.are.equal("Stormwind (modern)", modernStream[42][1].v.name)
    assert.are.equal("Stormwind (legacy)", legacyStream[42][1].v[1])
  end)
end)
