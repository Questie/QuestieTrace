-- Run from the addon root with: busted -p ".test.lua" .
-- Loads decoder.lua in an isolated env with _TEST set so it exposes its
-- internal functions instead of running the CLI (and calling os.exit).

local LibDeflate = dofile("tools/decoder/dependencies/LibDeflate.lua")
local BlizzardCBOR = dofile("tools/decoder/dependencies/BlizzardCBOR.lua")

---@return table decoder
local function LoadDecoder()
  local env = {}
  setmetatable(env, { __index = _G })
  env._G = env
  env._TEST = true
  env.arg = { [0] = "tools/decoder/decoder.lua" }

  local chunk = assert(loadfile("tools/decoder/decoder.lua"))
  setfenv(chunk, env)
  return chunk()
end

--- Encodes a payload the same way QuestieTrace's export pipeline does
--- (minus the print-safe prefix/suffix wrapping), so tests can build
--- valid input for the decoder without needing real export fixtures.
---@param payload any
---@return string encoded
local function EncodePayload(payload)
  local cbor = BlizzardCBOR.SerializeCBOR(payload)
  local compressed = LibDeflate:CompressDeflate(cbor)
  return LibDeflate:EncodeForPrint(compressed)
end

describe("decoder.ExtractEncodedPayload", function()
  local Decoder

  before_each(function()
    Decoder = LoadDecoder()
  end)

  it("should extract the payload wrapped in the QuestieTrace export markers", function()
    local encoded, err = Decoder.ExtractEncodedPayload("!QuestieTrace:1!ABCDEF!End:QuestieTrace:1!")

    assert.is_nil(err)
    assert.equal("ABCDEF", encoded)
  end)

  it("should error when the string is not wrapped in export markers", function()
    local encoded, err = Decoder.ExtractEncodedPayload("  ABCDEF  \n")

    assert.is_nil(encoded)
    assert.is_not_nil(err)
  end)

  it("should error when the markers are present but payload is empty", function()
    local encoded, err = Decoder.ExtractEncodedPayload("!QuestieTrace:1!!End:QuestieTrace:1!")

    assert.is_nil(encoded)
    assert.is_not_nil(err)
  end)

  it("should error when the file has no payload content", function()
    local encoded, err = Decoder.ExtractEncodedPayload("   \n  ")

    assert.is_nil(encoded)
    assert.is_not_nil(err)
  end)
end)

describe("decoder.HasTxtExtension", function()
  local Decoder

  before_each(function()
    Decoder = LoadDecoder()
  end)

  it("should accept a path ending in .txt", function()
    assert.is_true(Decoder.HasTxtExtension("export.txt"))
  end)

  it("should reject a path with a different extension", function()
    assert.is_false(Decoder.HasTxtExtension("export.lua"))
  end)

  it("should reject a path with no extension", function()
    assert.is_false(Decoder.HasTxtExtension("export"))
  end)

  it("should reject a path where .txt is not the final extension", function()
    assert.is_false(Decoder.HasTxtExtension("export.txt.bak"))
  end)
end)

describe("decoder.ReplaceExtensionWithLua", function()
  local Decoder

  before_each(function()
    Decoder = LoadDecoder()
  end)

  it("should replace a simple extension with .lua", function()
    assert.equal("foo.lua", Decoder.ReplaceExtensionWithLua("foo.txt"))
  end)

  it("should only replace the last extension", function()
    assert.equal("foo.tar.lua", Decoder.ReplaceExtensionWithLua("foo.tar.gz"))
  end)

  it("should append .lua when the file has no extension", function()
    assert.equal("foo.lua", Decoder.ReplaceExtensionWithLua("foo"))
  end)
end)

describe("decoder.Serialize", function()
  local Decoder

  before_each(function()
    Decoder = LoadDecoder()
  end)

  ---@param value any
  ---@return any
  local function RoundTrip(value)
    local serialized = "return " .. Decoder.Serialize(value, "")
    local chunk = assert(loadstring(serialized))
    return chunk()
  end

  it("should round-trip a table with string and number values", function()
    local result = RoundTrip({ name = "hero", level = 5 })

    assert.equal("hero", result.name)
    assert.equal(5, result.level)
  end)

  it("should round-trip nested tables", function()
    local result = RoundTrip({ outer = { inner = "value" } })

    assert.equal("value", result.outer.inner)
  end)

  it("should round-trip array-style numeric keys", function()
    local result = RoundTrip({ "a", "b", "c" })

    assert.equal("a", result[1])
    assert.equal("c", result[3])
  end)

  it("should escape special characters in strings", function()
    local result = RoundTrip({ text = 'has "quotes" and \n a newline' })

    assert.equal('has "quotes" and \n a newline', result.text)
  end)

  it("should round-trip booleans", function()
    local result = RoundTrip({ flag = true, other = false })

    assert.is_true(result.flag)
    assert.is_false(result.other)
  end)
end)

describe("decoder.DecodePayload", function()
  local Decoder

  before_each(function()
    Decoder = LoadDecoder()
  end)

  it("should decode a validly encoded table payload", function()
    local encoded = EncodePayload({ sessions = { { name = "test" } } })

    local payload, err = Decoder.DecodePayload(encoded)

    assert.is_nil(err)
    assert.equal("test", payload.sessions[1].name)
  end)

  it("should error when the encoded string is not valid print-safe data", function()
    local payload, err = Decoder.DecodePayload("not a valid encoded payload!!!")

    assert.is_nil(payload)
    assert.is_not_nil(err)
  end)

  it("should error when the decoded CBOR root is not a table", function()
    local encoded = EncodePayload(42)

    local payload, err = Decoder.DecodePayload(encoded)

    assert.is_nil(payload)
    assert.matches("not a table", err)
  end)
end)

describe("decoder.DecodeFile", function()
  local Decoder
  local tmpPath

  before_each(function()
    Decoder = LoadDecoder()
    tmpPath = os.tmpname()
  end)

  after_each(function()
    os.remove(tmpPath)
  end)

  ---@param content string
  local function WriteTempFile(content)
    local file = assert(io.open(tmpPath, "w"))
    file:write(content)
    file:close()
  end

  it("should decode a file wrapped in the export markers", function()
    local encoded = EncodePayload({ sessions = {}, hasExportableData = false })
    WriteTempFile("!QuestieTrace:1!" .. encoded .. "!End:QuestieTrace:1!")

    local serialized, err = Decoder.DecodeFile(tmpPath)

    assert.is_nil(err)
    local result = assert(loadstring(serialized))()
    assert.same({ sessions = {}, hasExportableData = false }, result)
  end)

  it("should error when a file contains only the raw payload (no markers)", function()
    local encoded = EncodePayload({ sessions = {} })
    WriteTempFile(encoded)

    local serialized, err = Decoder.DecodeFile(tmpPath)

    assert.is_nil(serialized)
    assert.is_not_nil(err)
  end)

  it("should error when the file does not exist", function()
    local serialized, err = Decoder.DecodeFile("/no/such/file-really.txt")

    assert.is_nil(serialized)
    assert.is_not_nil(err)
  end)

  it("should error when the file is empty", function()
    WriteTempFile("   \n")

    local serialized, err = Decoder.DecodeFile(tmpPath)

    assert.is_nil(serialized)
    assert.matches("empty", err)
  end)
end)

describe("decoder.WriteToFile", function()
  local Decoder
  local tmpPath

  before_each(function()
    Decoder = LoadDecoder()
    tmpPath = os.tmpname()
  end)

  after_each(function()
    os.remove(tmpPath)
  end)

  it("should write the given content to the output path", function()
    local ok, err = Decoder.WriteToFile("return {}", tmpPath)

    assert.is_true(ok)
    assert.is_nil(err)

    local file = assert(io.open(tmpPath, "r"))
    local content = file:read("*a")
    file:close()
    assert.equal("return {}", content)
  end)

  it("should error when the output path cannot be opened for writing", function()
    local ok, err = Decoder.WriteToFile("return {}", "/no/such/directory/output.lua")

    assert.is_false(ok)
    assert.is_not_nil(err)
  end)
end)
