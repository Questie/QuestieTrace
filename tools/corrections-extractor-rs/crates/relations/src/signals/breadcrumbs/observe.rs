//! What one episode says about each linked (breadcrumb B, target T) pair.
//!
//! A breadcrumb is a linked pair (see [`super::links`]) that is both OPTIONAL and DEPENDENT:
//! - optional: T is available to characters who never did B (a chain's target is not),
//! - dependent: B and T gate each other (an unrelated quest at the same giver does not).
//!
//! Each observation below is one of those two kinds of support, or a contradiction of them.

use super::links::{find_auto_pops, BreadcrumbParams, FxIndexMap, FxIndexSet, Links, Pair};
use crate::eligibility::{class_allowed, has_untracked_requirements, is_statically_eligible, race_allowed};
use crate::model::{CatalogQuest, Evidence, GiverKind, GiverRef, PlayerContext, QuestCatalog, QuestEpisode, QuestEventKind, QuestId, SessionTime};
use crate::text::to_fixed;
use crate::timeline::EpisodeTimeline;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

/// Declared in the order evidence examples and per-kind counts are listed in: optional kinds,
/// then dependent, then contradictions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationKind {
    // Optional
    /// T offered, taken or done while B was neither done nor in the log.
    Free,
    /// The client's IsBreadcrumbQuest(B) returned true.
    Flagged,
    // Dependent
    /// B left the log without a turn-in as T was accepted.
    ServerDrop,
    /// T missing at its giver while B was in the log, offered once B was turned in.
    Reveal,
    /// The server opened T's dialog right after B's turn-in.
    AutoPop,
    /// B missing at its giver while T was done or in the log.
    BlockB,
    // Contradictions
    /// T missing while B was neither done nor in the log, offered once B was done (a chain).
    Unlock,
    /// B offered while T was done or in the log.
    ShowB,
    /// T offered or accepted while B was in the log.
    ShowT,
    /// IsBreadcrumbQuest(B) returned false.
    NotFlagged,
}

impl ObservationKind {
    pub const OPTIONAL: [ObservationKind; 2] = [ObservationKind::Free, ObservationKind::Flagged];
    pub const DEPENDENT: [ObservationKind; 4] = [
        ObservationKind::ServerDrop,
        ObservationKind::Reveal,
        ObservationKind::AutoPop,
        ObservationKind::BlockB,
    ];
    pub const CONTRADICTING: [ObservationKind; 4] = [
        ObservationKind::Unlock,
        ObservationKind::ShowB,
        ObservationKind::ShowT,
        ObservationKind::NotFlagged,
    ];
    /// Every kind, in declaration order.
    pub const ALL: [ObservationKind; 10] = [
        ObservationKind::Free,
        ObservationKind::Flagged,
        ObservationKind::ServerDrop,
        ObservationKind::Reveal,
        ObservationKind::AutoPop,
        ObservationKind::BlockB,
        ObservationKind::Unlock,
        ObservationKind::ShowB,
        ObservationKind::ShowT,
        ObservationKind::NotFlagged,
    ];

    /// The name in the reports and in evidence texts.
    pub fn name(self) -> &'static str {
        match self {
            ObservationKind::Free => "free",
            ObservationKind::Flagged => "flagged",
            ObservationKind::ServerDrop => "serverDrop",
            ObservationKind::Reveal => "reveal",
            ObservationKind::AutoPop => "autoPop",
            ObservationKind::BlockB => "blockB",
            ObservationKind::Unlock => "unlock",
            ObservationKind::ShowB => "showB",
            ObservationKind::ShowT => "showT",
            ObservationKind::NotFlagged => "notFlagged",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub pair: Pair,
    pub kind: ObservationKind,
    pub evidence: Evidence,
}

/// How far before an accept to read the log, so the accepted quest's own log change is not seen yet.
const BEFORE: SessionTime = 0.001;

/// A complete offer list that left out a quest some lead-in might have hidden. Whether that meant
/// anything is only known once the episode shows when the quest was offered after all.
struct PendingAbsence {
    pair: Pair,
    t: SessionTime,
    breadcrumb_in_log: bool,
    giver: GiverRef,
}

fn describe_giver(giver: Option<GiverRef>) -> String {
    giver.map_or_else(|| "unknown giver".into(), GiverRef::label)
}

fn time(t: SessionTime) -> String {
    format!("{}s", to_fixed(t, 1))
}

/// Race and class allow the quest, and are actually known whenever the quest restricts them.
fn race_class_allow(quest: Option<&CatalogQuest>, player: &PlayerContext) -> bool {
    let Some(quest) = quest else { return true };
    let restricts = |mask: Option<u64>| mask.is_some_and(|mask| mask != 0);
    if (restricts(quest.required_races) && player.race_id.is_none()) || (restricts(quest.required_classes) && player.class_id.is_none()) {
        return false;
    }
    race_allowed(quest.required_races, player.race_id) && class_allowed(quest.required_classes, player.class_id)
}

pub fn observe_episode(episode: &QuestEpisode, links: &Links, catalog: &QuestCatalog, params: &BreadcrumbParams) -> Vec<Observation> {
    let mut observer = Observer {
        episode,
        timeline: EpisodeTimeline::new(episode, true),
        links,
        catalog,
        params,
        found: FxIndexMap::default(),
    };
    observer.session_start();
    let (mut available_at, pending) = observer.offer_lists();
    observer.accepts(&mut available_at);
    observer.auto_pops();
    observer.absences(&pending, &available_at);
    observer.client_flags();
    observer
        .found
        .into_iter()
        .map(|((pair, kind), evidence)| Observation { pair, kind, evidence })
        .collect()
}

struct Observer<'a> {
    episode: &'a QuestEpisode,
    timeline: EpisodeTimeline<'a>,
    links: &'a Links,
    catalog: &'a QuestCatalog,
    params: &'a BreadcrumbParams,
    /// First observation per pair and kind, in the order they were made.
    found: FxIndexMap<(Pair, ObservationKind), Evidence>,
}

impl Observer<'_> {
    fn done(&self, quest_id: QuestId, t: SessionTime) -> bool {
        self.timeline.is_completed_at(quest_id, t)
    }

    fn in_log(&self, quest_id: QuestId, t: SessionTime) -> bool {
        self.timeline.is_in_log_at(quest_id, t)
    }

    /// One observation per pair and kind is enough: support is counted per character.
    fn add(&mut self, breadcrumb: QuestId, target: QuestId, kind: ObservationKind, t: SessionTime, text: String) {
        self.found.entry((Pair { breadcrumb, target }, kind)).or_insert_with(|| Evidence {
            reference: self.episode.key.clone(),
            t: Some(t),
            text,
        });
    }

    /// "Not offered" only counts when nothing but relationships could have hidden the quest.
    fn absence_eligible(&self, quest_id: QuestId, t: SessionTime) -> bool {
        let Some(quest) = self.catalog.quest(quest_id) else { return false };
        if has_untracked_requirements(quest) || quest.needs_event() || !race_class_allow(Some(quest), &self.episode.player) {
            return false;
        }
        let level = self.timeline.level_at(t);
        if quest.required_level.is_some() && level.is_none() {
            return false;
        }
        is_statically_eligible(quest, &self.episode.player, level, -self.params.absence_level_margin) && !self.done(quest_id, t) && !self.in_log(quest_id, t)
    }

    /// T reached without B. Only counts when B's completion is observable at all, the character
    /// could have taken B, and no other lead-in of T explains it (T requiring "A or B" is not a
    /// breadcrumb). `log_t` is when to read the log, which for an accept is just before it.
    fn free_of(&self, breadcrumb: QuestId, target: QuestId, t: SessionTime, log_t: SessionTime) -> bool {
        if !self.links.ever_completed.contains(&breadcrumb) || self.done(breadcrumb, t) || self.in_log(breadcrumb, log_t) {
            return false;
        }
        if !race_class_allow(self.catalog.quest(breadcrumb), &self.episode.player) {
            return false;
        }
        self.links
            .breadcrumbs_of(target)
            .all(|other| other == breadcrumb || !(self.done(other, t) || self.in_log(other, log_t)))
    }

    /// Session start: T already done or in the log while B never was.
    fn session_start(&mut self) {
        let first_log = self.episode.quest_log.first();
        let start_t = first_log.map_or(0.0, |entry| entry.t);
        let initial = self.episode.completed.iter().flat_map(|completed| &completed.initial);
        let starting_targets: FxIndexSet<QuestId> = initial.chain(first_log.into_iter().flat_map(|entry| &entry.v)).copied().collect();
        for target in starting_targets {
            for breadcrumb in self.links.breadcrumbs_of(target) {
                if self.free_of(breadcrumb, target, start_t, start_t) {
                    let text = format!("{target} done or in the log at session start; {breadcrumb} never done");
                    self.add(breadcrumb, target, ObservationKind::Free, start_t, text);
                }
            }
        }
    }

    /// Offer lists: what was offered, and what a complete list left out. Returns when each quest
    /// was offered, and the absences to judge once the whole episode is known.
    fn offer_lists(&mut self) -> (FxHashMap<QuestId, Vec<SessionTime>>, Vec<PendingAbsence>) {
        let mut available_at: FxHashMap<QuestId, Vec<SessionTime>> = FxHashMap::default();
        let mut pending = Vec::new();
        for offer in &self.episode.offers {
            let t = offer.t;
            let place = describe_giver(offer.giver);
            for id in offer.available.iter().map(|quest| quest.id) {
                available_at.entry(id).or_default().push(t);
                for breadcrumb in self.links.breadcrumbs_of(id) {
                    if self.done(breadcrumb, t) {
                        continue;
                    }
                    if self.in_log(breadcrumb, t) {
                        let text = format!("{id} offered by {place} at {} while {breadcrumb} was in the log", time(t));
                        self.add(breadcrumb, id, ObservationKind::ShowT, t, text);
                    } else if self.free_of(breadcrumb, id, t, t) {
                        let text = format!("{id} offered by {place} at {}; {breadcrumb} never done", time(t));
                        self.add(breadcrumb, id, ObservationKind::Free, t, text);
                    }
                }
                for target in self.links.targets_of(id) {
                    if !self.done(id, t) && (self.done(target, t) || self.in_log(target, t)) {
                        let text = format!("{id} offered by {place} at {} while {target} was done or in the log", time(t));
                        self.add(id, target, ObservationKind::ShowB, t, text);
                    }
                }
            }

            let Some(giver) = offer.giver.filter(|giver| offer.list_complete && giver.kind != GiverKind::Item) else {
                continue;
            };
            let listed: FxHashSet<QuestId> = offer
                .available
                .iter()
                .map(|quest| quest.id)
                .chain(offer.active.iter().flatten().copied())
                .collect();
            for missing in self.links.started_at(giver) {
                if listed.contains(&missing) || !self.absence_eligible(missing, t) {
                    continue;
                }
                for breadcrumb in self.links.breadcrumbs_of(missing) {
                    if self.done(breadcrumb, t) || !self.links.ever_completed.contains(&breadcrumb) {
                        continue;
                    }
                    pending.push(PendingAbsence {
                        pair: Pair { breadcrumb, target: missing },
                        t,
                        breadcrumb_in_log: self.in_log(breadcrumb, t),
                        giver,
                    });
                }
                for target in self.links.targets_of(missing) {
                    if self.done(target, t) || self.in_log(target, t) {
                        let text = format!(
                            "{missing} missing from {place}'s full list at {} while {target} was done or in the log",
                            time(t)
                        );
                        self.add(missing, target, ObservationKind::BlockB, t, text);
                    }
                }
            }
        }
        (available_at, pending)
    }

    /// Accepts: T taken without B, or with B in the log (which the server should refuse or
    /// resolve by dropping B).
    fn accepts(&mut self, available_at: &mut FxHashMap<QuestId, Vec<SessionTime>>) {
        let events = &self.episode.quest_events;
        for accept in events.iter().filter(|event| event.kind == QuestEventKind::Accepted) {
            let (t, target) = (accept.t, accept.quest_id);
            available_at.entry(target).or_default().push(t);
            for breadcrumb in self.links.breadcrumbs_of(target) {
                if self.done(breadcrumb, t) {
                    continue;
                }
                if self.in_log(breadcrumb, t - BEFORE) {
                    // "Removed" is ingest's "left the log, reason unknown"; a player abandoning B says nothing either way.
                    let removal = events.iter().find(|other| {
                        other.quest_id == breadcrumb
                            && matches!(other.kind, QuestEventKind::Removed | QuestEventKind::Abandoned)
                            && (other.t - t).abs() <= self.params.server_drop_seconds
                    });
                    match removal.map(|removal| removal.kind) {
                        Some(QuestEventKind::Abandoned) => {}
                        Some(_) => {
                            let text = format!("{breadcrumb} left the log without a turn-in as {target} was accepted at {}", time(t));
                            self.add(breadcrumb, target, ObservationKind::ServerDrop, t, text);
                        }
                        None => {
                            let text = format!("{target} accepted at {} while {breadcrumb} was in the log", time(t));
                            self.add(breadcrumb, target, ObservationKind::ShowT, t, text);
                        }
                    }
                } else if self.free_of(breadcrumb, target, t, t - BEFORE) {
                    let text = format!(
                        "{target} accepted from {} at {}; {breadcrumb} never done",
                        describe_giver(accept.giver),
                        time(t)
                    );
                    self.add(breadcrumb, target, ObservationKind::Free, t, text);
                }
            }
        }
    }

    /// Server pops, whether or not the pair is linked: the pop itself is evidence about the pair.
    fn auto_pops(&mut self) {
        for pop in find_auto_pops(self.episode, self.params) {
            let Pair { breadcrumb, target } = pop.pair;
            let text = format!(
                "turned in {breadcrumb} at {}, {target} opened {}s later",
                describe_giver(pop.giver),
                to_fixed(pop.detail_t - pop.turn_in_t, 1)
            );
            self.add(breadcrumb, target, ObservationKind::AutoPop, pop.turn_in_t, text);
        }
    }

    /// A missing T (or one hidden while B was in the log) means something only when the same
    /// character is offered T right after B was completed, with no other completion or level-up in
    /// between.
    fn absences(&mut self, pending: &[PendingAbsence], available_at: &FxHashMap<QuestId, Vec<SessionTime>>) {
        let completions = self.timeline.completions_during_session();
        for absence in pending {
            let Pair { breadcrumb, target } = absence.pair;
            let t = absence.t;
            let offered_after = available_at.get(&target).into_iter().flatten().copied();
            let Some(shown) = offered_after.into_iter().find(|&shown| shown > t && self.done(breadcrumb, shown)) else {
                continue;
            };
            if self.timeline.level_at(shown) != self.timeline.level_at(t) {
                continue;
            }
            if completions
                .iter()
                .any(|completion| completion.t > t && completion.t <= shown && completion.v != breadcrumb)
            {
                continue;
            }
            let place = describe_giver(Some(absence.giver));
            if absence.breadcrumb_in_log {
                let text = format!(
                    "{target} missing from {place}'s full list at {} with {breadcrumb} in the log, offered at {} after its turn-in",
                    time(t),
                    time(shown)
                );
                self.add(breadcrumb, target, ObservationKind::Reveal, t, text);
            } else {
                let text = format!(
                    "{target} missing from {place}'s full list at {} before {breadcrumb}, offered at {} once it was done",
                    time(t),
                    time(shown)
                );
                self.add(breadcrumb, target, ObservationKind::Unlock, t, text);
            }
        }
    }

    /// IsBreadcrumbQuest(B), for every target B is linked to.
    fn client_flags(&mut self) {
        let Some(quest_info) = &self.episode.quest_info else { return };
        for (&breadcrumb, info) in quest_info {
            let Some(is_breadcrumb) = info.is_breadcrumb else { continue };
            let kind = if is_breadcrumb {
                ObservationKind::Flagged
            } else {
                ObservationKind::NotFlagged
            };
            for target in self.links.targets_of(breadcrumb) {
                self.add(breadcrumb, target, kind, 0.0, format!("IsBreadcrumbQuest({breadcrumb}) = {is_breadcrumb}"));
            }
        }
    }
}
