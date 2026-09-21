import { describe, expect, it } from "vitest";
import { parseItemLink } from "./itemLink";

describe("parseItemLink", () => {
  it("should parse a real item link into itemID and name", () => {
    expect(parseItemLink("|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h|r")).toEqual({
      itemID: 750,
      name: "Tough Wolf Meat",
    });
  });

  it("should parse an item link with a multi-word bracketed name", () => {
    expect(parseItemLink("|Hitem:45:0:0:0:0:0:0:0:1:0:0:0:0|h[Squire's Boots]|h|r")).toEqual({
      itemID: 45,
      name: "Squire's Boots",
    });
  });

  it("should return null for a non-item link string", () => {
    expect(parseItemLink("|Hquest:6:1|h[Bounty on Garrick Padfoot]|h|r")).toBeNull();
  });

  it("should return null for null, undefined, or empty input", () => {
    expect(parseItemLink(null)).toBeNull();
    expect(parseItemLink(undefined)).toBeNull();
    expect(parseItemLink("")).toBeNull();
  });
});
