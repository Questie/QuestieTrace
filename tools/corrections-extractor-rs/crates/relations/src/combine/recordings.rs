//! What combine reads from the recordings themselves, in one pass over the episodes.

use super::contradicted::{Check, Contradicted, Unchecked};
use super::edges::Truth;
use super::takers::{quests_taken, Takers};
use crate::episode_store::Episodes;
use crate::model::QuestEpisode;
use crate::signals::forever_episodes;
use anyhow::Result;
use rayon::prelude::*;

pub struct Recordings {
    /// The episodes the signals read ([`forever_episodes`]).
    pub episodes: usize,
    /// The characters behind them.
    pub characters: usize,
    pub takers: Takers,
    pub contradicted: Vec<Contradicted>,
}

impl Recordings {
    pub fn read(episodes: &dyn Episodes, truth: &Truth, unchecked: &Unchecked) -> Result<Recordings> {
        let mut check = Check::new(truth, unchecked);
        let mut recordings = Recordings {
            episodes: 0,
            characters: 0,
            takers: Takers::default(),
            contradicted: Vec::new(),
        };
        episodes.for_each_bucket(&mut |bucket| {
            let characters: Vec<Vec<&QuestEpisode>> = bucket
                .chunk_by(|a, b| a.contributor_id == b.contributor_id && a.character_key == b.character_key)
                .map(forever_episodes)
                .filter(|sessions| !sessions.is_empty())
                .collect();
            let shown: Vec<_> = characters
                .par_iter()
                .map(|sessions| (quests_taken(sessions), check.shown_by(sessions)))
                .collect();
            for (sessions, (taken, got)) in characters.iter().zip(shown) {
                recordings.episodes += sessions.len();
                recordings.characters += 1;
                recordings.takers.add(sessions.iter().find_map(|session| session.player.faction), taken);
                check.add(&sessions[0].character_key, got);
            }
            Ok(())
        })?;
        recordings.contradicted = check.finish();
        Ok(recordings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combine::contradicted::EVERYTHING_CHECKED;
    use crate::episode_store::{InMemory, PerCharacter};
    use crate::model::{Faction, QuestEventKind};
    use crate::test_support::{event, make_episode};

    #[test]
    fn counts_the_episodes_the_signals_read_and_each_character_once() {
        let episode = |character: &str, interface_version: Option<i64>| QuestEpisode {
            character_key: character.into(),
            interface_version,
            ..make_episode()
        };
        let episodes = [
            episode("a", Some(16001)),
            episode("a", Some(16002)),
            episode("b", None),
            // Old Classic trace: the signals skip it, so the summary must too.
            episode("c", Some(11507)),
        ];

        let read = Recordings::read(&InMemory(&episodes), &Truth::new(), &EVERYTHING_CHECKED).unwrap();
        assert_eq!((read.episodes, read.characters), (3, 2));
    }

    #[test]
    fn a_character_counts_once_as_a_taker_whatever_its_sessions_and_the_buckets() {
        // Two sessions of each of two Alliance characters: two takers, too few to tell the faction.
        let mut episodes = Vec::new();
        for character in ["a", "b"] {
            for _ in 0..2 {
                let mut episode = make_episode();
                episode.character_key = character.into();
                episode.quest_events = vec![event(1.0, QuestEventKind::Accepted, 93835, None)];
                episodes.push(episode);
            }
        }
        for character in ["h1", "h2", "h3"] {
            let mut episode = make_episode();
            episode.character_key = character.into();
            episode.player.faction = Some(Faction::Horde);
            episode.quest_events = vec![event(1.0, QuestEventKind::Accepted, 93836, None)];
            episodes.push(episode);
        }
        let is_other = |episodes: &[QuestEpisode]| {
            let one_bucket = Recordings::read(&InMemory(episodes), &Truth::new(), &EVERYTHING_CHECKED).unwrap();
            let per_character = Recordings::read(&PerCharacter(episodes), &Truth::new(), &EVERYTHING_CHECKED).unwrap();
            let answers = [one_bucket, per_character].map(|read| read.takers.is_for_the_other_faction(93836, 93835));
            assert_eq!(answers[0], answers[1], "whatever the buckets");
            answers[0]
        };
        assert!(!is_other(&episodes));
        let mut third = episodes[0].clone();
        third.character_key = "c".into();
        episodes.insert(4, third);
        assert!(is_other(&episodes));
    }
}
