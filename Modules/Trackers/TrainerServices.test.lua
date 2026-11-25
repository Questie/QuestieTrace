---@type { at: number, callback: fun() }[]
local timers
---@type table The tracker's private frame
local frame

---@param env table<string, any>
---@return table tracker
local function LoadTrainerServicesTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  env.now = 0
  -- Two clocks that run apart.
  env.GetTime = function() return env.now end
  env.GetTimePreciseSec = function() return env.now + 0.25 end

  timers = {}
  env.C_Timer = {
    After = function(delay, callback)
      timers[#timers + 1] = { at = env.now + delay, callback = callback }
    end,
  }
  frame = { registered = {} }
  env.CreateFrame = function()
    return {
      RegisterEvent = function(_, event) frame.registered[event] = true end,
      SetScript = function(_, name, callback) frame[name] = callback end,
    }
  end

  ---@type table
  local tracker
  for _, path in ipairs({ "Modules/globals.lua", "Modules/Privacy.lua", "Modules/Trackers/TrainerServices.lua" }) do
    local chunk = assert(loadfile(path))
    setfenv(chunk, env)
    if path:find("Trackers/", 1, true) then
      env.QuestieTraceCore.RegisterTracker = function(def) tracker = def end
    end
    chunk()
  end
  return tracker
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

describe("TrainerServices tracker", function()
  ---@class TestService
  ---@field name string? nil while the client has not loaded it
  ---@field rank string
  ---@field type string
  ---@field level number
  ---@field cost number
  ---@field skill string?
  ---@field skillRank number?
  ---@field spellID number?
  ---@field itemID number?

  local MAGE_TRAINER = "Creature-0-4621-1-167-331-000036BB3E"
  local COOKING_TRAINER = "Creature-0-4621-1-167-1355-000036BB3E"

  ---@type table<string, any>
  local env
  ---@type table
  local tracker
  ---@type table
  local capture
  ---@type TestService[] The open trainer's list
  local services
  ---@type string? UnitGUID("npc")
  local npcGUID
  ---@type table<string, boolean>
  local filters
  ---@type number Every sample reads the number of services once
  local samplesTaken
  ---@type number[] Service indices whose tooltip or link was read
  local idsRead

  ---@return TestService
  local function Fireball()
    return { name = "Fireball", rank = "Rank 2", type = "available", level = 6, cost = 100, spellID = 143 }
  end

  ---@return TestService
  local function Polymorph()
    return { name = "Polymorph", rank = "", type = "unavailable", level = 8, cost = 200, spellID = 118 }
  end

  ---@return TestService
  local function HerbBakedEgg()
    return {
      name = "Herb Baked Egg", rank = "", type = "unavailable", level = 0, cost = 50, skill = "Cooking", skillRank = 1, itemID = 6888,
    }
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

  ---Fires the event on the tracker's own frame, if it listens to it.
  ---@param event string
  local function FirePrivate(event)
    if frame.registered[event] then frame.OnEvent(frame, event) end
  end

  ---@param guid string?
  ---@param list TestService[]
  local function Open(guid, list)
    npcGUID, services = guid, list
    tracker.OnEvent(capture, "TRAINER_SHOW")
  end

  local function Close()
    npcGUID, services = nil, {}
    tracker.OnEvent(capture, "TRAINER_CLOSED")
  end

  ---Open the trainer, let the window settle and close it.
  ---@param guid string?
  ---@param list TestService[]
  local function Visit(guid, list)
    Open(guid, list)
    AdvanceTo(env.now + 1)
    Close()
    AdvanceTo(env.now + 1)
  end

  ---@param npcID number
  ---@return table<string, table[]>
  local function Streams(npcID)
    return capture.session.functions["TrainerServices"][npcID]
  end

  ---@param value any
  ---@return any
  local function Copy(value)
    if type(value) ~= "table" then return value end
    local out = {}
    for k, v in pairs(value) do out[k] = Copy(v) end
    return out
  end

  ---Functions every client has, in the shape Blizzard's trainer windows use them.
  local function NewClient()
    samplesTaken, idsRead = 0, {}
    services, npcGUID = {}, nil
    filters = { available = true, unavailable = true, used = false }
    local function Refuse() error("The tracker must not change the trainer window") end
    env = {
      UnitGUID = function(token) if token == "npc" then return npcGUID end end,
      UnitExists = function(token) return token == "npc" and npcGUID ~= nil end,
      UnitIsPlayer = function() return npcGUID ~= nil and npcGUID:find("^Player") ~= nil end,
      UnitPlayerControlled = function() return false end,
      GetNumTrainerServices = function()
        samplesTaken = samplesTaken + 1
        return #services
      end,
      GetTrainerServiceCost = function(index) return services[index].cost, false end,
      GetTrainerServiceSkillReq = function(index)
        local service = services[index]
        if not service.skill then return nil, 0, true end
        return service.skill, service.skillRank, true
      end,
      GetTrainerServiceTypeFilter = function(serviceType) return filters[serviceType] end,
      SetTrainerServiceTypeFilter = Refuse,
      ExpandTrainerSkillLine = Refuse,
      CollapseTrainerSkillLine = Refuse,
      SelectTrainerService = Refuse,
    }
  end

  ---Forever's functions, as its Blizzard_TrainerUI calls them.
  local function NewForeverClient()
    NewClient()
    env.Enum = { TooltipDataType = { Item = 0, Spell = 1, Unit = 2 } }
    env.C_Trainer = { GetTrainerType = function() return npcGUID == COOKING_TRAINER and 2 or 0 end }
    env.IsTradeskillTrainer = function() return npcGUID == COOKING_TRAINER end
    env.GetTrainerServiceInfo = function(index)
      local service = services[index]
      return service.name, service.type, 135812, service.level, service.rank, "Fire"
    end
    env.C_TooltipInfo = {
      GetTrainerService = function(index)
        idsRead[#idsRead + 1] = index
        local service = services[index]
        if service.spellID then return { type = 1, id = service.spellID, lines = {} } end
        if service.itemID then return { type = 0, id = service.itemID, lines = {} } end
      end,
    }
    tracker = LoadTrainerServicesTracker(env)
    capture = NewCapture()
    tracker.Init(capture)
  end

  ---The functions of Classic Era, TBC and Mists, whose list also has headers.
  local function NewClassicClient()
    NewClient()
    env.IsTradeskillTrainer = function() return npcGUID == COOKING_TRAINER end
    env.GetTrainerServiceInfo = function(index)
      local service = services[index]
      return service.name, service.rank, service.type, service.type == "header"
    end
    env.GetTrainerServiceLevelReq = function(index) return services[index].level end
    env.GetTrainerServiceItemLink = function(index)
      idsRead[#idsRead + 1] = index
      local service = services[index]
      if service.itemID then return ("|cffffffff|Hitem:%d::::::::10:::::::::|h[%s]|h|r"):format(service.itemID, service.name) end
    end
    tracker = LoadTrainerServicesTracker(env)
    capture = NewCapture()
    tracker.Init(capture)
  end

  describe("on Forever", function()
    before_each(NewForeverClient)

    it("should record each service under the trainer's NPC ID", function()
      AdvanceTo(10)
      Visit(MAGE_TRAINER, { Fireball(), Polymorph() })

      assert.are.same({
        ["Fireball:6"] = {
          { t = 10.1, tp = 10.35, v = { name = "Fireball", rank = "Rank 2", type = "available", level = 6, cost = 100, spellID = 143 } },
        },
        ["Polymorph:8"] = {
          { t = 10.1, tp = 10.35, v = { name = "Polymorph", type = "unavailable", level = 8, cost = 200, spellID = 118 } },
        },
      }, Streams(331))
    end)

    it("should record the required skill, and the item a recipe's tooltip shows", function()
      Visit(COOKING_TRAINER, { HerbBakedEgg() })

      assert.are.same(
        { name = "Herb Baked Egg", type = "unavailable", level = 0, cost = 50, skill = "Cooking", skillRank = 1, itemID = 6888 },
        Streams(1355)["Herb Baked Egg:0"][1].v
      )
    end)

    it("should record the kind of the trainer and the filters, change-only", function()
      Visit(MAGE_TRAINER, { Fireball() })
      Visit(MAGE_TRAINER, { Fireball() })
      filters.used = true
      Visit(COOKING_TRAINER, { HerbBakedEgg() })

      local functions = capture.session.functions
      local kinds = functions["C_Trainer.GetTrainerType"]
      assert.are.same({ 0, 2 }, { kinds[1].v, kinds[2].v })
      assert.are.equal(2, #kinds)
      assert.is_nil(functions["IsTradeskillTrainer"])
      local recordedFilters = functions["GetTrainerServiceTypeFilter"]
      assert.are.same({ { t = 0.1, tp = 0.35, v = true } }, recordedFilters.available)
      assert.are.same({ { t = 0.1, tp = 0.35, v = true } }, recordedFilters.unavailable)
      assert.are.same({ false, true }, { recordedFilters.used[1].v, recordedFilters.used[2].v })
      assert.are.equal(2, #recordedFilters.used)
    end)

    it("should add nothing when the same trainer is visited again, with another one in between", function()
      Visit(MAGE_TRAINER, { Fireball(), Polymorph() })
      Visit(COOKING_TRAINER, { HerbBakedEgg() })
      local servicesBefore = Copy(capture.session.functions["TrainerServices"])
      local idsReadBefore = #idsRead

      -- Learning a spell moved Polymorph to the front.
      Visit(MAGE_TRAINER, { Polymorph(), Fireball() })

      assert.are.same(servicesBefore, capture.session.functions["TrainerServices"])
      assert.are.equal(idsReadBefore, #idsRead)
    end)

    it("should append one entry for a service that changed", function()
      Open(MAGE_TRAINER, { Fireball(), Polymorph() })
      AdvanceTo(1)
      services[2].type = "available"
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)

      assert.are.same({ 1, 2 }, { #Streams(331)["Fireball:6"], #Streams(331)["Polymorph:8"] })
      assert.are.same(
        { name = "Polymorph", type = "available", level = 8, cost = 200, spellID = 118 },
        Streams(331)["Polymorph:8"][2].v
      )
    end)

    -- In game, 47 of 49 known services of a trainer had level 0, 23 of them with the name of another.
    it("should record every rank of a spell the player knows, without the level the client gives", function()
      Visit(MAGE_TRAINER, {
        { name = "Fireball", rank = "Rank 1", type = "used", level = 0, cost = 10, spellID = 133 },
        { name = "Fireball", rank = "Rank 2", type = "used", level = 0, cost = 100, spellID = 143 },
        { name = "Fireball", rank = "Rank 3", type = "unavailable", level = 12, cost = 300, spellID = 145 },
        { name = "Conjure Water", rank = "", type = "used", level = 0, cost = 20, spellID = 5504 },
        -- A few known services do have their level.
        { name = "Frost Armor", rank = "Rank 2", type = "used", level = 10, cost = 400, spellID = 7300 },
      })

      local values = {}
      for key, stream in pairs(Streams(331)) do
        assert.are.equal(1, #stream)
        values[key] = stream[1].v
      end
      assert.are.same({
        ["Fireball:0:Rank 1"] = { name = "Fireball", rank = "Rank 1", type = "used", cost = 10, spellID = 133 },
        ["Fireball:0:Rank 2"] = { name = "Fireball", rank = "Rank 2", type = "used", cost = 100, spellID = 143 },
        ["Fireball:12"] = { name = "Fireball", rank = "Rank 3", type = "unavailable", level = 12, cost = 300, spellID = 145 },
        ["Conjure Water:0"] = { name = "Conjure Water", type = "used", cost = 20, spellID = 5504 },
        ["Frost Armor:10"] = { name = "Frost Armor", rank = "Rank 2", type = "used", level = 10, cost = 400, spellID = 7300 },
      }, values)
    end)

    it("should keep what it recorded of a spell when the player learns it", function()
      Open(MAGE_TRAINER, { Fireball() })
      AdvanceTo(1)
      services[1].type, services[1].level = "used", 0
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)

      assert.are.same({ 1, 6 }, { #Streams(331)["Fireball:6"], Streams(331)["Fireball:6"][1].v.level })
      assert.are.equal("used", Streams(331)["Fireball:0:Rank 2"][1].v.type)
    end)

    it("should continue a recipe's entries when the player learns it", function()
      Open(COOKING_TRAINER, { HerbBakedEgg() })
      AdvanceTo(1)
      services[1].type = "used"
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)

      local egg = Streams(1355)["Herb Baked Egg:0"]
      assert.are.same({ "unavailable", "used" }, { egg[1].v.type, egg[2].v.type })
      assert.are.same({ 0, 6888, 6888 }, { egg[1].v.level, egg[1].v.itemID, egg[2].v.itemID })
      assert.is_nil(egg[2].v.level)
    end)

    it("should record services that appear after the window opened", function()
      Open(MAGE_TRAINER, {})
      AdvanceTo(1)
      assert.are.same({}, capture.session.functions["TrainerServices"])
      services = { Fireball() }
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)

      assert.are.equal(143, Streams(331)["Fireball:6"][1].v.spellID)
    end)

    it("should record a service once the client knows its name", function()
      local fireball = Fireball()
      fireball.name = nil
      Open(MAGE_TRAINER, { fireball, Polymorph() })
      AdvanceTo(1)
      assert.is_nil(Streams(331)["Fireball:6"])
      assert.are.equal(1, #Streams(331)["Polymorph:8"])

      fireball.name = "Fireball"
      FirePrivate("TRAINER_SERVICE_INFO_NAME_UPDATE")
      AdvanceTo(2)
      assert.are.equal(143, Streams(331)["Fireball:6"][1].v.spellID)
    end)

    it("should tell ranks of a spell apart before their rank text arrives, and add the text", function()
      local lowRank, highRank = Fireball(), Fireball()
      lowRank.rank, highRank.rank, highRank.level, highRank.spellID = "", "", 12, 145
      Open(MAGE_TRAINER, { lowRank, highRank })
      AdvanceTo(1)
      lowRank.rank, highRank.rank = "Rank 2", "Rank 3"
      FirePrivate("TRAINER_SERVICE_INFO_NAME_UPDATE")
      AdvanceTo(2)

      local low, high = Streams(331)["Fireball:6"], Streams(331)["Fireball:12"]
      assert.are.same({ 143, 145 }, { low[1].v.spellID, high[1].v.spellID })
      assert.is_nil(high[1].v.rank)
      assert.are.same({ "Rank 2", "Rank 3" }, { low[2].v.rank, high[2].v.rank })
    end)

    it("should read a service's ID again while the client has none", function()
      local polymorph = Polymorph()
      polymorph.spellID = nil
      Open(MAGE_TRAINER, { polymorph })
      AdvanceTo(1)
      polymorph.spellID = 118
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(3)

      assert.are.equal(118, Streams(331)["Polymorph:8"][2].v.spellID)
      assert.are.equal(2, #idsRead)
    end)

    it("should record a service listed twice only once, however often it is sampled", function()
      local other = Fireball()
      other.cost = 999
      Open(MAGE_TRAINER, { Fireball(), other })
      for second = 1, 5 do
        AdvanceTo(second)
        FirePrivate("TRAINER_UPDATE")
      end

      assert.are.equal(1, #Streams(331)["Fireball:6"])
      assert.are.equal(100, Streams(331)["Fireball:6"][1].v.cost)
    end)

    it("should take one sample for a burst of updates and another for a later one", function()
      Open(MAGE_TRAINER, { Fireball() })
      for _ = 1, 50 do FirePrivate("TRAINER_UPDATE") end
      AdvanceTo(0.05)
      for _ = 1, 50 do FirePrivate("TRAINER_UPDATE") end
      assert.are.equal(0, samplesTaken)
      AdvanceTo(5)
      assert.are.equal(1, samplesTaken)

      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(6)
      assert.are.equal(2, samplesTaken)
    end)

    it("should not read anything once the window closed", function()
      Open(MAGE_TRAINER, { Fireball() })
      Close()
      -- The player talks to another NPC while the client still has the list.
      npcGUID, services = COOKING_TRAINER, { Fireball() }
      AdvanceTo(1)
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)
      assert.are.equal(0, samplesTaken)
    end)

    it("should not let the previous window's pending sample run in the next window", function()
      Open(MAGE_TRAINER, { Fireball() })
      AdvanceTo(0.05)
      Close()
      Open(COOKING_TRAINER, { HerbBakedEgg() })
      AdvanceTo(0.12)
      assert.are.equal(0, samplesTaken)
      AdvanceTo(1)
      assert.are.equal(1, samplesTaken)
    end)

    it("should not take a pending sample after the capture stopped", function()
      Open(MAGE_TRAINER, { Fireball() })
      capture.active = false
      tracker.OnCaptureStopped(capture)
      AdvanceTo(1)
      FirePrivate("TRAINER_UPDATE")
      AdvanceTo(2)
      assert.are.equal(0, samplesTaken)
    end)

    it("should record the services again in the next capture", function()
      AdvanceTo(10)
      Visit(MAGE_TRAINER, { Fireball() })
      tracker.OnCaptureStopped(capture)
      capture = NewCapture()
      capture.startedAt = env.now
      tracker.Init(capture)
      Visit(MAGE_TRAINER, { Fireball() })

      local fireball = Streams(331)["Fireball:6"]
      assert.are.equal(1, #fireball)
      assert.is_true(math.abs(fireball[1].t - 0.1) < 0.001)
    end)

    it("should record nothing for a window without an NPC, and no player or pet GUID", function()
      for _, guid in ipairs({ "none", "Player-4618-0053656F", "Pet-0-4621-1-167-165189-0100A3B4C5", "Nonsense-guid" }) do
        Visit(guid ~= "none" and guid or nil, { Fireball() })
        for funcName, streams in pairs(capture.session.functions) do
          local streamsWithData = {}
          for key, stream in pairs(streams) do
            if next(stream) ~= nil then streamsWithData[#streamsWithData + 1] = key end
          end
          assert.are.same({}, streamsWithData, funcName .. " for " .. guid)
        end
      end
    end)

    it("should record nothing for an NPC whose GUID is restricted", function()
      env.issecretvalue = function(value) return value == MAGE_TRAINER end
      Visit(MAGE_TRAINER, { Fireball() })

      assert.are.same({}, capture.session.functions["TrainerServices"])
    end)
  end)

  describe("on Classic Era, TBC and Mists", function()
    before_each(NewClassicClient)

    it("should record the services without the headers", function()
      Visit(COOKING_TRAINER, {
        { name = "Trainer Header", rank = "", type = "header", level = 0, cost = 0 },
        HerbBakedEgg(),
        { name = "Cooking", rank = "Journeyman", type = "available", level = 10, cost = 500, skill = "Cooking", skillRank = 50 },
      })

      local egg = { name = "Herb Baked Egg", type = "unavailable", level = 0, cost = 50, skill = "Cooking", skillRank = 1, itemID = 6888 }
      local journeyman = {
        name = "Cooking", rank = "Journeyman", type = "available", level = 10, cost = 500, skill = "Cooking", skillRank = 50,
      }
      assert.are.same({
        ["Herb Baked Egg:0"] = { { t = 0.1, tp = 0.35, v = egg } },
        ["Cooking:10"] = { { t = 0.1, tp = 0.35, v = journeyman } },
      }, Streams(1355))
    end)

    it("should record whether it is a tradeskill trainer", function()
      Visit(COOKING_TRAINER, { HerbBakedEgg() })

      assert.are.same({ { t = 0.1, tp = 0.35, v = true } }, capture.session.functions["IsTradeskillTrainer"])
      assert.is_nil(capture.session.functions["C_Trainer.GetTrainerType"])
    end)
  end)
end)
