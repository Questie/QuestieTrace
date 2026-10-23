---@type table<string, table>
local encodedSessions = {}

---@param value any
---@return any
local function DeepCopy(value)
  if type(value) ~= "table" then return value end
  local out = {}
  for k, v in pairs(value) do out[k] = DeepCopy(v) end
  return out
end

--- Store a session the way SaveCapture does: as a record with an encoded string.
---@param session table
---@return EncodedSessionRecord
local function Encoded(session)
  local key = tostring(session)
  encodedSessions[key] = session
  return { name = session.name, schemaVersion = 15, eventCount = 0, encoded = key }
end

---@param env table<string, any>
---@return QuestieTraceCore
local function LoadExportModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = { l10n = function(s) return s end }

  -- Mock EncodeExportPayload to return a test string
  env.QuestieTraceCore.EncodeExportPayload = function() return "ENCODED_PAYLOAD" end
  -- Saved sessions built by Encoded() decode to a fresh copy, like the real codec.
  env.QuestieTraceCore.DecodeExportPayload = function(encoded) return DeepCopy(encodedSessions[encoded]) end

  local chunk = assert(loadfile("Modules/Export/Export.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

---@param env table<string, any>
---@param functions table
local function SetSessionFunctions(env, functions)
  env.QuestieTraceCharacter = {
    sessions = { Encoded({ functions = functions }) },
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
      sessions = { Encoded({ name = "saved", functions = {} }) },
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

describe("Export.DeleteReportedSessions", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportModule(env)
    env.QuestieTrace = { settings = { dataCollectionConsent = true } }
  end)

  it("should remove a reported saved session from QuestieTraceCharacter.sessions", function()
    local reported = { name = "reported", functions = {} }
    env.QuestieTraceCharacter = { sessions = { reported } }

    Core.DeleteReportedSessions({ reported })

    assert.equal(0, #env.QuestieTraceCharacter.sessions)
  end)

  it("should keep unrelated saved sessions when only one is reported", function()
    local reported = { name = "reported", functions = {} }
    local kept = { name = "kept", functions = {} }
    env.QuestieTraceCharacter = { sessions = { kept, reported } }

    Core.DeleteReportedSessions({ reported })

    assert.equal(1, #env.QuestieTraceCharacter.sessions)
    assert.equal("kept", env.QuestieTraceCharacter.sessions[1].name)
  end)

  it("should discard and restart the live session when it was reported", function()
    local live = { name = "live", functions = {}, events = { { t = 0, e = "PLAYER_LOGIN" } } }
    env.QuestieTraceCharacter = { sessions = {}, currentSession = live }
    local discardCalls, startCalls = 0, 0
    Core.DiscardCapture = function()
      discardCalls = discardCalls + 1
      env.QuestieTraceCharacter.currentSession = nil
    end
    Core.StartCapture = function() startCalls = startCalls + 1 end

    Core.DeleteReportedSessions({ live })

    assert.equal(1, discardCalls)
    assert.equal(1, startCalls)
  end)

  it("should do nothing when sourceSessions is not a table", function()
    env.QuestieTraceCharacter = { sessions = { { name = "kept", functions = {} } } }

    assert.has_no.errors(function()
      Core.DeleteReportedSessions(nil)
    end)
    assert.equal(1, #env.QuestieTraceCharacter.sessions)
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
    env.QuestieTraceCharacter = { sessions = { Encoded({ functions = {} }) } }

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

---@param count integer
---@return table[] events
local function MakeEvents(count)
  local events = {}
  for i = 1, count do
    events[i] = { t = i, e = "PLAYER_LOGIN" }
  end
  return events
end

describe("Export.BuildExportStats", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadExportModule(env)
  end)

  it("should count events across saved sessions and the current session", function()
    env.QuestieTraceCharacter = {
      sessions = {
        Encoded({ functions = {}, events = MakeEvents(3) }),
        Encoded({ functions = {}, events = MakeEvents(4) }),
      },
      currentSession = { functions = {}, events = MakeEvents(5) },
    }

    local stats = Core.BuildExportStats(Core.BuildExportPayload())

    assert.equal(12, stats.eventCount)
  end)

  it("should count zero events when there is nothing to export", function()
    env.QuestieTraceCharacter = { sessions = {} }

    local stats = Core.BuildExportStats(Core.BuildExportPayload())

    assert.equal(0, stats.eventCount)
  end)

  it("should count zero for a session whose events are missing or not a table", function()
    local payload = {
      sessions = {
        { functions = {} },
        { functions = {}, events = "not a table" },
        { functions = {}, events = MakeEvents(2) },
      },
    }

    assert.equal(2, Core.BuildExportStats(payload).eventCount)
  end)

  it("should not error when the payload has no sessions table", function()
    assert.equal(0, Core.BuildExportStats({}).eventCount)
  end)

  it("should not modify the payload or the export string", function()
    env.QuestieTraceCharacter = {
      sessions = { Encoded({ functions = {}, events = MakeEvents(3) }) },
    }
    --- Encode by listing the payload's top-level keys, so any field added by
    --- BuildExportStats would change the string.
    Core.EncodeExportPayload = function(payload)
      local keys = {}
      for k in pairs(payload) do keys[#keys + 1] = tostring(k) end
      table.sort(keys)
      return table.concat(keys, ",") .. ":" .. #payload.sessions
    end
    local payload = Core.BuildExportPayload()
    local before = DeepCopy(payload)
    local _, stringBefore = Core.BuildExportString(payload)

    Core.BuildExportStats(payload)

    local _, stringAfter = Core.BuildExportString(payload)
    assert.same(before, payload)
    assert.equal(stringBefore, stringAfter)
  end)

  it("should add up time recorded from saved session durations", function()
    local payload = {
      sessions = {
        { functions = {}, duration = 100, events = MakeEvents(1) },
        { functions = {}, duration = 250.5, events = MakeEvents(1) },
      },
    }

    assert.equal(350.5, Core.BuildExportStats(payload).timeRecorded)
  end)

  it("should use the last event time for a live session that has no duration yet", function()
    env.QuestieTraceCharacter = {
      sessions = { Encoded({ functions = {}, duration = 60, events = MakeEvents(2) }) },
      currentSession = { functions = {}, events = { { t = 0, e = "PLAYER_LOGIN" }, { t = 42.5, e = "PLAYER_LOGIN" } } },
    }

    local stats = Core.BuildExportStats(Core.BuildExportPayload())

    assert.equal(102.5, stats.timeRecorded)
  end)

  it("should count zero time for a session with no duration and no events", function()
    local payload = { sessions = { { functions = {} }, { functions = {}, events = {} } } }

    assert.equal(0, Core.BuildExportStats(payload).timeRecorded)
  end)

  it("should count accepted and turned-in quests across sessions", function()
    local payload = {
      sessions = {
        {
          functions = {},
          events = {
            { t = 1, e = "QUEST_ACCEPTED", a = { 101, n = 1 } },
            { t = 2, e = "QUEST_LOG_UPDATE" },
            { t = 3, e = "QUEST_TURNED_IN", a = { 101, 0, 0, n = 3 } },
          },
        },
        {
          functions = {},
          events = {
            { t = 1, e = "QUEST_ACCEPTED", a = { 102, n = 1 } },
            "not a table",
            { t = 2, e = "QUEST_REMOVED", a = { 102, n = 1 } },
          },
        },
      },
    }

    local stats = Core.BuildExportStats(payload)

    assert.equal(2, stats.questsAccepted)
    assert.equal(1, stats.questsTurnedIn)
    assert.equal(6, stats.eventCount)
  end)

  it("should count zero quests when there is nothing to export", function()
    local stats = Core.BuildExportStats({ sessions = {} })

    assert.equal(0, stats.questsAccepted)
    assert.equal(0, stats.questsTurnedIn)
  end)
end)
