//! Resolves legacy QUEST_GREETING titles to quest ids.
//!
//! Older traces record a greeting list as titles only (GetAvailableTitle per index). Every
//! submission also pairs titles with quest ids elsewhere: GetTitleText + GetQuestID in quest
//! dialogs, gossip list entries, and quest log rows. A title resolves when those pairings name
//! exactly one quest, first among pairings seen at the same giver, then among everything the same
//! faction saw in that locale.
//!
//! Titles are only held here transiently; episodes keep just the ones left unresolved, and only
//! when several contributors saw them: older traces did not sanitize greeting titles, and a title
//! that names the player is unique to that player.

use super::distill::{PendingGreeting, TitleObservation};
use crate::model::{Faction, GiverRef, OfferSnapshot, OfferedQuest, QuestId};
use crate::text::js_trim;
use indexmap::IndexSet;
use rustc_hash::FxHashMap;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GreetingStats {
    pub greetings: u64,
    pub complete_greetings: u64,
    pub titles_resolved: u64,
    pub titles_unresolved: u64,
    /// Unresolved titles left out of episodes because fewer than [`SHARED_BY`] contributors saw them.
    pub titles_withheld: u64,
    pub active_resolved: u64,
    pub active_unresolved: u64,
}

/// Contributors that must have seen a title before it may appear in an episode.
const SHARED_BY: usize = 2;

/// The quests one title was paired with.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Names {
    One(QuestId),
    /// Two or more quests: the title is ambiguous.
    Several,
}

impl Names {
    fn with(self, quest_id: QuestId) -> Names {
        match self {
            Names::One(id) if id == quest_id => self,
            _ => Names::Several,
        }
    }

    fn only(self) -> Option<QuestId> {
        match self {
            Names::One(id) => Some(id),
            Names::Several => None,
        }
    }
}

/// Everything known about one title in one locale.
#[derive(Default)]
struct TitleEntry {
    /// Pairings per observing faction (same-titled Alliance and Horde quests are common).
    by_faction: Vec<(Option<Faction>, Names)>,
    /// Pairings seen at a known giver.
    at_giver: Vec<(GiverRef, Names)>,
    /// Contributors who saw the title anywhere, up to [`SHARED_BY`].
    seen_by: Vec<String>,
}

fn pair<K: PartialEq>(pairings: &mut Vec<(K, Names)>, key: K, quest_id: QuestId) {
    match pairings.iter_mut().find(|(k, _)| *k == key) {
        Some((_, names)) => *names = names.with(quest_id),
        None => pairings.push((key, Names::One(quest_id))),
    }
}

fn lookup<K: PartialEq>(pairings: &[(K, Names)], key: &K) -> Option<Names> {
    pairings.iter().find(|(k, _)| k == key).map(|&(_, names)| names)
}

/// Title -> quest pairings of every submission, by locale and trimmed title.
#[derive(Default)]
pub struct TitleIndex {
    locales: FxHashMap<String, FxHashMap<String, TitleEntry>>,
}

/// Titles are only comparable within one known locale: None for a missing or empty one.
fn known(locale: Option<&str>) -> Option<&str> {
    locale.filter(|locale| !locale.is_empty())
}

impl TitleIndex {
    fn entry(&mut self, locale: Option<&str>, title: &str) -> Option<&mut TitleEntry> {
        let locale = known(locale)?;
        if !self.locales.contains_key(locale) {
            self.locales.insert(locale.to_owned(), FxHashMap::default());
        }
        let titles = self.locales.get_mut(locale).expect("inserted above");
        let title = normalize_title(title);
        if !titles.contains_key(title) {
            titles.insert(title.to_owned(), TitleEntry::default());
        }
        titles.get_mut(title)
    }

    fn get(&self, locale: Option<&str>, title: &str) -> Option<&TitleEntry> {
        self.locales.get(known(locale)?)?.get(normalize_title(title))
    }

    /// Records that a contributor saw a title anywhere: dialog, gossip list, quest log or greeting.
    /// A submission without a contributor id cannot tell who saw it, so it does not count (TS
    /// counted the empty id as one more contributor).
    pub fn note_sighting(&mut self, locale: Option<&str>, title: &str, contributor_id: &str) {
        if contributor_id.is_empty() {
            return;
        }
        let Some(entry) = self.entry(locale, title) else { return };
        if entry.seen_by.len() < SHARED_BY && !entry.seen_by.iter().any(|seen| seen == contributor_id) {
            entry.seen_by.push(contributor_id.to_owned());
        }
    }

    /// Whether enough contributors saw the title that it cannot be text about one player.
    pub fn is_shared(&self, locale: Option<&str>, title: &str) -> bool {
        self.get(locale, title).is_some_and(|entry| entry.seen_by.len() >= SHARED_BY)
    }

    /// Adds a pairing a session saw; `locale` and `faction` are the session's.
    pub fn add(&mut self, locale: Option<&str>, faction: Option<Faction>, observation: &TitleObservation) {
        let Some(entry) = self.entry(locale, &observation.title) else { return };
        pair(&mut entry.by_faction, faction, observation.quest_id);
        if let Some(giver) = observation.giver {
            pair(&mut entry.at_giver, giver, observation.quest_id);
        }
    }

    /// The single quest this title names (at this giver, else for this locale and faction); None
    /// when unknown or ambiguous. A title known at the giver is never looked up more widely.
    pub fn resolve(&self, locale: Option<&str>, faction: Option<Faction>, giver: Option<GiverRef>, title: &str) -> Option<QuestId> {
        let entry = self.get(locale, title)?;
        if let Some(names) = giver.and_then(|giver| lookup(&entry.at_giver, &giver)) {
            return names.only();
        }
        lookup(&entry.by_faction, &faction)?.only()
    }
}

/// Adds the quests behind a greeting's titles to its snapshot (which may already hold quest ids
/// read directly) and decides whether the list is complete.
pub fn resolve_greeting(
    offer: &mut OfferSnapshot,
    pending: &PendingGreeting,
    (locale, faction): (Option<&str>, Option<Faction>),
    index: &TitleIndex,
    stats: &mut GreetingStats,
) {
    let resolve = |title: &str| index.resolve(locale, faction, offer.giver, title);
    let mut ids: Vec<QuestId> = offer.available.iter().map(|quest| quest.id).collect();
    let mut unresolved: Vec<&String> = Vec::new();
    for title in &pending.available {
        match resolve(title) {
            Some(id) => ids.push(id),
            None => unresolved.push(title),
        }
    }
    let mut active: IndexSet<QuestId> = offer.active.iter().flatten().copied().collect();
    for title in &pending.active {
        match resolve(title) {
            Some(id) => {
                active.insert(id);
                stats.active_resolved += 1;
            }
            None => stats.active_unresolved += 1,
        }
    }
    stats.greetings += 1;
    stats.titles_resolved += (pending.available.len() - unresolved.len()) as u64;
    stats.titles_unresolved += unresolved.len() as u64;

    let distinct: IndexSet<QuestId> = ids.iter().copied().collect();
    // A quest read directly keeps its flags; the last entry of a repeated id wins, as in a JS Map.
    let known: FxHashMap<QuestId, OfferedQuest> = offer.available.drain(..).map(|quest| (quest.id, quest)).collect();
    offer.available = distinct
        .iter()
        .map(|&id| {
            known.get(&id).cloned().unwrap_or(OfferedQuest {
                id,
                trivial: None,
                frequency: None,
                repeatable: None,
            })
        })
        .collect();
    offer.active = Some(active.into_iter().collect());
    let shown: Vec<String> = unresolved
        .iter()
        .filter(|title| index.is_shared(locale, title))
        .map(|title| (*title).clone())
        .collect();
    stats.titles_withheld += (unresolved.len() - shown.len()) as u64;
    if !shown.is_empty() {
        offer.unresolved_titles = Some(shown);
    }
    // Two entries resolving to one id means one of them is really another quest.
    offer.list_complete = !pending.incomplete && unresolved.is_empty() && distinct.len() == ids.len() && ids.len() as f64 == pending.expected;
    if offer.list_complete {
        stats.complete_greetings += 1;
    }
}

/// Titles are compared without surrounding (JS) whitespace.
fn normalize_title(title: &str) -> &str {
    js_trim(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GiverKind, OfferSource};

    const GIVER: GiverRef = GiverRef { kind: GiverKind::Npc, id: 240 };

    fn observation(title: &str, quest_id: QuestId, giver: Option<i64>) -> TitleObservation {
        TitleObservation {
            title: title.into(),
            quest_id,
            giver: giver.map(|id| GiverRef { kind: GiverKind::Npc, id }),
        }
    }

    fn index() -> TitleIndex {
        let mut titles = TitleIndex::default();
        let alliance = Some(Faction::Alliance);
        // "Manhunt" names two quests in this locale, but only one at this giver.
        titles.add(Some("enUS"), alliance, &observation("Manhunt", 147, Some(240)));
        titles.add(Some("enUS"), alliance, &observation("Manhunt", 9000, Some(999)));
        titles.add(Some("enUS"), alliance, &observation("Westbrook Garrison Needs Help!", 239, None));
        titles.add(Some("enUS"), Some(Faction::Horde), &observation("Report to Kadrak", 6541, None));
        titles.add(Some("enUS"), alliance, &observation("Threat from Below", 99051, Some(240)));
        titles.add(Some("enUS"), alliance, &observation("Threat from Below", 99052, Some(240)));
        for contributor in ["a", "b"] {
            for title in ["Threat from Below", "Report to Kadrak", "Manhunt"] {
                titles.note_sighting(Some("enUS"), title, contributor);
            }
        }
        // Seen by one contributor only, e.g. an older trace's unsanitized title naming the player.
        titles.note_sighting(Some("enUS"), "Never Seen", "a");
        // A submission without a contributor id is not a second contributor.
        titles.note_sighting(Some("enUS"), "Never Seen", "");
        titles
    }

    fn greeting_offer(available: Vec<OfferedQuest>, active: Vec<QuestId>) -> OfferSnapshot {
        OfferSnapshot {
            t: 1.0,
            source: OfferSource::Greeting,
            giver: Some(GIVER),
            available,
            unresolved_titles: None,
            active: Some(active),
            list_complete: false,
        }
    }

    fn quest(id: QuestId) -> OfferedQuest {
        OfferedQuest {
            id,
            trivial: None,
            frequency: None,
            repeatable: None,
        }
    }

    fn resolve(offer: &mut OfferSnapshot, available: &[&str], expected: usize, active: &[&str]) {
        let pending = PendingGreeting {
            offer: 0,
            available: available.iter().map(|title| title.to_string()).collect(),
            active: active.iter().map(|title| title.to_string()).collect(),
            expected: expected as f64,
            incomplete: false,
        };
        resolve_greeting(
            offer,
            &pending,
            (Some("enUS"), Some(Faction::Alliance)),
            &index(),
            &mut GreetingStats::default(),
        );
    }

    fn greeting(available: &[&str], expected: usize, active: &[&str]) -> OfferSnapshot {
        let mut offer = greeting_offer(vec![], vec![]);
        resolve(&mut offer, available, expected, active);
        offer
    }

    fn ids(offer: &OfferSnapshot) -> Vec<QuestId> {
        offer.available.iter().map(|quest| quest.id).collect()
    }

    #[test]
    fn resolves_titles_at_the_giver_first_then_among_the_same_factions_pairings() {
        let offer = greeting(&["Manhunt", "Westbrook Garrison Needs Help!"], 2, &[]);
        assert_eq!(ids(&offer), [147, 239]);
        assert_eq!((offer.unresolved_titles, offer.list_complete), (None, true));
    }

    #[test]
    fn keeps_titles_that_are_unknown_ambiguous_or_only_seen_by_the_other_faction_unresolved() {
        let offer = greeting(&["Threat from Below", "Report to Kadrak"], 2, &[]);
        assert!(offer.available.is_empty());
        assert_eq!(
            offer.unresolved_titles.as_deref(),
            Some(&["Threat from Below".to_string(), "Report to Kadrak".to_string()][..])
        );
        assert!(!offer.list_complete);
    }

    #[test]
    fn withholds_unresolved_titles_only_one_contributor_ever_saw() {
        let offer = greeting(&["Report to Kadrak", "Never Seen"], 2, &[]);
        assert_eq!(offer.unresolved_titles.as_deref(), Some(&["Report to Kadrak".to_string()][..]));
        assert!(!offer.list_complete);
    }

    #[test]
    fn is_not_complete_when_titles_were_missing_from_the_trace_or_two_titles_map_to_one_quest() {
        assert!(!greeting(&["Manhunt"], 2, &[]).list_complete);
        assert!(!greeting(&["Manhunt", "Manhunt"], 2, &[]).list_complete);
    }

    #[test]
    fn completes_a_list_that_mixes_quest_ids_read_directly_with_titles() {
        let read = OfferedQuest {
            trivial: Some(false),
            ..quest(147)
        };
        let mut offer = greeting_offer(vec![read.clone()], vec![123]);
        resolve(&mut offer, &["Westbrook Garrison Needs Help!"], 2, &[]);
        assert_eq!(offer.available, [read, quest(239)]);
        assert_eq!((offer.active, offer.list_complete), (Some(vec![123]), true));
    }

    #[test]
    fn resolves_the_active_list_too() {
        let offer = greeting(&[], 0, &["Manhunt", "Never Seen"]);
        assert_eq!((offer.active, offer.list_complete), (Some(vec![147]), true));
    }

    #[test]
    fn titles_match_without_surrounding_js_whitespace_and_never_without_a_locale() {
        let titles = index();
        let alliance = Some(Faction::Alliance);
        assert_eq!(titles.resolve(Some("enUS"), alliance, None, "  Westbrook Garrison Needs Help! "), Some(239));
        // JS trim() removes a byte order mark but not U+0085 (Rust's trim() does the reverse).
        assert_eq!(
            titles.resolve(Some("enUS"), alliance, None, "\u{feff}Westbrook Garrison Needs Help!"),
            Some(239)
        );
        assert_eq!(titles.resolve(Some("enUS"), alliance, None, "\u{85}Westbrook Garrison Needs Help!"), None);
        assert_eq!(titles.resolve(Some(""), alliance, None, "Westbrook Garrison Needs Help!"), None);
        assert_eq!(titles.resolve(Some("deDE"), alliance, None, "Westbrook Garrison Needs Help!"), None);
    }
}
