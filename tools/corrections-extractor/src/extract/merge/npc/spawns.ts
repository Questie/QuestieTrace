// Custom merge functions for npc spawns and zoneID fields.
//
// spawns: accumulate per-zone coordinate sets from individual observations.
// zoneID: pick the most frequently observed zone from spawn observations.

import type { Observation } from "../../observation";
import type { SpawnObservationValue, TaggedSpawnObservation } from "../../observers/npc/spawns";
import { ZONE_SIZE_YARDS } from "./zone-sizes";

const TOKEN_PRIORITY: Record<string, number> = {
  questpoi: 4,
  questnpc: 3,
  npc: 2,
  target: 1,
};

// Spawn positions are recorded from where the player stood, up to ~28 yards from the unit, so
// positions closer together than this radius (in yards from a cluster's centre) can't be told apart.
// Map units cover very different distances per zone, so the radius is converted per zone and axis.
const SPAWN_CLUSTER_RADIUS_YARDS = 20;
// Zones without a known size use 40 yards per map unit, which makes the radius 0.5 map units.
const FALLBACK_YARDS_PER_UNIT = 40;
// A cluster must be seen in at least this share of the sessions that saw the NPC in the zone, which drops
// stray points from players passing by. Once 20 sessions saw it, single-session clusters are dropped too.
// The strongest cluster is always kept, so NPCs seen only a few times keep all their spawns.
const MIN_CLUSTER_SESSION_SHARE = 0.0025;
const MIN_SESSIONS_TO_DROP_SINGLE_SESSION_CLUSTERS = 20;
// Grid cells per radius, which is also how finely a cluster centre can be placed before it is refined
const CELLS_PER_RADIUS = 10;

function tokenPriority(token: string): number {
  return TOKEN_PRIORITY[token] ?? 0;
}

/** Tokens whose coordinates replace all other sources, in priority order. */
const EXCLUSIVE_TOKENS = ["questpoi", "questnpc"];

function findExclusiveToken(observations: TaggedSpawnObservation[]): string | undefined {
  return EXCLUSIVE_TOKENS.find((token) => observations.some((obs) => obs.token === token));
}

interface WeightedPoint {
  x: number;
  y: number;
  weight: number;
}

interface Cluster {
  x: number;
  y: number;
  members: number[];
}

function weightedMedian(values: Array<{ value: number; weight: number }>): number {
  values.sort((a, b) => a.value - b.value);
  const half = values.reduce((sum, v) => sum + v.weight, 0) / 2;
  let cumulative = 0;
  for (let i = 0; i < values.length; i++) {
    cumulative += values[i].weight;
    if (cumulative > half + 1e-9) return values[i].value;
    if (Math.abs(cumulative - half) <= 1e-9) return (values[i].value + values[i + 1].value) / 2;
  }
  return values[values.length - 1].value;
}

/**
 * Greedy density cover of weighted points (in yards): repeatedly takes the spot with the most
 * uncovered weight within the radius, moves it to the weighted median of the points it covers and
 * removes those, until every point is covered. Unlike sweeping through the points in order, the
 * clusters sit on the densest spots and every point of a cluster is within the radius of its centre.
 */
function coverClusters(points: WeightedPoint[], radius: number): Cluster[] {
  if (points.length === 0) return [];
  const order = points.map((_, i) => i).sort((a, b) => points[a].x - points[b].x || points[a].y - points[b].y);
  const res = radius / CELLS_PER_RADIUS;
  const reach = Math.ceil(radius / res) + 1;
  const offsets: Array<[number, number]> = [];
  for (let dx = -reach; dx <= reach; dx++) {
    for (let dy = -reach; dy <= reach; dy++) {
      if (Math.sqrt(dx * dx + dy * dy) * res <= radius + res) offsets.push([dx, dy]);
    }
  }

  const cellKey = (ix: number, iy: number) => (ix + 100000) * 200000 + (iy + 100000);
  const cellOf = (p: WeightedPoint) => cellKey(Math.floor(p.x / res), Math.floor(p.y / res));

  // Cells holding points
  const occupied = new Map<number, { ix: number; iy: number; points: number[]; weight: number }>();
  for (const i of order) {
    const key = cellOf(points[i]);
    let cell = occupied.get(key);
    if (!cell) {
      cell = { ix: Math.floor(points[i].x / res), iy: Math.floor(points[i].y / res), points: [], weight: 0 };
      occupied.set(key, cell);
    }
    cell.points.push(i);
    cell.weight += points[i].weight;
  }

  // Candidate centres: every cell near a point, scored with the weight of the cells around it. The reach is
  // a cell wider than the radius, so the score never falls short of the weight really within the radius.
  const candidateIndex = new Map<number, number>();
  const candidateX: number[] = [];
  const candidateY: number[] = [];
  const score: number[] = [];
  for (const cell of occupied.values()) {
    for (const [dx, dy] of offsets) {
      const key = cellKey(cell.ix + dx, cell.iy + dy);
      let candidate = candidateIndex.get(key);
      if (candidate === undefined) {
        candidate = score.length;
        candidateIndex.set(key, candidate);
        candidateX.push(cell.ix + dx);
        candidateY.push(cell.iy + dy);
        score.push(0);
      }
      score[candidate] += cell.weight;
    }
  }

  // Max-heap of (upper bound of the weight a candidate covers, candidate). Covered weight only falls as points
  // are removed, so the bound is replaced by the exact weight when a candidate is popped.
  const heap: Array<[number, number]> = [];
  const before = (a: [number, number], b: [number, number]) => a[0] > b[0] || (a[0] === b[0] && a[1] < b[1]);
  const push = (entry: [number, number]) => {
    heap.push(entry);
    let i = heap.length - 1;
    while (i > 0) {
      const parent = (i - 1) >> 1;
      if (!before(heap[i], heap[parent])) break;
      [heap[i], heap[parent]] = [heap[parent], heap[i]];
      i = parent;
    }
  };
  const pop = (): [number, number] => {
    const top = heap[0];
    const last = heap.pop()!;
    if (heap.length > 0) {
      heap[0] = last;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let best = i;
        if (l < heap.length && before(heap[l], heap[best])) best = l;
        if (r < heap.length && before(heap[r], heap[best])) best = r;
        if (best === i) break;
        [heap[i], heap[best]] = [heap[best], heap[i]];
        i = best;
      }
    }
    return top;
  };
  for (let c = 0; c < score.length; c++) push([score[c], c]);

  const alive = new Uint8Array(points.length).fill(1);
  const alivePointsNear = (x: number, y: number): number[] => {
    const ix = Math.floor(x / res);
    const iy = Math.floor(y / res);
    const found: number[] = [];
    for (let dx = -reach; dx <= reach; dx++) {
      for (let dy = -reach; dy <= reach; dy++) {
        const cell = occupied.get(cellKey(ix + dx, iy + dy));
        if (!cell) continue;
        for (const i of cell.points) {
          if (alive[i] && Math.hypot(points[i].x - x, points[i].y - y) <= radius + 1e-9) found.push(i);
        }
      }
    }
    return found.sort((a, b) => a - b);
  };

  const clusters: Cluster[] = [];
  let uncovered = points.length;
  while (uncovered > 0 && heap.length > 0) {
    const [bound, candidate] = pop();
    let x = (candidateX[candidate] + 0.5) * res;
    let y = (candidateY[candidate] + 0.5) * res;
    let members = alivePointsNear(x, y);
    const exact = members.reduce((sum, i) => sum + points[i].weight, 0);
    if (exact <= 0) continue;
    if (exact < bound) {
      push([exact, candidate]);
      continue;
    }

    for (let round = 0; round < 3 && members.length > 0; round++) {
      x = weightedMedian(members.map((i) => ({ value: points[i].x, weight: points[i].weight })));
      y = weightedMedian(members.map((i) => ({ value: points[i].y, weight: points[i].weight })));
      members = alivePointsNear(x, y);
    }
    if (members.length === 0) continue;
    for (const i of members) alive[i] = 0;
    uncovered -= members.length;
    clusters.push({ x, y, members });
  }
  return clusters;
}

/**
 * Clusters each zone's distinct coordinates (weighted by the sessions that saw them) and drops
 * weakly supported clusters. With `singleSpawnPerZone`, only the remaining cluster with the most
 * observations is kept per zone.
 */
function clusterByZone(
  byZone: Map<number, Array<[number, number]>>,
  observationCounts: Map<string, number>,
  coordSessions: Map<string, Set<string>>,
  singleSpawnPerZone: boolean,
): Record<number, Array<[number, number]>> {
  const result: Record<number, Array<[number, number]>> = {};
  for (const [zoneID, coords] of byZone) {
    const size = ZONE_SIZE_YARDS[zoneID];
    const yardsPerX = size ? size[0] / 100 : FALLBACK_YARDS_PER_UNIT;
    const yardsPerY = size ? size[1] / 100 : FALLBACK_YARDS_PER_UNIT;
    const points = coords.map(([x, y]) => ({
      x: x * yardsPerX,
      y: y * yardsPerY,
      weight: coordSessions.get(coordKey(zoneID, x, y))?.size ?? 1,
    }));

    const sessionsOf = (members: number[]) => {
      const sessions = new Set<string>();
      for (const i of members) {
        for (const session of coordSessions.get(coordKey(zoneID, coords[i][0], coords[i][1])) ?? []) sessions.add(session);
      }
      return sessions.size;
    };
    const candidates = coverClusters(points, SPAWN_CLUSTER_RADIUS_YARDS).map((c) => ({ ...c, sessions: sessionsOf(c.members) }));
    if (candidates.length === 0) continue;
    const zoneSessions = sessionsOf(coords.map((_, i) => i));
    const strongest = Math.max(...candidates.map((c) => c.sessions));
    const minSessions = Math.min(
      strongest,
      Math.max(zoneSessions * MIN_CLUSTER_SESSION_SHARE, zoneSessions >= MIN_SESSIONS_TO_DROP_SINGLE_SESSION_CLUSTERS ? 2 : 1),
    );
    let clusters = candidates.filter((c) => c.sessions >= minSessions);
    if (singleSpawnPerZone) {
      const weight = (c: Cluster) =>
        c.members.reduce((sum, i) => sum + (observationCounts.get(coordKey(zoneID, coords[i][0], coords[i][1])) ?? 1), 0);
      clusters = [clusters.reduce((best, c) => (weight(c) > weight(best) ? c : best))];
    }
    // Rounded to 2 decimal places
    const spawns = clusters.map((c): [number, number] => [
      Math.round((c.x / yardsPerX) * 100) / 100,
      Math.round((c.y / yardsPerY) * 100) / 100,
    ]);
    spawns.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    result[zoneID] = spawns;
  }
  return result;
}

function coordKey(zoneID: number, x: number, y: number): string {
  return `${zoneID}:${x.toFixed(4)},${y.toFixed(4)}`;
}

/**
 * Merges individual spawn observations into the Questie spawns table shape:
 *   {[zoneID]: {{x,y}, {x,y}, ...}, ...}
 *
 * When the same NPC has observations via different tokens at different
 * coordinates, questpoi observations (quest turn-in POIs from
 * C_QuestLog.GetQuestsOnMap) overrule ALL others, followed by questnpc.
 * Only coordinates of the highest such token are kept when present.
 *
 * This is because questnpc fires during quest interactions when the player
 * must be adjacent to the NPC, giving us the true spawn location. Target/npc
 * tokens can fire from a distance and give inaccurate positions.
 *
 * When no questnpc observations exist, falls back to keeping the best
 * coordinate per (zoneID, x, y) based on token priority. An NPC with npc
 * observations (an interaction window was open) gets a single spawn per zone;
 * in zones where it was interacted with, only those positions are used.
 */
export function mergeSpawns(
  observations: Observation<SpawnObservationValue>[],
  singleSpawnPerZone = false,
): Record<number, Array<[number, number]>> {
  const tagged = observations as TaggedSpawnObservation[];

  const exclusiveToken = findExclusiveToken(tagged);
  // An NPC the player opened an interaction window with (gossip, vendor, innkeeper, ...)
  // has a single spawn per zone: where it was interacted with, else its strongest cluster.
  const interactionZones = new Set(tagged.filter((obs) => obs.token === "npc").map((obs) => obs.value.zoneID));
  const interactable = interactionZones.size > 0;
  const isUsed = (obs: TaggedSpawnObservation) =>
    exclusiveToken ? obs.token === exclusiveToken : obs.token === "npc" || !interactionZones.has(obs.value.zoneID);

  const observationCounts = new Map<string, number>();
  const coordSessions = new Map<string, Set<string>>();
  for (const obs of tagged) {
    if (!isUsed(obs)) continue;
    const key = coordKey(obs.value.zoneID, obs.value.x, obs.value.y);
    observationCounts.set(key, (observationCounts.get(key) ?? 0) + 1);
    const sessions = coordSessions.get(key) ?? new Set<string>();
    sessions.add(obs.provenance.sessionKey ?? obs.provenance.session);
    coordSessions.set(key, sessions);
  }

  if (exclusiveToken) {
    // questpoi/questnpc override everything: only keep coordinates of that token
    const byZone = new Map<number, Array<[number, number]>>();
    for (const obs of tagged) {
      if (obs.token !== exclusiveToken) continue;
      const { zoneID, x, y } = obs.value;
      const coords = byZone.get(zoneID);
      if (coords) {
        // Deduplicate identical coordinates within a zone
        if (!coords.some((c) => c[0] === x && c[1] === y)) {
          coords.push([x, y]);
        }
      } else {
        byZone.set(zoneID, [[x, y]]);
      }
    }

    return clusterByZone(byZone, observationCounts, coordSessions, singleSpawnPerZone);
  }

  // No questnpc observations: fall back to per-coordinate priority merging.
  // For each zone+coordinate pair, keep only the observation with the highest
  // token priority (questnpc > npc > target).
  const bestPerCoord = new Map<string, TaggedSpawnObservation>();

  for (const obs of tagged) {
    if (!isUsed(obs)) continue;
    const { zoneID, x, y } = obs.value;
    const key = coordKey(zoneID, x, y);
    const existing = bestPerCoord.get(key);
    if (!existing || tokenPriority(obs.token) > tokenPriority(existing.token)) {
      bestPerCoord.set(key, obs);
    }
  }

  // Group by zone
  const byZone = new Map<number, Array<[number, number]>>();
  for (const obs of bestPerCoord.values()) {
    const { zoneID, x, y } = obs.value;
    const coords = byZone.get(zoneID);
    if (coords) {
      coords.push([x, y]);
    } else {
      byZone.set(zoneID, [[x, y]]);
    }
  }

  return clusterByZone(byZone, observationCounts, coordSessions, singleSpawnPerZone || interactable);
}

/**
 * Derives zoneID from spawn observations: the most frequently observed zone.
 * When counting, questnpc observations are weighted higher (count as 3) since
 * they represent adjacent NPC interactions, vs regular encounters (count as 1).
 * When questpoi observations exist, only those are counted.
 * Returns 0 (schema default) when no observations exist.
 */
export function mergeZoneID(
  observations: Observation<SpawnObservationValue>[],
): number {
  if (observations.length === 0) return 0;

  const zoneWeights = new Map<number, number>();
  const tagged = observations as TaggedSpawnObservation[];
  const hasQuestpoi = tagged.some((obs) => obs.token === "questpoi");
  for (const obs of tagged) {
    if (hasQuestpoi && obs.token !== "questpoi") continue;
    const { zoneID } = obs.value;
    const weight = tokenPriority(obs.token);
    zoneWeights.set(zoneID, (zoneWeights.get(zoneID) ?? 0) + weight);
  }

  let bestZone = 0;
  let bestWeight = 0;
  for (const [zoneID, weight] of zoneWeights) {
    if (weight > bestWeight) {
      bestWeight = weight;
      bestZone = zoneID;
    }
  }

  return bestZone;
}
