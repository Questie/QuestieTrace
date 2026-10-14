---@param env table<string, any>
---@return table
local function LoadUnitInteractionTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.GetTime = function() return 0 end
  env.GetTimePreciseSec = function() return 0 end

  for _, file in ipairs({ "Modules/globals.lua", "Modules/Privacy.lua" }) do
    local chunk = assert(loadfile(file))
    setfenv(chunk, env)
    chunk()
  end

  local Core = env.QuestieTraceCore
  ---@type table[]
  local trackers = {}
  local realRegisterTracker = Core.RegisterTracker
  Core.RegisterTracker = function(tracker)
    trackers[#trackers + 1] = tracker
    return realRegisterTracker(tracker)
  end

  local chunk = assert(loadfile("Modules/Trackers/UnitInteraction.lua"))
  setfenv(chunk, env)
  chunk()

  return trackers[1]
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

---@param guid string
---@param name string
---@param playerControlled boolean
---@return table<string, any>
local function TargetEnv(guid, name, playerControlled)
  return {
    UnitGUID = function(token) if token == "target" then return guid end end,
    UnitName = function(token) if token == "target" then return name end end,
    UnitPlayerControlled = function(token) return token == "target" and playerControlled end,
  }
end

describe("UnitInteraction tracker", function()
  it("should record an NPC target's GUID and name", function()
    local tracker = LoadUnitInteractionTracker(TargetEnv("Creature-0-1-1-1-999-000001", "Some Mob", false))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.equal("Creature-0-1-1-1-999-000001", capture.session.functions.UnitGUID.target[1].v)
    assert.are.equal("Some Mob", capture.session.functions.UnitName.target[1].v[1])
  end)

  it("should record a hunter pet target as no unit and record an empty name", function()
    local tracker = LoadUnitInteractionTracker(TargetEnv("Pet-0-1-1-1-165189-000001", "Raichu", false))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.equal(1, #capture.session.functions.UnitGUID.target)
    assert.is_nil(capture.session.functions.UnitGUID.target[1].v)
    assert.are.same({ { t = 0, tp = 0, v = { n = 2 } } }, capture.session.functions.UnitName.target)
  end)

  it("should record a player-controlled creature target (e.g. a warlock demon) as no unit and record an empty name", function()
    local tracker = LoadUnitInteractionTracker(TargetEnv("Creature-0-1-1-1-185317-000001", "Incubus", true))
    local capture = NewCapture()
    tracker.Init(capture)

    assert.are.equal(1, #capture.session.functions.UnitGUID.target)
    assert.is_nil(capture.session.functions.UnitGUID.target[1].v)
    assert.are.same({ { t = 0, tp = 0, v = { n = 2 } } }, capture.session.functions.UnitName.target)
  end)
end)
