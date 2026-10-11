// Reads the quest-relation surfaces of one stored Wowhead quest page (English, stripped HTML
// from scraper-questie's raw cache). Pure string -> facts; interpretation lives in ./interpret.ts.
//
// Surfaces, all inside the infobox:
//
//   Series                 <table class="series">: numbered rows, each holding one or more quests
//                          separated by <br/>. The page's own quest is <b>bold</b> without a link.
//   Requires, Requires Any, Requires In Progress, Unlocks, Disables
//                          [quest=N] markup inside WH.markup.printHtml("...") under that heading.
//   Storyline              <ol> of quests; the page's own quest is <li class="current">.
//
// Only these panels and the Gatherer quest payloads are read. Pages also embed comment authors
// (g_users); never extract anything outside the surfaces above.

/** Wowhead `side`: 1 Alliance, 2 Horde, 3 both. Missing means unknown. */
export type Side = 1 | 2 | 3;

export const MARKUP_PANELS = ["Requires", "Requires Any", "Requires In Progress", "Unlocks", "Disables"] as const;
export type MarkupPanel = (typeof MARKUP_PANELS)[number];

export interface SeriesMember {
  questId: number;
  /** The page's own quest (rendered bold, without a link). */
  current: boolean;
}

export interface SeriesRow {
  /** The row number Wowhead prints ("1.", "2.", ...), or the row's position when it was unparsed. */
  index: number;
  members: SeriesMember[];
  /** The row's markup was not the expected shape; it is kept, empty, so it still breaks adjacency. */
  unparsed?: true;
}

export interface Storyline {
  id: number;
  name: string;
  /** Quest ids in display order, including the page's own quest. */
  questIds: number[];
}

/**
 * Which Wowhead environment rendered the stored page, from its Gatherer data calls.
 * - forever: dataEnv 16, fetched from Wowhead's /forever pages.
 * - era: dataEnv 4. scraper-questie seeded Forever from the Era cache and only refetches pages
 *   whose sitemap entry changed, so these are inherited Era pages, not Forever evidence.
 */
export type PageEnvironment = "forever" | "era" | "unknown";

export interface QuestPageFacts {
  questId: number;
  environment: PageEnvironment;
  /** envChange.status on the page's own g_quests record (new, updated, unchanged), if present. */
  envChange?: string;
  /** Rows of the Series panel, in order; empty when the page has none. A row may have no members. */
  series: SeriesRow[];
  /** Quest ids per markup panel present on the page. */
  panels: Partial<Record<MarkupPanel, number[]>>;
  storyline?: Storyline;
  /**
   * What the page says about quests, keyed by id: the page's own g_quests record, Gatherer
   * payloads, and names/faction icons from Series and Storyline links.
   */
  quests: Map<number, QuestRecord>;
}

/** Facts used to tell variants, lead-ins and impossible edges apart. */
export interface QuestRecord {
  name?: string;
  side?: Side;
  /** Wowhead reqrace: a 64-bit race mask (Forever races use bits above 32). 0n or missing = any. */
  races?: bigint;
  /** Wowhead reqclass: class mask. 0 or missing = any. */
  classes?: number;
  /**
   * Whether the quest's own page lists objectives (kill/collect/use rows, not provided items).
   * False for "go and talk to X" quests, which is what breadcrumbs look like. Only set from the
   * quest's own page.
   */
  hasObjectives?: boolean;
}

const GATHERER_ENV_RE = /WH\.Gatherer\.addData\(\d+, (\d+),/g;
const GATHERER_QUESTS_RE = /WH\.Gatherer\.addData\(5, \d+, (\{.*\})\);/g;
const MARKUP_QUEST_RE = /\[quest=(\d+)/g;

export function parseQuestPage(questId: number, body: string): QuestPageFacts {
  const quests = new Map<number, QuestRecord>();
  readGathererQuests(body, quests);
  const own = readOwnQuestRecord(body, questId);
  const ownRecord: QuestRecord = { ...quests.get(questId), ...own.record };
  const objectives = hasObjectives(body);
  if (objectives !== undefined) ownRecord.hasObjectives = objectives;
  quests.set(questId, ownRecord);

  const series = parseSeries(body, questId, quests);
  const storyline = parseStoryline(body, questId, quests);

  const panels: QuestPageFacts["panels"] = {};
  for (const panel of MARKUP_PANELS) {
    const ids = markupPanelQuestIds(body, panel);
    if (ids !== undefined) panels[panel] = ids;
  }

  return { questId, environment: pageEnvironment(body), envChange: own.envChange, series, panels, storyline, quests };
}

export function pageEnvironment(body: string): PageEnvironment {
  const envs = new Set([...body.matchAll(GATHERER_ENV_RE)].map((match) => match[1]));
  if (envs.has("16")) return "forever";
  if (envs.has("4")) return "era";
  return "unknown";
}

/**
 * True when the objectives block (between the quest title and the first section heading) has an
 * objective row. Items the quest hands out (rows marked "(Provided)", or the table after a
 * "Provided item:" label) are not objectives.
 */
export function hasObjectives(body: string): boolean | undefined {
  const title = body.indexOf('<h1 class="heading-size-1">');
  if (title < 0) return undefined;
  const end = body.indexOf("<h2", title);
  const block = body.slice(title, end < 0 ? undefined : end);
  for (const table of block.matchAll(/<table class="icon-list">([\s\S]*?)<\/table>/g)) {
    if (/Provided item:\s*$/.test(block.slice(0, table.index))) continue;
    for (const row of table[1].matchAll(/<tr[^>]*>([\s\S]*?)<\/tr>/g)) {
      if (!row[1].includes("(Provided)")) return true;
    }
  }
  return false;
}

// --------------------------------------------------------------------------- Series

/**
 * Parses the Series table, filling in the page's own quest id for the bold member. Link names and
 * faction icons are added to `quests` when given.
 */
export function parseSeries(body: string, questId: number, quests?: Map<number, QuestRecord>): SeriesRow[] {
  const table = /<table class="series">([\s\S]*?)<\/table>/.exec(body);
  if (!table) return [];

  const rows: SeriesRow[] = [];
  for (const tr of table[1].matchAll(/<tr[^>]*>([\s\S]*?)<\/tr>/g)) {
    const row = /^<th>(\d+)\.<\/th><td>([\s\S]*)<\/td>$/.exec(tr[1]);
    if (!row) {
      rows.push({ index: rows.length + 1, members: [], unparsed: true });
      continue;
    }
    const members: SeriesMember[] = [];
    for (const part of row[2].split(/<br\s*\/?>/)) {
      const member = parseSeriesMember(part, questId, quests);
      if (member) members.push(member);
    }
    // Rows can be empty (a step Wowhead does not show in this environment, e.g. TBC quests on
    // Era pages). Keep them so the chain is not stitched together across the gap.
    rows.push({ index: Number(row[1]), members });
  }
  return rows;
}

function parseSeriesMember(html: string, questId: number, quests?: Map<number, QuestRecord>): SeriesMember | undefined {
  const side: Side | undefined = /class="icon-alliance/.test(html) ? 1 : /class="icon-horde/.test(html) ? 2 : undefined;
  const link = /<a href="[^"]*\/quest=(\d+)[^"]*">([^<]*)<\/a>/.exec(html);
  if (link) {
    const id = Number(link[1]);
    if (quests) fillIn(quests, id, { name: decodeEntities(link[2]), side });
    return { questId: id, current: false };
  }
  if (/<b>/.test(html)) {
    if (quests) fillIn(quests, questId, { side });
    return { questId, current: true };
  }
  return undefined;
}

// --------------------------------------------------------------------------- Markup panels

/**
 * Quest ids in the markup panel under `<th>{heading}</th>`, in display order, or undefined when the
 * page has no such panel. The heading must match exactly ("Requires" is not "Requires Any").
 */
export function markupPanelQuestIds(body: string, heading: MarkupPanel): number[] | undefined {
  const section = infoboxSection(body, heading);
  if (section === undefined) return undefined;
  const markup = /WH\.markup\.printHtml\("((?:[^"\\]|\\.)*)"/.exec(section);
  if (!markup) return [];
  return [...markup[1].matchAll(MARKUP_QUEST_RE)].map((match) => Number(match[1]));
}

/** The infobox column under `<th>{heading}</th>`, up to the next infobox heading. */
function infoboxSection(body: string, heading: string): string | undefined {
  const marker = `<tr class="infobox-heading">\n<th>${heading}</th>`;
  const start = body.indexOf(marker);
  if (start < 0) return undefined;
  const contentStart = start + marker.length;
  const next = body.indexOf('<tr class="infobox-heading">', contentStart);
  return body.slice(contentStart, next < 0 ? undefined : next);
}

// --------------------------------------------------------------------------- Storyline

export function parseStoryline(body: string, questId: number, quests?: Map<number, QuestRecord>): Storyline | undefined {
  const section = infoboxSection(body, "Storyline");
  if (section === undefined) return undefined;
  const title = /<a class="quick-facts-storyline-title" href="[^"]*-(\d+)">([^<]*)<\/a>/.exec(section);
  const list = /<ol>([\s\S]*?)<\/ol>/.exec(section);
  if (!title || !list) return undefined;

  const questIds: number[] = [];
  for (const item of list[1].matchAll(/<li( class="current")?>([\s\S]*?)<\/li>/g)) {
    if (item[1]) {
      questIds.push(questId);
      continue;
    }
    const link = /<a [^>]*href="[^"]*\/quest=(\d+)[^"]*">([^<]*)<\/a>/.exec(item[2]);
    if (!link) continue;
    questIds.push(Number(link[1]));
    if (quests) fillIn(quests, Number(link[1]), { name: decodeEntities(link[2]) });
  }
  return { id: Number(title[1]), name: decodeEntities(title[2]), questIds };
}

// --------------------------------------------------------------------------- Quest records

/** The quest fields Wowhead uses in both g_quests records and Gatherer type-5 payloads. */
interface WowheadQuest {
  name?: string;
  name_enus?: string;
  side?: number;
  _side?: number;
  reqrace?: number;
  reqclass?: number;
  envChange?: { status?: string };
}

function toRecord(quest: WowheadQuest): QuestRecord {
  const record: QuestRecord = {};
  const name = quest.name ?? quest.name_enus;
  if (name !== undefined) record.name = name;
  const side = toSide(quest.side ?? quest._side);
  if (side !== undefined) record.side = side;
  // Masks above 2^53 would already have lost precision in JSON.parse; Wowhead's use bits <= 34.
  if (typeof quest.reqrace === "number" && quest.reqrace > 0) record.races = BigInt(quest.reqrace);
  if (typeof quest.reqclass === "number" && quest.reqclass > 0) record.classes = quest.reqclass;
  return record;
}

/** Placeholder quests Blizzard never shipped ("<NYI>", "<TXT>", "[Never used]", "[DNT]"). They have no starters. */
export function isPlaceholderName(name: string | undefined): boolean {
  return name !== undefined && /<\s*(nyi|txt)\b|^\[(never used|dnt)\]/i.test(name);
}

/**
 * One quest directory across many pages. A quest's own page record wins; facts other pages show
 * about it (Gatherer payloads, Series names and icons) only fill gaps.
 */
export function mergeQuestRecords(pages: Iterable<QuestPageFacts>): Map<number, QuestRecord> {
  const merged = new Map<number, QuestRecord>();
  const all = [...pages];
  for (const page of all) {
    const own = page.quests.get(page.questId);
    if (own) merged.set(page.questId, { ...own });
  }
  for (const page of all) {
    for (const [id, record] of page.quests) fillIn(merged, id, record);
  }
  return merged;
}

/** Adds facts without overwriting what an earlier, more authoritative source already set. */
function fillIn(quests: Map<number, QuestRecord>, id: number, facts: QuestRecord): void {
  const record = quests.get(id) ?? {};
  for (const [key, value] of Object.entries(facts) as Array<[keyof QuestRecord, QuestRecord[keyof QuestRecord]]>) {
    if (value !== undefined && record[key] === undefined) (record as Record<string, unknown>)[key] = value;
  }
  quests.set(id, record);
}

function readGathererQuests(body: string, quests: Map<number, QuestRecord>): void {
  for (const match of body.matchAll(GATHERER_QUESTS_RE)) {
    let payload: Record<string, WowheadQuest>;
    try {
      payload = JSON.parse(match[1]) as Record<string, WowheadQuest>;
    } catch {
      continue;
    }
    for (const [id, quest] of Object.entries(payload)) fillIn(quests, Number(id), toRecord(quest));
  }
}

/** The page's own `$.extend(g_quests[id], {...});` line: its restrictions and envChange status. */
function readOwnQuestRecord(body: string, questId: number): { record?: QuestRecord; envChange?: string } {
  const marker = `$.extend(g_quests[${questId}], `;
  const start = body.indexOf(marker);
  if (start < 0) return {};
  const end = body.indexOf("\n", start);
  const line = body.slice(start + marker.length, end < 0 ? undefined : end).replace(/\);\s*$/, "");
  try {
    const quest = JSON.parse(line) as WowheadQuest;
    return { record: toRecord(quest), envChange: quest.envChange?.status };
  } catch {
    return {};
  }
}

function toSide(value: number | undefined): Side | undefined {
  return value === 1 || value === 2 || value === 3 ? value : undefined;
}

function decodeEntities(text: string): string {
  return text
    .replace(/&#0*39;|&apos;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");
}
