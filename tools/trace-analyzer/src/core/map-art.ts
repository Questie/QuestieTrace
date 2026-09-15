// ============================================================
// Zone map art registry — WoW Classic Era
//
// Maps a uiMapID (as reported by C_Map.GetBestMapForUnit) to the
// in-game map image used as the Position tab backdrop.
//
// IMPORTANT — the art must come from the `wow/classic/maps` path. Wowhead
// serves two sets of map art from the same host, and the non-classic path
// (`images/wow/maps/...`) is *retail*: its Stormwind has Lion's Rest and the
// Stormwind Embassy instead of The Park, its Orgrimmar is the Cataclysm
// rebuild, and its Azeroth map includes the Dragon Isles. The classic path
// below serves the maps a Classic player actually sees, already composited
// with the explored-area detail (Goldshire, roads, subzone names).
//
// Ids were produced by joining the Classic Era `UiMap` and `AreaTable` DB2
// tables (build 1.15.9.69722) and cross-checked against Questie's own
// uiMapIdToAreaId table; both sources agree on every entry.
// ============================================================

/** A map whose art can be displayed behind the position plot. */
export interface ZoneMapArt {
  /**
   * Image CDN id. For zones and cities this is the AreaTable ID
   * (e.g. Elwynn Forest is uiMap 1429 but area 12). The world map and the two
   * continents sit outside that namespace and use the CDN's own negative ids.
   */
  zoneId: number;
  /** Canonical enUS name, matched against GetZoneText as a fallback. */
  zoneName: string;
  /**
   * Classic Era uiMapIDs that resolve to this art. Both continents appear
   * under two ids, so there are 54 ids across 52 images.
   */
  uiMapIds: number[];
}

/**
 * Every Classic Era map: 44 outdoor zones, the 6 capitals, 3 battlegrounds,
 * both continents, and the Azeroth world map.
 *
 * On the classic path every image is a uniform 772x515 — the 3:2 rectangle
 * uiMap 0-1 coordinates span — so the plot can draw any of it directly.
 */
export const ZONE_MAP_ART: ZoneMapArt[] = [
  { zoneId:   36, zoneName: "Alterac Mountains",    uiMapIds: [1416] },
  { zoneId: 2597, zoneName: "Alterac Valley",       uiMapIds: [1459] },
  { zoneId: 3358, zoneName: "Arathi Basin",         uiMapIds: [1461] },
  { zoneId:   45, zoneName: "Arathi Highlands",     uiMapIds: [1417] },
  { zoneId:  331, zoneName: "Ashenvale",            uiMapIds: [1440] },
  { zoneId:   -1, zoneName: "Azeroth",              uiMapIds: [947] },
  { zoneId:   16, zoneName: "Azshara",              uiMapIds: [1447] },
  { zoneId:    3, zoneName: "Badlands",             uiMapIds: [1418] },
  { zoneId:    4, zoneName: "Blasted Lands",        uiMapIds: [1419] },
  { zoneId:   46, zoneName: "Burning Steppes",      uiMapIds: [1428] },
  { zoneId:  148, zoneName: "Darkshore",            uiMapIds: [1439] },
  { zoneId: 1657, zoneName: "Darnassus",            uiMapIds: [1457] },
  { zoneId:   41, zoneName: "Deadwind Pass",        uiMapIds: [1430] },
  { zoneId:  405, zoneName: "Desolace",             uiMapIds: [1443] },
  { zoneId:    1, zoneName: "Dun Morogh",           uiMapIds: [1426] },
  { zoneId:   14, zoneName: "Durotar",              uiMapIds: [1411] },
  { zoneId:   10, zoneName: "Duskwood",             uiMapIds: [1431] },
  { zoneId:   15, zoneName: "Dustwallow Marsh",     uiMapIds: [1445] },
  { zoneId:   -3, zoneName: "Eastern Kingdoms",     uiMapIds: [1415, 1463] },
  { zoneId:  139, zoneName: "Eastern Plaguelands",  uiMapIds: [1423] },
  { zoneId:   12, zoneName: "Elwynn Forest",        uiMapIds: [1429] },
  { zoneId:  361, zoneName: "Felwood",              uiMapIds: [1448] },
  { zoneId:  357, zoneName: "Feralas",              uiMapIds: [1444] },
  { zoneId:  267, zoneName: "Hillsbrad Foothills",  uiMapIds: [1424] },
  { zoneId: 1537, zoneName: "Ironforge",            uiMapIds: [1455] },
  { zoneId:   -6, zoneName: "Kalimdor",             uiMapIds: [1414, 1464] },
  { zoneId:   38, zoneName: "Loch Modan",           uiMapIds: [1432] },
  { zoneId:  493, zoneName: "Moonglade",            uiMapIds: [1450] },
  { zoneId:  215, zoneName: "Mulgore",              uiMapIds: [1412] },
  { zoneId: 1637, zoneName: "Orgrimmar",            uiMapIds: [1454] },
  { zoneId:   44, zoneName: "Redridge Mountains",   uiMapIds: [1433] },
  { zoneId:   51, zoneName: "Searing Gorge",        uiMapIds: [1427] },
  { zoneId: 1377, zoneName: "Silithus",             uiMapIds: [1451] },
  { zoneId:  130, zoneName: "Silverpine Forest",    uiMapIds: [1421] },
  { zoneId:  406, zoneName: "Stonetalon Mountains", uiMapIds: [1442] },
  { zoneId: 1519, zoneName: "Stormwind City",       uiMapIds: [1453] },
  { zoneId:   33, zoneName: "Stranglethorn Vale",   uiMapIds: [1434] },
  { zoneId:    8, zoneName: "Swamp of Sorrows",     uiMapIds: [1435] },
  { zoneId:  440, zoneName: "Tanaris",              uiMapIds: [1446] },
  { zoneId:  141, zoneName: "Teldrassil",           uiMapIds: [1438] },
  { zoneId:   17, zoneName: "The Barrens",          uiMapIds: [1413] },
  { zoneId:   47, zoneName: "The Hinterlands",      uiMapIds: [1425] },
  { zoneId:  400, zoneName: "Thousand Needles",     uiMapIds: [1441] },
  { zoneId: 1638, zoneName: "Thunder Bluff",        uiMapIds: [1456] },
  { zoneId:   85, zoneName: "Tirisfal Glades",      uiMapIds: [1420] },
  { zoneId:  490, zoneName: "Un'Goro Crater",       uiMapIds: [1449] },
  { zoneId: 1497, zoneName: "Undercity",            uiMapIds: [1458] },
  { zoneId: 3277, zoneName: "Warsong Gulch",        uiMapIds: [1460] },
  { zoneId:   28, zoneName: "Western Plaguelands",  uiMapIds: [1422] },
  { zoneId:   40, zoneName: "Westfall",             uiMapIds: [1436] },
  { zoneId:   11, zoneName: "Wetlands",             uiMapIds: [1437] },
  { zoneId:  618, zoneName: "Winterspring",         uiMapIds: [1452] },
];

/** Warn once per unknown uiMapID/zone pair so scrubbing does not spam. */
const warnedUnknown = new Set<string>();

/**
 * Resolve map art by uiMapID first, then by zone name.
 *
 * The zone-name fallback is locale-sensitive (GetZoneText returns translated
 * names on non-enUS clients); the uiMapID path is not, which is why it is
 * tried first. The fallback also covers retail traces, whose uiMapIDs differ
 * from Classic's but whose zone names largely match — though the art shown
 * will be the Classic version of that zone.
 *
 * Returns null when the map has no registry entry.
 */
export function findMapArt(
  uiMapId: number | null,
  zoneName: string | null
): ZoneMapArt | null {
  if (uiMapId != null) {
    const byId = ZONE_MAP_ART.find((a) => a.uiMapIds.includes(uiMapId));
    if (byId) return byId;
  }

  if (zoneName) {
    const byName = ZONE_MAP_ART.find((a) => a.zoneName === zoneName);
    if (byName) return byName;
  }

  if (uiMapId != null || zoneName) {
    const key = `${uiMapId ?? "?"}|${zoneName ?? "?"}`;
    if (!warnedUnknown.has(key)) {
      warnedUnknown.add(key);
      console.warn(
        `[map-art] No map art registered for uiMapID ${uiMapId ?? "(none)"} ` +
          `("${zoneName ?? "unknown zone"}"). Dungeon and raid maps are not ` +
          `included; add the AreaTable id to ZONE_MAP_ART in src/core/map-art.ts.`
      );
    }
  }

  return null;
}

/**
 * Build the art URL for a map.
 *
 * `zoom` is the right variant: it is a uniform 772x515 for every map kind,
 * whereas `original` varies (1022x668 for most, 1024x683 for some) and would
 * misalign the route by ~2% horizontally.
 */
export function mapArtUrl(art: ZoneMapArt): string {
  return `https://wow.zamimg.com/images/wow/classic/maps/enus/zoom/${art.zoneId}.jpg`;
}
