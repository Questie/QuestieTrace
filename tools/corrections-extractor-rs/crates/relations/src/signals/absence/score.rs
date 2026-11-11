//! Turns an edge's evidence into a calibrated score: the measured chance that such an edge is in
//! Questie's data.
//!
//! score = ceiling(group, tie) × (1 - e^(-4 × evidence)), where evidence is the sum over characters
//! of their best blame share (attribute.rs). Precision stops improving once about one character's
//! worth of clean evidence exists, hence the quick saturation. Ceilings are the precision of
//! candidates with evidence >= 0.5, judged against both ground-truth tiers pooled (2026-10-11,
//! 18.9k Forever sessions, episodes with sessionOrder), smoothed with a Beta(1, 3) prior so a
//! bucket with few edges stays modest. TP/judged per bucket:
//!
//! ```text
//!   group                   strong tie          weak tie        no tie
//!   prerequisite            394/417 -> 0.94     3/8 -> 0.33     0/1 -> 0.20
//!   pendingPrerequisite     Classic 72/83 -> 0.84, Forever 27/63 -> 0.42; weak 2/10 -> 0.21
//!   anyPrerequisite         4/4 -> 0.63         (no data)       (no data)
//!   chain                   32/32 -> 0.92       (no data)       (no data)
//!   breadcrumbs             11/21 -> 0.48       (no data)       (no data)
//!   exclusiveTo             3/3 -> 0.57         0/32 -> 0.03    (no data)
//!   disabledByQuest         0 of 9 edges, any tie -> 0.05
//!   availableUntilCompleted 1/2 -> 0.33         0/2 -> 0.17     (no data)
//! ```
//!
//! prerequisite: some character was blocked while never having taken P. pendingPrerequisite: P was
//! in the log at every block, which is exactly what a breadcrumb nearly everyone does looks like;
//! Forever uses those far more than Classic, hence the split by population. "Strong" ties are
//! same-name variants and observed hand-offs, "weak" ones a shared giver, a shared list or a
//! finisher that starts the other quest (locality.rs). Re-measure whenever the signal changes:
//! `relations score absence` at several --min-score values shows whether score bands still match
//! measured precision.

use super::locality::LocalReason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScoreGroup {
    Prerequisite,
    PendingPrerequisite,
    AnyPrerequisite,
    Chain,
    Breadcrumbs,
    ExclusiveTo,
    DisabledByQuest,
    AvailableUntilCompleted,
}

impl ScoreGroup {
    pub fn name(self) -> &'static str {
        match self {
            ScoreGroup::Prerequisite => "prerequisite",
            ScoreGroup::PendingPrerequisite => "pendingPrerequisite",
            ScoreGroup::AnyPrerequisite => "anyPrerequisite",
            ScoreGroup::Chain => "chain",
            ScoreGroup::Breadcrumbs => "breadcrumbs",
            ScoreGroup::ExclusiveTo => "exclusiveTo",
            ScoreGroup::DisabledByQuest => "disabledByQuest",
            ScoreGroup::AvailableUntilCompleted => "availableUntilCompleted",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tie {
    Strong,
    Weak,
    Absent,
}

impl Tie {
    fn of(reason: Option<LocalReason>) -> Tie {
        match reason {
            None => Tie::Absent,
            Some(reason) if reason.is_strong() => Tie::Strong,
            Some(_) => Tie::Weak,
        }
    }
}

/// Buckets without measurements.
const NO_DATA_CEILING: f64 = 0.1;
const FOREVER_PENDING_PREREQUISITE_CEILING: f64 = 0.42;

fn ceiling(group: ScoreGroup, tie: Tie) -> Option<f64> {
    use ScoreGroup as G;
    match (group, tie) {
        (G::Prerequisite, Tie::Strong) => Some(0.94),
        (G::Prerequisite, Tie::Weak) => Some(0.33),
        (G::Prerequisite, Tie::Absent) => Some(0.2),
        (G::PendingPrerequisite, Tie::Strong) => Some(0.84),
        (G::PendingPrerequisite, Tie::Weak) => Some(0.21),
        (G::AnyPrerequisite, Tie::Strong) => Some(0.63),
        (G::Chain, Tie::Strong) => Some(0.92),
        (G::Breadcrumbs, Tie::Strong) => Some(0.48),
        (G::ExclusiveTo, Tie::Strong) => Some(0.57),
        (G::ExclusiveTo, Tie::Weak) => Some(0.03),
        (G::DisabledByQuest, _) => Some(0.05),
        (G::AvailableUntilCompleted, Tie::Strong) => Some(0.33),
        (G::AvailableUntilCompleted, Tie::Weak) => Some(0.17),
        _ => None,
    }
}

/// `shares`: per character, the best blame share for this edge.
pub fn score_edge(group: ScoreGroup, shares: impl IntoIterator<Item = f64>, reason: Option<LocalReason>, forever_quest: bool) -> f64 {
    let tie = Tie::of(reason);
    let ceiling = if group == ScoreGroup::PendingPrerequisite && forever_quest && tie == Tie::Strong {
        FOREVER_PENDING_PREREQUISITE_CEILING
    } else {
        ceiling(group, tie).unwrap_or(NO_DATA_CEILING)
    };
    let evidence = shares.into_iter().fold(0.0, |sum, share| sum + share);
    ceiling * (1.0 - (-4.0 * evidence).exp())
}
