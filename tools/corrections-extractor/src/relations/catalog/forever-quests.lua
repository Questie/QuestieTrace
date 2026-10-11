-- Prints Forever's effective quest rows as JSON for catalog/materialize.ts.
--
-- Layering is QuestieDB's own, not a re-implementation: generator/flavor.lua loads
-- data/Forever and applies every Static Correction through the shipped registry
-- (legacy -> generated -> traces -> authored, replace / _add / _remove / `{}` deletion),
-- then runs the Derived Passes. Dynamic Corrections are evaluated per faction x class on top.
--
-- Run from a QuestieDB checkout with Lua 5.1:
--   lua <this file> <field,field,...> [fixture.lua]
--
-- Prints one JSON object:
--   sources        apply order: base data, Static Corrections, Derived Passes, Dynamic Corrections
--   quests         id -> {field = value} after Static Corrections and Derived Passes
--   dynamicViews   id -> distinct rows the faction x class views see, only for quests a Dynamic
--                  Correction touches. Every real character sees one of these, never the static row.
--   authoredFields id -> fields an authored forever*Fixes.lua provider replaces, adds to or removes from
--   inheritedIds   quests from the inherited Era baseline: data/Forever rows plus quests a
--                  Forever/legacy/ provider touches. Everything else is Forever-new.
-- Values are raw (null holes, empty tables and zeroes kept); catalog/build.ts normalizes them.
--
-- The optional fixture replaces QuestieDB's data and correction files with inline layers, so tests
-- exercise the real registry on tiny inputs. It returns `function(questKeys) return {
--   base = {[id] = row}, layers = {{name = "Forever/x.lua:Load", dynamic = bool?, rows = table|function}}
-- } end`, layers in apply order.

local fieldList, fixturePath = ...

local config = dofile("src/config.lua")
local meta = dofile("src/meta/questMeta.lua")
local flavor = config.flavorByName.Forever

-- src/corrections/tablePatch.lua: index + 1000 adds list elements, index - 1000 removes them.
local OPERATION_OFFSET = 1000
-- Same selection as tools/export/inline.lua: authored providers sit directly in Forever/.
local AUTHORED_PATTERN = "^Forever/forever[^/]+Fixes%.lua:"
-- Inherited Era Corrections (docs/forever.md); new Forever work never goes here.
local INHERITED_PATTERN = "^Forever/legacy/"
-- What Dynamic Corrections branch on (UnitFactionGroup / UnitClassBase tokens).
local FACTIONS = { "Alliance", "Horde" }
local CLASSES = { "WARRIOR", "PALADIN", "HUNTER", "ROGUE", "PRIEST", "SHAMAN", "MAGE", "WARLOCK", "DRUID" }

---------------------------------------------------------------------------------------------
-- JSON
---------------------------------------------------------------------------------------------

local function encodeString(value)
  local escaped = value:gsub('[%c"\\]', function(char)
    if char == '"' then return '\\"' end
    if char == "\\" then return "\\\\" end
    return string.format("\\u%04x", char:byte())
  end)
  return '"' .. escaped .. '"'
end

-- Not "%d": Lua 5.1 casts it to a C long, which is 32-bit on Windows and would cut Forever's
-- requiredRaces bits above 2^32. "%.0f" is exact for every integer JSON can carry (< 2^53).
local function encodeInteger(value)
  return string.format("%.0f", value)
end

-- Exported quest fields hold numbers, strings and positional tuples/lists only, so every table
-- is a JSON array and a nil hole becomes null.
local function encodeValue(value)
  local kind = type(value)
  if kind == "number" then
    assert(value == value and value ~= math.huge and value ~= -math.huge, "non-finite number")
    return value % 1 == 0 and encodeInteger(value) or string.format("%.17g", value)
  elseif kind == "string" then
    return encodeString(value)
  elseif kind == "boolean" then
    return tostring(value)
  elseif kind == "table" then
    local last = 0
    for key in pairs(value) do
      assert(type(key) == "number" and key >= 1 and key % 1 == 0, "non-positional table key " .. tostring(key))
      if key > last then last = key end
    end
    local parts = {}
    for index = 1, last do
      parts[index] = value[index] == nil and "null" or encodeValue(value[index])
    end
    return "[" .. table.concat(parts, ",") .. "]"
  end
  error("cannot encode a " .. kind)
end

local function sortedKeys(map)
  local keys = {}
  for key in pairs(map) do keys[#keys + 1] = key end
  table.sort(keys)
  return keys
end

---@param map table<number, any>
---@param encodeEntry fun(value: any): string
local function encodeIdMap(map, encodeEntry)
  local parts = {}
  for _, id in ipairs(sortedKeys(map)) do
    parts[#parts + 1] = '"' .. encodeInteger(id) .. '":' .. encodeEntry(map[id])
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

local fields = {}
for name in assert(fieldList, "usage: lua forever-quests.lua <field,field,...> [fixture.lua]"):gmatch("[^,]+") do
  fields[#fields + 1] = { name = name, index = assert(meta.keys[name], "unknown quest field " .. name) }
end

local function encodeRow(row)
  local parts = {}
  for _, field in ipairs(fields) do
    if row[field.index] ~= nil then
      parts[#parts + 1] = encodeString(field.name) .. ":" .. encodeValue(row[field.index])
    end
  end
  return "{" .. table.concat(parts, ",") .. "}"
end

---------------------------------------------------------------------------------------------
-- Inputs
---------------------------------------------------------------------------------------------

local function idSet(rows)
  local ids = {}
  for id in pairs(rows) do ids[id] = true end
  return ids
end

---@return table entities Quest rows after Static Corrections and Derived Passes
---@return table registry The correction registry those corrections were registered in
---@return string[] sources Base data, then the Derived Passes (corrections are listed later)
---@return table baseIds id -> true for every row in the uncorrected base data
local function loadForever()
  local flavorLoader = dofile("generator/flavor.lua")
  local loaded = flavorLoader.load(flavor, { Quest = true }, true)
  local raw = flavorLoader.load(flavor, { Quest = true }, false)
  -- flavor.lua keeps its prepared registry private; this one is built from the same files.
  local lib = dofile("generator/corrections.lua").prepare(flavor).lib
  local passes = {}
  for _, pass in ipairs(lib.Derived.Select("Quest")) do
    if not pass.expansions or pass.expansions[flavor.expansion] then
      passes[#passes + 1] = "derived pass " .. pass.name
    end
  end
  return loaded.Quest.entities, lib.Corrections, { loaded.Quest.path }, passes, idSet(raw.Quest.entities)
end

local function loadFixture(path)
  local lib = dofile("generator/runtime.lua").build()
  local registry = lib.Corrections
  local fixture = dofile(path)(lib.Enum.questKeys)
  for order, layer in ipairs(fixture.layers) do
    local register = layer.dynamic and registry.RegisterRuntimeCorrection or registry.RegisterCorrection
    local rows = layer.rows
    register(registry.OWNER, "Quest", layer.name, type(rows) == "function" and rows or function() return rows end, order)
  end
  local baseIds = idSet(fixture.base)
  registry.ApplyStaticToEntities("Quest", fixture.base, flavor, registry.OWNER)
  return fixture.base, registry, { path }, {}, baseIds
end

---------------------------------------------------------------------------------------------
-- Export
---------------------------------------------------------------------------------------------

local entities, registry, sources, derivedPasses, inheritedIds
if fixturePath then
  entities, registry, sources, derivedPasses, inheritedIds = loadFixture(fixturePath)
else
  entities, registry, sources, derivedPasses, inheritedIds = loadForever()
end

local function selectEntries(dynamic)
  local selected = {}
  for _, entry in ipairs(registry.Select({ datatype = "Quest", dynamic = dynamic, owner = registry.OWNER })) do
    if registry.EntryApplies(entry, flavor) then selected[#selected + 1] = entry end
  end
  return selected
end

-- Provenance from one provider's correction table: quests a legacy provider touches are
-- inherited, and authored providers record which fields they touch (_add/_remove keys included).
local authoredFields = {}
local function noteProvenance(entry, rows)
  local inherited = entry.name:match(INHERITED_PATTERN)
  local authored = entry.name:match(AUTHORED_PATTERN)
  for id, row in pairs(rows) do
    if inherited then inheritedIds[id] = true end
    if authored then
      for key in pairs(row) do
        if type(key) == "number" then
          local index = key >= OPERATION_OFFSET and key - OPERATION_OFFSET or (key <= 0 and key + OPERATION_OFFSET or key)
          authoredFields[id] = authoredFields[id] or {}
          authoredFields[id][assert(meta.names[index], "unknown quest field index " .. key)] = true
        end
      end
    end
  end
end

for _, entry in ipairs(selectEntries(false)) do
  sources[#sources + 1] = "src/corrections/" .. entry.name
  -- Re-running a provider costs time, so only the ones provenance cares about.
  if entry.name:match(AUTHORED_PATTERN) or entry.name:match(INHERITED_PATTERN) then noteProvenance(entry, entry.func()) end
end
for _, pass in ipairs(derivedPasses) do sources[#sources + 1] = pass end

-- Dynamic Corrections overlay the static row at query time. Evaluate them once per faction x
-- class and keep, per touched quest, the distinct rows those characters would read.
local dynamicEntries = selectEntries(true)
local views, touched = {}, {}
for _, faction in ipairs(FACTIONS) do
  for _, class in ipairs(CLASSES) do
    _G.UnitFactionGroup = function() return faction end
    _G.UnitClassBase = function() return class end
    local view = {}
    for _, entry in ipairs(dynamicEntries) do
      local rows = entry.func()
      for id in pairs(rows) do
        touched[id] = true
        -- Shallow copies are enough: the registry replaces field values and never mutates them.
        if view[id] == nil and entities[id] then
          local copy = {}
          for index, value in pairs(entities[id]) do copy[index] = value end
          view[id] = copy
        end
      end
      registry.MergeInto(view, rows, entry.options, entry)
      noteProvenance(entry, rows)
    end
    views[#views + 1] = view
  end
end
_G.UnitFactionGroup, _G.UnitClassBase = nil, nil
for _, entry in ipairs(dynamicEntries) do
  sources[#sources + 1] = "src/corrections/" .. entry.name .. " (dynamic, per faction and class)"
end

local dynamicViews = {}
for id in pairs(touched) do
  local seen, rows = {}, {}
  for _, view in ipairs(views) do
    local row = view[id] or entities[id]
    if row then
      local encoded = encodeRow(row)
      if not seen[encoded] then
        seen[encoded] = true
        rows[#rows + 1] = encoded
      end
    end
  end
  dynamicViews[id] = rows
end

io.write("{",
  '"sources":', encodeValue(sources), ",",
  '"quests":', encodeIdMap(entities, encodeRow), ",",
  '"dynamicViews":', encodeIdMap(dynamicViews, function(rows) return "[" .. table.concat(rows, ",") .. "]" end), ",",
  '"authoredFields":', encodeIdMap(authoredFields, function(set) return encodeValue(sortedKeys(set)) end), ",",
  '"inheritedIds":', encodeValue(sortedKeys(inheritedIds)),
  "}\n")
