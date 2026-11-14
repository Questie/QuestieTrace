//! Tunables of the absence signal. Written into the candidate file for reproducibility.

use crate::model::OfferSource;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbsenceParams {
    /// Seconds of log/completion sampling lag to correct for with quest events (see states.rs).
    pub state_slack: f64,
    /// A completed-set delta adding more quests than this is a catch-up read of old completions.
    pub catch_up_batch: usize,
    /// A list read within this many seconds of any dialog/accept/turn-in of X says nothing about X.
    pub self_slack: f64,
    /// Quests that always complete in one batch (Forever's zone copies) are one suspect. A bundle
    /// must be seen in this many batches, and be no larger than `max_bundle`, to rule out bulk adds.
    pub min_bundle_batches: usize,
    pub max_bundle: usize,
    /// Seconds after a turn-in in which an offer at the same giver counts as a hand-off.
    pub handoff_window: f64,
    /// Snapshot sources trusted as complete lists for absence. A standalone QUEST_DETAIL is also how
    /// finishers push a follow-up after a turn-in, so it is not the giver's whole list.
    pub blocked_sources: &'static [OfferSource],
    /// Gossip sub-menus carry no quest list. An empty gossip list read within this many seconds of
    /// a non-empty one from the same giver, with no quest event in between, is taken for one.
    pub sub_menu_window: f64,
    /// Distinct offered races/classes beyond which the observed demographics stop gating.
    pub open_demographics: usize,
    /// Characters allowed to contradict a hypothesis before it is dropped (sampling glitches).
    pub max_contradictions: usize,
    /// Share of offered characters that may lack a quest that still counts as required. One odd
    /// offer (an alternate starter, a stale read) must not erase a prerequisite for everyone.
    pub required_tolerance: f64,
    /// Blame weight of a suspect with no structural tie to the blocked quest (see locality.rs).
    pub unrelated_weight: f64,
    /// Suspects whose prior share of a moment is below this are lumped into the unknown cause.
    pub min_share: f64,
    /// Prior weight of "something we cannot see" as the cause of any blocked moment (attribute.rs).
    pub unknown_weight: f64,
    pub em_iterations: usize,
    /// Candidates scoring below this are left out of the file (they are almost never right).
    pub min_score: f64,
}

impl Default for AbsenceParams {
    fn default() -> Self {
        AbsenceParams {
            state_slack: 2.0,
            catch_up_batch: 60,
            self_slack: 10.0,
            min_bundle_batches: 2,
            max_bundle: 10,
            handoff_window: 30.0,
            blocked_sources: &[OfferSource::Gossip, OfferSource::Greeting],
            sub_menu_window: 60.0,
            open_demographics: 3,
            max_contradictions: 0,
            required_tolerance: 0.1,
            unrelated_weight: 0.02,
            min_share: 0.05,
            unknown_weight: 0.1,
            em_iterations: 30,
            min_score: 0.05,
        }
    }
}
