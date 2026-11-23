//! Prerequisites QuestieDB has that nearly every character did first, and a few did not. The game
//! does not let a character skip a prerequisite, so those few were recorded somewhere else (a test
//! realm, a private server), or the game changed while traces were recorded, or QuestieDB is wrong
//! and the prerequisite is optional. The first two hide relations, as the signals drop a
//! prerequisite one character lacked, so a person has to look: this is what the beta and the other
//! servers looked like before ingest left them out (see [`crate::ingest`] and RELATIONS.md, "Beta
//! and other servers").

use super::edges::Truth;
use crate::episode_store::Episodes;
use crate::model::{QuestEpisode, QuestEventKind, QuestId, RelationField, SessionTime};
use crate::signals::forever_episodes;
use anyhow::Result;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// Above this share, a prerequisite is not one everybody needs (several quests open it, or it
/// depends on the character), which is no sign of a bad recording. Also keeps quests few characters
/// got out of it: one of fewer than 50 is above it.
const MAX_SHARE: f64 = 0.02;
/// A completion this soon after a quest was seen counts as before it: the completed list is read
/// after the dialog.
const TOLERANCE_SECONDS: SessionTime = 2.0;

/// A prerequisite of QuestieDB that a few characters could get the quest without.
#[derive(Clone, Debug, PartialEq)]
pub struct Contradicted {
    pub quest: QuestId,
    pub prerequisite: QuestId,
    /// Characters seen able to get the quest, or having it.
    pub characters: usize,
    /// Those of them that had not done the prerequisite, by pseudonymous key.
    pub without: Vec<String>,
}

/// What is left out of the check: prerequisites the traces cannot show, or see as optional.
pub struct Unchecked<'a> {
    /// Repeatable quests never show as completed.
    pub is_repeatable: &'a dyn Fn(QuestId) -> bool,
    /// Whether a signal sees the second quest as an optional lead-in (breadcrumb) to the first.
    pub is_lead_in: &'a dyn Fn(QuestId, QuestId) -> bool,
}

/// Every prerequisite of QuestieDB that some characters contradict, but at most [`MAX_SHARE`] of
/// them; the rarest contradictions first.
pub fn contradicted_prerequisites(episodes: &dyn Episodes, truth: &Truth, unchecked: &Unchecked) -> Result<Vec<Contradicted>> {
    let required = required_quests(truth, unchecked);
    let tracked: FxHashSet<QuestId> = required.values().flatten().copied().collect();
    let mut found: FxHashMap<(QuestId, QuestId), Contradicted> = FxHashMap::default();
    episodes.for_each_bucket(&mut |bucket| {
        let characters: Vec<&[QuestEpisode]> = bucket
            .chunk_by(|a, b| a.contributor_id == b.contributor_id && a.character_key == b.character_key)
            .collect();
        let seen: Vec<Vec<Got>> = characters.par_iter().map(|sessions| quests_got(sessions, &required, &tracked)).collect();
        for (sessions, got) in characters.iter().zip(seen) {
            for Got { quest, prerequisite, had } in got {
                let counts = found.entry((quest, prerequisite)).or_insert_with(|| Contradicted {
                    quest,
                    prerequisite,
                    characters: 0,
                    without: Vec::new(),
                });
                counts.characters += 1;
                if !had {
                    counts.without.push(sessions[0].character_key.clone());
                }
            }
        }
        Ok(())
    })?;
    let share = |found: &Contradicted| found.without.len() as f64 / found.characters as f64;
    let mut contradicted: Vec<Contradicted> = found
        .into_values()
        .filter(|found| !found.without.is_empty() && share(found) <= MAX_SHARE)
        .collect();
    contradicted.sort_by(|a, b| {
        share(a)
            .total_cmp(&share(b))
            .then_with(|| (a.quest, a.prerequisite).cmp(&(b.quest, b.prerequisite)))
    });
    Ok(contradicted)
}

/// Per quest, the quests QuestieDB says must all be done before it: a one-quest preQuestSingle, or
/// the AND list. A quest an exclusiveTo sibling can stand in for is left out, and what `unchecked`
/// names.
fn required_quests(truth: &Truth, unchecked: &Unchecked) -> FxHashMap<QuestId, Vec<QuestId>> {
    let values = |quest: QuestId, field: RelationField| truth.get(&quest).and_then(|set| set.get(&field)).map_or(&[][..], Vec::as_slice);
    truth
        .keys()
        .filter_map(|&quest| {
            let required = match values(quest, RelationField::PreQuestSingle) {
                [] => values(quest, RelationField::PreQuestGroup),
                single @ [_] => single,
                _ => return None,
            };
            let required: Vec<QuestId> = required
                .iter()
                .copied()
                .filter(|&prerequisite| values(prerequisite, RelationField::ExclusiveTo).is_empty())
                .filter(|&prerequisite| !(unchecked.is_repeatable)(prerequisite) && !(unchecked.is_lead_in)(quest, prerequisite))
                .collect();
            (!required.is_empty()).then_some((quest, required))
        })
        .collect()
}

/// A quest one character could get or had, and whether it had done one of its prerequisites.
struct Got {
    quest: QuestId,
    prerequisite: QuestId,
    had: bool,
}

/// A moment in a character's sessions: which session, and the time in it.
type Moment = (usize, SessionTime);

/// The earlier of a quest's moments wins.
fn keep_first(moments: &mut FxHashMap<QuestId, Moment>, quest: QuestId, at: Moment) {
    moments
        .entry(quest)
        .and_modify(|first| *first = if at < *first { at } else { *first })
        .or_insert(at);
}

/// What one character's sessions (oldest first) show about the quests in `required`: those it could
/// get (a giver listed it, its accept dialog opened, it was accepted) or already had (in its log, or
/// completed), at the first moment that shows it, when its completed quests were known then.
fn quests_got(sessions: &[QuestEpisode], required: &FxHashMap<QuestId, Vec<QuestId>>, tracked: &FxHashSet<QuestId>) -> Vec<Got> {
    let (mut got, mut done) = (FxHashMap::default(), FxHashMap::default());
    let sessions = forever_episodes(sessions);
    for (index, session) in sessions.iter().enumerate() {
        let offered = session.offers.iter().flat_map(|offer| offer.available.iter().map(|quest| (quest.id, offer.t)));
        let events = session.quest_events.iter();
        let opened = events.filter(|event| matches!(event.kind, QuestEventKind::Detail | QuestEventKind::Accepted));
        let in_log = session.quest_log.iter().flat_map(|entry| entry.v.iter().map(|&quest| (quest, entry.t)));
        let completed = session.completed.iter();
        // Completed before the recording: before anything in it.
        let initial = completed
            .clone()
            .flat_map(|completed| completed.initial.iter().map(|&quest| (quest, f64::NEG_INFINITY)));
        let opened = opened.map(|event| (event.quest_id, event.t));
        for (quest, t) in offered.chain(opened).chain(in_log).chain(initial.clone()) {
            if required.contains_key(&quest) {
                keep_first(&mut got, quest, (index, t));
            }
        }
        let changes = completed.flat_map(|completed| {
            completed
                .changes
                .iter()
                .flat_map(|change| change.add.iter().flatten().map(|&quest| (quest, change.t)))
        });
        let turned_in = session.quest_events.iter().filter(|event| event.kind == QuestEventKind::TurnedIn);
        for (quest, t) in initial.chain(changes).chain(turned_in.map(|event| (event.quest_id, event.t))) {
            if tracked.contains(&quest) {
                keep_first(&mut done, quest, (index, t));
            }
        }
    }
    let knows_completed = |session: &QuestEpisode| {
        let level = session.levels.first().map_or(0, |level| level.v);
        // Past level 2 an empty list is one that had not loaded.
        session.completed.as_ref().is_some_and(|completed| !completed.initial.is_empty() || level <= 2)
    };
    let mut result = Vec::new();
    for (&quest, &(index, t)) in &got {
        if !knows_completed(sessions[index]) {
            continue;
        }
        for &prerequisite in &required[&quest] {
            let had = done.get(&prerequisite).is_some_and(|&first| first <= (index, t + TOLERANCE_SECONDS));
            result.push(Got { quest, prerequisite, had });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_store::{InMemory, PerCharacter};
    use crate::model::{CompletedChange, RelationSet};
    use crate::test_support::{event, gossip, make_episode, npc};
    use std::collections::BTreeMap;

    const QUEST: QuestId = 91743;
    const PREREQUISITE: QuestId = 92124;

    /// A character that turned the prerequisite in at 100 s and was offered the quest at `offered`.
    fn character(offered: SessionTime) -> QuestEpisode {
        let mut episode = make_episode();
        let completed = episode.completed.as_mut().unwrap();
        completed.initial = vec![7];
        completed.changes = vec![CompletedChange {
            t: 100.0,
            add: Some(vec![PREREQUISITE]),
            remove: None,
        }];
        episode.quest_events = vec![event(100.0, QuestEventKind::TurnedIn, PREREQUISITE, None)];
        episode.offers = vec![gossip(offered, npc(951), &[QUEST], &[])];
        episode
    }

    /// 60 characters that did the prerequisite first, then `others`; the keys of `others`.
    fn with_others(others: Vec<QuestEpisode>) -> (Vec<QuestEpisode>, Vec<String>) {
        let keys = others.iter().map(|episode| episode.character_key.clone()).collect();
        let mut episodes: Vec<QuestEpisode> = (0..60).map(|_| character(200.0)).collect();
        episodes.extend(others);
        (episodes, keys)
    }

    fn found(episodes: &[QuestEpisode], relations: &[(QuestId, RelationField, Vec<QuestId>)]) -> Vec<Contradicted> {
        let mut sets: BTreeMap<QuestId, RelationSet> = BTreeMap::new();
        for (quest, field, values) in relations {
            sets.entry(*quest).or_default().insert(*field, values.clone());
        }
        let truth: Truth = sets.iter().map(|(&quest, set)| (quest, set)).collect();
        let check = |episodes: &dyn Episodes| contradicted_prerequisites(episodes, &truth, &EVERYTHING_CHECKED).unwrap();
        let one_bucket = check(&InMemory(episodes));
        assert_eq!(check(&PerCharacter(episodes)), one_bucket, "whatever the buckets");
        one_bucket
    }

    const EVERYTHING_CHECKED: Unchecked = Unchecked {
        is_repeatable: &|_| false,
        is_lead_in: &|_, _| false,
    };

    fn single() -> Vec<(QuestId, RelationField, Vec<QuestId>)> {
        vec![(QUEST, RelationField::PreQuestSingle, vec![PREREQUISITE])]
    }

    #[test]
    fn one_character_among_many_that_got_the_quest_first_is_found() {
        let (episodes, keys) = with_others(vec![character(50.0)]);
        assert_eq!(
            found(&episodes, &single()),
            [Contradicted {
                quest: QUEST,
                prerequisite: PREREQUISITE,
                characters: 61,
                without: keys,
            }]
        );
    }

    #[test]
    fn nothing_is_found_where_everyone_did_the_prerequisite_first() {
        let (episodes, _) = with_others(vec![]);
        assert_eq!(found(&episodes, &single()), []);
    }

    #[test]
    fn a_turn_in_that_opens_the_quest_in_the_same_moment_is_before_it() {
        let (episodes, _) = with_others(vec![character(99.0)]);
        assert_eq!(found(&episodes, &single()), []);
    }

    #[test]
    fn the_first_time_the_quest_was_seen_decides_not_the_first_list_it_was_on() {
        let mut dialog_first = character(200.0);
        dialog_first.quest_events.insert(0, event(30.0, QuestEventKind::Detail, QUEST, Some(npc(951))));
        let (episodes, keys) = with_others(vec![dialog_first]);
        assert_eq!(found(&episodes, &single())[0].without, keys);
    }

    #[test]
    fn a_prerequisite_completed_before_the_recording_is_before_the_quest() {
        let mut done_before = character(50.0);
        done_before.completed.as_mut().unwrap().initial = vec![7, PREREQUISITE];
        done_before.quest_events.clear();
        let (episodes, _) = with_others(vec![done_before]);
        assert_eq!(found(&episodes, &single()), []);
    }

    #[test]
    fn a_prerequisite_done_in_an_earlier_session_is_before_the_quest() {
        let mut earlier = make_episode();
        earlier.completed.as_mut().unwrap().changes = vec![CompletedChange {
            t: 900.0,
            add: Some(vec![PREREQUISITE]),
            remove: None,
        }];
        let mut later = make_episode();
        later.character_key.clone_from(&earlier.character_key);
        later.session_order = 1;
        later.completed.as_mut().unwrap().initial = vec![7];
        later.offers = vec![gossip(50.0, npc(951), &[QUEST], &[])];
        let (episodes, _) = with_others(vec![earlier, later]);
        assert_eq!(found(&episodes, &single()), []);
    }

    #[test]
    fn what_many_characters_lacked_is_no_bad_recording() {
        let (episodes, _) = with_others(vec![character(50.0), character(50.0)]);
        assert_eq!(found(&episodes, &single()), [], "2 of 62 is more than 2%");
    }

    #[test]
    fn a_quest_in_the_log_or_completed_without_its_prerequisite_is_found() {
        let mut in_log = character(200.0);
        in_log.quest_log = vec![crate::model::TimedValue { t: 0.0, v: vec![QUEST] }];
        let mut completed = character(200.0);
        completed.completed.as_mut().unwrap().initial = vec![7, QUEST];
        // One at a time: two of 62 would be above the share that is reported.
        for (what, episode) in [("in the log", in_log), ("completed", completed)] {
            let (episodes, keys) = with_others(vec![episode]);
            assert_eq!(found(&episodes, &single())[0].without, keys, "{what}");
        }
        let mut both_completed = character(200.0);
        both_completed.completed.as_mut().unwrap().initial = vec![PREREQUISITE, QUEST];
        let (episodes, _) = with_others(vec![both_completed]);
        assert_eq!(found(&episodes, &single()), []);
    }

    #[test]
    fn a_character_whose_completed_quests_are_not_known_says_nothing() {
        let mut no_list = character(50.0);
        no_list.completed = None;
        let mut not_loaded = character(50.0);
        not_loaded.completed.as_mut().unwrap().initial.clear();
        for (what, episode) in [("no completed list", no_list), ("list not loaded", not_loaded.clone())] {
            let (episodes, _) = with_others(vec![episode]);
            assert_eq!(found(&episodes, &single()), [], "{what}");
        }
        // A new character's list really is empty.
        not_loaded.levels[0].v = 1;
        let (episodes, keys) = with_others(vec![not_loaded]);
        assert_eq!(found(&episodes, &single())[0].without, keys);
    }

    #[test]
    fn repeatable_quests_and_lead_ins_are_not_checked() {
        let (episodes, _) = with_others(vec![character(50.0)]);
        let set = RelationSet::from([(RelationField::PreQuestSingle, vec![PREREQUISITE])]);
        let truth: Truth = BTreeMap::from([(QUEST, &set)]);
        let found = |unchecked: &Unchecked| contradicted_prerequisites(&InMemory(&episodes), &truth, unchecked).unwrap().len();
        assert_eq!(found(&EVERYTHING_CHECKED), 1);
        let repeatable = Unchecked {
            is_repeatable: &|quest| quest == PREREQUISITE,
            ..EVERYTHING_CHECKED
        };
        assert_eq!(found(&repeatable), 0);
        let lead_in = Unchecked {
            is_lead_in: &|quest, lead_in| (quest, lead_in) == (QUEST, PREREQUISITE),
            ..EVERYTHING_CHECKED
        };
        assert_eq!(found(&lead_in), 0);
    }

    #[test]
    fn only_quests_everybody_must_have_done_count() {
        let (episodes, keys) = with_others(vec![character(50.0)]);
        use RelationField::{ExclusiveTo, PreQuestGroup, PreQuestSingle};
        // Every quest of an AND list is needed.
        let group = found(&episodes, &[(QUEST, PreQuestGroup, vec![PREREQUISITE])]);
        assert_eq!(group.iter().map(|found| &found.without).collect::<Vec<_>>(), [&keys]);
        // One of an OR list is enough, and preQuestSingle wins over an AND list.
        assert_eq!(found(&episodes, &[(QUEST, PreQuestSingle, vec![PREREQUISITE, 5])]), []);
        assert_eq!(
            found(&episodes, &[(QUEST, PreQuestSingle, vec![7]), (QUEST, PreQuestGroup, vec![PREREQUISITE])]),
            [],
            "7 is done by everyone"
        );
        // Questie takes an exclusive sibling in its place.
        let mut exclusive = single();
        exclusive.push((PREREQUISITE, ExclusiveTo, vec![5]));
        assert_eq!(found(&episodes, &exclusive), []);
    }
}
