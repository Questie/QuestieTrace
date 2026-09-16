#!/usr/bin/env lua
-- Decodes QuestieTrace export string(s) (as produced by Core.BuildExportString)
-- back into Lua tables and prints/saves them.
--
-- Reverses the pipeline in Modules/Export/Encoding.lua:
--   EncodeForPrint -> Deflate decompress -> CBOR deserialize
--
-- Usage:
--   lua decoder.lua                       -- batch: decode every .txt file in
--                                                  ./input/ into ./output/
--   lua decoder.lua <trace-file>           -- decode a single .txt file to stdout
--   lua decoder.lua <trace-file> -o <path> -- decode a single .txt file to <path>
--   lua decoder.lua --help
--
-- See README.md in this directory for details.

local lfs = require("lfs")

local scriptDir = arg[0]:match("^(.*)/[^/]+$") or "."
local defaultInputDir = scriptDir .. "/input"
local defaultOutputDir = scriptDir .. "/output"

local LibDeflate = dofile(scriptDir .. "/dependencies/LibDeflate.lua")
local BlizzardCBOR = dofile(scriptDir .. "/dependencies/BlizzardCBOR.lua")

---@param message string
local function Fail(message)
  io.stderr:write("error: " .. message .. "\n")
  os.exit(1)
end

---@param out file*
local function PrintUsage(out)
  out:write([[
Usage: lua decoder.lua [options] [trace-file]

Decodes QuestieTrace export string(s) into Lua table(s).

With no arguments, runs in batch mode: every .txt file in ./input/ (relative
to this script) is decoded and written as ./output/<basename>.lua. Both
directories are created automatically if they don't exist yet.

Arguments:
  [trace-file]          Optional. Path to a single .txt file containing a
                         QuestieTrace export string. When given, only that
                         file is decoded instead of running batch mode.

Options:
  -o, --output <path>   Only valid together with [trace-file]. Write the
                         decoded Lua table to <path> instead of stdout.
                         <path> may also be a directory, in which case the
                         output is written as <path>/<basename>.lua.
  -h, --help            Show this help message.
]])
end

---@param args string[]
---@return string|nil inputPath
---@return string|nil outputPath
local function ParseArgs(args)
  local inputPath, outputPath
  local i = 1
  while i <= #args do
    local a = args[i]
    if a == "-h" or a == "--help" then
      PrintUsage(io.stdout)
      os.exit(0)
    elseif a == "-o" or a == "--output" then
      outputPath = args[i + 1]
      if not outputPath then
        Fail(a .. " requires a path argument")
      end
      i = i + 1
    elseif a:sub(1, 1) == "-" then
      Fail("unknown option '" .. a .. "'")
    elseif not inputPath then
      inputPath = a
    else
      Fail("unexpected extra argument '" .. a .. "'")
    end
    i = i + 1
  end

  if outputPath and not inputPath then
    Fail("-o/--output requires [trace-file] to also be given")
  end

  return inputPath, outputPath
end

---@param path string
---@return string|nil mode "file", "directory", or nil if it doesn't exist
local function PathMode(path)
  local attrs = lfs.attributes(path)
  return attrs and attrs.mode or nil
end

---@param path string
---@return boolean
local function HasTxtExtension(path)
  return path:match("%.txt$") ~= nil
end

---@param path string
local function EnsureDirectoryExists(path)
  if PathMode(path) == nil then
    local ok, success, mkdirErr = pcall(lfs.mkdir, path)
    if not ok then
      Fail("failed to create directory '" .. path .. "': " .. tostring(success))
    elseif not success then
      Fail("failed to create directory '" .. path .. "': " .. tostring(mkdirErr))
    end
  elseif PathMode(path) ~= "directory" then
    Fail("path '" .. path .. "' exists and is not a directory")
  end
end

---@param path string
---@return string|nil content
---@return string|nil err
local function ReadFile(path)
  local file, openErr = io.open(path, "r")
  if not file then
    return nil, "cannot open input file '" .. path .. "': " .. tostring(openErr)
  end
  local raw = file:read("*a")
  file:close()
  if not raw or raw:match("^%s*$") then
    return nil, "input file '" .. path .. "' is empty"
  end
  return raw, nil
end

---@param raw string
---@return string|nil encoded
---@return string|nil err
local function ExtractEncodedPayload(raw)
  local encoded = raw:match("^%s*!QuestieTrace:%d+!(.-)!End:QuestieTrace:%d+!%s*$")
  if not encoded then
    return nil, "export string must be wrapped in !QuestieTrace:N! and !End:QuestieTrace:N! markers"
  end
  if encoded == "" then
    return nil, "export markers found but payload is empty"
  end
  return encoded, nil
end

---@param encoded string
---@return table|nil payload
---@return string|nil err
local function DecodePayload(encoded)
  local compressed = LibDeflate:DecodeForPrint(encoded)
  if not compressed then
    return nil, "payload is not valid QuestieTrace export data (DecodeForPrint failed)"
  end

  local cbor = LibDeflate:DecompressDeflate(compressed)
  if not cbor then
    return nil, "payload could not be decompressed (DecompressDeflate failed)"
  end

  local ok, payload = pcall(BlizzardCBOR.DeserializeCBOR, cbor)
  if not ok then
    return nil, "payload could not be deserialized as CBOR: " .. tostring(payload)
  end

  if type(payload) ~= "table" then
    return nil, "decoded payload is not a table (got " .. type(payload) .. ")"
  end

  return payload, nil
end

---@param value any
---@param indent string
local function Serialize(value, indent)
  local t = type(value)
  if t == "table" then
    local nextIndent = indent .. "  "
    local parts = { "{\n" }
    for k, v in pairs(value) do
      local key
      if type(k) == "number" then
        key = "[" .. k .. "]"
      else
        key = "[" .. string.format("%q", tostring(k)) .. "]"
      end
      parts[#parts + 1] = nextIndent .. key .. " = " .. Serialize(v, nextIndent) .. ",\n"
    end
    parts[#parts + 1] = indent .. "}"
    return table.concat(parts)
  elseif t == "string" then
    return string.format("%q", value)
  else
    return tostring(value)
  end
end

--- Runs the full decode pipeline for a single input file.
---@param inputPath string
---@return string|nil serialized
---@return string|nil err
local function DecodeFile(inputPath)
  local raw, err = ReadFile(inputPath)
  if not raw then
    return nil, err
  end

  local encoded
  encoded, err = ExtractEncodedPayload(raw)
  if not encoded then
    return nil, err
  end

  local payload
  payload, err = DecodePayload(encoded)
  if not payload then
    return nil, err
  end

  return "return " .. Serialize(payload, ""), nil
end

---@param serialized string
---@param outputPath string
---@return boolean ok
---@return string|nil err
local function WriteToFile(serialized, outputPath)
  local file, openErr = io.open(outputPath, "w")
  if not file then
    return false, "cannot open output file '" .. outputPath .. "': " .. tostring(openErr)
  end

  local writeOk, writeErr = file:write(serialized)
  local closeOk, closeErr = file:close()

  if not writeOk then
    return false, "cannot write output file '" .. outputPath .. "': " .. tostring(writeErr)
  end
  if not closeOk then
    return false, "cannot close output file '" .. outputPath .. "': " .. tostring(closeErr)
  end

  return true, nil
end

---@param basename string
---@return string
local function ReplaceExtensionWithLua(basename)
  return (basename:gsub("%.[^.]*$", "")) .. ".lua"
end

---@param inputPath string
---@param outputPath string|nil
local function RunSingleFile(inputPath, outputPath)
  local serialized, err = DecodeFile(inputPath)
  if not serialized then
    Fail(err)
  end

  if not outputPath then
    print(serialized)
    return
  end

  if PathMode(outputPath) == "directory" then
    local basename = inputPath:match("([^/]+)$") or inputPath
    outputPath = outputPath .. "/" .. ReplaceExtensionWithLua(basename)
  end

  local ok, writeErr = WriteToFile(serialized, outputPath)
  if not ok then
    Fail(writeErr)
  end
end

---@param dirPath string
---@return string[]
local function ListTxtFiles(dirPath)
  local files = {}
  for name in lfs.dir(dirPath) do
    if name ~= "." and name ~= ".." and HasTxtExtension(name) then
      local fullPath = dirPath .. "/" .. name
      if PathMode(fullPath) == "file" then
        files[#files + 1] = name
      end
    end
  end
  table.sort(files)
  return files
end

---@param inputDir string
---@param outputDir string
local function RunBatch(inputDir, outputDir)
  EnsureDirectoryExists(inputDir)
  EnsureDirectoryExists(outputDir)

  local files = ListTxtFiles(inputDir)
  if #files == 0 then
    print("no .txt files found in '" .. inputDir .. "'")
    return
  end

  local succeeded, failed = 0, 0
  local usedOutputPaths = {}
  for _, name in ipairs(files) do
    local inputPath = inputDir .. "/" .. name
    local serialized, err = DecodeFile(inputPath)
    if not serialized then
      io.stderr:write("skip " .. name .. ": " .. err .. "\n")
      failed = failed + 1
    else
      local outputPath = outputDir .. "/" .. ReplaceExtensionWithLua(name)
      local clashingName = usedOutputPaths[outputPath]
      if clashingName then
        io.stderr:write("skip " .. name .. ": output path '" .. outputPath ..
          "' collides with '" .. clashingName .. "'\n")
        failed = failed + 1
      else
        local ok, writeErr = WriteToFile(serialized, outputPath)
        if not ok then
          io.stderr:write("skip " .. name .. ": " .. writeErr .. "\n")
          failed = failed + 1
        else
          usedOutputPaths[outputPath] = name
          succeeded = succeeded + 1
        end
      end
    end
  end

  print(string.format("decoded %d file(s), %d failed", succeeded, failed))
  if failed > 0 then
    os.exit(1)
  end
end

local function Main()
  local inputPath, outputPath = ParseArgs(arg)

  if not inputPath then
    RunBatch(defaultInputDir, defaultOutputDir)
    return
  end

  if PathMode(inputPath) == nil then
    Fail("input path '" .. inputPath .. "' does not exist")
  end

  if not HasTxtExtension(inputPath) then
    Fail("input file '" .. inputPath .. "' must have a .txt extension")
  end

  RunSingleFile(inputPath, outputPath)
end

-- Skip auto-running when loaded by the test suite (which sets _TEST and only
-- wants access to the functions below, not to run the CLI / call os.exit).
-- luacheck: globals _TEST
if not _TEST then
  Main()
  return
end

return {
  ExtractEncodedPayload = ExtractEncodedPayload,
  ReplaceExtensionWithLua = ReplaceExtensionWithLua,
  Serialize = Serialize,
  DecodePayload = DecodePayload,
  DecodeFile = DecodeFile,
  WriteToFile = WriteToFile,
  PathMode = PathMode,
  HasTxtExtension = HasTxtExtension,
}
