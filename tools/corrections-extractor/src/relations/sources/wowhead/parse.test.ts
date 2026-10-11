import { describe, expect, it } from "vitest";
import { hasObjectives, isPlaceholderName, mergeQuestRecords, parseQuestPage, parseSeries } from "./parse";

// Trimmed excerpts of real stored pages: only the markup the parser reads, with the whitespace
// Wowhead emits around infobox headings.

function infoboxColumn(heading: string, content: string): string {
  const head = `<td>\n<table class="infobox-inner-table">\n<tr class="infobox-heading">\n<th>${heading}</th> </tr>\n`;
  return `${head}<tr>\n<td>\n${content}</td></tr>\n</table>\n</td>\n`;
}

function markupColumn(heading: string, markup: string): string {
  const options = `{\n                dbPage: true,            });\n        </script>\n`;
  return infoboxColumn(heading, `<script>\n            WH.markup.printHtml("${markup}", "infobox-contents-1", ${options}`);
}

// Quest 19 Tharil'zun on Forever: a grouped first row with Forever-new quests.
const QUEST_19 = [
  `<script>WH.Gatherer.addData(5, 16, {"89":{"name_enus":"The Everstill Bridge","_side":1,"reqclass":0,"reqrace":0}});</script>`,
  `<table class="infobox"><tr>`,
  infoboxColumn(
    "Series",
    `<table class="series"><tr><th>1.</th><td><div><a href="/forever/quest=20/blackrock-menace">Blackrock Menace</a><br/>` +
      `<a href="/forever/quest=98386/althers-mill">Alther&#039;s Mill</a><br/>` +
      `<span class="icon-horde"><a href="/forever/quest=98387/blackrock-blockade">Blackrock Blockade</a></span></div></td></tr>` +
      `<tr><th>2.</th><td><div></div></td></tr>` +
      `<tr><th>3.</th><td><div><span class="icon-alliance-padded"><b>Tharil'zun</b></span></div></td></tr></table> `,
  ),
  `</tr></table>`,
  `<h1 class="heading-size-1">Tharil'zun</h1>Bring Tharil'zun's Head to Marshal Marris.<table class="icon-list">`,
  `<tr data-icon-list-quantity="1"><th></th><td><a href="/forever/item=1260/tharilzuns-head">Tharil'zun's Head</a></td></tr>`,
  `</table>`,
  `<h2 class="heading-size-3">Description</h2>`,
  `$.extend(g_quests[19], {"id":19,"level":25,"name":"Tharil'zun","reqrace":4294967372,"side":1,"envChange":{"status":"unchanged","labels":[],"lines":[]}});`,
  `var _ = g_users; _["SomeCommenter"]={"border":0};`,
].join("\n");

// Quest 7886 Talismans of Merit, an inherited Era page with every markup panel kind.
const QUEST_7886 = [
  `<script>WH.Gatherer.addData(5, 4, {"7887":{"name_enus":"Talismans of Merit","_side":0}});</script>`,
  `<table class="infobox"><tr>`,
  markupColumn("Requires Any", `[div style=\\"max-height:10.5em\\"][quest=7886 icon=false][br][quest=7887 icon=false][br][\\/div]`),
  markupColumn("Requires", `[quest=7700 icon=false][br]`),
  markupColumn("Unlocks", `[quest=7886 icon=false][br][quest=7887 icon=false][br]`),
  `</tr></table>`,
].join("\n");

const QUEST_92742 = [
  `<script>WH.Gatherer.addData(5, 16, {});</script>`,
  `<table class="infobox"><tr>`,
  infoboxColumn(
    "Storyline",
    `<div class="quick-facts-storyline">\n` +
      `<a class="quick-facts-storyline-title" href="https://www.wowhead.com/forever/storyline/toxic-soil-6026">Toxic Soil</a>\n` +
      `<div class="quick-facts-storyline-list">\n<ol><li class="current"><span>Testing the Wells</span></li>` +
      `<li><a class="" href="/forever/quest=92744/murloc-gills">Murloc Gills</a></li>` +
      `<li><a class="" href="/forever/quest=92750/detonation-at-a-distance">Detonation at a Distance</a></li></ol>\n</div>\n</div>\n`,
  ),
  `</tr></table>`,
].join("\n");

describe("parseQuestPage", () => {
  it("keeps every quest of a Series row, fills in the bold current quest, and keeps empty rows", () => {
    const page = parseQuestPage(19, QUEST_19);
    expect(page.series).toEqual([
      { index: 1, members: [{ questId: 20, current: false }, { questId: 98386, current: false }, { questId: 98387, current: false }] },
      { index: 2, members: [] },
      { index: 3, members: [{ questId: 19, current: true }] },
    ]);
    expect(page.quests.get(98386)?.name).toBe("Alther's Mill");
    expect(page.quests.get(98387)?.side).toBe(2);
  });

  it("reads the page's own record: environment, envChange, side and a race mask above 32 bits", () => {
    const page = parseQuestPage(19, QUEST_19);
    expect(page.environment).toBe("forever");
    expect(page.envChange).toBe("unchanged");
    expect(page.quests.get(19)).toMatchObject({ name: "Tharil'zun", side: 1, races: 4294967372n, hasObjectives: true });
    expect(page.quests.get(89)).toMatchObject({ name: "The Everstill Bridge", side: 1 });
  });

  it("never reads comment authors", () => {
    expect(JSON.stringify([...parseQuestPage(19, QUEST_19).quests.values()], (_, v) => (typeof v === "bigint" ? String(v) : v))).not.toContain(
      "SomeCommenter",
    );
  });

  it("reads markup panels by exact heading, in display order, including the page's own quest", () => {
    const page = parseQuestPage(7886, QUEST_7886);
    expect(page.environment).toBe("era");
    expect(page.panels).toEqual({ Requires: [7700], "Requires Any": [7886, 7887], Unlocks: [7886, 7887] });
    expect(page.series).toEqual([]);
  });

  it("reads a Storyline with the current quest in place", () => {
    const page = parseQuestPage(92742, QUEST_92742);
    expect(page.storyline).toEqual({ id: 6026, name: "Toxic Soil", questIds: [92742, 92744, 92750] });
  });
});

describe("hasObjectives", () => {
  const block = (objectives: string) =>
    `<h1 class="heading-size-1">Q</h1>Text.${objectives}\n<h2 class="heading-size-3">Rewards</h2><table class="icon-list"><tr><td>reward</td></tr></table>`;

  it("is false for talk-to quests, ignoring reward lists after the first section heading", () => {
    expect(hasObjectives(block(""))).toBe(false);
  });

  it("does not count items the quest provides", () => {
    expect(hasObjectives(block(`<div class="pad"></div>\n\nProvided item: \n<table class="icon-list">\n<tr><td>A Clue</td></tr>\n</table>`))).toBe(false);
    expect(hasObjectives(block(`<table class="icon-list">\n<tr><td>Banner (Provided)</td></tr>\n</table>`))).toBe(false);
    expect(hasObjectives(block(`<table class="icon-list">\n<tr><td>Banner (Provided)</td></tr>\n<tr><td>Headsplitter slain</td></tr>\n</table>`))).toBe(true);
  });

  it("is unknown when the page has no quest title", () => {
    expect(hasObjectives("<html></html>")).toBeUndefined();
  });
});

describe("isPlaceholderName", () => {
  it("recognizes unshipped placeholder quests only", () => {
    expect(isPlaceholderName("<NYI> <TXT> Undead Priest Robe")).toBe(true);
    expect(isPlaceholderName("<nyi><TXT> Centaur Hoofprints")).toBe(true);
    expect(isPlaceholderName("[Never used]")).toBe(true);
    expect(isPlaceholderName("Jim's Song <CHANGE TO GOSSIP>")).toBe(false);
    expect(isPlaceholderName(undefined)).toBe(false);
  });
});

describe("mergeQuestRecords", () => {
  it("prefers a quest's own page over what other pages say about it", () => {
    // Quest 19's Series shows 98387 with a Horde icon; 98387's own page says both factions.
    const series = parseQuestPage(19, QUEST_19);
    const own = parseQuestPage(98387, `$.extend(g_quests[98387], {"name":"Blackrock Blockade","side":3});`);
    const merged = mergeQuestRecords([series, own]);
    expect(merged.get(98387)?.side).toBe(3);
    expect(merged.get(98386)?.name).toBe("Alther's Mill");
  });
});

describe("parseSeries", () => {
  it("keeps a row it cannot parse as an empty, flagged row so its neighbours are not adjacent", () => {
    const body =
      `<table class="series"><tr><th>1.</th><td><div><a href="/forever/quest=1/a">A</a></div></td></tr>` +
      `<tr class="unexpected"><th>2.</th><td colspan="2"><div><a href="/forever/quest=2/b">B</a></div></td></tr>` +
      `<tr><th>3.</th><td><div><b>C</b></div></td></tr></table>`;
    expect(parseSeries(body, 3)).toEqual([
      { index: 1, members: [{ questId: 1, current: false }] },
      { index: 2, members: [], unparsed: true },
      { index: 3, members: [{ questId: 3, current: true }] },
    ]);
  });
});
