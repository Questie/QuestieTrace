//! Reads the quest-relation surfaces of one stored Wowhead quest page (English, stripped HTML from
//! scraper-questie's raw cache). Pure string -> facts; interpretation lives in `interpret`.
//!
//! Surfaces, all inside the infobox:
//!
//! ```text
//! Series                 <table class="series">: numbered rows, each holding one or more quests
//!                        separated by <br/>. The page's own quest is <b>bold</b> without a link.
//! Requires, Requires Any, Requires In Progress, Unlocks, Disables
//!                        [quest=N] markup inside WH.markup.printHtml("...") under that heading.
//! Storyline              <ol> of quests; the page's own quest is <li class="current">.
//! ```
//!
//! Only these panels and the Gatherer quest payloads are read. Pages also embed comment authors
//! (g_users); never extract anything outside the surfaces above.

use crate::model::QuestId;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Wowhead `side`. Missing means unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Alliance,
    Horde,
    Both,
}

impl Side {
    fn from_wowhead(value: &Value) -> Option<Side> {
        match value.as_i64()? {
            1 => Some(Side::Alliance),
            2 => Some(Side::Horde),
            3 => Some(Side::Both),
            _ => None,
        }
    }
}

/// The infobox panels that hold `[quest=N]` markup. Declaration order is display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkupPanel {
    Requires,
    RequiresAny,
    RequiresInProgress,
    Unlocks,
    Disables,
}

impl MarkupPanel {
    pub const ALL: [MarkupPanel; 5] = [
        MarkupPanel::Requires,
        MarkupPanel::RequiresAny,
        MarkupPanel::RequiresInProgress,
        MarkupPanel::Unlocks,
        MarkupPanel::Disables,
    ];

    /// The infobox heading, matched exactly ("Requires" is not "Requires Any").
    pub fn heading(self) -> &'static str {
        match self {
            MarkupPanel::Requires => "Requires",
            MarkupPanel::RequiresAny => "Requires Any",
            MarkupPanel::RequiresInProgress => "Requires In Progress",
            MarkupPanel::Unlocks => "Unlocks",
            MarkupPanel::Disables => "Disables",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesMember {
    pub quest_id: QuestId,
    /// The page's own quest (rendered bold, without a link).
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesRow {
    /// The row number Wowhead prints ("1.", "2.", ...), or the row's position when it was unparsed.
    pub index: i64,
    pub members: Vec<SeriesMember>,
    /// The row's markup was not the expected shape; it is kept, empty, so it still breaks adjacency.
    pub unparsed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storyline {
    pub id: i64,
    pub name: String,
    /// Quest ids in display order, including the page's own quest.
    pub quest_ids: Vec<QuestId>,
}

/// Which Wowhead environment rendered the stored page, from its Gatherer data calls.
/// - `Forever`: dataEnv 16, fetched from Wowhead's /forever pages.
/// - `Era`: dataEnv 4. scraper-questie seeded Forever from the Era cache and only refetches pages
///   whose sitemap entry changed, so these are inherited Era pages, not Forever evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PageEnvironment {
    Forever,
    Era,
    Unknown,
}

impl PageEnvironment {
    pub fn name(self) -> &'static str {
        match self {
            PageEnvironment::Forever => "forever",
            PageEnvironment::Era => "era",
            PageEnvironment::Unknown => "unknown",
        }
    }
}

/// Facts used to tell variants, lead-ins and impossible edges apart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestRecord {
    pub name: Option<String>,
    pub side: Option<Side>,
    /// Wowhead reqrace: a 64-bit race mask (Forever races use bits above 32). None = any.
    pub races: Option<u64>,
    /// Wowhead reqclass: class mask. None = any.
    pub classes: Option<u64>,
    /// Whether the quest's own page lists objectives (kill/collect/use rows, not provided items).
    /// False for "go and talk to X" quests, which is what breadcrumbs look like. Only set from the
    /// quest's own page.
    pub has_objectives: Option<bool>,
}

impl QuestRecord {
    /// Adds facts without overwriting what an earlier, more authoritative source already set.
    pub fn fill_in(&mut self, facts: &QuestRecord) {
        fn fill<T: Clone>(own: &mut Option<T>, other: &Option<T>) {
            if own.is_none() {
                own.clone_from(other);
            }
        }
        fill(&mut self.name, &facts.name);
        fill(&mut self.side, &facts.side);
        fill(&mut self.races, &facts.races);
        fill(&mut self.classes, &facts.classes);
        fill(&mut self.has_objectives, &facts.has_objectives);
    }
}

/// Quest facts by id.
pub type QuestLookup = BTreeMap<QuestId, QuestRecord>;

fn fill_in(quests: &mut QuestLookup, id: QuestId, facts: &QuestRecord) {
    quests.entry(id).or_default().fill_in(facts);
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuestPageFacts {
    pub quest_id: QuestId,
    pub environment: PageEnvironment,
    /// envChange.status on the page's own g_quests record (new, updated, unchanged), if present.
    pub env_change: Option<String>,
    /// Rows of the Series panel, in order; empty when the page has none. A row may have no members.
    pub series: Vec<SeriesRow>,
    /// Quest ids per markup panel present on the page.
    pub panels: BTreeMap<MarkupPanel, Vec<QuestId>>,
    pub storyline: Option<Storyline>,
    /// What the page says about quests: the page's own g_quests record, Gatherer payloads, and
    /// names/faction icons from Series and Storyline links.
    pub quests: QuestLookup,
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

// `[0-9]`, not `\d`: Unicode digits are never ids. `(?s)` makes `.` cross lines like JS `[\s\S]`.
static GATHERER_ENV: LazyLock<Regex> = LazyLock::new(|| regex(r"WH\.Gatherer\.addData\([0-9]+, ([0-9]+),"));
static GATHERER_QUESTS: LazyLock<Regex> = LazyLock::new(|| regex(r"WH\.Gatherer\.addData\(5, [0-9]+, (\{.*\})\);"));
static ICON_LIST: LazyLock<Regex> = LazyLock::new(|| regex(r#"(?s)<table class="icon-list">(.*?)</table>"#));
static TABLE_ROW: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<tr[^>]*>(.*?)</tr>"));
static SERIES_TABLE: LazyLock<Regex> = LazyLock::new(|| regex(r#"(?s)<table class="series">(.*?)</table>"#));
static SERIES_ROW: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)^<th>([0-9]+)\.</th><td>(.*)</td>$"));
static LINE_BREAK: LazyLock<Regex> = LazyLock::new(|| regex(r"<br\s*/?>"));
static SERIES_LINK: LazyLock<Regex> = LazyLock::new(|| regex(r#"<a href="[^"]*/quest=([0-9]+)[^"]*">([^<]*)</a>"#));
static PRINT_HTML: LazyLock<Regex> = LazyLock::new(|| regex(r#"WH\.markup\.printHtml\("((?:[^"\\]|\\.)*)""#));
static MARKUP_QUEST: LazyLock<Regex> = LazyLock::new(|| regex(r"\[quest=([0-9]+)"));
static STORYLINE_TITLE: LazyLock<Regex> = LazyLock::new(|| regex(r#"<a class="quick-facts-storyline-title" href="[^"]*-([0-9]+)">([^<]*)</a>"#));
static ORDERED_LIST: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)<ol>(.*?)</ol>"));
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| regex(r#"(?s)<li( class="current")?>(.*?)</li>"#));
static STORYLINE_LINK: LazyLock<Regex> = LazyLock::new(|| regex(r#"<a [^>]*href="[^"]*/quest=([0-9]+)[^"]*">([^<]*)</a>"#));
static APOSTROPHE: LazyLock<Regex> = LazyLock::new(|| regex(r"&#0*39;|&apos;"));
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)<\s*(nyi|txt)(?-u:\b)|^\[(never used|dnt)\]"));

pub fn parse_quest_page(quest_id: QuestId, body: &str) -> QuestPageFacts {
    let mut quests = QuestLookup::new();
    read_gatherer_quests(body, &mut quests);
    let own = read_own_quest_record(body, quest_id);
    // The page's own record wins; Gatherer payloads only fill its gaps.
    let mut own_record = own.record;
    if let Some(gathered) = quests.get(&quest_id) {
        own_record.fill_in(gathered);
    }
    own_record.has_objectives = has_objectives(body);
    quests.insert(quest_id, own_record);

    let series = parse_series(body, quest_id, &mut quests);
    let storyline = parse_storyline(body, quest_id, &mut quests);
    let panels = MarkupPanel::ALL
        .into_iter()
        .filter_map(|panel| markup_panel_quest_ids(body, panel).map(|ids| (panel, ids)))
        .collect();

    QuestPageFacts {
        quest_id,
        environment: page_environment(body),
        env_change: own.env_change,
        series,
        panels,
        storyline,
        quests,
    }
}

pub fn page_environment(body: &str) -> PageEnvironment {
    let mut environment = PageEnvironment::Unknown;
    for data_env in GATHERER_ENV.captures_iter(body).map(|captures| captures.extract::<1>().1[0]) {
        match data_env {
            "16" => return PageEnvironment::Forever,
            "4" => environment = PageEnvironment::Era,
            _ => {}
        }
    }
    environment
}

/// True when the objectives block (between the quest title and the first section heading) has an
/// objective row. Items the quest hands out (rows marked "(Provided)", or the table after a
/// "Provided item:" label) are not objectives. None when the page has no quest title.
pub fn has_objectives(body: &str) -> Option<bool> {
    let title = body.find(r#"<h1 class="heading-size-1">"#)?;
    let block = &body[title..];
    let block = block.find("<h2").map_or(block, |end| &block[..end]);
    for table in ICON_LIST.captures_iter(block) {
        let start = table.get(0).map_or(0, |whole| whole.start());
        if block[..start].trim_end().ends_with("Provided item:") {
            continue;
        }
        if TABLE_ROW.captures_iter(&table[1]).any(|row| !row[1].contains("(Provided)")) {
            return Some(true);
        }
    }
    Some(false)
}

// --------------------------------------------------------------------------- Series

/// Parses the Series table, filling in the page's own quest id for the bold member. Link names and
/// faction icons are added to `quests`.
pub fn parse_series(body: &str, quest_id: QuestId, quests: &mut QuestLookup) -> Vec<SeriesRow> {
    let Some(table) = SERIES_TABLE.captures(body) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for tr in TABLE_ROW.captures_iter(&table[1]) {
        let parsed = SERIES_ROW.captures(&tr[1]).and_then(|row| Some((row[1].parse().ok()?, row.get(2)?.as_str())));
        let Some((index, cells)) = parsed else {
            rows.push(SeriesRow {
                index: rows.len() as i64 + 1,
                members: Vec::new(),
                unparsed: true,
            });
            continue;
        };
        let members = LINE_BREAK.split(cells).filter_map(|part| parse_series_member(part, quest_id, quests)).collect();
        // Rows can be empty (a step Wowhead does not show in this environment, e.g. TBC quests on
        // Era pages). Keep them so the chain is not stitched together across the gap.
        rows.push(SeriesRow {
            index,
            members,
            unparsed: false,
        });
    }
    rows
}

fn parse_series_member(html: &str, quest_id: QuestId, quests: &mut QuestLookup) -> Option<SeriesMember> {
    let side = if html.contains(r#"class="icon-alliance"#) {
        Some(Side::Alliance)
    } else if html.contains(r#"class="icon-horde"#) {
        Some(Side::Horde)
    } else {
        None
    };
    if let Some(link) = SERIES_LINK.captures(html) {
        let id = link[1].parse().ok()?;
        let facts = QuestRecord {
            name: Some(decode_entities(&link[2])),
            side,
            ..QuestRecord::default()
        };
        fill_in(quests, id, &facts);
        return Some(SeriesMember { quest_id: id, current: false });
    }
    if html.contains("<b>") {
        fill_in(
            quests,
            quest_id,
            &QuestRecord {
                side,
                ..QuestRecord::default()
            },
        );
        return Some(SeriesMember { quest_id, current: true });
    }
    None
}

// --------------------------------------------------------------------------- Markup panels

/// Quest ids in the markup panel under `<th>{heading}</th>`, in display order, or None when the
/// page has no such panel.
pub fn markup_panel_quest_ids(body: &str, panel: MarkupPanel) -> Option<Vec<QuestId>> {
    let section = infobox_section(body, panel.heading())?;
    let Some(markup) = PRINT_HTML.captures(section) else {
        return Some(Vec::new());
    };
    Some(MARKUP_QUEST.captures_iter(&markup[1]).filter_map(|quest| quest[1].parse().ok()).collect())
}

/// The infobox column under `<th>{heading}</th>`, up to the next infobox heading.
fn infobox_section<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    const HEADING_ROW: &str = r#"<tr class="infobox-heading">"#;
    let marker = format!("{HEADING_ROW}\n<th>{heading}</th>");
    let content = &body[body.find(&marker)? + marker.len()..];
    Some(content.find(HEADING_ROW).map_or(content, |next| &content[..next]))
}

// --------------------------------------------------------------------------- Storyline

pub fn parse_storyline(body: &str, quest_id: QuestId, quests: &mut QuestLookup) -> Option<Storyline> {
    let section = infobox_section(body, "Storyline")?;
    let title = STORYLINE_TITLE.captures(section)?;
    let list = ORDERED_LIST.captures(section)?;

    let mut quest_ids = Vec::new();
    for item in LIST_ITEM.captures_iter(&list[1]) {
        if item.get(1).is_some() {
            quest_ids.push(quest_id);
            continue;
        }
        let Some(link) = STORYLINE_LINK.captures(&item[2]) else {
            continue;
        };
        let Ok(id) = link[1].parse() else {
            continue;
        };
        quest_ids.push(id);
        let facts = QuestRecord {
            name: Some(decode_entities(&link[2])),
            ..QuestRecord::default()
        };
        fill_in(quests, id, &facts);
    }
    Some(Storyline {
        id: title[1].parse().ok()?,
        name: decode_entities(&title[2]),
        quest_ids,
    })
}

// --------------------------------------------------------------------------- Quest records

/// A quest object as Wowhead writes it in both g_quests records and Gatherer type-5 payloads:
/// `name` (or `name_enus`), `side` (or `_side`), `reqrace`, `reqclass`.
fn to_record(quest: &Value) -> QuestRecord {
    // `a ?? b`: the second key only counts when the first is missing or null.
    let either = |first: &str, second: &str| quest.get(first).filter(|value| !value.is_null()).or_else(|| quest.get(second));
    QuestRecord {
        name: either("name", "name_enus").and_then(Value::as_str).map(str::to_owned),
        side: either("side", "_side").and_then(Side::from_wowhead),
        // Masks above 2^53 would already have lost precision in Wowhead's JSON; theirs use bits <= 34.
        races: quest.get("reqrace").and_then(Value::as_u64).filter(|&races| races > 0),
        classes: quest.get("reqclass").and_then(Value::as_u64).filter(|&classes| classes > 0),
        has_objectives: None,
    }
}

/// Placeholder quests Blizzard never shipped ("<NYI>", "<TXT>", "[Never used]", "[DNT]"). They have no starters.
pub fn is_placeholder_name(name: Option<&str>) -> bool {
    name.is_some_and(|name| PLACEHOLDER.is_match(name))
}

/// One quest directory across many pages. A quest's own page record wins; facts other pages show
/// about it (Gatherer payloads, Series names and icons) only fill gaps.
pub fn merge_quest_records(pages: &[QuestPageFacts]) -> QuestLookup {
    let mut merged = QuestLookup::new();
    for page in pages {
        if let Some(own) = page.quests.get(&page.quest_id) {
            merged.insert(page.quest_id, own.clone());
        }
    }
    for page in pages {
        for (&id, record) in &page.quests {
            fill_in(&mut merged, id, record);
        }
    }
    merged
}

fn read_gatherer_quests(body: &str, quests: &mut QuestLookup) {
    for payload in GATHERER_QUESTS.captures_iter(body) {
        let Ok(payload) = serde_json::from_str::<serde_json::Map<String, Value>>(&payload[1]) else {
            continue;
        };
        for (id, quest) in &payload {
            if let Ok(id) = id.parse() {
                fill_in(quests, id, &to_record(quest));
            }
        }
    }
}

#[derive(Default)]
struct OwnQuestRecord {
    record: QuestRecord,
    env_change: Option<String>,
}

/// The page's own `$.extend(g_quests[id], {...});` line: its restrictions and envChange status.
fn read_own_quest_record(body: &str, quest_id: QuestId) -> OwnQuestRecord {
    let marker = format!("$.extend(g_quests[{quest_id}], ");
    let Some(start) = body.find(&marker) else {
        return OwnQuestRecord::default();
    };
    let rest = &body[start + marker.len()..];
    let line = rest.find('\n').map_or(rest, |end| &rest[..end]);
    let json = line.trim_end().strip_suffix(");").unwrap_or(line);
    match serde_json::from_str::<Value>(json) {
        Ok(quest) => OwnQuestRecord {
            record: to_record(&quest),
            env_change: quest
                .get("envChange")
                .and_then(|change| change.get("status"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        Err(_) => OwnQuestRecord::default(),
    }
}

/// The entities Wowhead escapes in names, replaced one after another with `&amp;` last, so an
/// escaped entity such as `&amp;lt;` decodes once, to `&lt;`.
fn decode_entities(text: &str) -> String {
    APOSTROPHE
        .replace_all(text, "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // Trimmed excerpts of real stored pages: only the markup the parser reads, with the whitespace
    // Wowhead emits around infobox headings.

    fn infobox_column(heading: &str, content: &str) -> String {
        format!("<td>\n<table class=\"infobox-inner-table\">\n<tr class=\"infobox-heading\">\n<th>{heading}</th> </tr>\n<tr>\n<td>\n{content}</td></tr>\n</table>\n</td>\n")
    }

    fn markup_column(heading: &str, markup: &str) -> String {
        let options = "{\n                dbPage: true,            });\n        </script>\n";
        infobox_column(
            heading,
            &format!("<script>\n            WH.markup.printHtml(\"{markup}\", \"infobox-contents-1\", {options}"),
        )
    }

    /// Quest 19 Tharil'zun on Forever: a grouped first row with Forever-new quests.
    pub(crate) fn quest_19() -> String {
        [
            r#"<script>WH.Gatherer.addData(5, 16, {"89":{"name_enus":"The Everstill Bridge","_side":1,"reqclass":0,"reqrace":0}});</script>"#.to_owned(),
            r#"<table class="infobox"><tr>"#.to_owned(),
            infobox_column(
                "Series",
                concat!(
                    r#"<table class="series"><tr><th>1.</th><td><div><a href="/forever/quest=20/blackrock-menace">Blackrock Menace</a><br/>"#,
                    r#"<a href="/forever/quest=98386/althers-mill">Alther&#039;s Mill</a><br/>"#,
                    r#"<span class="icon-horde"><a href="/forever/quest=98387/blackrock-blockade">Blackrock Blockade</a></span></div></td></tr>"#,
                    r#"<tr><th>2.</th><td><div></div></td></tr>"#,
                    r#"<tr><th>3.</th><td><div><span class="icon-alliance-padded"><b>Tharil'zun</b></span></div></td></tr></table> "#,
                ),
            ),
            "</tr></table>".to_owned(),
            r#"<h1 class="heading-size-1">Tharil'zun</h1>Bring Tharil'zun's Head to Marshal Marris.<table class="icon-list">"#.to_owned(),
            r#"<tr data-icon-list-quantity="1"><th></th><td><a href="/forever/item=1260/tharilzuns-head">Tharil'zun's Head</a></td></tr>"#.to_owned(),
            "</table>".to_owned(),
            r#"<h2 class="heading-size-3">Description</h2>"#.to_owned(),
            r#"$.extend(g_quests[19], {"id":19,"level":25,"name":"Tharil'zun","reqrace":4294967372,"side":1,"envChange":{"status":"unchanged","labels":[],"lines":[]}});"#.to_owned(),
            r#"var _ = g_users; _["SomeCommenter"]={"border":0};"#.to_owned(),
        ]
        .join("\n")
    }

    /// Quest 7886 Talismans of Merit, an inherited Era page with every markup panel kind.
    fn quest_7886() -> String {
        [
            r#"<script>WH.Gatherer.addData(5, 4, {"7887":{"name_enus":"Talismans of Merit","_side":0}});</script>"#.to_owned(),
            r#"<table class="infobox"><tr>"#.to_owned(),
            markup_column(
                "Requires Any",
                r#"[div style=\"max-height:10.5em\"][quest=7886 icon=false][br][quest=7887 icon=false][br][\/div]"#,
            ),
            markup_column("Requires", "[quest=7700 icon=false][br]"),
            markup_column("Unlocks", "[quest=7886 icon=false][br][quest=7887 icon=false][br]"),
            "</tr></table>".to_owned(),
        ]
        .join("\n")
    }

    fn quest_92742() -> String {
        [
            "<script>WH.Gatherer.addData(5, 16, {});</script>".to_owned(),
            r#"<table class="infobox"><tr>"#.to_owned(),
            infobox_column(
                "Storyline",
                concat!(
                    "<div class=\"quick-facts-storyline\">\n",
                    "<a class=\"quick-facts-storyline-title\" href=\"https://www.wowhead.com/forever/storyline/toxic-soil-6026\">Toxic Soil</a>\n",
                    "<div class=\"quick-facts-storyline-list\">\n<ol><li class=\"current\"><span>Testing the Wells</span></li>",
                    "<li><a class=\"\" href=\"/forever/quest=92744/murloc-gills\">Murloc Gills</a></li>",
                    "<li><a class=\"\" href=\"/forever/quest=92750/detonation-at-a-distance\">Detonation at a Distance</a></li></ol>\n</div>\n</div>\n",
                ),
            ),
            "</tr></table>".to_owned(),
        ]
        .join("\n")
    }

    fn member(quest_id: QuestId, current: bool) -> SeriesMember {
        SeriesMember { quest_id, current }
    }

    fn row(index: i64, members: Vec<SeriesMember>) -> SeriesRow {
        SeriesRow {
            index,
            members,
            unparsed: false,
        }
    }

    #[test]
    fn series_keeps_every_quest_of_a_row_the_bold_current_quest_and_empty_rows() {
        let page = parse_quest_page(19, &quest_19());
        assert_eq!(
            page.series,
            vec![
                row(1, vec![member(20, false), member(98386, false), member(98387, false)]),
                row(2, vec![]),
                row(3, vec![member(19, true)]),
            ]
        );
        assert_eq!(page.quests[&98386].name.as_deref(), Some("Alther's Mill"));
        assert_eq!(page.quests[&98387].side, Some(Side::Horde));
    }

    #[test]
    fn reads_the_pages_own_record_environment_env_change_side_and_a_race_mask_above_32_bits() {
        let page = parse_quest_page(19, &quest_19());
        assert_eq!(page.environment, PageEnvironment::Forever);
        assert_eq!(page.env_change.as_deref(), Some("unchanged"));
        let own = &page.quests[&19];
        assert_eq!(
            (own.name.as_deref(), own.side, own.races, own.has_objectives),
            (Some("Tharil'zun"), Some(Side::Alliance), Some(4294967372), Some(true))
        );
        assert_eq!(
            (page.quests[&89].name.as_deref(), page.quests[&89].side),
            (Some("The Everstill Bridge"), Some(Side::Alliance))
        );
    }

    #[test]
    fn never_reads_comment_authors() {
        let page = parse_quest_page(19, &quest_19());
        assert!(page.quests.values().all(|record| record.name.as_deref() != Some("SomeCommenter")));
        assert_eq!(page.quests.keys().copied().collect::<Vec<_>>(), vec![19, 20, 89, 98386, 98387]);
    }

    #[test]
    fn reads_markup_panels_by_exact_heading_in_display_order_including_the_pages_own_quest() {
        let page = parse_quest_page(7886, &quest_7886());
        assert_eq!(page.environment, PageEnvironment::Era);
        assert_eq!(
            page.panels,
            BTreeMap::from([
                (MarkupPanel::Requires, vec![7700]),
                (MarkupPanel::RequiresAny, vec![7886, 7887]),
                (MarkupPanel::Unlocks, vec![7886, 7887]),
            ])
        );
        assert!(page.series.is_empty());
    }

    #[test]
    fn reads_a_storyline_with_the_current_quest_in_place() {
        let page = parse_quest_page(92742, &quest_92742());
        assert_eq!(
            page.storyline,
            Some(Storyline {
                id: 6026,
                name: "Toxic Soil".into(),
                quest_ids: vec![92742, 92744, 92750],
            })
        );
    }

    fn objectives_block(objectives: &str) -> String {
        format!(
            "<h1 class=\"heading-size-1\">Q</h1>Text.{objectives}\n<h2 class=\"heading-size-3\">Rewards</h2><table class=\"icon-list\"><tr><td>reward</td></tr></table>"
        )
    }

    #[test]
    fn talk_to_quests_have_no_objectives_whatever_their_reward_lists_say() {
        assert_eq!(has_objectives(&objectives_block("")), Some(false));
    }

    #[test]
    fn items_the_quest_provides_are_not_objectives() {
        let provided_label = "<div class=\"pad\"></div>\n\nProvided item: \n<table class=\"icon-list\">\n<tr><td>A Clue</td></tr>\n</table>";
        assert_eq!(has_objectives(&objectives_block(provided_label)), Some(false));
        let provided_row = "<table class=\"icon-list\">\n<tr><td>Banner (Provided)</td></tr>\n</table>";
        assert_eq!(has_objectives(&objectives_block(provided_row)), Some(false));
        let with_kill = "<table class=\"icon-list\">\n<tr><td>Banner (Provided)</td></tr>\n<tr><td>Headsplitter slain</td></tr>\n</table>";
        assert_eq!(has_objectives(&objectives_block(with_kill)), Some(true));
    }

    #[test]
    fn objectives_are_unknown_without_a_quest_title() {
        assert_eq!(has_objectives("<html></html>"), None);
    }

    #[test]
    fn recognizes_unshipped_placeholder_quests_only() {
        assert!(is_placeholder_name(Some("<NYI> <TXT> Undead Priest Robe")));
        assert!(is_placeholder_name(Some("<nyi><TXT> Centaur Hoofprints")));
        assert!(is_placeholder_name(Some("[Never used]")));
        assert!(!is_placeholder_name(Some("Jim's Song <CHANGE TO GOSSIP>")));
        assert!(!is_placeholder_name(None));
    }

    #[test]
    fn a_quests_own_page_wins_over_what_other_pages_say_about_it() {
        // Quest 19's Series shows 98387 with a Horde icon; 98387's own page says both factions.
        let series = parse_quest_page(19, &quest_19());
        let own = parse_quest_page(98387, r#"$.extend(g_quests[98387], {"name":"Blackrock Blockade","side":3});"#);
        let merged = merge_quest_records(&[series, own]);
        assert_eq!(merged[&98387].side, Some(Side::Both));
        assert_eq!(merged[&98386].name.as_deref(), Some("Alther's Mill"));
    }

    #[test]
    fn an_unparsable_series_row_stays_as_an_empty_flagged_row_so_its_neighbours_are_not_adjacent() {
        let body = concat!(
            r#"<table class="series"><tr><th>1.</th><td><div><a href="/forever/quest=1/a">A</a></div></td></tr>"#,
            r#"<tr class="unexpected"><th>2.</th><td colspan="2"><div><a href="/forever/quest=2/b">B</a></div></td></tr>"#,
            r#"<tr><th>3.</th><td><div><b>C</b></div></td></tr></table>"#,
        );
        assert_eq!(
            parse_series(body, 3, &mut QuestLookup::new()),
            vec![
                row(1, vec![member(1, false)]),
                SeriesRow {
                    index: 2,
                    members: vec![],
                    unparsed: true
                },
                row(3, vec![member(3, true)]),
            ]
        );
    }
}
