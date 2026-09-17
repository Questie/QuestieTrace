---@param env table<string, any>
---@return QuestieTraceCore
local function LoadExportModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = { l10n = function(s) return s end }

  -- Mock EncodeExportPayload to return a test string
  env.QuestieTraceCore.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end

  local chunk = assert(loadfile("Modules/Export/Export.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

---@param env table<string, any>
---@param functions table
local function SetSessionFunctions(env, functions)
  env.QuestieTraceCharacter = {
    sessions = { { functions = functions } },
  }
end

describe("Export.ScrubFunctions", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportModule(env)
  end)

  it("should scrub the player token's UnitGUID and UnitName", function()
    SetSessionFunctions(env, {
      UnitGUID = {
        player = { { t = 0, tp = 0, v = "Player-1-000001" } },
        npc = { { t = 0, tp = 0, v = "Creature-0-1-1-1-123-000001" } },
      },
      UnitName = {
        player = { { t = 0, tp = 0, v = { "Hero", "Realm", n = 2 } } },
        questnpc = { { t = 0, tp = 0, v = { "Some NPC", nil, n = 2 } } },
      },
    })

    local functions = Core.BuildExportPayload().sessions[1].functions

    assert.is_nil(functions.UnitGUID.player)
    assert.is_not_nil(functions.UnitGUID.npc)
    assert.is_nil(functions.UnitName.player)
    assert.is_not_nil(functions.UnitName.questnpc)
  end)

  it("should scrub a non-player token when its UnitGUID matches the player's GUID", function()
    -- e.g. the player targeted themselves, so the "target" token observed the player's GUID.
    SetSessionFunctions(env, {
      UnitGUID = {
        player = { { t = 0, tp = 0, v = "Player-1-000001" } },
        target = { { t = 5, tp = 5, v = "Player-1-000001" } },
      },
      UnitName = {
        player = { { t = 0, tp = 0, v = { "Hero", "Realm", n = 2 } } },
        target = { { t = 5, tp = 5, v = { "Hero", "Realm", n = 2 } } },
      },
    })

    local functions = Core.BuildExportPayload().sessions[1].functions

    assert.is_nil(functions.UnitGUID.target)
    assert.is_nil(functions.UnitName.target)
  end)

  it("should preserve unrelated observations in the same token stream", function()
    SetSessionFunctions(env, {
      UnitGUID = {
        player = { { t = 0, tp = 0, v = "Player-1-000001" } },
        target = {
          { t = 1, tp = 1, v = "Player-1-000001" },
          { t = 2, tp = 2, v = "Creature-0-1-1-1-999-000002" },
        },
      },
      UnitName = {
        player = { { t = 0, tp = 0, v = { "Hero", "Realm", n = 2 } } },
        target = {
          { t = 1, tp = 1, v = { "Hero", "Realm", n = 2 } },
          { t = 2, tp = 2, v = { "Some Mob", nil, n = 2 } },
        },
      },
    })

    local functions = Core.BuildExportPayload().sessions[1].functions

    assert.equal(1, #functions.UnitGUID.target)
    assert.equal("Creature-0-1-1-1-999-000002", functions.UnitGUID.target[1].v)
    assert.equal(1, #functions.UnitName.target)
    assert.equal("Some Mob", functions.UnitName.target[1].v[1])
  end)

  it("should not scrub anything when the player's GUID is unknown", function()
    SetSessionFunctions(env, {
      UnitGUID = {
        npc = { { t = 0, tp = 0, v = "Creature-0-1-1-1-123-000001" } },
      },
      UnitName = {
        questnpc = { { t = 0, tp = 0, v = { "Some NPC", nil, n = 2 } } },
      },
    })

    local functions = Core.BuildExportPayload().sessions[1].functions

    assert.is_not_nil(functions.UnitGUID.npc)
    assert.is_not_nil(functions.UnitName.questnpc)
  end)

  it("should not mutate the original saved session", function()
    SetSessionFunctions(env, {
      UnitGUID = { player = { { t = 0, tp = 0, v = "Player-1-000001" } } },
      UnitName = { player = { { t = 0, tp = 0, v = { "Hero", "Realm", n = 2 } } } },
    })

    Core.BuildExportPayload()

    local savedSession = env.QuestieTraceCharacter.sessions[1]
    assert.is_not_nil(savedSession.functions.UnitGUID.player)
    assert.is_not_nil(savedSession.functions.UnitName.player)
  end)

  it("should not error when functions is missing UnitGUID or UnitName", function()
    SetSessionFunctions(env, {})

    assert.has_no.errors(function()
      Core.BuildExportPayload()
    end)
  end)
end)

describe("Export.BuildExportString", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportModule(env)
    SetSessionFunctions(env, {})
  end)

  it("should have prefix, payload, and suffix in correct order", function()
    local ok, result = Core.BuildExportString()

    assert.is_true(ok)
    assert.equal("!QuestieTrace:1!ENCODED_PAYLOAD!End:QuestieTrace:1!", result)
  end)

  it("should still return a formatted string even when there is nothing to export", function()
    local env2 = {}
    local Core2 = LoadExportModule(env2)
    env2.QuestieTraceCharacter = { sessions = {} }

    local ok, result = Core2.BuildExportString()

    assert.is_true(ok)
    assert.matches("^!QuestieTrace:1!.*!End:QuestieTrace:1!$", result)
  end)
end)

describe("Export.currentSession inclusion", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportModule(env)
  end)

  it("should include the current unsaved session when it has events", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = {
        name = "unsaved",
        functions = {},
        events = { { t = 0, e = "PLAYER_LOGIN" } },
      },
    }

    local payload = Core.BuildExportPayload()

    assert.equal(1, #payload.sessions)
    assert.equal("unsaved", payload.sessions[1].name)
  end)

  it("should include both saved sessions and the current unsaved session", function()
    env.QuestieTraceCharacter = {
      sessions = { { name = "saved", functions = {} } },
      currentSession = {
        name = "unsaved",
        functions = {},
        events = { { t = 0, e = "PLAYER_LOGIN" } },
      },
    }

    local payload = Core.BuildExportPayload()

    assert.equal(2, #payload.sessions)
    assert.equal("saved", payload.sessions[1].name)
    assert.equal("unsaved", payload.sessions[2].name)
  end)

  it("should not include the current session if it has no events", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = { name = "unsaved", functions = {}, events = {} },
    }

    local payload = Core.BuildExportPayload()

    assert.equal(0, #payload.sessions)
  end)

  it("should not mutate the original currentSession", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = {
        name = "unsaved",
        functions = { UnitGUID = { player = { { t = 0, tp = 0, v = "Player-1-000001" } } } },
        events = { { t = 0, e = "PLAYER_LOGIN" } },
      },
    }

    Core.BuildExportPayload()

    assert.is_not_nil(env.QuestieTraceCharacter.currentSession.functions.UnitGUID.player)
  end)
end)

describe("Export.BuildExportPayload", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportModule(env)
  end)

  it("should set hasExportableData to false when there is no data", function()
    env.QuestieTraceCharacter = { sessions = {} }

    local payload = Core.BuildExportPayload()

    assert.is_false(payload.hasExportableData)
    assert.equal(0, #payload.sessions)
  end)

  it("should set hasExportableData to true when there are saved sessions", function()
    env.QuestieTraceCharacter = { sessions = { { functions = {} } } }

    local payload = Core.BuildExportPayload()

    assert.is_true(payload.hasExportableData)
    assert.equal(1, #payload.sessions)
  end)

  it("should set hasExportableData to true when current session has events", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = { functions = {}, events = { { t = 0, e = "PLAYER_LOGIN" } } },
    }

    local payload = Core.BuildExportPayload()

    assert.is_true(payload.hasExportableData)
    assert.equal(1, #payload.sessions)
  end)

  it("should set hasExportableData to false when current session has no events", function()
    env.QuestieTraceCharacter = {
      sessions = {},
      currentSession = { functions = {}, events = {} },
    }

    local payload = Core.BuildExportPayload()

    assert.is_false(payload.hasExportableData)
    assert.equal(0, #payload.sessions)
  end)
end)
