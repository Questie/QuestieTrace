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
    assert.is_nil(next(capture.session.functions["GetFactionInfoByID"]))
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
    assert.is_nil(next(capture.session.functions["C_Reputation.GetFactionDataByID"]))
  end)

  it("should add a faction discovered by a reputation change to FactionOrder and record nil answers once", function()
    local panel = { 72 }
    env.C_Reputation = {
      GetNumFactions = function() return #panel end,
      GetFactionDataByIndex = function(index)
        local factionID = panel[index]
        if not factionID then return nil end
        return { isHeader = false, isCollapsed = false, factionID = factionID }
      end,
      GetFactionDataByID = function(factionID)
        if factionID == 72 then return { name = "Stormwind", factionID = 72 } end
        if factionID == 369 then return { name = "Gadgetzan", factionID = 369, currentStanding = 525 } end
        return nil
      end,
    }

    tracker = select(2, LoadReputationTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    -- A kill gives Gadgetzan reputation: the faction joins the panel without a quest turn-in.
    panel[2] = 369
    tracker.OnEvent(capture, "CHAT_MSG_COMBAT_FACTION_CHANGE")
    local functions = capture.session.functions
    assert.are.same({ 72, 369 }, functions.FactionOrder[2].v)
    assert.are.equal(525, functions["C_Reputation.GetFactionDataByID"][369][1].v.currentStanding)

    -- A faction the client stops answering is recorded as nil once, not on every sample.
    panel[3] = 99999
    tracker.OnEvent(capture, "UPDATE_FACTION")
    tracker.OnEvent(capture, "UPDATE_FACTION")
    local missing = functions["C_Reputation.GetFactionDataByID"][99999]
    assert.are.equal(1, #missing)
    assert.is_nil(missing[1].v)
  end)

  it("should never read the guild faction, which is named after the player's guild", function()
    local guildReads = 0
    env.C_Reputation = {
      GetNumFactions = function() return 2 end,
      GetFactionDataByIndex = function(index)
        if index == 1 then return { isHeader = false, isCollapsed = false, factionID = 1168 } end
        if index == 2 then return { isHeader = false, isCollapsed = false, factionID = 72 } end
      end,
      GetFactionDataByID = function(factionID)
        if factionID == 1168 then
          guildReads = guildReads + 1
          return { name = "Knights of Dawn", factionID = 1168 }
        end
        if factionID == 72 then return { name = "Stormwind", factionID = 72 } end
      end,
    }
    env.GetFactionInfoByID = function(factionID)
      if factionID == 1168 then guildReads = guildReads + 1 end
    end

    tracker = select(2, LoadReputationTracker(env))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.same({ 1168, 72 }, capture.session.functions.FactionOrder[1].v, "the faction ID itself stays in FactionOrder")
    assert.are.equal(0, guildReads)
    assert.is_nil(capture.session.functions["C_Reputation.GetFactionDataByID"][1168])
    assert.are.equal("Stormwind", capture.session.functions["C_Reputation.GetFactionDataByID"][72][1].v.name)
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
