// Parses WoW item hyperlinks as they appear in QuestieTrace sessions, e.g.
// "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r".
//
// Only the itemID and display name are extracted - the rest of the link payload
// (enchant/gem/suffix/etc. fields) is not needed for Questie DB fact extraction.

export interface ParsedItemLink {
  itemID: number;
  name: string;
}

const ITEM_LINK_RE = /\|Hitem:(\d+):[^|]*\|h\[([^\]]*)\]\|h/;

export function parseItemLink(link: string | null | undefined): ParsedItemLink | null {
  if (typeof link !== "string" || link.length === 0) {
    return null;
  }

  const match = ITEM_LINK_RE.exec(link);
  if (!match) {
    return null;
  }

  const itemID = Number(match[1]);
  if (!Number.isFinite(itemID)) {
    return null;
  }

  return { itemID, name: match[2] };
}
