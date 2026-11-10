//! Merging the observations of one field into one value per entity. Ports `extract/aggregate.ts`.
//!
//! Default (scalar) policy: highest confidence wins; ties go to the most observed value, then
//! to the most recently observed one, then to the value seen first. Fields whose merge is not
//! "pick one winner" keep every observation and merge them themselves.

use crate::observation::{Confidence, Localized, Observation, SessionId, Sessions};
use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

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

/// One session's observations of a scalar field, merged per entity and value in the order they
/// were first observed. The observing thread builds it, so the fold adds one group per value
/// instead of every observation (a session repeats the same quest level hundreds of times).
pub struct SessionScalars<T> {
    groups: Vec<SessionGroup<T>>,
}

struct SessionGroup<T> {
    entity_id: i64,
    value: T,
    confidence: Confidence,
    count: usize,
    /// The first observation's t, which starts a new group's latest_t.
    first_t: f64,
    /// The largest t that is not NaN; NaN when every t is.
    max_t: f64,
}

impl<T> Default for SessionScalars<T> {
    fn default() -> Self {
        SessionScalars { groups: Vec::new() }
    }
}

impl<T: Clone + Eq + Hash> SessionScalars<T> {
    pub fn new(observations: Vec<Observation<T>>) -> Self {
        let mut positions: HashMap<(i64, T), usize, FxBuildHasher> = HashMap::default();
        let mut groups: Vec<SessionGroup<T>> = Vec::new();
        for obs in observations {
            match positions.entry((obs.entity_id, obs.value)) {
                Entry::Occupied(at) => {
                    let group = &mut groups[*at.get()];
                    group.count += 1;
                    group.confidence = group.confidence.max(obs.confidence);
                    if obs.t > group.max_t || (group.max_t.is_nan() && !obs.t.is_nan()) {
                        group.max_t = obs.t;
                    }
                }
                Entry::Vacant(slot) => {
                    let (entity_id, value) = slot.key().clone();
                    slot.insert(groups.len());
                    groups.push(SessionGroup {
                        entity_id,
                        value,
                        confidence: obs.confidence,
                        count: 1,
                        first_t: obs.t,
                        max_t: obs.t,
                    });
                }
            }
        }
        SessionScalars { groups }
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

impl<T: Clone + Eq + Hash> SessionScalars<Localized<T>> {
    /// Splits translations by locale, keeping the first-observed order within each.
    pub fn by_locale(self) -> impl Iterator<Item = (String, SessionScalars<T>)> {
        let mut by_locale: IndexMap<String, SessionScalars<T>> = IndexMap::new();
        for group in self.groups {
            let Localized { locale, value } = group.value;
            let part = SessionGroup {
                entity_id: group.entity_id,
                value,
                confidence: group.confidence,
                count: group.count,
                first_t: group.first_t,
                max_t: group.max_t,
            };
            by_locale.entry(locale).or_default().groups.push(part);
        }
        by_locale.into_iter()
    }
}

/// Per entity, the distinct values observed, in the order they were first observed.
pub struct ScalarSummary<T: Eq + Hash> {
    entities: IndexMap<i64, IndexMap<T, ScalarGroup<T>, FxBuildHasher>, FxBuildHasher>,
}

impl<T: Eq + Hash> Default for ScalarSummary<T> {
    fn default() -> Self {
        ScalarSummary { entities: IndexMap::default() }
    }
}

impl<T: Clone + Eq + Hash> ScalarSummary<T> {
    /// aggregate.ts `addToScalarSummary`. Observations of one session must be added together.
    pub fn add(&mut self, observations: impl IntoIterator<Item = Observation<T>>, sessions: &Sessions) {
        let mut run: Vec<Observation<T>> = Vec::new();
        for obs in observations {
            if run.last().is_some_and(|last| last.session != obs.session) {
                let session = run[0].session;
                self.add_session(SessionScalars::new(std::mem::take(&mut run)), session, sessions);
            }
            run.push(obs);
        }
        if let Some(first) = run.first() {
            let session = first.session;
            self.add_session(SessionScalars::new(run), session, sessions);
        }
    }

    /// Adds one session's observations, already merged per entity and value.
    pub fn add_session(&mut self, scalars: SessionScalars<T>, session: SessionId, sessions: &Sessions) {
        let session_time = sessions.server_time(session);
        for group in scalars.groups {
            self.add_group(group, session, session_time);
        }
    }

    /// Merges a session's group into the entity's group of the same value, exactly as adding its
    /// observations one by one would.
    fn add_group(&mut self, part: SessionGroup<T>, session: SessionId, session_time: Option<f64>) {
        let groups = self.entities.entry(part.entity_id).or_default();
        match groups.get_mut(&part.value) {
            None => {
                // The first observation sets latest_t; later ones raise it (a NaN first t sticks).
                let latest_t = if part.max_t > part.first_t { part.max_t } else { part.first_t };
                groups.insert(
                    part.value.clone(),
                    ScalarGroup {
                        value: part.value,
                        confidence: part.confidence,
                        count: part.count,
                        latest_t,
                        timed_session_count: usize::from(session_time.is_some()),
                        last_session: session,
                        newest_session_time: session_time,
                    },
                );
            }
            Some(group) => {
                group.count += part.count;
                group.confidence = group.confidence.max(part.confidence);
                if part.max_t > group.latest_t {
                    group.latest_t = part.max_t;
                }
                // A new session id means a new session: one session's groups come together.
                if session_time.is_some() && session != group.last_session {
                    group.timed_session_count += 1;
                }
                group.last_session = session;
                if let Some(time) = session_time {
                    if group.newest_session_time.is_none_or(|newest| time > newest) {
                        group.newest_session_time = Some(time);
                    }
                }
            }
        }
    }

    /// aggregate.ts `aggregateScalarSummary`: one fact per entity by the default policy.
    pub fn aggregate(&self) -> IndexMap<i64, T> {
        self.entities
            .iter()
            .map(|(&id, groups)| {
                let mut sorted: Vec<&ScalarGroup<T>> = groups.values().collect();
                sorted.sort_by(|a, b| default_order(a, b));
                (id, sorted[0].value.clone())
            })
            .collect()
    }

    /// aggregate.ts `aggregateNewestScalarSummary`, for values that change over time (a renamed
    /// NPC): among the values seen in at least two sessions with a server time, the one seen in
    /// the newest session wins; without such a value, the default policy decides.
    pub fn aggregate_newest(&self) -> IndexMap<i64, T> {
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
                (id, winner.value.clone())
            })
            .collect()
    }
}

impl<T: Eq + Hash + crate::memory::HeapSize> ScalarSummary<T> {
    /// (value groups, approximate bytes) held.
    pub fn memory(&self) -> (usize, usize) {
        let mut rows = 0;
        let mut bytes = 0;
        for groups in self.entities.values() {
            rows += groups.len();
            bytes += crate::memory::MAP_ENTRY_OVERHEAD + std::mem::size_of::<(i64, IndexMap<T, ScalarGroup<T>>)>();
            for (key, group) in groups {
                bytes += crate::memory::MAP_ENTRY_OVERHEAD + std::mem::size_of::<(T, ScalarGroup<T>)>() + key.heap_size() + group.value.heap_size();
            }
        }
        (rows, bytes)
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

/// Custom-merge fields: every observation, grouped by entity in the order they were added.
pub struct ObservationsByEntity<T> {
    entities: IndexMap<i64, Vec<Observation<T>>, FxBuildHasher>,
}

impl<T> Default for ObservationsByEntity<T> {
    fn default() -> Self {
        ObservationsByEntity { entities: IndexMap::default() }
    }
}

impl<T: crate::memory::HeapSize> ObservationsByEntity<T> {
    /// (observations, approximate bytes) held.
    pub fn memory(&self) -> (usize, usize) {
        let rows = self.entities.values().map(Vec::len).sum();
        let bytes = self
            .entities
            .values()
            .map(|list| {
                crate::memory::MAP_ENTRY_OVERHEAD
                    + std::mem::size_of::<(i64, Vec<Observation<T>>)>()
                    + list.capacity() * std::mem::size_of::<Observation<T>>()
                    + list.iter().map(|obs| obs.value.heap_size()).sum::<usize>()
            })
            .sum();
        (rows, bytes)
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

    /// One merged value per entity from `merge` over its observations, in the order the
    /// entities were first observed (aggregateField with a custom merge).
    pub fn merge<R>(&self, mut merge: impl FnMut(&[Observation<T>]) -> R) -> IndexMap<i64, R> {
        self.entities.iter().map(|(&id, list)| (id, merge(list))).collect()
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
        let s = sessions.add(None);
        let mut summary = ScalarSummary::default();
        summary.add(
            [
                obs(1, "low", Confidence::Low, 1.0, s),
                obs(1, "low", Confidence::Low, 2.0, s),
                obs(1, "high", Confidence::High, 3.0, s),
            ],
            &sessions,
        );
        assert_eq!(summary.aggregate()[&1], "high");
    }

    #[test]
    fn a_full_tie_goes_to_the_value_seen_first() {
        let mut sessions = Sessions::default();
        let s = sessions.add(None);
        let mut summary = ScalarSummary::default();
        summary.add(
            [obs(1, "first", Confidence::Medium, 5.0, s), obs(1, "second", Confidence::Medium, 5.0, s)],
            &sessions,
        );
        assert_eq!(summary.aggregate()[&1], "first");
    }

    #[test]
    fn newest_wins_only_with_two_timed_sessions() {
        let mut sessions = Sessions::default();
        let old_a = sessions.add(Some(100.0));
        let old_b = sessions.add(Some(200.0));
        let new = sessions.add(Some(300.0));
        let mut summary = ScalarSummary::default();
        summary.add([obs(1, "Old Name", Confidence::High, 1.0, old_a)], &sessions);
        summary.add([obs(1, "Old Name", Confidence::High, 1.0, old_b)], &sessions);
        summary.add([obs(1, "New Name", Confidence::High, 1.0, new)], &sessions);
        // "New Name" is newest but seen in one session only; "Old Name" qualifies.
        assert_eq!(summary.aggregate_newest()[&1], "Old Name");
        let newer = sessions.add(Some(400.0));
        summary.add([obs(1, "New Name", Confidence::High, 1.0, newer)], &sessions);
        assert_eq!(summary.aggregate_newest()[&1], "New Name");
    }
}
