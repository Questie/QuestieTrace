//! The DB2 CSV exports the TypeScript tool ships in `extract/resources/`, compiled in from
//! there so both tools read one copy. A newer build means a new file name: update the paths
//! here (the build fails until then).

use crate::js;
use std::collections::HashMap;
use std::sync::LazyLock;

macro_rules! resource {
    ($file:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../corrections-extractor/src/extract/resources/",
            $file
        ))
    };
}

const AREA_TABLE: &str = resource!("AreaTable.1.60.1.70009.csv");
const QUEST_SORT: &str = resource!("QuestSort.1.60.1.70009.csv");
const QUEST_XP: &str = resource!("QuestXP.1.60.1.70862.csv");
const UI_MAP: &str = resource!("UiMap.1.60.1.70009.csv");

/// csv.ts `parseCsv`: the subset of CSV the DB2 exports use (quoted fields, "" escapes).
fn parse_csv(csv: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = csv.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' && chars.peek() == Some(&'"') {
                field.push('"');
                chars.next();
            } else if c == '"' {
                in_quotes = false;
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            '\r' => {}
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn column(header: &[String], name: &str) -> Option<usize> {
    header.iter().position(|column| column == name)
}

/// `name.trim().toLocaleLowerCase()`.
fn normalize_name(name: &str) -> String {
    js::trim(name).to_lowercase()
}

/// Display name -> id over `columns`, keeping the first or the lowest id per name.
fn ids_by_name(csv: &str, columns: &[&str], keep_lowest: bool) -> HashMap<String, f64> {
    let rows = parse_csv(csv);
    let Some(header) = rows.first() else { return HashMap::new() };
    let Some(id_index) = column(header, "ID") else { return HashMap::new() };
    let name_indexes: Vec<usize> = columns.iter().filter_map(|name| column(header, name)).collect();
    let mut by_name = HashMap::new();
    for row in &rows[1..] {
        let id = js::number(row.get(id_index).map_or("", String::as_str));
        if !id.is_finite() || id == 0.0 {
            continue;
        }
        for &index in &name_indexes {
            let Some(name) = row.get(index).filter(|name| !name.is_empty()) else {
                continue;
            };
            let entry = by_name.entry(normalize_name(name));
            match entry {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(id);
                }
                std::collections::hash_map::Entry::Occupied(mut slot) if keep_lowest && id < *slot.get() => {
                    slot.insert(id);
                }
                std::collections::hash_map::Entry::Occupied(_) => {}
            }
        }
    }
    by_name
}

/// uiMapID -> AreaTable areaID: rows describe the same map when their display names match;
/// for duplicate area names the lowest areaID wins (ui-map-to-area.ts).
static AREA_ID_BY_UI_MAP_ID: LazyLock<HashMap<u64, f64>> = LazyLock::new(|| {
    let area_by_name = ids_by_name(AREA_TABLE, &["AreaName_lang"], true);
    let ui_map_by_name = ids_by_name(UI_MAP, &["Name_lang"], false);
    ui_map_by_name
        .iter()
        .filter_map(|(name, &ui_map_id)| area_by_name.get(name).map(|&area_id| (ui_map_id.to_bits(), area_id)))
        .collect()
});

/// ui-map-to-area.ts `areaIdForUiMapId`: the AreaTable areaID of a uiMapID, as Questie keys
/// zones; None for maps without an area (continents, dungeons, unknown ids).
pub fn area_id_for_ui_map_id(ui_map_id: Option<f64>) -> Option<f64> {
    let id = ui_map_id.filter(|id| id.is_finite())?;
    AREA_ID_BY_UI_MAP_ID.get(&(id + 0.0).to_bits()).copied()
}

/// zone-or-sort.ts: quest-log header title -> signed zoneOrSort (AreaTable ids positive,
/// QuestSort ids negative). The first match of a name wins; areas before sorts.
static ZONE_OR_SORT_BY_NAME: LazyLock<HashMap<String, f64>> = LazyLock::new(|| {
    let mut lookup = HashMap::new();
    // Row by row, both name columns of a row before the next row, as addNames does.
    for (csv, columns, sign) in [
        (AREA_TABLE, &["AreaName_lang", "ZoneName"][..], 1.0),
        (QUEST_SORT, &["SortName_lang"][..], -1.0),
    ] {
        let rows = parse_csv(csv);
        let Some(header) = rows.first() else { continue };
        let Some(id_index) = column(header, "ID") else { continue };
        let name_indexes: Vec<usize> = columns.iter().filter_map(|name| column(header, name)).collect();
        for row in &rows[1..] {
            let id = js::number(row.get(id_index).map_or("", String::as_str));
            if !id.is_finite() || id == 0.0 {
                continue;
            }
            for &index in &name_indexes {
                let Some(name) = row.get(index).filter(|name| !name.is_empty()) else {
                    continue;
                };
                lookup.entry(normalize_name(name)).or_insert(sign * id);
            }
        }
    }
    lookup
});

/// zone-or-sort.ts `zoneOrSortIdForName`.
pub fn zone_or_sort_id_for_name(name: &str) -> Option<f64> {
    ZONE_OR_SORT_BY_NAME.get(&normalize_name(name)).copied()
}

/// quest-xp.ts: Difficulty_1 ..= Difficulty_7 hold XP; _0, _8 and _9 are always 0.
const FIRST_DIFFICULTY: usize = 1;
const LAST_DIFFICULTY: usize = 7;

/// level -> raw XP per difficulty (index = difficulty).
static QUEST_XP_BY_LEVEL: LazyLock<HashMap<i64, [f64; LAST_DIFFICULTY + 1]>> = LazyLock::new(|| {
    let rows = parse_csv(QUEST_XP);
    let header = &rows[0];
    let id_index = column(header, "ID").expect("QuestXP has an ID column");
    let mut by_level = HashMap::new();
    for row in &rows[1..] {
        // A missing cell is JS undefined, which Number() turns into NaN.
        let level = row.get(id_index).map_or(f64::NAN, |cell| js::number(cell));
        if !js::is_integer(level) {
            continue;
        }
        let mut values = [f64::NAN; LAST_DIFFICULTY + 1];
        for (difficulty, value) in values.iter_mut().enumerate().skip(FIRST_DIFFICULTY) {
            let index = column(header, &format!("Difficulty_{difficulty}"));
            *value = index.and_then(|i| row.get(i)).map_or(f64::NAN, |cell| js::number(cell));
        }
        by_level.insert(level as i64, values);
    }
    by_level
});

/// quest-xp.ts `roundQuestXp`: the rounding the client applies to quest XP.
pub fn round_quest_xp(xp: f64) -> f64 {
    if xp <= 100.0 {
        5.0 * ((xp + 2.0) / 5.0).floor()
    } else if xp <= 500.0 {
        10.0 * ((xp + 5.0) / 10.0).floor()
    } else if xp <= 1000.0 {
        25.0 * ((xp + 12.0) / 25.0).floor()
    } else {
        50.0 * ((xp + 25.0) / 50.0).floor()
    }
}

fn level_key(level: f64) -> Option<i64> {
    js::is_integer(level).then_some(level as i64)
}

/// quest-xp.ts `questXpFor`: the raw XP of a level and difficulty, if the table has any.
pub fn quest_xp_for(level: f64, difficulty: usize) -> Option<f64> {
    let xp = *QUEST_XP_BY_LEVEL.get(&level_key(level)?)?.get(difficulty)?;
    (xp > 0.0).then_some(xp)
}

/// quest-xp.ts `questXpDifficulty`: the one difficulty whose rounded XP is `shown_xp` at
/// `level`; None when no or several difficulties match.
pub fn quest_xp_difficulty(level: f64, shown_xp: f64) -> Option<usize> {
    let values = QUEST_XP_BY_LEVEL.get(&level_key(level)?)?;
    let mut found = None;
    for (difficulty, &value) in values.iter().enumerate().skip(FIRST_DIFFICULTY) {
        // NaN fails `> 0`, so missing cells never match, as in TS.
        let has_xp = value > 0.0;
        if !has_xp || round_quest_xp(value) != shown_xp {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(difficulty);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elwynn_forest_maps_from_its_ui_map_to_its_area() {
        // ui-map-to-area.test.ts: Elwynn Forest is uiMapID 1429, areaID 12.
        assert_eq!(area_id_for_ui_map_id(Some(1429.0)), Some(12.0));
        assert_eq!(area_id_for_ui_map_id(Some(947.0)), None, "Azeroth has no area");
        assert_eq!(area_id_for_ui_map_id(None), None);
    }

    #[test]
    fn zone_names_are_positive_and_sort_names_negative() {
        assert_eq!(zone_or_sort_id_for_name(" dun morogh "), Some(1.0));
        assert_eq!(zone_or_sort_id_for_name("Epic"), Some(-1.0));
        assert_eq!(zone_or_sort_id_for_name("No Such Zone"), None);
    }

    #[test]
    fn quest_xp_matches_a_single_rounded_difficulty() {
        // Level 1: 10, 20, 40, 60, 80, 100, 120 for difficulties 1..=7.
        assert_eq!(quest_xp_for(1.0, 3), Some(40.0));
        assert_eq!(quest_xp_difficulty(1.0, 40.0), Some(3));
        assert_eq!(quest_xp_difficulty(1.0, 41.0), None);
        assert_eq!(round_quest_xp(1234.0), 1250.0);
    }
}
