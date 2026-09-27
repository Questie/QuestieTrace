import { describe, expect, it } from "vitest";
import { mergeSpawns, mergeZoneID } from "./spawnsMerge";
import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "./spawns";

function obs(
  entityId: number,
  value: SpawnObservationValue,
  t: number,
  token: string = "npc",
): TaggedSpawnObservation {
  return {
    entityId,
    value,
    confidence: "high",
    provenance: { session: "test", t },
    token,
  };
}

describe("mergeSpawns", () => {
  it("should build zone -> coordinate array mapping", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10),
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 20), // duplicate
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 30),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[30.01, 86.02]],
      12: [[47.46, 62.18]],
    });
  });

  it("should cluster nearby coordinates and average them", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10),
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 20),
      obs(823, { zoneID: 40, x: 30.01, y: 86.03 }, 30), // close enough to cluster
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

  it("should prefer questnpc token over target for same coordinate", () => {
    const observations: TaggedSpawnObservation[] = [
      // Seen via target first (lower priority)
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "target"),
      // Same coordinate via questnpc (higher priority) — should win
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 20, "questnpc"),
    ];

    // questnpc observation wins, keeping just one entry
    expect(mergeSpawns(observations)).toEqual({
      40: [[30.01, 86.02]],
    });
  });

  it("should override ALL coordinates when questnpc exists (even different coords)", () => {
    const observations: TaggedSpawnObservation[] = [
      // Different coordinates via lower-priority tokens
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "target"),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 20, "npc"),
      // questnpc at a different location
      obs(823, { zoneID: 40, x: 50.0, y: 70.0 }, 30, "questnpc"),
    ];

    // Only questnpc coordinates should survive
    expect(mergeSpawns(observations)).toEqual({
      40: [[50.0, 70.0]],
    });
  });

  it("should keep all questnpc coordinates when multiple exist", () => {
    const observations: TaggedSpawnObservation[] = [
      // Multiple questnpc observations at different coords
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "questnpc"),
      obs(823, { zoneID: 40, x: 31.0, y: 87.0 }, 20, "questnpc"),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 30, "questnpc"),
      // Lower priority observations should be discarded
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 40, "target"),
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
      obs(823, { zoneID: 40, x: 21.83, y: 45.3 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 21.83, y: 45.32 }, 2, "questnpc"),
      obs(823, { zoneID: 40, x: 21.83, y: 45.35 }, 3, "questnpc"),
      obs(823, { zoneID: 40, x: 21.84, y: 45.26 }, 4, "questnpc"),
      obs(823, { zoneID: 40, x: 21.84, y: 45.29 }, 5, "questnpc"),
      obs(823, { zoneID: 40, x: 21.84, y: 45.3 }, 6, "questnpc"),
      obs(823, { zoneID: 40, x: 21.84, y: 45.34 }, 7, "questnpc"),
      obs(823, { zoneID: 40, x: 21.84, y: 45.35 }, 8, "questnpc"),
      obs(823, { zoneID: 40, x: 21.86, y: 45.28 }, 9, "questnpc"),
      obs(823, { zoneID: 40, x: 21.86, y: 45.3 }, 10, "questnpc"),
      obs(823, { zoneID: 40, x: 21.87, y: 45.29 }, 11, "questnpc"),
      obs(823, { zoneID: 40, x: 21.87, y: 45.3 }, 12, "questnpc"),
      obs(823, { zoneID: 40, x: 21.88, y: 45.31 }, 13, "questnpc"),
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
      obs(823, { zoneID: 40, x: 30.0, y: 86.0 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 30.02, y: 86.01 }, 2, "questnpc"),
      obs(823, { zoneID: 40, x: 30.04, y: 86.0 }, 3, "questnpc"),
      // Cluster 2: around (47, 62) - far from cluster 1
      obs(823, { zoneID: 40, x: 47.46, y: 62.18 }, 4, "questnpc"),
      obs(823, { zoneID: 40, x: 47.48, y: 62.2 }, 5, "questnpc"),
    ];

    const result = mergeSpawns(observations);
    // Should have 2 separate clusters
    expect(result[40]).toHaveLength(2);
  });

  it("should cluster points exactly at threshold distance", () => {
    const observations: TaggedSpawnObservation[] = [
      // Points exactly 0.05 apart (at threshold) - use simpler values to avoid floating point issues
      obs(823, { zoneID: 40, x: 30.0, y: 86.0 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 30.05, y: 86.0 }, 2, "questnpc"),
    ];

    const result = mergeSpawns(observations);
    // Points exactly 0.05 apart may or may not cluster due to floating point precision.
    // The important thing is that points closer than 0.05 cluster and points farther don't.
    expect(result[40]).toHaveLength(1);
  });

  it("should NOT cluster points just beyond threshold", () => {
    const observations: TaggedSpawnObservation[] = [
      // Points 0.051 apart (just beyond threshold)
      obs(823, { zoneID: 40, x: 30.0, y: 86.0 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 30.051, y: 86.0 }, 2, "questnpc"),
    ];

    const result = mergeSpawns(observations);
    // Should NOT cluster (distance > threshold)
    expect(result[40]).toHaveLength(2);
  });

  it("should prefer questnpc token for zoneID weight", () => {
    const observations: TaggedSpawnObservation[] = [
      // 2 target observations in zone 12
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 1, "target"),
      obs(823, { zoneID: 12, x: 48.0, y: 63.0 }, 2, "target"),
      // 1 questnpc observation in zone 40 (weight 3 > 2)
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 3, "questnpc"),
    ];

    // zone 40 has weight 3, zone 12 has weight 2
    expect(mergeZoneID(observations)).toBe(40);
  });

  it("should return empty object when no observations", () => {
    expect(mergeSpawns([])).toEqual({});
  });

  it("should handle multiple zones with multiple coordinates each", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 1),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 2),
      obs(823, { zoneID: 40, x: 31.0, y: 87.0 }, 3),
      obs(823, { zoneID: 12, x: 48.0, y: 63.0 }, 4),
    ];

    const result = mergeSpawns(observations);
    expect(result[40]).toEqual(expect.arrayContaining([[30.01, 86.02], [31.0, 87.0]]));
    expect(result[12]).toEqual(expect.arrayContaining([[47.46, 62.18], [48.0, 63.0]]));
    expect(result[40]!.length).toBe(2);
    expect(result[12]!.length).toBe(2);
  });
});

describe("mergeZoneID", () => {
  it("should return the most frequently observed zone", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 1),
      obs(823, { zoneID: 40, x: 31.0, y: 87.0 }, 2),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 3),
      obs(823, { zoneID: 40, x: 32.0, y: 88.0 }, 4),
      obs(823, { zoneID: 12, x: 48.0, y: 63.0 }, 5),
    ];

    expect(mergeZoneID(observations)).toBe(40);
  });

  it("should break ties by higher token priority", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 1, "target"),
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 2, "npc"),
    ];

    // Both weight 1 and 2, zone 40 (npc=2) wins over zone 12 (target=1)
    expect(mergeZoneID(observations)).toBe(40);
  });

  it("should break equal weight ties by first encountered (stable)", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 1),
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 2),
    ];

    // Both have count 1, zone 12 encountered first
    expect(mergeZoneID(observations)).toBe(12);
  });
});
