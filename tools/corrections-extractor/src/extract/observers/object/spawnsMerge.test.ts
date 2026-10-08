import { describe, expect, it } from "vitest";
import { mergeSpawns, mergeZoneID, type TaggedSpawnObservation } from "./spawnsMerge";
import type { SpawnObservationValue } from "./spawns";

function obs(
  entityId: number,
  value: SpawnObservationValue,
  t: number,
  token: string = "target",
): TaggedSpawnObservation {
  return {
    entityId,
    value,
    confidence: "high",
    provenance: { session: "test", t },
    token,
  };
}

describe("mergeSpawns (object)", () => {
  it("should build zone -> coordinate array mapping", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 10),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 20),
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 30),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[30.01, 86.02]],
      12: [[47.46, 62.18]],
    });
  });

  it("should cluster nearby coordinates and average them", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 10),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 20),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.03 }, 30),
    ];

    // These coordinates are within threshold and get clustered/averaged
    // The two duplicates of (30.01, 86.02) are deduplicated first (4 decimal places)
    // Then (86.02 + 86.03) / 2 = 86.025, which rounds to 86.03
    expect(mergeSpawns(observations)).toEqual({
      40: [
        [30.01, 86.03], // rounded to 2 decimal places
      ],
    });
  });

  it("should return empty object when no observations", () => {
    expect(mergeSpawns([])).toEqual({});
  });

  it("should prefer questnpc token over target for same coordinate", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "target"),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 20, "questnpc"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[30.01, 86.02]],
    });
  });

  it("should override ALL coordinates when questnpc exists (even different coords)", () => {
    const observations: TaggedSpawnObservation[] = [
      // Different coordinates via lower-priority tokens
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "target"),
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 20, "npc"),
      // questnpc at a different location
      obs(2843, { zoneID: 40, x: 50.0, y: 70.0 }, 30, "questnpc"),
    ];

    // Only questnpc coordinates should survive
    expect(mergeSpawns(observations)).toEqual({
      40: [[50.0, 70.0]],
    });
  });

  it("should keep all questnpc coordinates when multiple exist", () => {
    const observations: TaggedSpawnObservation[] = [
      // Multiple questnpc observations at different coords
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "questnpc"),
      obs(2843, { zoneID: 40, x: 31.0, y: 87.0 }, 20, "questnpc"),
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 30, "questnpc"),
      // Lower priority observations should be discarded
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 40, "target"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [
        [30.01, 86.02],
        [31.0, 87.0],
      ],
      12: [[47.46, 62.18]],
    });
  });

  it("should cluster nearby coordinates and average them", () => {
    const observations: TaggedSpawnObservation[] = [
      // Multiple observations very close together (should cluster into one)
      obs(2843, { zoneID: 40, x: 21.83, y: 45.3 }, 1, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.83, y: 45.32 }, 2, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.83, y: 45.35 }, 3, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.84, y: 45.26 }, 4, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.84, y: 45.29 }, 5, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.84, y: 45.3 }, 6, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.84, y: 45.34 }, 7, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.84, y: 45.35 }, 8, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.86, y: 45.28 }, 9, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.86, y: 45.3 }, 10, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.87, y: 45.29 }, 11, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.87, y: 45.3 }, 12, "questnpc"),
      obs(2843, { zoneID: 40, x: 21.88, y: 45.31 }, 13, "questnpc"),
    ];

    const result = mergeSpawns(observations);
    // All these points should cluster into a single spawn point
    expect(result[40]).toHaveLength(1);
    // The centroid should be rounded to 2 decimal places: (21.85, 45.31)
    expect(result[40]![0][0]).toBe(21.85);
    expect(result[40]![0][1]).toBe(45.31);
  });

  it("should keep separate clusters when points are far apart", () => {
    const observations: TaggedSpawnObservation[] = [
      // Cluster 1: around (30, 86)
      obs(2843, { zoneID: 40, x: 30.0, y: 86.0 }, 1, "questnpc"),
      obs(2843, { zoneID: 40, x: 30.02, y: 86.01 }, 2, "questnpc"),
      obs(2843, { zoneID: 40, x: 30.04, y: 86.0 }, 3, "questnpc"),
      // Cluster 2: around (47, 62) - far from cluster 1
      obs(2843, { zoneID: 40, x: 47.46, y: 62.18 }, 4, "questnpc"),
      obs(2843, { zoneID: 40, x: 47.48, y: 62.2 }, 5, "questnpc"),
    ];

    const result = mergeSpawns(observations);
    // Should have 2 separate clusters
    expect(result[40]).toHaveLength(2);
  });

  it("should merge clusters when a point is within threshold of multiple clusters", () => {
    const observations: TaggedSpawnObservation[] = [
      // A and B are > 0.05 apart, so they form separate clusters
      obs(2843, { zoneID: 40, x: 10.0, y: 50.0 }, 1, "questnpc"),
      obs(2843, { zoneID: 40, x: 10.01, y: 50.06 }, 2, "questnpc"),
      // C is within 0.05 of both A and B: it must merge the two clusters into one
      obs(2843, { zoneID: 40, x: 10.02, y: 50.03 }, 3, "questnpc"),
    ];

    const result = mergeSpawns(observations);
    // Joining only the first match would leave 2 clusters; merging all matches yields 1
    expect(result[40]).toHaveLength(1);
    // Centroid of (10.00, 50.00), (10.01, 50.06), (10.02, 50.03) rounded to 2 decimals
    expect(result[40]![0]).toEqual([10.01, 50.03]);
  });

  it("should round singleton coordinates to 2 decimal places (questnpc branch)", () => {
    const observations: TaggedSpawnObservation[] = [
      // Single 3-decimal coordinate must be rounded like a cluster centroid
      obs(2843, { zoneID: 40, x: 30.123, y: 86.456 }, 1, "questnpc"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[30.12, 86.46]],
    });
  });

  it("should round singleton coordinates to 2 decimal places (fallback branch)", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 40, x: 47.123, y: 62.456 }, 1, "target"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[47.12, 62.46]],
    });
  });

  it("should handle multiple zones with multiple coordinates each", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 1),
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 2),
      obs(2843, { zoneID: 40, x: 31.0, y: 87.0 }, 3),
      obs(2843, { zoneID: 12, x: 48.0, y: 63.0 }, 4),
    ];

    const result = mergeSpawns(observations);
    expect(result[40]).toEqual(expect.arrayContaining([[30.01, 86.02], [31.0, 87.0]]));
    expect(result[12]).toEqual(expect.arrayContaining([[47.46, 62.18], [48.0, 63.0]]));
    expect(result[40]!.length).toBe(2);
    expect(result[12]!.length).toBe(2);
  });
});

describe("mergeZoneID (object)", () => {
  it("should return the most frequently observed zone", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 1),
      obs(2843, { zoneID: 40, x: 31.0, y: 87.0 }, 2),
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 3),
      obs(2843, { zoneID: 40, x: 32.0, y: 88.0 }, 4),
      obs(2843, { zoneID: 12, x: 48.0, y: 63.0 }, 5),
    ];

    expect(mergeZoneID(observations)).toBe(40);
  });

  it("should return 0 when no observations", () => {
    expect(mergeZoneID([])).toBe(0);
  });

  it("should break ties by higher token priority", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 1, "target"),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 2, "npc"),
    ];

    expect(mergeZoneID(observations)).toBe(40);
  });

  it("should break equal weight ties by first encountered (stable)", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 1, "target"),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 2, "target"),
    ];

    expect(mergeZoneID(observations)).toBe(12);
  });

  it("should prefer questnpc token for zoneID weight", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(2843, { zoneID: 12, x: 47.46, y: 62.18 }, 1, "target"),
      obs(2843, { zoneID: 12, x: 48.0, y: 63.0 }, 2, "target"),
      obs(2843, { zoneID: 40, x: 30.01, y: 86.02 }, 3, "questnpc"),
    ];

    expect(mergeZoneID(observations)).toBe(40);
  });
});
