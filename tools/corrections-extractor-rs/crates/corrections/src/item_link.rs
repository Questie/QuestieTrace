//! WoW item hyperlinks as they appear in sessions, e.g.
//! "|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h[|r". Only the itemID and display name
//! are read. Ports `extract/itemLink.ts`, whose pattern is
//! `/\|Hitem:(\d+):[^|]*\|h\[([^\]]*)\]\|h/`.

use crate::js;

#[derive(Clone, PartialEq, Debug)]
pub struct ParsedItemLink<'l> {
    pub item_id: f64,
    pub name: &'l str,
}

/// The first match of the pattern at or after `from`, as (item id digits, name).
fn find(link: &str) -> Option<(&str, &str)> {
    let mut from = 0;
    while let Some(rel) = link[from..].find("|Hitem:") {
        let start = from + rel;
        if let Some(found) = match_at(&link[start + "|Hitem:".len()..]) {
            return Some(found);
        }
        from = start + 1;
    }
    None
}

/// Matches `(\d+):[^|]*\|h\[([^\]]*)\]\|h` at the start of `rest`.
fn match_at(rest: &str) -> Option<(&str, &str)> {
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after_digits = rest[digits..].strip_prefix(':')?;
    // [^|]* is greedy but cannot cross a '|', so it ends at the first one.
    let pipe = after_digits.find('|')?;
    let after_payload = after_digits[pipe..].strip_prefix("|h[")?;
    let close = after_payload.find(']')?;
    after_payload[close..].strip_prefix("]|h")?;
    Some((&rest[..digits], &after_payload[..close]))
}

pub fn parse_item_link(link: Option<&str>) -> Option<ParsedItemLink<'_>> {
    let link = link.filter(|l| !l.is_empty())?;
    let (digits, name) = find(link)?;
    let item_id = js::number(digits);
    item_id.is_finite().then_some(ParsedItemLink { item_id, name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_item_id_and_name() {
        let parsed = parse_item_link(Some("|cffffffff|Hitem:750::::::::1::::::::::|h[Tough Wolf Meat]|h|r")).unwrap();
        assert_eq!((parsed.item_id, parsed.name), (750.0, "Tough Wolf Meat"));
    }

    #[test]
    fn rejects_links_without_the_item_shape() {
        assert_eq!(parse_item_link(Some("|Hspell:133|h[Fireball]|h")), None);
        assert_eq!(parse_item_link(Some("|Hitem:abc:|h[x]|h")), None);
        assert_eq!(parse_item_link(None), None);
    }

    #[test]
    fn a_name_with_a_closing_bracket_is_cut_like_the_regex_does() {
        // [^\]]* stops at the first ']', which must then be followed by "|h".
        assert_eq!(parse_item_link(Some("|Hitem:1:|h[a]b]|h")), None);
    }
}
