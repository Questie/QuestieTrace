---@type QuestieTraceCore
local Core = QuestieTraceCore
---@type fun(...): PackedArgs
local PackArgs = Core.PackArgs
---@type fun(t1: any, t2: any, ignore_mt: boolean?, visited: table?): boolean
local DeepCompare = Core.DeepCompare

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- GetSpellBookItemName(slot, "spell") -> string spellName,
--                                        string spellSubName,
--                                        number spellID
--
-- GetSpellBookItemInfo(slot, "spell") -> string spellType,
--                                        number id
--
-- IsPassiveSpell(slot, "spell") -> 1|nil isPassive
--
-- C_SpellBook.GetSpellBookItemType(slot, bank) -> Enum.SpellBookItemType itemType,
--                                                 number actionID,
--                                                 number? spellID
--   Forever only, where the three functions above are gone. Not recorded
--   itself: it feeds the two streams below.
--
-- SpellBook (custom stream) -> number[] spellIDs  -- ordered known spell IDs
--
-- PlayerKnownSpells (delta stream) -> set<number> spellIDs
---------------------------------------------------------------------------

---@type string
local BOOK_TYPE = "spell"

--- Spells quests depend on that the spell book does not or may not list:
--- C_SpellBook clients are asked about each of them.
---@type number[]
local UNLISTED_SPELL_IDS = {
  -- Riding, which is passive
  824, -- Horse Riding
  825, -- Wolf Riding
  826, -- Ram Riding
  828, -- Tiger Riding
  10861, -- Raptor Riding
  10906, -- Undead Horsemanship
  10907, -- Mechanostrider Piloting
  18995, -- Kodo Riding
  33388, -- Apprentice Riding
  33391, -- Journeyman Riding
  1285849, -- Galestrider Riding
  -- Recipes: QuestieDB hides the quests that teach them from players who know them (requiredSpell)
  1229432, -- Camp Tent
  1229504, -- Faction Banner
  1229517, -- Camp Chair
  1229705, -- Incense Candle
  1229745, -- Fish Bowl
  1230117, -- First Aid Kit
  1230161, -- Lodestone
  1230171, -- Sharpening Wheel
  1230564, -- Mana Well
  1230643, -- Enchanted Lute
  1230656, -- Reagent Bot
  1263425, -- Faction Banner
}

---@type table<string, FunctionStreamEntry[]|table<string|number, FunctionStreamEntry[]>>
local functions
---@type table<string, DeltaStream>
local functionsDelta
---@type table<number, PackedArgs?>
local prevName
---@type table<number, PackedArgs?>
local prevInfo
---@type table<number, number?>
local prevPassive
---@type table<number, boolean>
local knownSlots
---@type table<number, boolean>
local currentKnownSpells
---@type boolean
local hasSpellBookApi

---Whether the client lists its spell book through C_SpellBook. Other clients
---have a C_SpellBook without GetSpellBookItemType and keep the old functions.
---Checked before those, since other addons bring them back on Forever without
---the spell ID this tracker reads.
---@return boolean
local function HasSpellBookApi()
  return type(C_SpellBook) == "table" and type(C_SpellBook.GetSpellBookItemType) == "function"
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

--- Append a parameterized packed value only when it changed.
---@param funcName string
---@param key number
---@param prev table<number, PackedArgs?>
---@param t number
---@param tp number
---@param value PackedArgs?
local function AppendPackedIfChanged(funcName, key, prev, t, tp, value)
  local old = prev[key]
  ---@type boolean
  local changed

  if old == nil and value == nil then
    changed = functions[funcName][key] == nil or #functions[funcName][key] == 0
  elseif old == nil or value == nil then
    changed = true
  else
    changed = not DeepCompare(old, value)
  end

  if not changed then return end

  local stream = functions[funcName][key]
  if not stream then
    stream = {}
    functions[funcName][key] = stream
  end
  stream[#stream + 1] = { t = t, tp = tp, v = value }
  prev[key] = value
end

--- Append a parameterized scalar only when it changed.
---@param funcName string
---@param key number
---@param prev table<number, number?>
---@param t number
---@param tp number
---@param value number?
local function AppendScalarIfChanged(funcName, key, prev, t, tp, value)
  local old = prev[key]
  local stream = functions[funcName][key]
  local hasEntries = stream ~= nil and #stream > 0
  if old == value and hasEntries then return end

  if not stream then
    stream = {}
    functions[funcName][key] = stream
  end
  stream[#stream + 1] = { t = t, tp = tp, v = value }
  prev[key] = value
end

--- Append the ordered spell list when it changed.
---@param t number
---@param tp number
---@param ids number[]
local function AppendSpellBookIfChanged(t, tp, ids)
  local stream = functions["SpellBook"]
  local prev = stream[#stream]
  if not prev or not DeepCompare(prev.v, ids) then
    local copy = {}
    for i = 1, #ids do
      copy[i] = ids[i]
    end
    stream[#stream + 1] = { t = t, tp = tp, v = copy }
  end
end

--- Diff and append PlayerKnownSpells delta entries.
---@param capture CaptureState
---@param newSet table<number, boolean>
local function AppendKnownSpellDelta(capture, newSet)
  local added, removed = {}, {}
  for spellID in pairs(newSet) do
    if not currentKnownSpells[spellID] then
      added[#added + 1] = spellID
    end
  end
  for spellID in pairs(currentKnownSpells) do
    if not newSet[spellID] then
      removed[#removed + 1] = spellID
    end
  end

  if #added > 0 or #removed > 0 then
    table.sort(added)
    table.sort(removed)

    local delta = {
      t = GetTime() - capture.startedAt,
      tp = GetTimePreciseSec() - capture.startedAtPrecise,
    }
    if #added > 0 then delta.add = added end
    if #removed > 0 then delta.remove = removed end

    functionsDelta["PlayerKnownSpells"].delta[#functionsDelta["PlayerKnownSpells"].delta + 1] = delta
  end

  currentKnownSpells = newSet
end

--- Sample the spell book through C_SpellBook. GetSpellBookItemType returns plain
--- values, where GetSpellBookItemInfo returns a table for each slot.
---@param capture CaptureState
local function SampleSpellBookApi(capture)
  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise
  local playerBank = Enum.SpellBookSpellBank.Player
  local learnedSpell = Enum.SpellBookItemType.Spell

  local orderedSpellIDs = {}
  local newKnownSpells = {}
  local slot = 1
  local itemType, spellID = C_SpellBook.GetSpellBookItemType(slot, playerBank)
  -- Profession spells come after the slots the skill lines count, so the book
  -- ends at its first empty slot.
  while itemType do
    -- Left out: spells not learned yet, and flyouts, whose spells have slots
    -- of their own.
    if itemType == learnedSpell and not newKnownSpells[spellID] then
      newKnownSpells[spellID] = true
      orderedSpellIDs[#orderedSpellIDs + 1] = spellID
    end
    slot = slot + 1
    itemType, spellID = C_SpellBook.GetSpellBookItemType(slot, playerBank)
  end

  for i = 1, #UNLISTED_SPELL_IDS do
    if C_SpellBook.IsSpellKnown(UNLISTED_SPELL_IDS[i]) then
      newKnownSpells[UNLISTED_SPELL_IDS[i]] = true
    end
  end
  -- The book can list fewer spells than the player knows, e.g. while it loads.
  for knownSpellID in pairs(currentKnownSpells) do
    if not newKnownSpells[knownSpellID] and C_SpellBook.IsSpellKnown(knownSpellID) then
      newKnownSpells[knownSpellID] = true
    end
  end

  AppendSpellBookIfChanged(t, tp, orderedSpellIDs)
  AppendKnownSpellDelta(capture, newKnownSpells)
end

--- Enumerate the entire player spellbook, including profession tabs, through
--- the old slot functions.
---@param capture CaptureState
local function SampleSpellBookSlots(capture)
  local t = GetTime() - capture.startedAt
  local tp = GetTimePreciseSec() - capture.startedAtPrecise

  local seenSlots = {}
  local orderedSpellIDs = {}
  local seenSpellIDs = {}
  local newKnownSpells = {}

  if type(GetSpellBookItemName) == "function" then
    local slot = 1
    while true do
      local nameValue = PackArgs(GetSpellBookItemName(slot, BOOK_TYPE))
      if nameValue[1] == nil then break end
      local spellID = nameValue[3]

      seenSlots[slot] = true
      knownSlots[slot] = true

      AppendPackedIfChanged("GetSpellBookItemName", slot, prevName, t, tp, nameValue)

      if type(GetSpellBookItemInfo) == "function" then
        local infoValue = PackArgs(GetSpellBookItemInfo(slot, BOOK_TYPE))
        AppendPackedIfChanged("GetSpellBookItemInfo", slot, prevInfo, t, tp, infoValue)
      end

      if type(IsPassiveSpell) == "function" then
        local passiveValue = IsPassiveSpell(slot, BOOK_TYPE)
        AppendScalarIfChanged("IsPassiveSpell", slot, prevPassive, t, tp, passiveValue)
      end

      if type(spellID) == "number" then
        newKnownSpells[spellID] = true
        if not seenSpellIDs[spellID] then
          seenSpellIDs[spellID] = true
          orderedSpellIDs[#orderedSpellIDs + 1] = spellID
        end
      end

      slot = slot + 1
    end
  end

  for slot in pairs(knownSlots) do
    if not seenSlots[slot] then
      local nameOk, nameValue = SafePackedCall(GetSpellBookItemName, slot, BOOK_TYPE)
      if nameOk and nameValue then
        AppendPackedIfChanged("GetSpellBookItemName", slot, prevName, t, tp, nameValue)
      end

      local infoOk, infoValue = SafePackedCall(GetSpellBookItemInfo, slot, BOOK_TYPE)
      if infoOk and infoValue then
        AppendPackedIfChanged("GetSpellBookItemInfo", slot, prevInfo, t, tp, infoValue)
      end

      local passiveOk, passiveValue = SafeScalarCall(IsPassiveSpell, slot, BOOK_TYPE)
      if passiveOk then
        AppendScalarIfChanged("IsPassiveSpell", slot, prevPassive, t, tp, passiveValue)
      end
    end
  end

  AppendSpellBookIfChanged(t, tp, orderedSpellIDs)
  AppendKnownSpellDelta(capture, newKnownSpells)
end

---@param capture CaptureState
local function SampleSpellBook(capture)
  if not hasSpellBookApi then
    SampleSpellBookSlots(capture)
    return
  end

  -- Reported like any Lua error, but the capture and the other trackers go on.
  local ok, err = pcall(SampleSpellBookApi, capture)
  if not ok then geterrorhandler()(err) end
end

Core.RegisterTracker({
  events = {
    "SPELLS_CHANGED",
    "PLAYER_ENTERING_WORLD",
  },

  ---@param capture CaptureState
  Init = function(capture)
    functions = capture.session.functions
    functionsDelta = capture.session.functionsDelta

    hasSpellBookApi = HasSpellBookApi()

    functions["SpellBook"] = {}
    if not hasSpellBookApi then
      functions["GetSpellBookItemName"] = {}
      functions["GetSpellBookItemInfo"] = {}
      functions["IsPassiveSpell"] = {}
    end

    prevName = {}
    prevInfo = {}
    prevPassive = {}
    knownSlots = {}

    ---@type table<number, boolean>
    local initialKnownSpells = {}
    currentKnownSpells = initialKnownSpells
    functionsDelta["PlayerKnownSpells"] = {
      t = 0,
      tp = 0,
      initial = {},
      delta = {},
    }

    SampleSpellBook(capture)
    functionsDelta["PlayerKnownSpells"].initial = {}
    for spellID in pairs(currentKnownSpells) do
      functionsDelta["PlayerKnownSpells"].initial[#functionsDelta["PlayerKnownSpells"].initial + 1] = spellID
    end
    table.sort(functionsDelta["PlayerKnownSpells"].initial)
    functionsDelta["PlayerKnownSpells"].delta = {}
  end,

  ---@param capture CaptureState
  OnEvent = function(capture)
    SampleSpellBook(capture)
  end,
})
