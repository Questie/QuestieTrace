---@type QuestieTraceCore
local Core = QuestieTraceCore
---@type fun(...): PackedArgs
local PackArgs = Core.PackArgs
---@type fun(t1: any, t2: any, ignore_mt: boolean?, visited: table?): boolean
local DeepCompare = Core.DeepCompare

local C_After = C_Timer.After

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- The client sends item data after MERCHANT_SHOW: until it arrives, a slot has
-- a price but neither name nor link. So a window is sampled again on updates.
--
-- GetMerchantNumItems() -> number numMerchantItems
-- GetMerchantFilter() -> number filter  -- LE_LOOT_FILTER_*; it decides which items the slots list
-- GetMerchantItemID(index) -> number? itemID
-- GetMerchantItemLink(index) -> string? itemLink  -- carries the item ID
-- GetMerchantItemInfo(index) ->
--   string name, number texture, number price, number quantity,
--   number numAvailable, boolean isPurchasable, boolean isUsable,
--   boolean extendedCost, number? currencyID, number? spellID
--
-- C_MerchantFrame.GetItemInfo(index) -> MerchantItemInfo table:
--   { name=string?, texture=number, price=number, stackCount=number,
--     numAvailable=number, isPurchasable=boolean, isUsable=boolean,
--     hasExtendedCost=boolean, currencyID=number?, spellID=number?,
--     isQuestStartItem=boolean }

---@type table<string, FunctionStream>
local functions
---@type table<number, FunctionStreamEntry[]>? [index] -> stream
local merchantItemStreams
---@type table<number, FunctionStreamEntry[]>? [index] -> stream for modern API
local merchantModernItemStreams
---@type boolean?
local hasModernItemAPI

--- Seconds between an update and the sample it asks for, so a burst of updates
--- (one per arriving item) costs one sample. Item data arrives within 40 ms for
--- 9 of 10 items, and 1 window in 100 closes within 0.7 s.
---@type number
local SAMPLE_DELAY = 0.1

---@class MerchantWindow
---@field capture CaptureState
---@field itemCount number? Slots in the last sample
---@field isMissingItems boolean Whether a slot of the last sample had no link
---@field isSamplePending boolean

---@type MerchantWindow? The open window, if any. Each opening gets its own table.
local openWindow

---Safely call a function and return the first result.
---@param fn function?
---@param ... any
---@return boolean ok
---@return any value
local function SafeScalarCall(fn, ...)
  if type(fn) ~= "function" then return false, nil end
  local ok, value = pcall(fn, ...)
  if not ok then return false, nil end
  return true, value
end

---Safely call a function and pack all returned values.
---@param fn function?
---@param ... any
---@return boolean ok
---@return PackedArgs? value
local function SafePackedCall(fn, ...)
  if type(fn) ~= "function" then return false, nil end
  local packed = PackArgs(pcall(fn, ...))
  if not packed[1] then return false, nil end

  local out = { n = packed.n - 1 }
  for i = 2, packed.n do
    out[i - 1] = packed[i]
  end
  return true, out
end

---Get or create a parameterized function stream for a given function name and key.
---@param funcName string
---@param key string|number
---@return FunctionStreamEntry[]
local function GetOrCreateParamStream(funcName, key)
  if not functions[funcName] then
    functions[funcName] = {}
  end
  local streams = functions[funcName]
  ---@cast streams table<string|number, FunctionStreamEntry[]>
  if not streams[key] then
    streams[key] = {}
  end
  return streams[key]
end

---Append a scalar value to a stream if it changed.
---@param stream FunctionStreamEntry[]
---@param t number
---@param tp number
---@param value any
local function AppendScalarIfChanged(stream, t, tp, value)
  local prev = stream[#stream]
  if not prev or prev.v ~= value then
    stream[#stream + 1] = { t = t, tp = tp, v = value }
  end
end

---Append a packed/table value to a stream if it changed.
---@param stream FunctionStreamEntry[]
---@param t number
---@param tp number
---@param value any
local function AppendIfChanged(stream, t, tp, value)
  local prev = stream[#stream]
  local changed

  if prev == nil and value == nil then
    changed = #stream == 0
  elseif prev == nil or value == nil then
    changed = true
  elseif type(value) == "table" then
    changed = not DeepCompare(value, prev.v)
  else
    changed = value ~= prev.v
  end

  if changed then
    stream[#stream + 1] = { t = t, tp = tp, v = value }
  end
end

---Ensure stream references exist for a merchant item index (legacy API).
---@param index number
---@return FunctionStreamEntry[] stream
local function GetOrCreateLegacyItemStream(index)
  merchantItemStreams = merchantItemStreams or {}
  if not merchantItemStreams[index] then
    merchantItemStreams[index] = GetOrCreateParamStream("GetMerchantItemInfo", index)
  end
  return merchantItemStreams[index]
end

---Ensure stream references exist for a merchant item index (modern API).
---@param index number
---@return FunctionStreamEntry[] stream
local function GetOrCreateModernItemStream(index)
  merchantModernItemStreams = merchantModernItemStreams or {}
  if not merchantModernItemStreams[index] then
    merchantModernItemStreams[index] = GetOrCreateParamStream("C_MerchantFrame.GetItemInfo", index)
  end
  return merchantModernItemStreams[index]
end

---Sample GetMerchantNumItems and append the observed API return.
---@param t number
---@param tp number
---@return number? itemCount
local function SampleMerchantCount(t, tp)
  local ok, itemCount = SafeScalarCall(GetMerchantNumItems)
  if not ok then return nil end

  local countStream = functions["GetMerchantNumItems"]
  ---@cast countStream FunctionStreamEntry[]
  AppendScalarIfChanged(countStream, t, tp, itemCount)

  if type(itemCount) == "number" then return itemCount end
  return nil
end

---Append a slot's answer to a function that returns one value. A slot without
---an answer yet gets no stream.
---@param funcName "GetMerchantItemID"|"GetMerchantItemLink"
---@param fn function?
---@param t number
---@param tp number
---@param index number
---@return any value
local function ProbeSlotScalar(funcName, fn, t, tp, index)
  -- Clients without the function have no streams, see Init.
  local streams = functions[funcName]
  if not streams then return nil end

  local ok, value = SafeScalarCall(fn, index)
  if ok and (value ~= nil or streams[index]) then
    AppendScalarIfChanged(GetOrCreateParamStream(funcName, index), t, tp, value)
  end
  return value
end

---Probe one merchant item index using both legacy and modern APIs when available.
---@param t number
---@param tp number
---@param index number
---@return boolean hasLink Whether the client has the item's data
local function ProbeMerchantItem(t, tp, index)
  -- Legacy API: GetMerchantItemInfo
  local legacyOk, legacyValue = SafePackedCall(GetMerchantItemInfo, index)
  if legacyOk and legacyValue and legacyValue.n > 0 then
    local stream = GetOrCreateLegacyItemStream(index)
    AppendIfChanged(stream, t, tp, legacyValue)
  end

  ProbeSlotScalar("GetMerchantItemID", GetMerchantItemID, t, tp, index)
  local link = ProbeSlotScalar("GetMerchantItemLink", GetMerchantItemLink, t, tp, index)

  -- Modern API: C_MerchantFrame.GetItemInfo (when available)
  if hasModernItemAPI then
    local modernOk, modernValue = SafeScalarCall(C_MerchantFrame.GetItemInfo, index)
    if modernOk and modernValue then
      -- Modern API returns a table, wrap it in PackedArgs for consistency
      modernValue = { n = 1, modernValue }
      local stream = GetOrCreateModernItemStream(index)
      AppendIfChanged(stream, t, tp, modernValue)
    end
  end

  return link ~= nil
end

---Sample current merchant items by calling raw APIs.
---@param window MerchantWindow
local function SampleMerchantOpen(window)
  local capture = window.capture
  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  local filterStream = functions["GetMerchantFilter"] --[[@as FunctionStreamEntry[]?]]
  if filterStream then
    local ok, filter = SafeScalarCall(GetMerchantFilter)
    if ok then AppendScalarIfChanged(filterStream, t, tp, filter) end
  end

  window.itemCount = SampleMerchantCount(t, tp)
  window.isMissingItems = false
  for index = 1, window.itemCount or 0 do
    if not ProbeMerchantItem(t, tp, index) then window.isMissingItems = true end
  end
end

---Sample merchant APIs at close without inventing inactive values.
---Known indices are probed because close is the causal event where raw
---merchant APIs may naturally change to nil/zero/error states.
---@param capture CaptureState
local function SampleMerchantClose(capture)
  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise
  SampleMerchantCount(t, tp)

  if merchantItemStreams then
    for index in pairs(merchantItemStreams) do
      ProbeMerchantItem(t, tp, index)
    end
  end
  if merchantModernItemStreams then
    for index in pairs(merchantModernItemStreams) do
      ProbeMerchantItem(t, tp, index)
    end
  end
  for index in pairs(functions["GetMerchantItemLink"]) do
    ---@cast index number
    ProbeMerchantItem(t, tp, index)
  end
end

---------------------------------------------------------------------------
-- Private update frame
-- MERCHANT_UPDATE is what Blizzard's merchant window redraws an item on when its
-- data arrives; MERCHANT_FILTER_ITEM_UPDATE names an item. Both are sampling
-- triggers only and stay out of the recorded event stream.
---------------------------------------------------------------------------

---@type Frame
local updateFrame = CreateFrame("Frame")
updateFrame:RegisterEvent("MERCHANT_UPDATE")
updateFrame:RegisterEvent("MERCHANT_FILTER_ITEM_UPDATE")
updateFrame:SetScript("OnEvent", function()
  local window = openWindow
  if not window or window.isSamplePending then return end

  -- With every link known, only a list of another length is news.
  if not window.isMissingItems then
    local ok, itemCount = SafeScalarCall(GetMerchantNumItems)
    if not ok or itemCount == window.itemCount then return end
  end

  window.isSamplePending = true
  C_After(SAMPLE_DELAY, function()
    if openWindow ~= window then return end
    window.isSamplePending = false
    SampleMerchantOpen(window)
  end)
end)

Core.RegisterTracker({
  events = { "MERCHANT_SHOW", "MERCHANT_CLOSED" },

  ---@param capture CaptureState
  Init = function(capture)
    functions = capture.session.functions
    merchantItemStreams = nil
    merchantModernItemStreams = nil

    hasModernItemAPI = (C_MerchantFrame ~= nil and type(C_MerchantFrame.GetItemInfo) == "function")

    functions["GetMerchantNumItems"] = {}
    functions["GetMerchantItemInfo"] = {}
    functions["C_MerchantFrame.GetItemInfo"] = {}
    functions["GetMerchantItemLink"] = {}
    if type(GetMerchantItemID) == "function" then
      functions["GetMerchantItemID"] = {}
    end
    if type(GetMerchantFilter) == "function" then
      functions["GetMerchantFilter"] = {}
    end
  end,

  ---@param capture CaptureState
  ---@param event string
  OnEvent = function(capture, event)
    if event == "MERCHANT_SHOW" then
      openWindow = { capture = capture, isMissingItems = false, isSamplePending = false }
      SampleMerchantOpen(openWindow)
    elseif event == "MERCHANT_CLOSED" then
      openWindow = nil
      SampleMerchantClose(capture)
    end
  end,

  OnCaptureStopped = function()
    openWindow = nil
  end,
})
