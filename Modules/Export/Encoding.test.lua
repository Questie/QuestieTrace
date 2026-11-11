---@param env table<string, any>
---@return QuestieTraceCore
local function LoadEncodingModule(env)
  setmetatable(env, { __index = _G })
  env._G = env
  env.QuestieTraceCore = { Debug = function() end }
  env.LibStub = function()
    return {
      EncodeForPrint = function(_, compressed) return compressed end,
      DecodeForPrint = function(_, encoded) return encoded end,
    }
  end
  env.Enum = { CompressionMethod = { Deflate = 1 }, CompressionLevel = { Default = 1 } }

  local secret = setmetatable({}, {
    __index = function() error("secret value was indexed") end,
    __tostring = function() error("secret value was stringified") end,
    __concat = function() error("secret value was concatenated") end,
  })
  env.secret = secret
  env.issecretvalue = function(value) return value == secret end

  -- Like the real client, refuse to serialize secret values.
  local function Serialize(value)
    if value == secret then
      error("attempted to serialize a secret value")
    end
    if type(value) == "table" then
      for k, v in pairs(value) do
        Serialize(k)
        Serialize(v)
      end
    end
    return "CBOR"
  end
  env.C_EncodingUtil = {
    SerializeCBOR = Serialize,
    DeserializeCBOR = function() end,
    CompressString = function(cbor) return cbor end,
    DecompressString = function() end,
  }

  local chunk = assert(loadfile("Modules/Export/Encoding.lua"))
  setfenv(chunk, env)
  chunk()

  return env.QuestieTraceCore
end

describe("Encoding.EncodeExportPayload", function()
  ---@type table<string, any>
  local env
  ---@type QuestieTraceCore
  local Core

  before_each(function()
    env = {}
    Core = LoadEncodingModule(env)
  end)

  it("should encode a payload without secret values unchanged", function()
    local payload = { sessions = { { events = { { e = "X", a = { 1, 2, n = 2 } } } } } }

    local encoded, err = Core.EncodeExportPayload(payload)

    assert.equal("CBOR", encoded)
    assert.is_nil(err)
    assert.equal(1, payload.sessions[1].events[1].a[1])
  end)

  it("should remove nested secret values and encode the rest", function()
    local payload = { sessions = { { events = { { e = "X", a = { 1, env.secret, 3, n = 3 } } } } } }

    local encoded, err = Core.EncodeExportPayload(payload)

    assert.equal("CBOR", encoded)
    assert.is_nil(err)
    local args = payload.sessions[1].events[1].a
    assert.equal(1, args[1])
    assert.is_nil(args[2])
    assert.equal(3, args[3])
    assert.equal(3, args.n)
  end)

  it("should remove secret table values without indexing them", function()
    local payload = { a = env.secret, b = { c = env.secret, d = "kept" } }

    local encoded = Core.EncodeExportPayload(payload)

    assert.equal("CBOR", encoded)
    assert.is_nil(payload.a)
    assert.is_nil(payload.b.c)
    assert.equal("kept", payload.b.d)
  end)

  it("should remove secret keys", function()
    local payload = { [env.secret] = "value", other = 1 }

    local encoded = Core.EncodeExportPayload(payload)

    assert.equal("CBOR", encoded)
    assert.equal(1, payload.other)
    assert.is_nil(payload[env.secret])
  end)

  it("should report the second error when the retry after removing secrets still fails", function()
    env.C_EncodingUtil.SerializeCBOR = function(value)
      if value.a == env.secret then error("attempted to serialize a secret value") end
      error("depth exceeded")
    end

    local encoded, err = Core.EncodeExportPayload({ a = env.secret })

    assert.is_nil(encoded)
    assert.matches("depth exceeded", err)
  end)

  it("should report at most five removed paths and never stringify secrets", function()
    local reported
    Core.Debug = function(...) reported = { ... } end
    local payload = { events = {} }
    for i = 1, 8 do payload.events[i] = { a = { env.secret } } end

    Core.EncodeExportPayload(payload)

    assert.equal(8, reported[2])
    local paths = reported[4]
    local count = select(2, paths:gsub("payload%.events", ""))
    assert.equal(5, count)
    assert.matches("payload%.events%.1%.a%.1", paths)
  end)

  it("should report the original error when failure is not caused by secret values", function()
    env.C_EncodingUtil.SerializeCBOR = function() error("depth exceeded") end

    local encoded, err = Core.EncodeExportPayload({ a = 1 })

    assert.is_nil(encoded)
    assert.matches("depth exceeded", err)
  end)

  it("should not retry when issecretvalue is unavailable", function()
    env.issecretvalue = nil
    local payload = { a = env.secret }

    local encoded, err = Core.EncodeExportPayload(payload)

    assert.is_nil(encoded)
    assert.matches("secret value", err)
  end)
end)
