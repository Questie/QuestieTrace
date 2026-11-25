---@type QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- WoW API return schemas (for trace analyzer display labels)
---------------------------------------------------------------------------
-- UnitRace(unit)  -> string localizedRaceName,
--                    string englishRaceName,
--                    number raceID
--
-- UnitClass(unit) -> string className,
--                    string classFilename,
--                    number classID
--
-- UnitClassBase(unit) -> string classFilename,
--                        number classID
--
-- UnitSex(unit)   -> number sex  -- 1=unknown, 2=male, 3=female
--
-- UnitFactionGroup(unit) -> string englishFaction,  -- "Alliance"|"Horde"|"Neutral"
--                           string localizedFaction
--
-- GetLocale() -> string locale
--
-- GetBuildInfo() -> select(4, ...) returns number interfaceVersion
--
-- C_GameRules.IsHardcoreActive() -> boolean active
--
-- C_Seasons.HasActiveSeason() -> boolean value      -- not on Forever
-- C_Seasons.GetActiveSeason() -> number? seasonID   -- not on Forever
---------------------------------------------------------------------------

---Record the rules of the realm: its kind (hardcore, season), never its name.
---@param functions table<string, FunctionStream>
local function RecordRealmRules(functions)
  ---@param key string
  ---@param fn function
  local function Record(key, fn)
    ---@type FunctionStreamEntry[]
    local stream = {}
    functions[key] = stream
    Core.SampleScalar(stream, 0, 0, fn)
  end

  if C_GameRules and C_GameRules.IsHardcoreActive then
    Record("C_GameRules.IsHardcoreActive", C_GameRules.IsHardcoreActive)
  end
  -- Forever has no C_Seasons, but Questie defines a stand-in there. Only the client's own namespace is secure.
  if C_Seasons and issecurevariable("C_Seasons") then
    Record("C_Seasons.HasActiveSeason", C_Seasons.HasActiveSeason)
    Record("C_Seasons.GetActiveSeason", C_Seasons.GetActiveSeason)
  end
end

Core.RegisterTracker({
  -- No events -- sampled once at t=0 only

  ---@param capture CaptureState
  Init = function(capture)
    ---@type table<string, FunctionStream>
    local functions = capture.session.functions
    ---@type string, string, number
    local raceL, raceE, raceID = UnitRace("player")
    ---@type string, string, number
    local classL, classE, classID = UnitClass("player")
    ---@type number?
    local sex = UnitSex("player")
    ---@type string, string
    local factionE, factionL = UnitFactionGroup("player")
    ---@type string
    local locale = GetLocale()
    ---@type number
    local interfaceVersion = select(4, GetBuildInfo())

    functions["UnitRace"] = {
      ["player"] = { { t = 0, tp = 0, v = { raceL, raceE, raceID, n = 3 } } },
    }
    functions["UnitClass"] = {
      ["player"] = { { t = 0, tp = 0, v = { classL, classE, classID, n = 3 } } },
    }
    -- UnitClassBase is absent on some clients. Capture it only when the API is
    -- available so replay can prefer the exact base-class tuple without forcing
    -- older traces/clients to synthesize it from UnitClass.
    if type(UnitClassBase) == "function" then
      local ok, classFilename, classBaseID = pcall(UnitClassBase, "player")
      if ok then
        functions["UnitClassBase"] = {
          ["player"] = { { t = 0, tp = 0, v = { classFilename, classBaseID, n = 2 } } },
        }
      end
    end
    functions["UnitSex"] = {
      ["player"] = { { t = 0, tp = 0, v = sex } },
    }
    functions["UnitFactionGroup"] = {
      ["player"] = { { t = 0, tp = 0, v = { factionE, factionL, n = 2 } } },
    }
    functions["GetLocale"] = {
      ["player"] = { { t = 0, tp = 0, v = locale } },
    }
    functions["GetBuildInfo"] = {
      ["player"] = { { t = 0, tp = 0, v = interfaceVersion } },
    }

    RecordRealmRules(functions)
  end,
})
