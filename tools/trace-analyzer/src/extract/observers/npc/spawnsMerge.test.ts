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

  it("should deduplicate identical coordinates within a zone", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10),
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 20),
      obs(823, { zoneID: 40, x: 30.01, y: 86.03 }, 30), // slightly different
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [
        [30.01, 86.02],
        [30.01, 86.03],
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
