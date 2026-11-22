---@class QuestieTraceCore
local Core = QuestieTraceCore

---------------------------------------------------------------------------
-- Encoding (CBOR + compression + print-safe encoding)
---------------------------------------------------------------------------
-- Handles serialization of payload tables into compressed, print-safe strings.
-- Pipeline: Lua table -> CBOR -> Deflate compress -> EncodeForPrint
---------------------------------------------------------------------------

-- LibDeflate for print-safe encoding of compressed binary payloads.
---@type LibDeflate
local LibDeflate = LibStub("LibDeflate", true)

---------------------------------------------------------------------------
-- Encoding support checking
---------------------------------------------------------------------------

--- Check whether the client has the required compression and encoding APIs.
--- Every supported WoW client should have these; if missing, fail loudly.
---@return boolean
function Core.HasCodecSupport()
  local hasBlizzardEncoding = C_EncodingUtil ~= nil
    and C_EncodingUtil.SerializeCBOR ~= nil
    and C_EncodingUtil.DeserializeCBOR ~= nil
    and C_EncodingUtil.CompressString ~= nil
    and C_EncodingUtil.DecompressString ~= nil

  local hasDeflateEnums = Enum ~= nil
    and Enum.CompressionMethod ~= nil
    and Enum.CompressionMethod.Deflate ~= nil
    and Enum.CompressionLevel ~= nil
    and Enum.CompressionLevel.Default ~= nil

  local hasLibDeflate = LibDeflate ~= nil
    and LibDeflate.EncodeForPrint ~= nil
    and LibDeflate.DecodeForPrint ~= nil

  return hasBlizzardEncoding and hasDeflateEnums and hasLibDeflate
end

---------------------------------------------------------------------------
-- Encoding/Decoding
---------------------------------------------------------------------------

--- Bytes of compressed data turned into printable text per step. A multiple
--- of 3, because EncodeForPrint turns every 3 bytes into 4 characters on
--- their own, so encoding in chunks gives exactly the same text.
local PRINT_CHUNK_SIZE = 30000

--- Serialize and compress a payload. Kept separate from EncodeForPrint so
--- that step can yield, which is not possible inside pcall in Lua 5.1.
---@param payload table
---@return boolean ok
---@return any compressedOrError
local function TryCompress(payload)
  return pcall(function()
    local cbor = C_EncodingUtil.SerializeCBOR(payload)
    return C_EncodingUtil.CompressString(cbor, Enum.CompressionMethod.Deflate, Enum.CompressionLevel.Default)
  end)
end

--- LibDeflate:EncodeForPrint, calling `yield` before each chunk if given.
---@param compressed string
---@param yield fun()?
---@return string
local function EncodeForPrint(compressed, yield)
  if (not yield) then
    return LibDeflate:EncodeForPrint(compressed)
  end
  local parts = {}
  for i = 1, #compressed, PRINT_CHUNK_SIZE do
    yield()
    parts[#parts + 1] = LibDeflate:EncodeForPrint(compressed:sub(i, i + PRINT_CHUNK_SIZE - 1))
  end
  return table.concat(parts)
end

--- Maximum number of removed secret value paths to report through Core.Debug.
local MAX_REPORTED_SECRET_PATHS = 5

--- Whether tainted code may traverse `value`. Tables the client protects (e.g.
--- secret tables) raise an error on access; canaccesstable tells them apart.
---@param value table
---@return boolean
local function CanAccessTable(value)
  return type(canaccesstable) ~= "function" or canaccesstable(value)
end

--- Describe where in the payload a secret was found, from the keys leading to it.
--- Only called for removals, where every key in `keyStack` is known not to be
--- secret (stringifying a secret would throw).
---@param keyStack any[]
---@param depth integer
---@param leaf string
---@return string
local function DescribePath(keyStack, depth, leaf)
  local parts = { "payload" }
  for i = 1, depth do
    parts[#parts + 1] = tostring(keyStack[i])
  end
  parts[#parts + 1] = leaf
  return table.concat(parts, ".")
end

--- Remove every secret value (as keys or values) from `value`, in place.
---
--- Why this exists: the client marks some API results and event arguments as
--- "secret values". Tainted addon code may hold them in tables, but
--- C_EncodingUtil.SerializeCBOR refuses to serialize them for tainted callers
--- and throws "attempted to serialize a secret value". A single one anywhere
--- in a payload fails the whole export.
---
--- Secrets are never persisted by SavedVariables, so after a /reload a session
--- no longer contains them. Removing them (rather than substituting a marker)
--- makes an export from a live session match what the same session looks like
--- after a /reload.
---
--- Secret values are never indexed or stringified: touching their contents
--- would raise an error too. Secret tables are removed whole, and so are tables
--- the client reports as inaccessible (canaccesstable), because traversing them
--- would raise an error as well.
---
--- Tables already seen are skipped, so a cyclic payload cannot recurse forever.
--- A cycle is not this function's problem to report: the retried encode fails on it.
---@param value table Must be accessible (see CanAccessTable).
---@param keyStack any[] Keys leading to `value`, for diagnostics.
---@param depth integer Number of valid entries in `keyStack`.
---@param removedPaths string[] Collects up to MAX_REPORTED_SECRET_PATHS removed locations.
---@param visited table<table, true> Tables already traversed.
---@return integer removedCount
local function RemoveSecretValues(value, keyStack, depth, removedPaths, visited)
  visited[value] = true
  local removed = 0
  for key, child in pairs(value) do
    local keyIsSecret = issecretvalue(key)
    local childIsSecret = issecretvalue(child)
    if keyIsSecret or childIsSecret or (type(child) == "table" and (not CanAccessTable(child))) then
      value[key] = nil
      removed = removed + 1
      if #removedPaths < MAX_REPORTED_SECRET_PATHS then
        removedPaths[#removedPaths + 1] = DescribePath(keyStack, depth, keyIsSecret and "<secret key>" or tostring(key))
      end
    elseif type(child) == "table" and (not visited[child]) then
      keyStack[depth + 1] = key
      removed = removed + RemoveSecretValues(child, keyStack, depth + 1, removedPaths, visited)
    end
  end
  return removed
end

--- Encode a payload table into a compressed, print-safe string.
--- Pipeline: Lua table -> CBOR -> Deflate compress -> EncodeForPrint
---
--- If encoding fails and the payload contains secret values, they are removed
--- from `payload` in place and encoding is retried once (see RemoveSecretValues).
---
--- With `yield`, the slow EncodeForPrint step is split into chunks and `yield`
--- is called before each one, so a coroutine can spread it over several frames.
--- The result is the same either way.
---@param payload table
---@param yield fun()? Called before each EncodeForPrint chunk.
---@return string? encodedPayload Nil if codec support is missing or encoding fails.
---@return string? err Failure reason when encoding was attempted but failed. Nil if codec support is missing.
function Core.EncodeExportPayload(payload, yield)
  if (not Core.HasCodecSupport()) then
    return nil
  end

  local ok, compressed = TryCompress(payload)

  if (not ok) and type(issecretvalue) == "function" then
    if (not CanAccessTable(payload)) then
      return nil, "payload table is not accessible"
    end
    local removedPaths = {}
    local sanitized, removed = pcall(RemoveSecretValues, payload, {}, 0, removedPaths, {})
    if (not sanitized) then
      local stringOk, message = pcall(tostring, removed)
      return nil, stringOk and type(message) == "string" and message or "unknown error"
    end
    if removed > 0 then
      Core.Debug("Removed", removed, "secret value(s) before encoding, e.g.", table.concat(removedPaths, ", "))
      ok, compressed = TryCompress(payload)
    end
  end

  if ok and type(compressed) == "string" then
    return EncodeForPrint(compressed, yield)
  end

  if (not ok) then
    local stringOk, message = pcall(tostring, compressed)
    if stringOk and type(message) == "string" then
      return nil, message
    end
    return nil, "unknown error"
  end

  return nil, "unexpected result type: " .. type(compressed)
end

--- Decode a string built by Core.EncodeExportPayload back into a table.
---@param encoded string
---@return table? payload Nil if codec support is missing or decoding fails.
function Core.DecodeExportPayload(encoded)
  if (not Core.HasCodecSupport()) then
    return nil
  end

  local ok, decoded = pcall(function()
    local compressed = LibDeflate:DecodeForPrint(encoded)
    local cbor = C_EncodingUtil.DecompressString(compressed, Enum.CompressionMethod.Deflate)
    return C_EncodingUtil.DeserializeCBOR(cbor)
  end)

  if ok and type(decoded) == "table" then
    return decoded
  end

  return nil
end
