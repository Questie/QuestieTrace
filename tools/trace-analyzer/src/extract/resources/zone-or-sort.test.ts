import { describe, expect, it } from "vitest";
import { zoneOrSortIdForName } from "./zone-or-sort";

describe("zoneOrSortIdForName", () => {
  // The lookup must work regardless of which AreaTable/QuestSort build is in
  // the resources directory - these entries exist in every supported build.
  it("should resolve zone names from the available AreaTable CSV", () => {
    expect(zoneOrSortIdForName("Westfall")).toBe(40);
    expect(zoneOrSortIdForName("WESTFALL")).toBe(40);
    expect(zoneOrSortIdForName("  westfall  ")).toBe(40);
  });

  it("should resolve special headers to negative QuestSort IDs", () => {
    expect(zoneOrSortIdForName("Epic")).toBe(-1);
  });

  it("should return null for unknown or non-string input", () => {
    expect(zoneOrSortIdForName("Not a DB2 header")).toBeNull();
    expect(zoneOrSortIdForName(42)).toBeNull();
    expect(zoneOrSortIdForName(null)).toBeNull();
  });
});
