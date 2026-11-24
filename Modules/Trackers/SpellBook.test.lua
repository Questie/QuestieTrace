---@param env table<string, any>
---@return table tracker
local function LoadSpellBookTracker(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = {}
  -- Two clocks that run apart, both running before the capture starts.
  env.GetTime = function() return 100 + (env.now or 0) end
  env.GetTimePreciseSec = function() return 5000 + (env.now or 0) + 0.25 end

  for _, path in ipairs({ "Modules/globals.lua", "Modules/Trackers/SpellBook.lua" }) do
    local chunk = assert(loadfile(path))
    setfenv(chunk, env)
    chunk()
  end
  return env.QuestieTraceCore._trackers[1]
end

---@return table
local function NewCapture()
  return {
    active = true,
    token = 1,
    startedAt = 100,
    startedAtPrecise = 5000,
    session = { functions = {}, functionsDelta = {} },
  }
end

-- Values of the Forever client (1.60.1).
local SPELL, FUTURE_SPELL, FLYOUT = 1, 2, 4
local PLAYER_BANK = 0

---Give `env` the spell book functions of the Forever client.
---@param env table<string, any>
---@param book table[] One { itemType, actionID, overridingSpellID? } per slot; tests change it in place
---@param known table<number, boolean> Spells the client knows beside those the book lists
local function MockForeverSpellBook(env, book, known)
  env.Enum = {
    SpellBookSpellBank = { Player = PLAYER_BANK, Pet = 1 },
    SpellBookItemType = { None = 0, Spell = SPELL, FutureSpell = FUTURE_SPELL, PetAction = 3, Flyout = FLYOUT },
  }
  env.C_SpellBook = {
    GetSpellBookItemType = function(slot, bank)
      assert(bank == PLAYER_BANK, "Only the player's own spells are read")
      local item = book[slot]
      if not item then return end
      return item[1], item[2], item[1] ~= FLYOUT and (item[3] or item[2]) or nil
    end,
    IsSpellKnown = function(spellID)
      if known[spellID] ~= nil then return known[spellID] end
      for _, item in ipairs(book) do
        if item[1] == SPELL and item[2] == spellID then return true end
      end
      return false
    end,
  }
end

describe("SpellBook tracker", function()
  ---@type table<string, any>
  local env
  ---@type table
  local capture
  ---@type string[] Errors passed to the client's error handler
  local errors

  ---@return table tracker
  local function StartCapture()
    local tracker = LoadSpellBookTracker(env)
    capture = NewCapture()
    tracker.Init(capture)
    return tracker
  end

  ---@return DeltaStream
  local function KnownSpells()
    return capture.session.functionsDelta["PlayerKnownSpells"]
  end

  before_each(function()
    errors = {}
    env = { geterrorhandler = function() return function(err) errors[#errors + 1] = err end end }
  end)

  describe("on a client with C_SpellBook.GetSpellBookItemType", function()
    it("should record the learned spells up to the first empty slot", function()
      MockForeverSpellBook(env, {
        { SPELL, 6603 },
        { FLYOUT, 270 },
        { SPELL, 19740 },
        { FUTURE_SPELL, 10308 },
        { SPELL, 2481 },
        { SPELL, 6603 },
        nil,
        { SPELL, 999 },
      }, {})

      StartCapture()

      assert.are.same({ 2481, 6603, 19740 }, KnownSpells().initial)
      assert.are.same({ { t = 0, tp = 0.25, v = { 6603, 19740, 2481 } } }, capture.session.functions["SpellBook"])
    end)

    it("should record the base spell of an overridden one", function()
      MockForeverSpellBook(env, { { SPELL, 6603 }, { SPELL, 5143, 7268 } }, {})

      StartCapture()

      assert.are.same({ 5143, 6603 }, KnownSpells().initial)
    end)

    it("should record every spell of a large book", function()
      local book, expected = {}, {}
      for slot = 1, 600 do book[slot], expected[slot] = { SPELL, slot }, slot end
      MockForeverSpellBook(env, book, {})

      StartCapture()

      assert.are.same(expected, KnownSpells().initial)
    end)

    it("should record a spell learned later with its time, and nothing while the book stays the same", function()
      local book = { { SPELL, 6603 }, { SPELL, 635 } }
      MockForeverSpellBook(env, book, {})
      local tracker = StartCapture()

      env.now = 5
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({}, KnownSpells().delta)
      assert.are.equal(1, #capture.session.functions["SpellBook"])

      env.now = 9
      table.insert(book, 2, { SPELL, 639 })
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({ { t = 9, tp = 9.25, add = { 639 } } }, KnownSpells().delta)
      assert.are.same({ t = 9, tp = 9.25, v = { 6603, 639, 635 } }, capture.session.functions["SpellBook"][2])
    end)

    it("should record a spell that takes another one's slot", function()
      local book, known = { { SPELL, 6603 }, { SPELL, 635 }, { SPELL, 1 } }, { [635] = true }
      MockForeverSpellBook(env, book, known)
      local tracker = StartCapture()

      book[2] = { SPELL, 639 }
      tracker.OnEvent(capture, "SPELLS_CHANGED")

      assert.are.same({ 639 }, KnownSpells().delta[1].add)
      assert.is_nil(KnownSpells().delta[1].remove)
    end)

    it("should start every capture with its own spells", function()
      local book = { { SPELL, 6603 }, { SPELL, 635 } }
      MockForeverSpellBook(env, book, {})
      local tracker = StartCapture()

      capture = NewCapture()
      tracker.Init(capture)

      assert.are.same({ 635, 6603 }, KnownSpells().initial)
      assert.are.same({ 6603, 635 }, capture.session.functions["SpellBook"][1].v)
    end)

    it("should record a spell as removed only when the client no longer knows it", function()
      local book, known = { { SPELL, 6603 }, { SPELL, 635 }, { SPELL, 20375 } }, {}
      MockForeverSpellBook(env, book, known)
      local tracker = StartCapture()

      -- The book is still loading: the spells are known all the same.
      known[635], known[20375] = true, true
      book[3], book[2] = nil, nil
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({}, KnownSpells().delta)
      assert.are.same({ 6603 }, capture.session.functions["SpellBook"][2].v)

      -- A talent spell is unlearned.
      known[20375] = false
      book[2] = { SPELL, 635 }
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({ 20375 }, KnownSpells().delta[1].remove)
      assert.is_nil(KnownSpells().delta[1].add)
      assert.are.equal(1, #KnownSpells().delta)
    end)

    it("should record riding spells and recipes the book does not list", function()
      local known = { [10906] = true, [1230117] = true }
      MockForeverSpellBook(env, { { SPELL, 6603 } }, known)
      local tracker = StartCapture()
      assert.are.same({ 6603, 10906, 1230117 }, KnownSpells().initial)

      env.now = 3
      known[825] = true
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({ { t = 3, tp = 3.25, add = { 825 } } }, KnownSpells().delta)
      assert.are.equal(1, #capture.session.functions["SpellBook"])
      assert.are.same({ 6603 }, capture.session.functions["SpellBook"][1].v)

      known[825] = false
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({ 825 }, KnownSpells().delta[2].remove)
    end)

    it("should not use the old functions another addon provides", function()
      MockForeverSpellBook(env, { { SPELL, 6603 } }, {})
      env.GetSpellBookItemName = function(slot) if slot == 1 then return "Attack", "" end end
      env.GetSpellBookItemInfo = function() error("must not be called") end

      StartCapture()

      assert.are.same({ 6603 }, KnownSpells().initial)
      assert.is_nil(capture.session.functions["GetSpellBookItemName"])
    end)

    it("should report a failing client function, keep the capture running and catch up when it works again", function()
      MockForeverSpellBook(env, { { SPELL, 6603 } }, {})
      local isSpellKnown = env.C_SpellBook.IsSpellKnown
      env.C_SpellBook.IsSpellKnown = function() error("not available yet") end

      local tracker = StartCapture()
      assert.are.same({}, KnownSpells().initial)
      assert.are.equal(1, #errors)
      assert.is_truthy(errors[1]:find("not available yet", 1, true))

      env.C_SpellBook.IsSpellKnown = isSpellKnown
      tracker.OnEvent(capture, "SPELLS_CHANGED")
      assert.are.same({ 6603 }, KnownSpells().delta[1].add)
      assert.are.equal(1, #errors)
    end)
  end)

  it("should keep using the old functions on clients whose C_SpellBook has no spell list", function()
    env.C_SpellBook = { IsSpellKnown = function() error("must not be called") end }
    env.GetSpellBookItemName = function(slot) if slot == 1 then return "Fireball", "Rank 1", 133 end end
    env.GetSpellBookItemInfo = function() return "SPELL", 133 end
    env.IsPassiveSpell = function() return nil end

    StartCapture()

    assert.are.same({ 133 }, KnownSpells().initial)
    assert.are.equal("Fireball", capture.session.functions["GetSpellBookItemName"][1][1].v[1])
    assert.are.equal(133, capture.session.functions["GetSpellBookItemInfo"][1][1].v[2])
  end)

  it("should record nothing and not fail on a client without any spell book function", function()
    StartCapture()

    assert.are.same({}, KnownSpells().initial)
  end)
end)
