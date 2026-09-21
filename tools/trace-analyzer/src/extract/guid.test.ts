import { describe, expect, it } from "vitest";
import { parseGuid } from "./guid";

describe("parseGuid", () => {
  it("should parse a Creature GUID into kind npc with correct id and spawnUID", () => {
    expect(parseGuid("Creature-0-5208-0-7-823-000031")).toEqual({
      kind: "npc",
      id: 823,
      spawnUID: "000031",
    });
  });

  it("should parse a GameObject GUID into kind object with correct id and spawnUID", () => {
    expect(parseGuid("GameObject-0-5208-0-7-2843-0000399")).toEqual({
      kind: "object",
      id: 2843,
      spawnUID: "0000399",
    });
  });

  it("should parse a Pet GUID as kind npc", () => {
    expect(parseGuid("Pet-0-5208-0-7-42-000001").kind).toBe("npc");
  });

  it("should parse a Vehicle GUID as kind npc", () => {
    expect(parseGuid("Vehicle-0-5208-0-7-42-000001").kind).toBe("npc");
  });

  it("should classify a Player GUID as kind player without an id", () => {
    expect(parseGuid("Player-5284-0362")).toEqual({
      kind: "player",
      id: null,
      spawnUID: null,
    });
  });

  it("should classify an Item GUID as kind item without an id", () => {
    expect(parseGuid("Item-5208-0-0000123456").kind).toBe("item");
  });

  it("should return kind unknown for an unrecognized prefix", () => {
    expect(parseGuid("Corpse-0-5208-0-7-823-000031").kind).toBe("unknown");
  });

  it("should return kind unknown for null, undefined, or empty input", () => {
    expect(parseGuid(null)).toEqual({ kind: "unknown", id: null, spawnUID: null });
    expect(parseGuid(undefined)).toEqual({ kind: "unknown", id: null, spawnUID: null });
    expect(parseGuid("")).toEqual({ kind: "unknown", id: null, spawnUID: null });
  });
});
