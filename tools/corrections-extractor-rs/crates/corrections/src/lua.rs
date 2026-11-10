//! Lua module writers: the forever*Traces corrections modules, the l10n lookup modules and the
//! QuestXP support file. Ports `extract/schema/{corrections-writer,lookup-writer,quest-xp-writer}.ts`.
//!
//! The TS writers render JS values, so records are handed over as [`Js`] values in exactly the
//! shape TS builds; [`Js::object`] keeps JS property order.

use crate::js::format_number;
use std::fmt::Write;

/// A JS value as the TS writers see it.
#[derive(Clone, Debug, PartialEq)]
pub enum Js {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<Js>),
    /// Entries in JS property order: build with [`Js::object`].
    Object(Vec<(String, Js)>),
}

/// The array index a key names, as JS orders properties: canonical integers 0 ..= 2^32 - 2.
fn array_index(key: &str) -> Option<u64> {
    let canonical = !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) && (key == "0" || !key.starts_with('0'));
    let index: u64 = if canonical && key.len() <= 10 { key.parse().ok()? } else { return None };
    (index <= 4_294_967_294).then_some(index)
}

impl Js {
    /// An object from entries in insertion order, reordered like JS: array-index keys ascending,
    /// then the other keys in insertion order. A repeated key keeps its first position.
    pub fn object(entries: impl IntoIterator<Item = (String, Js)>) -> Js {
        let mut unique: Vec<(String, Js)> = Vec::new();
        for (key, value) in entries {
            match unique.iter_mut().find(|(k, _)| *k == key) {
                Some(existing) => existing.1 = value,
                None => unique.push((key, value)),
            }
        }
        unique.sort_by_key(|(key, _)| array_index(key).map_or((1, 0), |i| (0, i)));
        Js::Object(unique)
    }

    pub fn str(s: impl Into<String>) -> Js {
        Js::Str(s.into())
    }

    pub fn nums(values: impl IntoIterator<Item = f64>) -> Js {
        Js::Array(values.into_iter().map(Js::Num).collect())
    }
}

/// corrections-writer.ts `luaString`: a Lua 5.1 string literal, escaping control characters
/// (Lua 5.1 has no \xHH, so others use zero-padded decimal).
pub fn lua_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0b}' => out.push_str("\\v"),
            '\u{00}'..='\u{06}' | '\u{0e}'..='\u{1f}' | '\u{7f}' => write!(out, "\\{:03}", c as u32).expect("writing to a String"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A table constructor key: `[7]` for digit-only keys, `["name"]` otherwise.
fn lua_key(key: &str) -> String {
    if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
        format!("[{key}]")
    } else {
        format!("[{}]", lua_string(key))
    }
}

/// Starter/finisher tables `{creatures, objects[, items]}` render positionally.
fn positional_arrays(entries: &[(String, Js)]) -> Option<Vec<Vec<Js>>> {
    if entries.is_empty() || entries.len() > 3 || !entries.iter().all(|(k, _)| matches!(k.as_str(), "creatures" | "objects" | "items")) {
        return None;
    }
    let array = |name: &str| -> Vec<Js> {
        match entries.iter().find(|(k, _)| k == name).map(|(_, v)| v) {
            // `obj.creatures ?? []`: only a missing key or null falls back.
            None | Some(Js::Null) => Vec::new(),
            Some(Js::Array(items)) => items.clone(),
            // A non-array would render through luaValue in TS; it never occurs in records.
            Some(other) => vec![other.clone()],
        }
    };
    let has_items = entries.iter().any(|(k, _)| k == "items");
    Some(if has_items { vec![array("creatures"), array("objects"), array("items")] } else { vec![array("creatures"), array("objects")] })
}

/// corrections-writer.ts `luaValue`.
pub fn lua_value(value: &Js) -> String {
    match value {
        Js::Null => "nil".into(),
        Js::Num(n) if n.is_finite() => format_number(*n),
        Js::Num(_) => "nil".into(),
        Js::Bool(b) => (if *b { "true" } else { "false" }).into(),
        Js::Str(s) => lua_string(s),
        Js::Array(items) if items.is_empty() => "nil".into(),
        Js::Array(items) => format!("{{{}}}", items.iter().map(lua_value).collect::<Vec<_>>().join(",")),
        Js::Object(entries) => {
            if let Some(mut positions) = positional_arrays(entries) {
                // Trailing empty arrays are omitted (Questie convention).
                while positions.last().is_some_and(Vec::is_empty) {
                    positions.pop();
                }
                if positions.is_empty() {
                    return "nil".into();
                }
                return format!("{{{}}}", positions.into_iter().map(|p| lua_value(&Js::Array(p))).collect::<Vec<_>>().join(","));
            }
            if entries.is_empty() {
                return "nil".into();
            }
            let rendered: Vec<String> = entries
                .iter()
                .map(|(key, entry)| {
                    let inner = lua_value(entry);
                    // Flat number arrays get an extra table layer: Questie reads {[idx] = {{id1, id2}}}.
                    // (`[].every` is true, so an empty array renders as {nil}.)
                    match entry {
                        Js::Array(items) if items.iter().all(|v| matches!(v, Js::Num(_))) => format!("{}={{{inner}}}", lua_key(key)),
                        _ => format!("{}={inner}", lua_key(key)),
                    }
                })
                .collect();
            format!("{{{}}}", rendered.join(","))
        }
    }
}

/// Maps internal field names to QuestieDB keys table field names (`_add` for correction
/// fields layered on base DB fields).
fn questie_field_name(field: &str) -> &str {
    match field {
        "startedBy" => "startedBy_add",
        "finishedBy" => "finishedBy_add",
        "questStarts" => "questStarts_add",
        "questEnds" => "questEnds_add",
        "npcDrops" => "npcDrops_add",
        "objectDrops" => "objectDrops_add",
        other => other,
    }
}

/// Fields held back from export: the little data there is would produce too many wrong entries.
fn is_field_excluded(field: &str) -> bool {
    matches!(field, "requiredLevel" | "zoneOrSort")
}

/// Header lines every generated module carries.
pub struct Header<'a> {
    pub source_file_count: usize,
    pub session_count: usize,
    /// `Date.toISOString()` of the run.
    pub generated_at: &'a str,
}

impl Header<'_> {
    fn lines(&self) -> [String; 4] {
        [
            "-- Generated by QuestieTrace extract (tools/corrections-extractor)".into(),
            format!("-- Source trace files: {}", self.source_file_count),
            format!("-- Sessions aggregated: {}", self.session_count),
            format!("-- Generated at: {}", self.generated_at),
        ]
    }
}

/// One entity's sparse record: field name -> value, in insertion order.
pub type Record = Vec<(String, Js)>;

/// corrections-writer.ts `writeQuestieCorrectionsLua`. `records` must be sorted by id or not:
/// they are written in ascending id order; entities without fields are skipped.
pub fn write_corrections(module: &str, keys_local: &str, records: &[(i64, Record)], header: &Header) -> String {
    let mut lines: Vec<String> = vec![
        format!("---@class {module}"),
        format!("local {module} = QuestieLoader:CreateModule(\"{module}\")"),
        String::new(),
        "---@type QuestieDB".into(),
        "local QuestieDB = QuestieLoader:ImportModule(\"QuestieDB\")".into(),
        String::new(),
    ];
    lines.extend(header.lines());
    lines.push(format!("function {module}:Load()"));
    lines.push(format!("    local {keys_local} = QuestieDB.{keys_local}"));
    lines.push(String::new());
    lines.push("    return {".into());

    let mut sorted: Vec<&(i64, Record)> = records.iter().collect();
    sorted.sort_by_key(|(id, _)| *id);
    for (id, record) in sorted {
        if record.is_empty() {
            continue;
        }
        lines.push(format!("        [{id}] = {{"));
        for (field, value) in record {
            if is_field_excluded(field) {
                continue;
            }
            let rendered = match (field.as_str(), value) {
                ("objectives", Js::Object(entries)) => render_objectives(entries),
                _ => lua_value(value),
            };
            lines.push(format!("            [{keys_local}.{}] = {rendered},", questie_field_name(field)));
        }
        lines.push("        },".into());
    }
    lines.push("    }".into());
    lines.push("end".into());
    lines.join("\n")
}

/// Quest objectives: a positional `{creatureObj, objectObj, itemObj}` table with each id in
/// its own table; missing positions are nil and trailing ones are dropped.
fn render_objectives(entries: &[(String, Js)]) -> String {
    let position = |index: &str| -> Option<&Js> { entries.iter().find(|(k, _)| k == index).map(|(_, v)| v).filter(|v| !matches!(v, Js::Null)) };
    let mut positions: Vec<Option<&Js>> = vec![position("1"), position("2"), position("3")];
    while positions.len() > 1 && positions.last().is_some_and(Option::is_none) {
        positions.pop();
    }
    let rendered: Vec<String> = positions
        .into_iter()
        .map(|p| match p {
            None => "nil".into(),
            Some(Js::Array(ids)) => format!("{{{}}}", ids.iter().map(|id| format!("{{{}}}", lua_value(id))).collect::<Vec<_>>().join(",")),
            Some(other) => lua_value(other),
        })
        .collect();
    format!("{{{}}}", rendered.join(","))
}

/// The l10n table a lookup module fills.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LookupTable {
    Npc,
    Quest,
    Item,
    Object,
}

impl LookupTable {
    fn name(self) -> &'static str {
        match self {
            LookupTable::Npc => "npcNameLookup",
            LookupTable::Quest => "questLookup",
            LookupTable::Item => "itemLookup",
            LookupTable::Object => "objectLookup",
        }
    }
}

fn lookup_value(value: &Js) -> String {
    match value {
        Js::Null => "nil".into(),
        Js::Str(s) => lua_string(s),
        Js::Array(items) => format!("{{{}}}", items.iter().map(lookup_value).collect::<Vec<_>>().join(",")),
        other => panic!("unsupported lookup value: {other:?}"),
    }
}

/// lookup-writer.ts `writeLookupLua`: one locale's translations as a Questie l10n lookup module.
pub fn write_lookup(table: LookupTable, locale: &str, entries: &[(i64, Js)]) -> String {
    let mut sorted: Vec<&(i64, Js)> = entries.iter().collect();
    sorted.sort_by_key(|(id, _)| *id);
    let mut lines: Vec<String> = vec![
        format!("if GetLocale() ~= {} then", lua_string(locale)),
        "    return".into(),
        "end".into(),
        String::new(),
        "---@type l10n".into(),
        "local l10n = QuestieLoader:ImportModule(\"l10n\")".into(),
        String::new(),
        format!("l10n.{}[{}] = loadstring([[return {{", table.name(), lua_string(locale)),
    ];
    for (id, value) in sorted {
        // Questie writes the outer quest tuple with a space after the comma, unlike every other table.
        let rendered = match (table, value) {
            (LookupTable::Quest, Js::Array(items)) => format!("{{{}}}", items.iter().map(lookup_value).collect::<Vec<_>>().join(", ")),
            _ => lookup_value(value),
        };
        lines.push(format!("[{id}] = {rendered},"));
    }
    lines.push("}]])".into());
    lines.push(String::new());
    lines.join("\n")
}

/// quest-xp-writer.ts `writeQuestXpLua`: `QuestXP.db[id] = {level, xp}` rows, sorted by id.
pub fn write_quest_xp(rows: &[(i64, f64, f64)], header: &Header) -> String {
    let mut lines: Vec<String> = vec!["---@type QuestXP".into(), "local QuestXP = QuestieLoader:ImportModule(\"QuestXP\")".into(), String::new()];
    lines.extend(header.lines());
    let mut sorted: Vec<&(i64, f64, f64)> = rows.iter().collect();
    sorted.sort_by_key(|(id, _, _)| *id);
    for (id, level, xp) in sorted {
        lines.push(format!("QuestXP.db[{id}] = {{{}, {}}}", format_number(*level), format_number(*xp)));
    }
    lines.join("\n")
}

/// `Date.prototype.toISOString` for milliseconds since the Unix epoch.
pub fn iso_timestamp(unix_millis: i64) -> String {
    let days = unix_millis.div_euclid(86_400_000);
    let ms_of_day = unix_millis.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: Header = Header { source_file_count: 1, session_count: 1, generated_at: "2020-01-01T00:00:00.000Z" };

    fn record(fields: Vec<(&str, Js)>) -> Record {
        fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }

    fn npc_lua(records: &[(i64, Record)]) -> String {
        write_corrections("ForeverNpcTraces", "npcKeys", records, &HEADER)
    }

    fn quest_lua(records: &[(i64, Record)]) -> String {
        write_corrections("ForeverQuestTraces", "questKeys", records, &HEADER)
    }

    fn ids(values: &[f64]) -> Js {
        Js::nums(values.iter().copied())
    }

    fn relation(fields: Vec<(&str, Js)>) -> Js {
        Js::object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)))
    }

    #[test]
    fn emits_a_load_function_returning_corrections_keyed_by_field_lookups() {
        let lua = npc_lua(&[(823, record(vec![("name", Js::str("Deputy Willem"))]))]);
        assert!(lua.contains("---@class ForeverNpcTraces"));
        assert!(lua.contains("local ForeverNpcTraces = QuestieLoader:CreateModule(\"ForeverNpcTraces\")"));
        assert!(lua.contains("local QuestieDB = QuestieLoader:ImportModule(\"QuestieDB\")"));
        assert!(lua.contains("function ForeverNpcTraces:Load()"));
        assert!(lua.contains("    local npcKeys = QuestieDB.npcKeys"));
        assert!(lua.contains("[823] = {"));
        assert!(lua.contains("[npcKeys.name] = \"Deputy Willem\","));
        assert!(lua.contains("    }\nend"));
    }

    #[test]
    fn only_emits_fields_present_on_a_record() {
        let lua = npc_lua(&[(823, record(vec![("name", Js::str("Deputy Willem")), ("minLevel", Js::Num(10.0))]))]);
        assert!(lua.contains("[npcKeys.name]"));
        assert!(lua.contains("[npcKeys.minLevel] = 10,"));
        assert!(!lua.contains("maxLevel"));
    }

    #[test]
    fn skips_entities_with_no_corrected_fields() {
        let lua = npc_lua(&[(1, record(vec![])), (2, record(vec![("name", Js::str("Has Data"))]))]);
        assert!(!lua.contains("[1] = {"));
        assert!(lua.contains("[2] = {"));
    }

    #[test]
    fn includes_the_header_with_file_and_session_counts_and_timestamp() {
        let header = Header { source_file_count: 2, session_count: 5, generated_at: "2020-01-01T00:00:00.000Z" };
        let lua = write_corrections("ForeverNpcTraces", "npcKeys", &[], &header);
        assert!(lua.contains("-- Source trace files: 2"));
        assert!(lua.contains("-- Sessions aggregated: 5"));
        assert!(lua.contains("-- Generated at: 2020-01-01T00:00:00.000Z"));
    }

    #[test]
    fn escapes_strings_and_formats_nil_number_and_boolean_values() {
        let lua = npc_lua(&[(1, record(vec![("name", Js::str("Say \"Hi\"")), ("minLevel", Js::Num(12.0)), ("flag", Js::Bool(true)), ("missing", Js::Null)]))]);
        assert!(lua.contains("[npcKeys.name] = \"Say \\\"Hi\\\"\","));
        assert!(lua.contains("[npcKeys.minLevel] = 12,"));
        assert!(lua.contains("[npcKeys.flag] = true,"));
        assert!(lua.contains("[npcKeys.missing] = nil,"));
    }

    #[test]
    fn escapes_control_characters_so_a_string_stays_on_one_line() {
        let lua = npc_lua(&[(1, record(vec![("name", Js::str("Telenos \nLeafwhisper"))]))]);
        let name_line = lua.lines().find(|l| l.contains("npcKeys.name")).unwrap();
        assert_eq!(name_line, "            [npcKeys.name] = \"Telenos \\nLeafwhisper\",");
        assert_eq!(lua_string("a\u{01}b\u{7f}"), "\"a\\001b\\127\"");
    }

    #[test]
    fn renders_arrays_and_starter_finisher_tables_positionally() {
        let lua = quest_lua(&[(
            1,
            record(vec![
                ("breadcrumbs", ids(&[33.0, 45.0])),
                ("startedBy", relation(vec![("creatures", ids(&[33.0])), ("objects", ids(&[])), ("items", ids(&[45.0]))])),
                ("finishedBy", relation(vec![("creatures", ids(&[40.0])), ("objects", ids(&[12.0]))])),
                ("startedByEmpty", relation(vec![("creatures", ids(&[])), ("objects", ids(&[])), ("items", ids(&[]))])),
            ]),
        )]);
        assert!(lua.contains("[questKeys.breadcrumbs] = {33,45},"));
        assert!(lua.contains("[questKeys.startedBy_add] = {{33},nil,{45}},"));
        assert!(lua.contains("[questKeys.finishedBy_add] = {{40},{12}},"));
        assert!(lua.contains("[questKeys.startedByEmpty] = nil,"));
    }

    #[test]
    fn renders_two_position_finisher_and_quest_end_tables() {
        let lua = quest_lua(&[
            (1, record(vec![("finishedBy", relation(vec![("creatures", ids(&[10.0, 20.0])), ("objects", ids(&[30.0]))]))])),
            (2, record(vec![("questEnds", relation(vec![("creatures", ids(&[40.0])), ("objects", ids(&[]))]))])),
        ]);
        assert!(lua.contains("[questKeys.finishedBy_add] = {{10,20},{30}},"));
        assert!(lua.contains("[questKeys.questEnds_add] = {{40}},"));
        let npc = npc_lua(&[(2, record(vec![("questEnds", relation(vec![("creatures", ids(&[])), ("objects", ids(&[50.0]))]))]))]);
        assert!(npc.contains("[npcKeys.questEnds_add] = {nil,{50}},"));
    }

    #[test]
    fn returns_an_empty_table_body_without_records() {
        assert!(npc_lua(&[]).contains("    return {\n    }\nend"));
    }

    #[test]
    fn renders_objectives_positionally_with_each_id_wrapped() {
        let objectives = |entries: Vec<(&str, &[f64])>| record(vec![("objectives", Js::object(entries.into_iter().map(|(k, v)| (k.to_owned(), ids(v)))))]);
        assert!(quest_lua(&[(207, objectives(vec![("3", &[2005.0, 2006.0])]))]).contains("[questKeys.objectives] = {nil,nil,{{2005},{2006}}},"));
        assert!(quest_lua(&[(207, objectives(vec![("1", &[2005.0, 2006.0])]))]).contains("[questKeys.objectives] = {{{2005},{2006}}},"));
        assert!(quest_lua(&[(38, objectives(vec![("2", &[729.0])]))]).contains("[questKeys.objectives] = {nil,{{729}}},"));
        assert!(quest_lua(&[(12, objectives(vec![("2", &[589.0]), ("1", &[504.0, 95.0])]))]).contains("[questKeys.objectives] = {{{504},{95}},{{589}}},"));
    }

    #[test]
    fn renders_item_drops_with_the_add_suffix() {
        let lua = write_corrections("ForeverItemTraces", "itemKeys", &[(123, record(vec![("npcDrops", ids(&[1001.0, 1002.0])), ("objectDrops", ids(&[2001.0]))]))], &HEADER);
        assert!(lua.contains("[itemKeys.npcDrops_add] = {1001,1002},"));
        assert!(lua.contains("[itemKeys.objectDrops_add] = {2001},"));
    }

    #[test]
    fn holds_back_excluded_fields() {
        let lua = quest_lua(&[(1, record(vec![("requiredLevel", Js::Num(5.0)), ("zoneOrSort", Js::Num(12.0)), ("questLevel", Js::Num(6.0))]))]);
        assert!(!lua.contains("requiredLevel") && !lua.contains("zoneOrSort"));
        assert!(lua.contains("[questKeys.questLevel] = 6,"));
    }

    #[test]
    fn object_values_with_number_arrays_get_an_extra_table_layer_and_index_keys_sort_first() {
        let spawns = Js::object(vec![
            ("1519".to_owned(), Js::Array(vec![ids(&[45.5, 30.25])])),
            ("12".to_owned(), Js::Array(vec![ids(&[1.0, 2.0])])),
        ]);
        assert_eq!(lua_value(&spawns), "{[12]={{1,2}},[1519]={{45.5,30.25}}}");
        assert_eq!(lua_value(&Js::object(vec![("7".to_owned(), ids(&[1.0, 2.0]))])), "{[7]={{1,2}}}");
    }

    #[test]
    fn quest_xp_rows_are_sorted_by_id() {
        let header = Header { source_file_count: 1, session_count: 3, generated_at: "2026-01-01T00:00:00.000Z" };
        let lua = write_quest_xp(&[(312, 10.0, 900.0), (99, 5.0, 110.0)], &header);
        assert!(lua.contains("local QuestXP = QuestieLoader:ImportModule(\"QuestXP\")"));
        assert!(lua.contains("-- Sessions aggregated: 3"));
        assert!(lua.find("QuestXP.db[99] = {5, 110}").unwrap() < lua.find("QuestXP.db[312] = {10, 900}").unwrap());
    }

    #[test]
    fn quest_lookups_put_a_space_after_the_tuple_comma_only() {
        let lua = write_lookup(LookupTable::Quest, "deDE", &[(9, Js::Array(vec![Js::str("Titel"), Js::Array(vec![Js::str("Ziel")])])), (2, Js::Array(vec![Js::str("Zwei"), Js::Null]))]);
        assert!(lua.starts_with("if GetLocale() ~= \"deDE\" then\n    return\nend\n"));
        assert!(lua.contains("l10n.questLookup[\"deDE\"] = loadstring([[return {\n[2] = {\"Zwei\", nil},\n[9] = {\"Titel\", {\"Ziel\"}},\n}]])\n"));
    }

    #[test]
    fn iso_timestamps_match_date_to_iso_string() {
        assert_eq!(iso_timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_timestamp(1_577_836_800_000), "2020-01-01T00:00:00.000Z");
        // Expected values from `new Date(ms).toISOString()` in Node.
        assert_eq!(iso_timestamp(1_794_328_017_488), "2026-11-10T16:26:57.488Z");
    }
}
