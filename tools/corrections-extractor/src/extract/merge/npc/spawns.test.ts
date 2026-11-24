import { describe, expect, it } from "vitest";
import { mergeSpawns, mergeZoneID } from "./spawns";
import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "../../observers/npc/spawns";

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

  it("should cluster nearby coordinates into their median", () => {
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
    // The median should be rounded to 2 decimal places: (21.84, 45.3)
    expect(result[40]![0][0]).toBe(21.84);
    expect(result[40]![0][1]).toBe(45.3);
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

  it("should collapse observations around a static entity into one spawn", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 267, x: 62.52, y: 19.74 }, 1, "target"),
      obs(823, { zoneID: 267, x: 62.57, y: 19.9 }, 2, "target"),
      obs(823, { zoneID: 267, x: 62.59, y: 19.75 }, 3, "target"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      267: [[62.57, 19.75]],
    });
  });

  it("should keep only the most observed cluster per zone for quest entities", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 1, "npc"),
      obs(823, { zoneID: 40, x: 30.0, y: 60.0 }, 2, "npc"),
      obs(823, { zoneID: 40, x: 30.2, y: 60.0 }, 3, "npc"),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 4, "npc"),
    ];

    expect(mergeSpawns(observations, true)).toEqual({
      40: [[30.1, 60]],
      12: [[47.46, 62.18]],
    });
  });

  it("should keep only the most observed questnpc cluster per zone for quest entities", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 10.2, y: 50.0 }, 2, "questnpc"),
      obs(823, { zoneID: 40, x: 30.0, y: 60.0 }, 3, "questnpc"),
    ];

    expect(mergeSpawns(observations, true)).toEqual({
      40: [[10.1, 50]],
    });
  });

  it("should count repeated observations of the same spot when picking the quest entity spawn", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 1, "npc"),
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 2, "npc"),
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 3, "npc"),
      obs(823, { zoneID: 40, x: 30.0, y: 60.0 }, 4, "npc"),
      obs(823, { zoneID: 40, x: 30.2, y: 60.0 }, 5, "npc"),
    ];

    expect(mergeSpawns(observations, true)).toEqual({
      40: [[10, 50]],
    });
  });

  it("should keep every cluster for entities that are not quest entities", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 1, "target"),
      obs(823, { zoneID: 40, x: 30.0, y: 60.0 }, 2, "target"),
    ];

    expect(mergeSpawns(observations, false)).toEqual({
      40: [[10, 50], [30, 60]],
    });
  });

  it("should merge clusters whose centroids end up within the threshold", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 10.0, y: 50.0 }, 1, "npc"),
      obs(823, { zoneID: 40, x: 10.3, y: 50.45 }, 2, "npc"),
      obs(823, { zoneID: 40, x: 10.31, y: 50.0 }, 3, "npc"),
      obs(823, { zoneID: 40, x: 10.32, y: 50.4 }, 4, "npc"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[10.31, 50.2]],
    });
  });

  it("should group the same map distance more widely in a small zone than in a large one", () => {
    const spread = (zoneID: number): TaggedSpawnObservation[] => [
      obs(823, { zoneID, x: 50.0, y: 50.0 }, 1, "target"),
      obs(823, { zoneID, x: 50.8, y: 50.0 }, 2, "target"),
    ];

    // Ironforge is ~8 yards per map unit horizontally, the Barrens ~100
    expect(mergeSpawns(spread(1537))).toEqual({ 1537: [[50.4, 50]] });
    expect(mergeSpawns(spread(17))[17]).toHaveLength(2);
  });

  it("should measure the grouping radius per axis", () => {
    const apart = (dx: number, dy: number): TaggedSpawnObservation[] => [
      obs(823, { zoneID: 17, x: 50.0, y: 50.0 }, 1, "target"),
      obs(823, { zoneID: 17, x: 50.0 + dx, y: 50.0 + dy }, 2, "target"),
    ];

    // The Barrens are ~101 yards per unit horizontally and ~68 vertically: 0.3 units is 30 or 20 yards
    expect(mergeSpawns(apart(0.3, 0))[17]).toHaveLength(1);
    expect(mergeSpawns(apart(0.3, 0))[17]).toEqual([[50.15, 50]]);
    expect(mergeSpawns(apart(0.8, 0))[17]).toHaveLength(2);
    expect(mergeSpawns(apart(0, 0.8))[17]).toHaveLength(2);
  });

  it("should use 40 yards per map unit for zones of unknown size", () => {
    const apart = (dx: number): TaggedSpawnObservation[] => [
      obs(823, { zoneID: 99999, x: 50.0, y: 50.0 }, 1, "target"),
      obs(823, { zoneID: 99999, x: 50.0 + dx, y: 50.0 }, 2, "target"),
    ];

    expect(mergeSpawns(apart(0.5))[99999]).toHaveLength(1);
    expect(mergeSpawns(apart(1.3))[99999]).toHaveLength(2);
  });

  it("should not chain a line of points longer than the radius into a single cluster", () => {
    // Zone 40 is 35 yards per unit horizontally: six points 17.5 yards apart span 87.5 yards
    const observations: TaggedSpawnObservation[] = [10.0, 10.5, 11.0, 11.5, 12.0, 12.5].map((x, i) =>
      obs(823, { zoneID: 40, x, y: 50.0 }, i, "target"),
    );

    const spawns = mergeSpawns(observations)[40]!;
    expect(spawns).toHaveLength(2);
    for (const [x] of observations.map((o) => [o.value.x])) {
      expect(Math.min(...spawns.map(([sx]) => Math.abs(sx - x) * 35))).toBeLessThanOrEqual(20);
    }
  });

  it("should give the same spawns whatever order the observations come in", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 60; i++) {
      observations.push({
        ...obs(823, { zoneID: 40, x: 20 + (i % 10) * 0.4, y: 40 + Math.floor(i / 10) * 0.4 }, i, "target"),
        provenance: { session: "s", sessionKey: `k${i}`, t: i },
      });
    }

    expect(mergeSpawns([...observations].reverse())).toEqual(mergeSpawns(observations));
  });

  it("should cluster two points that are within the radius of their shared centre", () => {
    // 0.9 units are 31.5 yards in zone 40, so each point is 15.75 yards from the middle
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.0, y: 86.0 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 30.9, y: 86.0 }, 2, "questnpc"),
    ];

    expect(mergeSpawns(observations)[40]).toEqual([[30.45, 86]]);
  });

  it("should NOT cluster two points that are further apart than twice the radius", () => {
    // 1.3 units are 45.5 yards in zone 40
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.0, y: 86.0 }, 1, "questnpc"),
      obs(823, { zoneID: 40, x: 31.3, y: 86.0 }, 2, "questnpc"),
    ];

    expect(mergeSpawns(observations)[40]).toHaveLength(2);
  });

  it("should round singleton coordinates to 2 decimal places (questnpc branch)", () => {
    const observations: TaggedSpawnObservation[] = [
      // Single 3-decimal coordinate must be rounded like a cluster centroid
      obs(823, { zoneID: 40, x: 30.123, y: 86.456 }, 1, "questnpc"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[30.12, 86.46]],
    });
  });

  it("should round singleton coordinates to 2 decimal places (fallback branch)", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 47.123, y: 62.456 }, 1, "npc"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[47.12, 62.46]],
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

  it("should drop a single-session cluster when the strongest cluster has many sessions", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 25; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 30, y: 30 }, i, "target"), provenance: { session: "s", sessionKey: `k${i}`, t: i } });
    }
    observations.push({ ...obs(823, { zoneID: 40, x: 60, y: 60 }, 99, "target"), provenance: { session: "s", sessionKey: "stray", t: 99 } });

    expect(mergeSpawns(observations)).toEqual({ 40: [[30, 30]] });
  });

  it("should keep a cluster seen in two sessions", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 25; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 30, y: 30 }, i, "target"), provenance: { session: "s", sessionKey: `k${i}`, t: i } });
    }
    observations.push({ ...obs(823, { zoneID: 12, x: 60, y: 60 }, 98, "target"), provenance: { session: "s", sessionKey: "a", t: 98 } });
    observations.push({ ...obs(823, { zoneID: 12, x: 60, y: 60 }, 99, "target"), provenance: { session: "s", sessionKey: "b", t: 99 } });

    expect(mergeSpawns(observations)).toEqual({ 40: [[30, 30]], 12: [[60, 60]] });
  });

  it("should keep a weak cluster for rarely seen NPCs even next to a stronger one", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 5; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 30, y: 30 }, i, "target"), provenance: { session: "s", sessionKey: `k${i}`, t: i } });
    }
    observations.push({ ...obs(823, { zoneID: 40, x: 60, y: 60 }, 9, "target"), provenance: { session: "s", sessionKey: "other", t: 9 } });

    expect(mergeSpawns(observations)).toEqual({ 40: [[30, 30], [60, 60]] });
  });

  it("should always keep the strongest cluster of a zone", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 40; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 2 + (i % 8) * 12, y: 5 + Math.floor(i / 8) * 18 }, i, "target"), provenance: { session: "s", sessionKey: `k${i}`, t: i } });
    }

    expect(mergeSpawns(observations)[40]).toHaveLength(40);
  });

  it("should drop a cluster below 0.25% of the sessions seen in the zone", () => {
    const sessions = (key: string, count: number, x: number): TaggedSpawnObservation[] =>
      Array.from({ length: count }, (_, i) => ({
        ...obs(823, { zoneID: 40, x, y: 50 }, i, "target"),
        provenance: { session: "s", sessionKey: `${key}${i}`, t: i },
      }));

    // 2,000 sessions: 0.25% is 5 sessions
    expect(mergeSpawns([...sessions("a", 1992, 20), ...sessions("b", 4, 60)])).toEqual({ 40: [[20, 50]] });
    expect(mergeSpawns([...sessions("a", 1992, 20), ...sessions("b", 5, 60)])).toEqual({ 40: [[20, 50], [60, 50]] });
  });

  it("should drop a zone an NPC was only seen in by a stray session once many sessions saw it elsewhere", () => {
    const seen = (zoneID: number, count: number, x: number): TaggedSpawnObservation[] =>
      Array.from({ length: count }, (_, i) => ({
        ...obs(823, { zoneID, x, y: 50 }, i, "target"),
        provenance: { session: "s", sessionKey: `${zoneID}-${i}`, t: i },
      }));

    expect(mergeSpawns([...seen(40, 30, 20), ...seen(12, 1, 60)])).toEqual({ 40: [[20, 50]] });
    expect(mergeSpawns([...seen(40, 3, 20), ...seen(12, 1, 60)])).toEqual({ 40: [[20, 50]], 12: [[60, 50]] });
  });

  it("should only drop single-session clusters once 20 sessions saw the NPC", () => {
    const seen = (key: string, count: number, x: number): TaggedSpawnObservation[] =>
      Array.from({ length: count }, (_, i) => ({
        ...obs(823, { zoneID: 40, x, y: 50 }, i, "target"),
        provenance: { session: "s", sessionKey: `${key}${i}`, t: i },
      }));

    expect(mergeSpawns([...seen("a", 18, 20), ...seen("b", 1, 60)])[40]).toHaveLength(2);
    expect(mergeSpawns([...seen("a", 19, 20), ...seen("b", 1, 60)])[40]).toHaveLength(1);
  });

  it("should leave no kept observation further than the radius from a spawn", () => {
    // 10 sessions only, so no cluster is dropped; zone 40 is 35 x 23.3 yards per unit
    const observations: TaggedSpawnObservation[] = [];
    let seed = 7;
    const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
    for (let i = 0; i < 400; i++) {
      observations.push({
        ...obs(823, { zoneID: 40, x: Math.round((20 + random() * 8) * 1000) / 1000, y: Math.round((40 + random() * 8) * 1000) / 1000 }, i, "target"),
        provenance: { session: "s", sessionKey: `k${i % 10}`, t: i },
      });
    }

    const spawns = mergeSpawns(observations)[40]!;
    for (const { value } of observations) {
      const nearest = Math.min(...spawns.map(([x, y]) => Math.hypot((x - value.x) * 35, (y - value.y) * 23.33)));
      expect(nearest).toBeLessThanOrEqual(20.5);
    }
  });

  it("should not let one session's many positions outweigh the sessions of another cluster", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 25; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 30, y: 30 }, i, "target"), provenance: { session: "s", sessionKey: `k${i}`, t: i } });
    }
    for (let i = 0; i < 50; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 60, y: 60 + i * 0.001 }, 100 + i, "target"), provenance: { session: "s", sessionKey: "one", t: 100 + i } });
    }

    expect(mergeSpawns(observations)).toEqual({ 40: [[30, 30]] });
  });

  it("should pick the single quest entity spawn among well supported clusters", () => {
    const observations: TaggedSpawnObservation[] = [];
    for (let i = 0; i < 25; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 30, y: 30 }, i, "npc"), provenance: { session: "s", sessionKey: `k${i}`, t: i } });
    }
    for (let i = 0; i < 50; i++) {
      observations.push({ ...obs(823, { zoneID: 40, x: 60, y: 60 + i * 0.001 }, 100 + i, "npc"), provenance: { session: "s", sessionKey: "one", t: 100 + i } });
    }

    expect(mergeSpawns(observations, true)).toEqual({ 40: [[30, 30]] });
  });

  it("should keep single-session clusters of rarely seen NPCs", () => {
    const observations: TaggedSpawnObservation[] = [
      { ...obs(823, { zoneID: 40, x: 30, y: 30 }, 1, "target"), provenance: { session: "s", sessionKey: "a", t: 1 } },
      { ...obs(823, { zoneID: 40, x: 60, y: 60 }, 2, "target"), provenance: { session: "s", sessionKey: "b", t: 2 } },
    ];

    expect(mergeSpawns(observations)).toEqual({ 40: [[30, 30], [60, 60]] });
  });

  it("should place a cluster at the median so an outlier does not pull it away", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.0, y: 30.0 }, 1, "target"),
      obs(823, { zoneID: 40, x: 30.1, y: 30.1 }, 2, "target"),
      obs(823, { zoneID: 40, x: 30.2, y: 30.2 }, 3, "target"),
      obs(823, { zoneID: 40, x: 30.4, y: 30.4 }, 4, "target"),
    ];

    expect(mergeSpawns(observations)).toEqual({ 40: [[30.15, 30.15]] });
  });

  it("should return empty object when no observations", () => {
    expect(mergeSpawns([])).toEqual({});
  });

  it("should handle multiple zones with multiple coordinates each", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 1, "target"),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 2, "target"),
      obs(823, { zoneID: 40, x: 34.0, y: 90.0 }, 3, "target"),
      obs(823, { zoneID: 12, x: 52.0, y: 67.0 }, 4, "target"),
    ];

    const result = mergeSpawns(observations);
    expect(result[40]).toEqual(expect.arrayContaining([[30.01, 86.02], [34.0, 90.0]]));
    expect(result[12]).toEqual(expect.arrayContaining([[47.46, 62.18], [52.0, 67.0]]));
    expect(result[40]!.length).toBe(2);
    expect(result[12]!.length).toBe(2);
  });

  it("should reduce an interacted npc to a single spawn per zone at its interaction positions", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 50.0, y: 50.0 }, 1, "npc"),
      obs(823, { zoneID: 40, x: 50.2, y: 50.0 }, 2, "npc"),
      obs(823, { zoneID: 40, x: 52.0, y: 50.0 }, 3, "npc"),
      obs(823, { zoneID: 40, x: 58.0, y: 45.0 }, 4, "target"),
      obs(823, { zoneID: 40, x: 58.0, y: 45.0 }, 5, "target"),
      obs(823, { zoneID: 40, x: 58.0, y: 45.0 }, 6, "target"),
      obs(823, { zoneID: 12, x: 20.0, y: 20.0 }, 7, "npc"),
    ];

    expect(mergeSpawns(observations)).toEqual({ 40: [[50.1, 50]], 12: [[20, 20]] });
  });

  it("should not let ignored targets add support to an interaction cluster", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 50.0, y: 50.0 }, 1, "npc"),
      obs(823, { zoneID: 40, x: 50.0, y: 50.0 }, 2, "npc"),
      obs(823, { zoneID: 40, x: 60.0, y: 60.0 }, 3, "npc"),
      obs(823, { zoneID: 40, x: 60.0, y: 60.0 }, 4, "target"),
      obs(823, { zoneID: 40, x: 60.0, y: 60.0 }, 5, "target"),
      obs(823, { zoneID: 40, x: 60.0, y: 60.0 }, 6, "target"),
    ];

    expect(mergeSpawns(observations)).toEqual({ 40: [[50, 50]] });
  });

  it("should reduce an interacted npc's targets in a zone without interactions to a single spawn", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 50.0, y: 50.0 }, 1, "npc"),
      obs(823, { zoneID: 12, x: 20.0, y: 20.0 }, 2, "target"),
      obs(823, { zoneID: 12, x: 20.1, y: 20.0 }, 3, "target"),
      obs(823, { zoneID: 12, x: 30.0, y: 30.0 }, 4, "target"),
    ];

    expect(mergeSpawns(observations)).toEqual({ 40: [[50, 50]], 12: [[20.05, 20]] });
  });

  it("should override questnpc and other coordinates when questpoi exists", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 40, x: 30.01, y: 86.02 }, 10, "target"),
      obs(823, { zoneID: 12, x: 47.46, y: 62.18 }, 20, "questnpc"),
      obs(823, { zoneID: 40, x: 50.0, y: 70.0 }, 30, "questpoi"),
    ];

    expect(mergeSpawns(observations)).toEqual({
      40: [[50.0, 70.0]],
    });
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

  it("should only count questpoi zones when questpoi exists", () => {
    const observations: TaggedSpawnObservation[] = [
      obs(823, { zoneID: 12, x: 1, y: 1 }, 10, "questnpc"),
      obs(823, { zoneID: 12, x: 2, y: 2 }, 20, "questnpc"),
      obs(823, { zoneID: 40, x: 50.0, y: 70.0 }, 30, "questpoi"),
    ];

    expect(mergeZoneID(observations)).toBe(40);
  });
});
