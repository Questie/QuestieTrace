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

  it("should record a player target as no unit so the previous NPC does not linger", function()
    local guid = "Creature-0-1-1-1-999-000001"
    local name = "Some Mob"
    local tracker = LoadUnitInteractionTracker(TargetEnv("ignored", "ignored", false))
    local env = getfenv(tracker.OnEvent)
    env.UnitGUID = function(token) if token == "target" then return guid end end
    env.UnitName = function(token) if token == "target" then return name end end
    local capture = NewCapture()
    tracker.Init(capture)

    guid, name = "Player-1-000001", "Someplayer"
    tracker.OnEvent(capture, "QUEST_DETAIL")

    assert.are.equal(2, #capture.session.functions.UnitGUID.target)
    assert.is_nil(capture.session.functions.UnitGUID.target[2].v)
    assert.are.same({ n = 2 }, capture.session.functions.UnitName.target[2].v)
  end)
end)

describe("UnitInteraction tracker item quest givers", function()
  local ITEM_GUID = "Item-4619-0-4000000ABCDEF012"

  ---@class QuestGiverFixture
  ---@field guid string? Returned by UnitGUID("questnpc")
  ---@field itemIDs table<string, number> What the client knows, by item GUID
  ---@field asked string[] Arguments of every C_Item.GetItemIDByGUID call, as strings
  ---@field env table<string, any>
  ---@field tracker table

  ---@return QuestGiverFixture
  local function NewFixture()
    ---@type QuestGiverFixture
    local fixture = { guid = nil, itemIDs = { [ITEM_GUID] = 1307 }, asked = {}, env = {}, tracker = {} }
    fixture.env = {
      UnitGUID = function(token) if token == "questnpc" then return fixture.guid end end,
      UnitName = function(token) if token == "questnpc" and fixture.guid then return "Gold Pickup Schedule" end end,
      UnitPlayerControlled = function() return false end,
      C_Item = {
        GetItemIDByGUID = function(itemGUID)
          fixture.asked[#fixture.asked + 1] = tostring(itemGUID)
          return fixture.itemIDs[itemGUID]
        end,
      },
    }
    fixture.tracker = LoadUnitInteractionTracker(fixture.env)
    return fixture
  end

  it("should record the item ID under the GUID and at the time UnitGUID recorded", function()
    local fixture = NewFixture()
    local capture = NewCapture()
    fixture.tracker.Init(capture)

    fixture.guid = ITEM_GUID
    fixture.env.GetTime = function() return 5 end
    fixture.env.GetTimePreciseSec = function() return 5.25 end
    fixture.tracker.OnEvent(capture, "QUEST_DETAIL")

    local functions = capture.session.functions
    assert.are.same({ t = 5, tp = 5.25, v = ITEM_GUID }, functions.UnitGUID.questnpc[2])
    assert.are.same({ [ITEM_GUID] = { { t = 5, tp = 5.25, v = 1307 } } }, functions["C_Item.GetItemIDByGUID"])
  end)

  it("should keep the known ID without asking again when the client has forgotten the item", function()
    local fixture = NewFixture()
    local capture = NewCapture()
    fixture.guid = ITEM_GUID
    fixture.tracker.Init(capture)

    fixture.itemIDs = {}
    fixture.tracker.OnEvent(capture, "QUEST_ACCEPTED")
    fixture.tracker.OnEvent(capture, "UPDATE_MOUSEOVER_UNIT")

    assert.are.same({ ITEM_GUID }, fixture.asked)
    assert.are.same({ { t = 0, tp = 0, v = 1307 } }, capture.session.functions["C_Item.GetItemIDByGUID"][ITEM_GUID])
  end)

  it("should ask again until the client knows the item", function()
    local fixture = NewFixture()
    local capture = NewCapture()
    fixture.guid, fixture.itemIDs = ITEM_GUID, {}
    fixture.tracker.Init(capture)
    assert.are.same({}, capture.session.functions["C_Item.GetItemIDByGUID"])

    fixture.itemIDs = { [ITEM_GUID] = 1307 }
    fixture.tracker.OnEvent(capture, "QUEST_PROGRESS")

    assert.are.equal(1307, capture.session.functions["C_Item.GetItemIDByGUID"][ITEM_GUID][1].v)
  end)

  it("should ask only for item GUIDs", function()
    local fixture = NewFixture()
    local secretGUID = "Item-4619-0-40000000SECRET00"
    fixture.env.issecretvalue = function(value) return value == secretGUID end
    local capture = NewCapture()
    fixture.tracker.Init(capture)

    for _, guid in ipairs({
      "Creature-0-1-1-1-999-000001", "Vehicle-0-1-1-1-998-000001", "GameObject-0-1-1-1-55-000001",
      "Player-1-000001", "Pet-0-1-1-1-165189-000001", "Party-0-000001", secretGUID,
    }) do
      fixture.guid = guid
      fixture.tracker.OnEvent(capture, "QUEST_DETAIL")
    end
    -- An item withheld from UnitGUID is withheld here too.
    fixture.guid = ITEM_GUID
    fixture.env.UnitPlayerControlled = function() return true end
    fixture.tracker.OnEvent(capture, "QUEST_DETAIL")

    assert.are.same({}, fixture.asked)
    assert.are.same({}, capture.session.functions["C_Item.GetItemIDByGUID"])
  end)

  it("should keep recording the quest giver when the lookup fails", function()
    local fixture = NewFixture()
    fixture.env.C_Item.GetItemIDByGUID = function() error("bad item") end
    local capture = NewCapture()
    fixture.guid = ITEM_GUID
    fixture.tracker.Init(capture)

    local functions = capture.session.functions
    assert.are.same({}, functions["C_Item.GetItemIDByGUID"])
    assert.are.equal(ITEM_GUID, functions.UnitGUID.questnpc[1].v)
    assert.are.equal("Gold Pickup Schedule", functions.UnitName.questnpc[1].v[1])
  end)

  it("should create no stream on clients without the API", function()
    local fixture = NewFixture()
    fixture.env.C_Item = {}
    local capture = NewCapture()
    fixture.guid = ITEM_GUID
    fixture.tracker.Init(capture)

    assert.is_nil(capture.session.functions["C_Item.GetItemIDByGUID"])
    assert.are.equal(ITEM_GUID, capture.session.functions.UnitGUID.questnpc[1].v)
  end)

  it("should start the next capture empty and record the item again", function()
    local fixture = NewFixture()
    local first, second = NewCapture(), NewCapture()
    fixture.guid = ITEM_GUID
    fixture.tracker.Init(first)

    fixture.guid = nil
    fixture.tracker.Init(second)
    assert.are.same({}, second.session.functions["C_Item.GetItemIDByGUID"])

    fixture.guid = ITEM_GUID
    fixture.tracker.OnEvent(second, "QUEST_DETAIL")
    assert.are.equal(1307, second.session.functions["C_Item.GetItemIDByGUID"][ITEM_GUID][1].v)
  end)
end)
