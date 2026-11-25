---@type { at: number, callback: fun() }[]
local timers
---@type table Every frame the tracker created, as one
local frame

---@param env table<string, any>
---@return table tracker
local function LoadMerchantTracker(env)
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
      UnregisterEvent = function(_, event) frame.registered[event] = nil end,
      SetScript = function(_, name, callback) frame[name] = callback end,
    }
  end

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

  local chunk = assert(loadfile("Modules/Trackers/Merchant.lua"))
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

describe("Merchant tracker", function()
  ---@type table<string, any>
  local env

  before_each(function()
    env = {
      GetMerchantNumItems = function() return 1 end,
      GetMerchantItemInfo = function() return "Empty Vial", 134939, 20, 1 end,
    }
  end)

  it("should record GetMerchantItemLink per index when the merchant opens", function()
    env.GetMerchantItemLink = function(index)
      if index ~= 1 then
        return nil
      end
      return "|Hitem:3371::::::::10:::::::::|h[Empty Vial]|h|r"
    end
    local tracker = LoadMerchantTracker(env)
    local capture = NewCapture()
    tracker.Init(capture)

    tracker.OnEvent(capture, "MERCHANT_SHOW")

    assert.are.equal("|Hitem:3371::::::::10:::::::::|h[Empty Vial]|h|r", capture.session.functions["GetMerchantItemLink"][1][1].v)
  end)

  it("should record an unchanged link once and the link going away on close", function()
    local link = "|Hitem:3371::::::::10:::::::::|h[Empty Vial]|h|r"
    env.GetMerchantItemLink = function()
      return link
    end
    local tracker = LoadMerchantTracker(env)
    local capture = NewCapture()
    tracker.Init(capture)

    tracker.OnEvent(capture, "MERCHANT_SHOW")
    tracker.OnEvent(capture, "MERCHANT_SHOW")
    link = nil
    tracker.OnEvent(capture, "MERCHANT_CLOSED")

    local stream = capture.session.functions["GetMerchantItemLink"][1]
    assert.are.equal(2, #stream)
    assert.is_nil(stream[2].v)
  end)

  it("should not record GetMerchantItemLink when the API doesn't exist", function()
    local tracker = LoadMerchantTracker(env)
    local capture = NewCapture()
    tracker.Init(capture)

    tracker.OnEvent(capture, "MERCHANT_SHOW")

    assert.is_nil(next(capture.session.functions["GetMerchantItemLink"]))
  end)

  describe("on a client that loads item data late", function()
    ---@class TestMerchantItem
    ---@field id number
    ---@field name string
    ---@field isLoaded boolean Whether the client has the item's data

    ---@type TestMerchantItem[]? The open merchant's list
    local items
    ---@type number Every sample of a window's slots reads the first slot's link once
    local samplesTaken
    ---@type table
    local tracker
    ---@type table
    local capture

    ---@param id number
    ---@param name string
    ---@param isLoaded boolean
    ---@return TestMerchantItem
    local function Item(id, name, isLoaded)
      return { id = id, name = name, isLoaded = isLoaded }
    end

    ---@param index number
    ---@return string
    local function Link(index)
      local item = assert(items)[index]
      return ("|Hitem:%d::::::::10:::::::::|h[%s]|h|r"):format(item.id, item.name)
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
    ---@param ... any
    local function FirePrivate(event, ...)
      if frame.registered[event] then frame.OnEvent(frame, event, ...) end
    end

    ---@param list TestMerchantItem[]
    local function Open(list)
      items = list
      tracker.OnEvent(capture, "MERCHANT_SHOW")
    end

    local function Close()
      items = nil
      tracker.OnEvent(capture, "MERCHANT_CLOSED")
    end

    ---@param funcName string
    ---@param index number
    ---@return table[] values
    local function Values(funcName, index)
      local values = {}
      for i, entry in ipairs(capture.session.functions[funcName][index] or {}) do
        values[i] = entry.v == nil and "nil" or entry.v
      end
      return values
    end

    before_each(function()
      samplesTaken = 0
      items = nil
      env = {
        GetMerchantNumItems = function() return items and #items or 0 end,
        GetMerchantItemID = function(index) return items and items[index] and items[index].id or nil end,
        GetMerchantItemLink = function(index)
          if index == 1 then samplesTaken = samplesTaken + 1 end
          if items and items[index] and items[index].isLoaded then return Link(index) end
        end,
        C_MerchantFrame = {
          -- Forever answers nothing once the merchant is closed.
          GetItemInfo = function(index)
            local item = items and items[index]
            if not item then return end
            return {
              name = item.isLoaded and item.name or nil, texture = 1, price = 20, stackCount = 1, numAvailable = -1,
              isPurchasable = true, isUsable = true, hasExtendedCost = false, isQuestStartItem = false,
            }
          end,
        },
      }
      tracker = LoadMerchantTracker(env)
      capture = NewCapture()
      tracker.Init(capture)
    end)

    it("should record the name and link of an item once its data arrives", function()
      Open({ Item(3371, "Empty Vial", true), Item(2320, "Coarse Thread", false), Item(2321, "Fine Thread", true) })
      assert.are.same({}, Values("GetMerchantItemLink", 2))
      AdvanceTo(10)
      assert(items)[2].isLoaded = true
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(11)

      assert.are.same({ Link(2) }, Values("GetMerchantItemLink", 2))
      local entry = capture.session.functions["GetMerchantItemLink"][2][1]
      assert.are.equal(10.1, entry.t)
      assert.are.equal(10.35, entry.tp)
      local info = capture.session.functions["C_MerchantFrame.GetItemInfo"]
      assert.is_nil(info[2][1].v[1].name)
      assert.are.equal("Coarse Thread", info[2][2].v[1].name)
      assert.are.equal(1, #info[1])
    end)

    for _, event in ipairs({ "ITEM_DATA_LOAD_RESULT", "GET_ITEM_INFO_RECEIVED" }) do
      it("should record an item whose data only " .. event .. " reports", function()
        Open({ Item(3371, "Empty Vial", true), Item(2320, "Coarse Thread", false) })
        assert(items)[2].isLoaded = true
        FirePrivate(event, 2320, true)
        AdvanceTo(1)

        assert.are.same({ Link(2) }, Values("GetMerchantItemLink", 2))
      end)

      it("should not sample for a " .. event .. " about an item the merchant is not waiting for", function()
        Open({ Item(3371, "Empty Vial", true), Item(2320, "Coarse Thread", false) })
        samplesTaken = 0
        -- What an auction house scan of another addon fires, an item the merchant sells already, and a failed load.
        for itemID = 1, 1000 do FirePrivate(event, itemID, true) end
        FirePrivate(event, 3371, true)
        FirePrivate(event, 2320, false)
        FirePrivate(event, nil, true)
        assert.are.equal(0, #timers)
        AdvanceTo(5)
        assert.are.equal(0, samplesTaken)
      end)

      it("should take one sample for a burst of " .. event, function()
        Open({ Item(3371, "Empty Vial", false), Item(2320, "Coarse Thread", false), Item(2321, "Fine Thread", false) })
        for index, itemID in ipairs({ 3371, 2320, 2321 }) do
          assert(items)[index].isLoaded = true
          FirePrivate(event, itemID, true)
        end
        samplesTaken = 0
        AdvanceTo(5)

        assert.are.equal(1, samplesTaken)
        assert.are.same({ Link(3) }, Values("GetMerchantItemLink", 3))
      end)

      it("should listen to " .. event .. " only while an item of the open merchant is missing", function()
        assert.is_nil(frame.registered[event], "no merchant was opened yet")
        Open({ Item(3371, "Empty Vial", true) })
        assert.is_nil(frame.registered[event], "the window is complete")
        Close()

        Open({ Item(3371, "Empty Vial", false) })
        assert.is_true(frame.registered[event], "an item is missing")
        assert(items)[1].isLoaded = true
        FirePrivate(event, 3371, true)
        AdvanceTo(1)
        assert.is_nil(frame.registered[event], "the window became complete")
        Close()

        Open({ Item(3371, "Empty Vial", false) })
        assert.is_true(frame.registered[event])
        Close()
        assert.is_nil(frame.registered[event], "the window was closed")

        Open({ Item(3371, "Empty Vial", false) })
        tracker.OnCaptureStopped(capture)
        assert.is_nil(frame.registered[event], "the capture stopped")
      end)
    end

    it("should not listen for items when the missing slot has no item ID, as a currency", function()
      Open({ Item(3371, "Empty Vial", true), { name = "Honor Points", isLoaded = false } })

      assert.is_nil(frame.registered["ITEM_DATA_LOAD_RESULT"])
      assert.is_nil(frame.registered["GET_ITEM_INFO_RECEIVED"])
      samplesTaken = 0
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(1)
      assert.are.equal(1, samplesTaken, "updates of the window still sample it")
    end)

    it("should not wait for item data on clients that know no slot's item ID", function()
      env.GetMerchantItemID = nil
      tracker = LoadMerchantTracker(env)
      capture = NewCapture()
      tracker.Init(capture)
      Open({ Item(3371, "Empty Vial", false) })

      assert.is_nil(frame.registered["ITEM_DATA_LOAD_RESULT"])
    end)

    it("should not listen for a previous window's missing items", function()
      Open({ Item(3371, "Empty Vial", false) })
      Close()
      Open({ Item(2320, "Coarse Thread", false) })
      samplesTaken = 0
      FirePrivate("ITEM_DATA_LOAD_RESULT", 3371, true)
      assert.are.equal(0, #timers)
      FirePrivate("ITEM_DATA_LOAD_RESULT", 2320, true)
      assert.are.equal(1, #timers)
    end)

    it("should sample again when the filter learns about an item", function()
      Open({ Item(3371, "Empty Vial", false) })
      assert(items)[1].isLoaded = true
      FirePrivate("MERCHANT_FILTER_ITEM_UPDATE", 3371)
      AdvanceTo(1)

      assert.are.same({ Link(1) }, Values("GetMerchantItemLink", 1))
    end)

    it("should take one sample for a burst of updates and another for a later one", function()
      Open({ Item(3371, "Empty Vial", false) })
      samplesTaken = 0
      for _ = 1, 50 do FirePrivate("MERCHANT_UPDATE") end
      AdvanceTo(0.05)
      for _ = 1, 50 do FirePrivate("MERCHANT_UPDATE") end
      assert.are.equal(0, samplesTaken)
      AdvanceTo(5)
      assert.are.equal(1, samplesTaken)

      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(6)
      assert.are.equal(2, samplesTaken)
    end)

    it("should not sample a complete window again", function()
      Open({ Item(3371, "Empty Vial", true), Item(2320, "Coarse Thread", false) })
      assert(items)[2].isLoaded = true
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(1)
      samplesTaken = 0

      for second = 2, 5 do
        FirePrivate("MERCHANT_UPDATE")
        FirePrivate("MERCHANT_FILTER_ITEM_UPDATE", 3371)
        AdvanceTo(second)
      end
      assert.are.equal(0, samplesTaken)
      assert.are.equal(0, #timers)
    end)

    it("should sample a complete window when its number of items changes", function()
      Open({ Item(3371, "Empty Vial", true) })
      items = { Item(3371, "Empty Vial", true), Item(2320, "Coarse Thread", true) }
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(1)

      assert.are.same({ Link(2) }, Values("GetMerchantItemLink", 2))
      local counts = capture.session.functions["GetMerchantNumItems"]
      assert.are.equal(2, counts[#counts].v)
    end)

    it("should not read anything once the window closed", function()
      Open({ Item(3371, "Empty Vial", false) })
      FirePrivate("MERCHANT_UPDATE")
      Close()
      local reads = 0
      env.GetMerchantNumItems = function()
        reads = reads + 1
        return 0
      end
      AdvanceTo(1)
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(2)
      assert.are.equal(0, reads)
    end)

    it("should not let the previous window's pending sample run in the next window", function()
      Open({ Item(3371, "Empty Vial", false) })
      FirePrivate("MERCHANT_UPDATE")
      Close()
      Open({ Item(2320, "Coarse Thread", false) })
      samplesTaken = 0
      AdvanceTo(1)
      assert.are.equal(0, samplesTaken)

      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(2)
      assert.are.equal(1, samplesTaken)
    end)

    it("should not take a pending sample after the capture stopped, nor in the next capture", function()
      Open({ Item(3371, "Empty Vial", false) })
      FirePrivate("MERCHANT_UPDATE")
      capture.active = false
      tracker.OnCaptureStopped(capture)
      samplesTaken = 0
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(1)
      assert.are.equal(0, samplesTaken)

      capture.active, capture.token, capture.session = true, 2, { functions = {} }
      tracker.Init(capture)
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(2)
      assert.are.equal(0, samplesTaken)
    end)

    it("should record item IDs before the data arrives, and nil after the close", function()
      Open({ Item(3371, "Empty Vial", true), Item(2320, "Coarse Thread", false) })
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(1)
      Close()

      assert.are.same({ 3371, "nil" }, Values("GetMerchantItemID", 1))
      assert.are.same({ 2320, "nil" }, Values("GetMerchantItemID", 2))
    end)

    it("should not record item IDs and filters on clients without these functions, even if an addon adds them later", function()
      env.GetMerchantItemID = nil
      capture = NewCapture()
      tracker.Init(capture)
      env.GetMerchantItemID = function() return nil end
      env.GetMerchantFilter = function() return 1 end
      Open({ Item(3371, "Empty Vial", true) })
      Close()

      assert.is_nil(capture.session.functions["GetMerchantItemID"])
      assert.is_nil(capture.session.functions["GetMerchantFilter"])
    end)

    it("should record the filter each sample was taken with", function()
      local filter = 2
      env.GetMerchantFilter = function() return filter end
      capture = NewCapture()
      tracker.Init(capture)
      Open({ Item(3371, "Empty Vial", false) })
      filter = 1
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(1)
      FirePrivate("MERCHANT_UPDATE")
      AdvanceTo(2)

      local stream = capture.session.functions["GetMerchantFilter"]
      assert.are.same({ 2, 1 }, { stream[1].v, stream[2].v })
      assert.are.equal(2, #stream)
    end)
  end)
end)
