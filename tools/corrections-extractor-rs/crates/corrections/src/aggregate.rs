//! Merging the observations of one field into one value per entity. Ports `extract/aggregate.ts`.
//!
//! Default (scalar) policy: highest confidence wins; ties go to the most observed value, then to
//! the one of the newest session (server time), then to the most recent time within a session,
//! then to the value seen first. Fields whose merge is not "pick one winner" keep every
//! observation and merge them themselves.

use crate::js;
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
    /// Latest t among the observations of the sessions at newest_session_time; None when that is.
    pub newest_session_t: Option<f64>,
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
    /// `Math.max` of every t: NaN when any is.
    latest_t: f64,
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
                    group.latest_t = js::max(&[group.latest_t, obs.t]);
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
                        latest_t: obs.t,
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
                latest_t: group.latest_t,
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
                        newest_session_t: session_time.map(|_| part.latest_t),
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
                        group.newest_session_t = Some(part.latest_t);
                    } else if group.newest_session_time == Some(time) {
                        let latest = group.newest_session_t.unwrap_or(f64::NEG_INFINITY);
                        group.newest_session_t = Some(js::max(&[latest, part.latest_t]));
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
            .map(|(&id, groups)| (id, newest_winner(groups.values().collect()).value.clone()))
            .collect()
    }

    /// aggregate.ts `aggregateAgreedScalarSummary`, for values that rarely change but are sometimes
    /// observed wrongly (an NPC subname attached to a neighbouring NPC): the value seen in the most
    /// sessions wins, but only when at least [`AGREED_MIN_SESSIONS`] sessions saw it and it has
    /// [`AGREED_MIN_LEAD`] times the sessions of the runner-up. Entities without such a value are
    /// left out. Only sessions with a server time are counted.
    pub fn aggregate_agreed(&self) -> IndexMap<i64, T> {
        self.agreed(false)
    }

    /// Like [`Self::aggregate_agreed`], but a value nothing else was observed against wins even when
    /// it is seen in a single session: nothing contradicts it, and a rare entity would otherwise
    /// get no value.
    pub fn aggregate_agreed_or_uncontested(&self) -> IndexMap<i64, T> {
        self.agreed(true)
    }

    fn agreed(&self, accept_uncontested: bool) -> IndexMap<i64, T> {
        self.entities
            .iter()
            .filter_map(|(&id, groups)| {
                let mut sorted: Vec<&ScalarGroup<T>> = groups.values().collect();
                sorted.sort_by(|a, b| b.timed_session_count.cmp(&a.timed_session_count).then_with(|| default_order(a, b)));
                let winner = sorted[0];
                if accept_uncontested && sorted.len() == 1 {
                    return Some((id, winner.value.clone()));
                }
                let has_lead = sorted
                    .get(1)
                    .is_none_or(|runner_up| winner.timed_session_count >= AGREED_MIN_LEAD * runner_up.timed_session_count);
                (winner.timed_session_count >= AGREED_MIN_SESSIONS && has_lead).then(|| (id, winner.value.clone()))
            })
            .collect()
    }
}

impl ScalarSummary<String> {
    /// aggregate.ts `aggregateNewestNameScalarSummary`: like [`Self::aggregate_newest`], for names
    /// that exist in several variants at once (a creature spawning as male and as female is named
    /// "Nachtläufer" or "Nachtläuferin"): when the newest name has well supported near-identical
    /// variants (directly or through other such variants), the shortest of them wins (alphabetically on equal length), so the result does not
    /// change with whichever session is newest. A name without such variants, such as a rename, is
    /// decided as in [`Self::aggregate_newest`].
    pub fn aggregate_newest_name(&self) -> IndexMap<i64, String> {
        self.entities.iter().map(|(&id, groups)| (id, newest_name(groups.values().collect()))).collect()
    }
}

/// The winner of [`ScalarSummary::aggregate_newest_name`] among one entity's names.
fn newest_name(all: Vec<&ScalarGroup<String>>) -> String {
    let best = all.iter().map(|group| group.timed_session_count).max().unwrap_or(0);
    let is_strong =
        |group: &ScalarGroup<String>| group.timed_session_count >= VARIANT_MIN_SESSIONS && group.timed_session_count as f64 >= best as f64 * VARIANT_MIN_SHARE;
    let winner = newest_winner(all.clone());
    if !is_strong(winner) {
        return winner.value.clone();
    }
    let strong: Vec<(Vec<u16>, &String)> = all
        .into_iter()
        .filter(|group| is_strong(group))
        .map(|group| (utf16(&group.value), &group.value))
        .collect();
    // The variants connected to the winner through any chain of close strong names, so the group is
    // the same whichever of them is newest.
    let mut variants: Vec<usize> = strong.iter().position(|(_, name)| **name == winner.value).into_iter().collect();
    let mut next = 0;
    while next < variants.len() {
        for (at, (name, _)) in strong.iter().enumerate() {
            if !variants.contains(&at) && edit_distance(name, &strong[variants[next]].0) <= VARIANT_MAX_EDIT_DISTANCE {
                variants.push(at);
            }
        }
        next += 1;
    }
    let shortest = variants
        .into_iter()
        .map(|at| &strong[at])
        .min_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    shortest.expect("the winner is one of them").1.clone()
}

/// A JS string's units, which its length, indexing and `<` go by.
fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn edit_distance(a: &[u16], b: &[u16]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, unit) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, other) in b.iter().enumerate() {
            let replaced = previous[j] + usize::from(unit != other);
            row.push(replaced.min(previous[j + 1] + 1).min(row[j] + 1));
        }
        previous = row;
    }
    previous[b.len()]
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

/// Sessions a name needs to count as a variant, and its share of the sessions of the best supported name.
const VARIANT_MIN_SESSIONS: usize = 5;
const VARIANT_MIN_SHARE: f64 = 0.25;
/// Gendered forms of one name differ by an ending or two ("Kanonier"/"Kanonierin").
const VARIANT_MAX_EDIT_DISTANCE: usize = 3;

/// Sessions a value needs before it can win by agreement.
const AGREED_MIN_SESSIONS: usize = 2;
/// How many times more sessions than the runner-up the winning value needs.
const AGREED_MIN_LEAD: usize = 2;

/// Newest first, like the TS comparator `b - a`. A total order, as `sort_by` requires: NaN
/// (never recorded in real traces) sorts as the oldest instead of comparing equal to all.
fn newest_first(a: f64, b: f64) -> Ordering {
    let key = |x: f64| if x.is_nan() { f64::NEG_INFINITY } else { x + 0.0 };
    key(b).partial_cmp(&key(a)).expect("NaN was mapped away")
}

/// The winner of [`ScalarSummary::aggregate_newest`] among one entity's values, of which there is at least one.
fn newest_winner<T>(mut all: Vec<&ScalarGroup<T>>) -> &ScalarGroup<T> {
    let mut candidates: Vec<&ScalarGroup<T>> = all.iter().copied().filter(|g| g.timed_session_count >= NEWEST_MIN_SESSIONS).collect();
    // Candidates were seen in timed sessions, so they all have a newest session time.
    candidates.sort_by(|a, b| newest_first(session_time(a), session_time(b)).then_with(|| default_order(a, b)));
    if let Some(winner) = candidates.first() {
        return winner;
    }
    all.sort_by(|a, b| default_order(a, b));
    all[0]
}

/// Values seen in no session with a server time sort as the oldest.
fn session_time<T>(group: &ScalarGroup<T>) -> f64 {
    group.newest_session_time.unwrap_or(f64::NEG_INFINITY)
}

fn default_order<T>(a: &ScalarGroup<T>, b: &ScalarGroup<T>) -> Ordering {
    b.confidence
        .cmp(&a.confidence)
        .then_with(|| b.count.cmp(&a.count))
        // `latest_t` is relative to the start of its session, so it only compares values of one session.
        .then_with(|| newest_first(session_time(a), session_time(b)))
        // Only the observations of the newest session are comparable; untimed groups fall back to latest_t.
        .then_with(|| match (a.newest_session_t, b.newest_session_t) {
            (Some(a_t), Some(b_t)) => newest_first(a_t, b_t),
            _ => newest_first(a.latest_t, b.latest_t),
        })
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

    /// Entity 1's values after one observation of each: (value, server time, t within its session).
    fn seen_once(values: &[(&str, Option<f64>, f64)]) -> ScalarSummary<String> {
        let mut sessions = Sessions::default();
        let mut summary = ScalarSummary::default();
        for &(value, server_time, t) in values {
            let session = sessions.add(server_time);
            summary.add([obs(1, value, Confidence::High, t, session)], &sessions);
        }
        summary
    }

    #[test]
    fn a_tie_goes_to_the_newest_session_not_to_a_later_time_within_an_older_one() {
        let summary = seen_once(&[("Older", Some(100.0), 3000.0), ("Newer", Some(200.0), 10.0)]);
        assert_eq!(summary.aggregate()[&1], "Newer");
    }

    #[test]
    fn a_tie_goes_to_a_session_with_a_server_time() {
        let summary = seen_once(&[("Untimed", None, 3000.0), ("Timed", Some(200.0), 10.0)]);
        assert_eq!(summary.aggregate()[&1], "Timed");
    }

    #[test]
    fn a_tie_compares_the_time_within_the_newest_session_not_the_latest_time_of_any_session() {
        let summary = seen_once(&[
            ("A", Some(100.0), 3000.0),
            ("A", Some(200.0), 10.0),
            ("B", Some(150.0), 5.0),
            ("B", Some(200.0), 50.0),
        ]);
        assert_eq!(summary.aggregate()[&1], "B");
    }

    #[test]
    fn a_tie_takes_the_latest_time_of_every_session_at_the_newest_server_time() {
        let summary = seen_once(&[
            ("A", Some(200.0), 40.0),
            ("A", Some(200.0), 10.0),
            ("B", Some(200.0), 20.0),
            ("B", Some(200.0), 30.0),
        ]);
        assert_eq!(summary.aggregate()[&1], "A");
    }

    #[test]
    fn a_tie_between_sessions_without_a_server_time_goes_to_the_later_time_within_the_session() {
        let summary = seen_once(&[("Early", None, 10.0), ("Late", None, 20.0)]);
        assert_eq!(summary.aggregate()[&1], "Late");
    }

    /// Entity 1's values, each seen in that many sessions, one session newer than the one before.
    fn seen_in_sessions(values: &[(&str, usize)]) -> ScalarSummary<String> {
        let mut sessions = Sessions::default();
        let mut summary = ScalarSummary::default();
        let mut time = 0.0;
        for &(value, count) in values {
            for _ in 0..count {
                time += 100.0;
                let session = sessions.add(Some(time));
                summary.add([obs(1, value, Confidence::High, 1.0, session)], &sessions);
            }
        }
        summary
    }

    fn newest_name_of(values: &[(&str, usize)]) -> String {
        seen_in_sessions(values).aggregate_newest_name()[&1].clone()
    }

    #[test]
    fn the_shortest_of_the_well_supported_variants_of_a_name_wins_whichever_is_newest() {
        assert_eq!(
            newest_name_of(&[("Kanonier von Stormwind", 8), ("Kanonierin von Stormwind", 10)]),
            "Kanonier von Stormwind"
        );
        assert_eq!(
            newest_name_of(&[("Kanonierin von Stormwind", 10), ("Kanonier von Stormwind", 8)]),
            "Kanonier von Stormwind"
        );
        assert_eq!(newest_name_of(&[("Kanonierin", 10), ("Kanonier", 10), ("Kanoniere", 10)]), "Kanonier");
        assert_eq!(newest_name_of(&[("Wache", 20), ("Wachen", 20)]), "Wache");
    }

    #[test]
    fn the_alphabetically_first_of_two_variants_of_equal_length_wins() {
        assert_eq!(newest_name_of(&[("Magicus", 6), ("Magicum", 6)]), "Magicum");
        assert_eq!(newest_name_of(&[("Magicum", 6), ("Magicus", 6)]), "Magicum");
    }

    #[test]
    fn the_newest_name_wins_over_variants_with_too_few_sessions_or_too_small_a_share() {
        assert_eq!(newest_name_of(&[("Kanonier", 40), ("Kanonierin", 4)]), "Kanonierin");
        assert_eq!(newest_name_of(&[("Kanonier", 100), ("Kanonierin", 10)]), "Kanonierin");
        assert_eq!(
            newest_name_of(&[("Kanonier", 20), ("Kanonierin", 5)]),
            "Kanonier",
            "exactly the minimum of both"
        );
        assert_eq!(
            newest_name_of(&[("Kanonier", 8), ("Kanonierin", 4)]),
            "Kanonierin",
            "enough of a share, too few sessions"
        );
        // A weak variant of a strong newest name is not considered, however short.
        assert_eq!(newest_name_of(&[("Kanonier", 4), ("Kanonierin", 40)]), "Kanonierin");
    }

    #[test]
    fn the_newest_name_wins_over_names_that_are_not_its_variants() {
        assert_eq!(newest_name_of(&[("Axle Smoothslice", 10), ("Edine Sharphorn", 10)]), "Edine Sharphorn");
        assert_eq!(newest_name_of(&[("Wache", 10), ("Wachenab", 10)]), "Wache", "3 edits apart");
        assert_eq!(newest_name_of(&[("Wache", 10), ("Wachenabc", 10)]), "Wachenabc", "4 edits apart");
    }

    #[test]
    fn the_same_variant_of_a_chain_wins_whichever_end_of_the_chain_is_newest() {
        assert_eq!(newest_name_of(&[("Wachenab", 10), ("Wache", 10), ("Wachenabcd", 10)]), "Wache");
        assert_eq!(newest_name_of(&[("Wache", 10), ("Wachenabcd", 10), ("Wachenab", 10)]), "Wache");
    }

    #[test]
    fn a_single_name_wins() {
        assert_eq!(newest_name_of(&[("Kanonier", 1)]), "Kanonier");
    }

    #[test]
    fn the_distance_between_names_counts_the_units_of_a_js_string() {
        let distance = |a: &str, b: &str| edit_distance(&utf16(a), &utf16(b));
        assert_eq!(distance("Wache", "Wache"), 0);
        assert_eq!(distance("", "Wache"), 5);
        assert_eq!(distance("Kanonier", "Kanonierin"), 2);
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("ä", "a"), 1);
        assert_eq!(distance("𝄞", ""), 2, "two units");
    }

    #[test]
    fn agreement_picks_the_value_of_the_most_sessions_not_the_newest() {
        assert_eq!(seen_in_sessions(&[("Tailor", 3), ("Leatherworker", 1)]).aggregate_agreed()[&1], "Tailor");
    }

    /// Not in the TS tests: sessions decide, not how often one session observed a value.
    #[test]
    fn agreement_counts_sessions_not_observations() {
        let mut sessions = Sessions::default();
        let mut summary = ScalarSummary::default();
        let one = sessions.add(Some(100.0));
        summary.add((0..10).map(|t| obs(1, "Leatherworker", Confidence::High, f64::from(t), one)), &sessions);
        for time in [200.0, 300.0, 400.0] {
            let session = sessions.add(Some(time));
            summary.add([obs(1, "Tailor", Confidence::High, 1.0, session)], &sessions);
        }
        assert_eq!(summary.aggregate_agreed()[&1], "Tailor");
    }

    #[test]
    fn agreement_leaves_out_a_value_seen_in_only_one_session() {
        assert!(seen_in_sessions(&[("Tailor", 1)]).aggregate_agreed().is_empty());
    }

    #[test]
    fn agreement_needs_twice_the_sessions_of_the_runner_up() {
        assert_eq!(seen_in_sessions(&[("Tailor", 4), ("Leatherworker", 2)]).aggregate_agreed()[&1], "Tailor");
        assert!(seen_in_sessions(&[("Tailor", 3), ("Leatherworker", 2)]).aggregate_agreed().is_empty());
    }

    #[test]
    fn an_uncontested_value_wins_from_a_single_session_with_or_without_a_server_time() {
        assert_eq!(seen_in_sessions(&[("Tailor", 1)]).aggregate_agreed_or_uncontested()[&1], "Tailor");
        assert_eq!(seen_once(&[("Tailor", None, 1.0)]).aggregate_agreed_or_uncontested()[&1], "Tailor");
    }

    #[test]
    fn a_contested_value_still_needs_agreement() {
        assert!(seen_in_sessions(&[("Tailor", 1), ("Leatherworker", 1)])
            .aggregate_agreed_or_uncontested()
            .is_empty());
        assert!(seen_in_sessions(&[("Tailor", 3), ("Leatherworker", 2)])
            .aggregate_agreed_or_uncontested()
            .is_empty());
    }

    #[test]
    fn agreement_only_counts_sessions_with_a_server_time() {
        let mut sessions = Sessions::default();
        let untimed = sessions.add(None);
        let timed = sessions.add(Some(200.0));
        let mut summary = ScalarSummary::default();
        summary.add(
            [obs(1, "Tailor", Confidence::High, 1.0, untimed), obs(1, "Tailor", Confidence::High, 1.0, timed)],
            &sessions,
        );
        assert!(summary.aggregate_agreed().is_empty());
    }
}
