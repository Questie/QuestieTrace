import { describe, expect, it } from "vitest";
import { areaIdForUiMapId } from "./ui-map-to-area";

// Values verified against the UiMap/AreaTable CSVs in this directory:
//   UiMap    1429 "Elwynn Forest" -> AreaTable 12
//   UiMap    1436 "Westfall"      -> AreaTable 40
//   UiMap    1411 "Durotar"       -> AreaTable 14
// Continents and the world map have no AreaTable row and must not map.
describe("areaIdForUiMapId", () => {
  it("should map zone uiMapIDs to their AreaTable areaIDs", () => {
    expect(areaIdForUiMapId(1429)).toBe(12);
    expect(areaIdForUiMapId(1436)).toBe(40);
    expect(areaIdForUiMapId(1411)).toBe(14);
  });

  it("should return null for ids with no area (continents, world map, unknown)", () => {
    expect(areaIdForUiMapId(947)).toBeNull(); // Azeroth (world map)
    expect(areaIdForUiMapId(1414)).toBeNull(); // Kalimdor (continent)
    expect(areaIdForUiMapId(1415)).toBeNull(); // Eastern Kingdoms (continent)
    expect(areaIdForUiMapId(123456)).toBeNull(); // unknown
  });

  it("should return null for non-numeric input", () => {
    expect(areaIdForUiMapId(null)).toBeNull();
    expect(areaIdForUiMapId(undefined)).toBeNull();
    expect(areaIdForUiMapId(Number.NaN)).toBeNull();
  });
});
