//! What the fold collects from observed sessions, in fold order: per-value summaries of the
//! scalar fields and every observation of the fields with a custom merge.

use crate::aggregate::ObservationsByEntity;
use crate::observation::{Observation, SessionId, Sessions};
use crate::observe::StagedSession;

#[derive(Default)]
pub struct Accumulator {
    pub sessions: Sessions,
    pub required_level: ObservationsByEntity<f64>,
}

fn assign<T>(observations: Vec<Observation<T>>, session: SessionId) -> impl Iterator<Item = Observation<T>> {
    observations.into_iter().map(move |mut obs| {
        obs.session = session;
        obs
    })
}

impl Accumulator {
    /// Folded sessions so far.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Adds one observed session; sessions must come in fold order.
    pub fn add(&mut self, staged: StagedSession, contributor: Option<&str>) {
        let session = self.sessions.add(contributor, staged.server_time);
        self.required_level.add(assign(staged.required_level, session));
    }
}
