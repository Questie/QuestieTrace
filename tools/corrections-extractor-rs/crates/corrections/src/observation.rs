//! Observations: what one session showed about one entity's field.
//!
//! Observers only observe: they walk one session and return observations. Merging them across
//! sessions into one value per entity happens once, at finalize, never inside an observer.

use indexmap::IndexSet;

/// How directly a value was observed. Ordered: `High` is the best.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// A folded session, as an index into [`Sessions`]. Every folded session has its own id, so
/// comparing ids compares sessions (TS compares `provenance.sessionKey`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SessionId(pub u32);

impl SessionId {
    /// An observation that has not been folded yet.
    pub const UNSET: SessionId = SessionId(u32::MAX);
}

/// One observation of one entity's field (TS `Observation<T>` with its provenance).
#[derive(Clone, Debug, PartialEq)]
pub struct Observation<T> {
    /// npcID, questID, itemID or objectID.
    pub entity_id: i64,
    pub value: T,
    pub confidence: Confidence,
    /// Seconds into the session at which the value was observed.
    pub t: f64,
    /// Set by the fold; [`SessionId::UNSET`] while observing.
    pub session: SessionId,
}

impl<T> Observation<T> {
    pub fn new(entity_id: i64, value: T, confidence: Confidence, t: f64) -> Self {
        Observation {
            entity_id,
            value,
            confidence,
            t,
            session: SessionId::UNSET,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Observation<U> {
        Observation {
            entity_id: self.entity_id,
            value: f(self.value),
            confidence: self.confidence,
            t: self.t,
            session: self.session,
        }
    }
}

/// A pseudonymous contributor, as an index into [`Sessions`]' contributor table.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContributorId(u32);

/// What merges may know about the sessions observations came from.
#[derive(Default)]
pub struct Sessions {
    sessions: Vec<SessionInfo>,
    contributors: IndexSet<String>,
}

struct SessionInfo {
    contributor: Option<ContributorId>,
    /// First GetServerTime() of the session: the realm time around its start.
    server_time: Option<f64>,
}

impl Sessions {
    /// Registers the next folded session.
    pub fn add(&mut self, contributor: Option<&str>, server_time: Option<f64>) -> SessionId {
        let contributor = contributor
            .filter(|c| !c.is_empty())
            .map(|c| ContributorId(self.contributors.insert_full(c.to_owned()).0 as u32));
        self.sessions.push(SessionInfo { contributor, server_time });
        SessionId(self.sessions.len() as u32 - 1)
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// The submitting contributor; None when the submission did not say.
    pub fn contributor(&self, session: SessionId) -> Option<ContributorId> {
        self.sessions[session.0 as usize].contributor
    }

    /// The session's first server time; None when it was not recorded.
    pub fn server_time(&self, session: SessionId) -> Option<f64> {
        self.sessions[session.0 as usize].server_time
    }
}

/// A value observed under a client locale (`GetLocale()`), for translations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Localized<T> {
    pub locale: String,
    pub value: T,
}
