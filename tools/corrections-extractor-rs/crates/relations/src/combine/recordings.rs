//! What combine reads from the recordings themselves, in one pass over the episodes.

use super::contradicted::{Check, Contradicted, Got, Unchecked};
use super::edges::Truth;
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
    pub contradicted: Vec<Contradicted>,
}

impl Recordings {
    pub fn read(episodes: &dyn Episodes, truth: &Truth, unchecked: &Unchecked) -> Result<Recordings> {
        let mut check = Check::new(truth, unchecked);
        let mut recordings = Recordings {
            episodes: 0,
            characters: 0,
            contradicted: Vec::new(),
        };
        episodes.for_each_bucket(&mut |bucket| {
            let characters: Vec<Vec<&QuestEpisode>> = bucket
                .chunk_by(|a, b| a.contributor_id == b.contributor_id && a.character_key == b.character_key)
                .map(forever_episodes)
                .filter(|sessions| !sessions.is_empty())
                .collect();
            let shown: Vec<Vec<Got>> = characters.par_iter().map(|sessions| check.shown_by(sessions)).collect();
            for (sessions, got) in characters.iter().zip(shown) {
                recordings.episodes += sessions.len();
                recordings.characters += 1;
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
    use crate::episode_store::InMemory;
    use crate::test_support::make_episode;

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
}
