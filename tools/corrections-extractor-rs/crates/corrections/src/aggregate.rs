//! Merging the observations of one field into one fact per entity. Ports `extract/aggregate.ts`.
//!
//! Default (scalar) policy: highest confidence wins; ties go to the most observed value, then
//! to the most recently observed one, then to the value seen first. Fields whose merge is not
//! "pick one winner" keep every observation and merge them themselves.

use crate::observation::{Confidence, Observation, SessionId, Sessions};
use indexmap::IndexMap;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// One merged value per entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Fact<T> {
    pub value: T,
    pub confidence: Confidence,
    pub observation_count: usize,
    /// Distinct values seen and not picked (scalar merge only; 0 for custom merges).
    pub alternative_count: usize,
}

pub fn highest_confidence<T>(observations: &[Observation<T>]) -> Confidence {
    observations.iter().map(|o| o.confidence).max().unwrap_or(Confidence::Low).max(Confidence::Low)
}

/// A number as a scalar value: compared like `JSON.stringify` keys, so `-0` equals `0`.
#[derive(Clone, Copy, Debug)]
pub struct Num(pub f64);

impl Num {
    fn key(self) -> u64 {
        if self.0.is_nan() {
            f64::NAN.to_bits()
        } else {
            (self.0 + 0.0).to_bits()
        }
    }
}

impl PartialEq for Num {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Num {}

impl Hash for Num {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

/// Everything the scalar merge needs about one value of one entity.
#[derive(Clone, Debug)]
pub struct ScalarGroup<T> {
    pub value: T,
    pub confidence: Confidence,
    pub count: usize,
    pub latest_t: f64,
    /// Distinct sessions with a server time that observed this value.
    pub timed_session_count: usize,
    last_session: SessionId,
    /// Newest server time among those sessions; None when none had one.
    pub newest_session_time: Option<f64>,
}

/// Per entity, the distinct values observed, in the order they were first observed.
pub struct ScalarSummary<T: Eq + Hash> {
    entities: IndexMap<i64, IndexMap<T, ScalarGroup<T>>>,
}

impl<T: Eq + Hash> Default for ScalarSummary<T> {
    fn default() -> Self {
        ScalarSummary { entities: IndexMap::new() }
    }
}

impl<T: Clone + Eq + Hash> ScalarSummary<T> {
    /// aggregate.ts `addToScalarSummary`. Observations of one session must be added together.
    pub fn add(&mut self, observations: impl IntoIterator<Item = Observation<T>>, sessions: &Sessions) {
        for obs in observations {
            let session_time = sessions.server_time(obs.session);
            let groups = self.entities.entry(obs.entity_id).or_default();
            match groups.get_mut(&obs.value) {
                None => {
                    groups.insert(
                        obs.value.clone(),
                        ScalarGroup {
                            value: obs.value,
                            confidence: obs.confidence,
                            count: 1,
                            latest_t: obs.t,
                            timed_session_count: usize::from(session_time.is_some()),
                            last_session: obs.session,
                            newest_session_time: session_time,
                        },
                    );
                }
                Some(group) => {
                    group.count += 1;
                    group.confidence = group.confidence.max(obs.confidence);
                    if obs.t > group.latest_t {
                        group.latest_t = obs.t;
                    }
                    // Observations of one session are added together, so a new id means a new session.
                    if session_time.is_some() && obs.session != group.last_session {
                        group.timed_session_count += 1;
                    }
                    group.last_session = obs.session;
                    if let Some(time) = session_time {
                        if group.newest_session_time.is_none_or(|newest| time > newest) {
                            group.newest_session_time = Some(time);
                        }
                    }
                }
            }
        }
    }

    pub fn entities(&self) -> impl Iterator<Item = (i64, impl Iterator<Item = &ScalarGroup<T>>)> {
        self.entities.iter().map(|(&id, groups)| (id, groups.values()))
    }

    /// aggregate.ts `aggregateScalarSummary`: one fact per entity by the default policy.
    pub fn aggregate(&self) -> IndexMap<i64, Fact<T>> {
        self.entities
            .iter()
            .map(|(&id, groups)| {
                let mut sorted: Vec<&ScalarGroup<T>> = groups.values().collect();
                sorted.sort_by(|a, b| default_order(a, b));
                (id, to_fact(sorted[0], &sorted))
            })
            .collect()
    }

    /// aggregate.ts `aggregateNewestScalarSummary`, for values that change over time (a renamed
    /// NPC): among the values seen in at least two sessions with a server time, the one seen in
    /// the newest session wins; without such a value, the default policy decides.
    pub fn aggregate_newest(&self) -> IndexMap<i64, Fact<T>> {
        self.entities
            .iter()
            .map(|(&id, groups)| {
                let mut all: Vec<&ScalarGroup<T>> = groups.values().collect();
                let mut candidates: Vec<&ScalarGroup<T>> = all.iter().copied().filter(|g| g.timed_session_count >= NEWEST_MIN_SESSIONS).collect();
                // Candidates were seen in timed sessions, so they all have a newest session time.
                candidates.sort_by(|a, b| {
                    let newest = |g: &ScalarGroup<T>| g.newest_session_time.unwrap_or(f64::NAN);
                    newest_first(newest(a), newest(b)).then_with(|| default_order(a, b))
                });
                let winner = match candidates.first() {
                    Some(winner) => *winner,
                    None => {
                        all.sort_by(|a, b| default_order(a, b));
                        all[0]
                    }
                };
                (id, to_fact(winner, &all))
            })
            .collect()
    }
}

/// Sessions a value needs before it can win by being the newest.
const NEWEST_MIN_SESSIONS: usize = 2;

/// Newest first, like the TS comparator `b - a`. A total order, as `sort_by` requires: NaN
/// (never recorded in real traces) sorts as the oldest instead of comparing equal to all.
fn newest_first(a: f64, b: f64) -> Ordering {
    let key = |x: f64| if x.is_nan() { f64::NEG_INFINITY } else { x + 0.0 };
    key(b).partial_cmp(&key(a)).expect("NaN was mapped away")
}

fn default_order<T>(a: &ScalarGroup<T>, b: &ScalarGroup<T>) -> Ordering {
    b.confidence
        .cmp(&a.confidence)
        .then_with(|| b.count.cmp(&a.count))
        .then_with(|| newest_first(a.latest_t, b.latest_t))
}

fn to_fact<T: Clone>(winner: &ScalarGroup<T>, groups: &[&ScalarGroup<T>]) -> Fact<T> {
    Fact {
        value: winner.value.clone(),
        confidence: winner.confidence,
        observation_count: groups.iter().map(|g| g.count).sum(),
        alternative_count: groups.len() - 1,
    }
}

/// Custom-merge fields: every observation, grouped by entity in the order they were added.
pub struct ObservationsByEntity<T> {
    entities: IndexMap<i64, Vec<Observation<T>>>,
}

impl<T> Default for ObservationsByEntity<T> {
    fn default() -> Self {
        ObservationsByEntity { entities: IndexMap::new() }
    }
}

impl<T> ObservationsByEntity<T> {
    pub fn add(&mut self, observations: impl IntoIterator<Item = Observation<T>>) {
        for obs in observations {
            self.entities.entry(obs.entity_id).or_default().push(obs);
        }
    }

    pub fn get(&self, entity_id: i64) -> Option<&[Observation<T>]> {
        self.entities.get(&entity_id).map(Vec::as_slice)
    }

    pub fn iter(&self) -> impl Iterator<Item = (i64, &[Observation<T>])> {
        self.entities.iter().map(|(&id, list)| (id, list.as_slice()))
    }

    /// Every observation, entity by entity.
    pub fn all(&self) -> impl Iterator<Item = &Observation<T>> {
        self.entities.values().flatten()
    }

    /// One fact per entity from `merge` over its observations (aggregateField with a custom merge).
    pub fn merge<R>(&self, mut merge: impl FnMut(&[Observation<T>]) -> R) -> IndexMap<i64, Fact<R>> {
        self.entities
            .iter()
            .map(|(&id, list)| {
                let fact = Fact {
                    value: merge(list),
                    confidence: highest_confidence(list),
                    observation_count: list.len(),
                    alternative_count: 0,
                };
                (id, fact)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(entity: i64, value: &str, confidence: Confidence, t: f64, session: SessionId) -> Observation<String> {
        Observation {
            entity_id: entity,
            value: value.to_owned(),
            confidence,
            t,
            session,
        }
    }

    #[test]
    fn higher_confidence_beats_more_observations() {
        let mut sessions = Sessions::default();
        let s = sessions.add(None, None);
        let mut summary = ScalarSummary::default();
        summary.add(
            [
                obs(1, "low", Confidence::Low, 1.0, s),
                obs(1, "low", Confidence::Low, 2.0, s),
                obs(1, "high", Confidence::High, 3.0, s),
            ],
            &sessions,
        );
        let fact = &summary.aggregate()[&1];
        assert_eq!((fact.value.as_str(), fact.observation_count, fact.alternative_count), ("high", 3, 1));
    }

    #[test]
    fn a_full_tie_goes_to_the_value_seen_first() {
        let mut sessions = Sessions::default();
        let s = sessions.add(None, None);
        let mut summary = ScalarSummary::default();
        summary.add(
            [obs(1, "first", Confidence::Medium, 5.0, s), obs(1, "second", Confidence::Medium, 5.0, s)],
            &sessions,
        );
        assert_eq!(summary.aggregate()[&1].value, "first");
    }

    #[test]
    fn newest_wins_only_with_two_timed_sessions() {
        let mut sessions = Sessions::default();
        let old_a = sessions.add(None, Some(100.0));
        let old_b = sessions.add(None, Some(200.0));
        let new = sessions.add(None, Some(300.0));
        let mut summary = ScalarSummary::default();
        summary.add([obs(1, "Old Name", Confidence::High, 1.0, old_a)], &sessions);
        summary.add([obs(1, "Old Name", Confidence::High, 1.0, old_b)], &sessions);
        summary.add([obs(1, "New Name", Confidence::High, 1.0, new)], &sessions);
        // "New Name" is newest but seen in one session only; "Old Name" qualifies.
        assert_eq!(summary.aggregate_newest()[&1].value, "Old Name");
        let newer = sessions.add(None, Some(400.0));
        summary.add([obs(1, "New Name", Confidence::High, 1.0, newer)], &sessions);
        assert_eq!(summary.aggregate_newest()[&1].value, "New Name");
    }
}
