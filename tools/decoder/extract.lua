#!/usr/bin/env lua
-- Extracts "export_string" fields from trace-data submission JSON files and
-- writes each one as a plain .txt file into ./input/, ready to be picked up
-- by decoder.lua's batch mode.
--
-- Usage:
--   lua extract.lua [trace-data-dir]
--
-- Defaults to ../../../trace-data (relative to this script) when no
-- directory is given.

local lfs = require("lfs")

local scriptDir = arg[0]:match("^(.*)/[^/]+$") or "."
local defaultTraceDataDir = scriptDir .. "/../../../trace-data"
local inputDir = scriptDir .. "/input"

---@param message string
local function Fail(message)
  io.stderr:write("error: " .. message .. "\n")
  os.exit(1)
end

---@param path string
---@return string|nil mode "file", "directory", or nil if it doesn't exist
local function PathMode(path)
  local attrs = lfs.attributes(path)
  return attrs and attrs.mode or nil
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

---@param dirPath string
---@return string[]
local function ListJsonFilesRecursive(dirPath)
  local files = {}
  for name in lfs.dir(dirPath) do
    if name ~= "." and name ~= ".." then
      local fullPath = dirPath .. "/" .. name
      local mode = PathMode(fullPath)
      if mode == "directory" then
        for _, nested in ipairs(ListJsonFilesRecursive(fullPath)) do
          files[#files + 1] = nested
        end
      elseif mode == "file" and name:match("%.json$") then
        files[#files + 1] = fullPath
      end
    end
  end
  return files
end

---@param path string
---@return string|nil content
local function ReadFile(path)
  local file = io.open(path, "r")
  if not file then
    return nil
  end
  local raw = file:read("*a")
  file:close()
  return raw
end

--- Pulls a top-level "field": "value" string out of a flat JSON object.
-- Safe here because both "id" and "export_string" values only ever contain
-- characters from [A-Za-z0-9():!], never quotes, backslashes, or control
-- characters that would require proper JSON unescaping.
---@param raw string
---@param field string
---@return string|nil value
local function ExtractJsonStringField(raw, field)
  return raw:match('"' .. field .. '"%s*:%s*"([^"]*)"')
end

---@param serialized string
---@param outputPath string
local function WriteToFile(serialized, outputPath)
  local file, openErr = io.open(outputPath, "w")
  if not file then
    Fail("cannot open output file '" .. outputPath .. "': " .. tostring(openErr))
  end
  file:write(serialized)
  file:close()
end

local function Main()
  local traceDataDir = arg[1] or defaultTraceDataDir

  if PathMode(traceDataDir) ~= "directory" then
    Fail("trace-data directory not found: '" .. traceDataDir .. "'")
  end

  EnsureDirectoryExists(inputDir)

  local jsonFiles = ListJsonFilesRecursive(traceDataDir)
  table.sort(jsonFiles)

  local written, skipped = 0, 0
  for _, jsonPath in ipairs(jsonFiles) do
    local raw = ReadFile(jsonPath)
    if not raw then
      io.stderr:write("warn: cannot read '" .. jsonPath .. "'\n")
      skipped = skipped + 1
    else
      local exportString = ExtractJsonStringField(raw, "export_string")
      if not exportString or exportString == "" then
        io.stderr:write("warn: no export_string in '" .. jsonPath .. "'\n")
        skipped = skipped + 1
      else
        local id = ExtractJsonStringField(raw, "id")
          or jsonPath:match("([^/]+)%.json$")
        WriteToFile(exportString, inputDir .. "/" .. id .. ".txt")
        written = written + 1
      end
    end
  end

  print(string.format("wrote %d file(s) to '%s', skipped %d", written, inputDir, skipped))
end

Main()
