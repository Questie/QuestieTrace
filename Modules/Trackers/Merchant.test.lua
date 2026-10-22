---@param env table<string, any>
---@return table tracker
local function LoadMerchantTracker(env)
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
end)
