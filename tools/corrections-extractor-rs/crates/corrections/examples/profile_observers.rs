//! Times decoding and every observer on the submissions given on the command line, on one
//! thread. Each observer runs twice per session: the first run includes decoding the streams it
//! reads first (they decode lazily), the second is the observer alone.
//!
//! cargo run --release --example profile_observers -- <submission.json>...

use corrections::observe::is_forever_session;
use corrections::observers::{item, npc, object, quest};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use trace_data::submission::Submission;
use trace_data::{Inflater, Session};

type Observer = (&'static str, fn(&Session) -> usize);

macro_rules! observer {
    ($name:literal, $f:path) => {
        ($name, |s: &Session| $f(s).len())
    };
}

const OBSERVERS: &[Observer] = &[
    observer!("quest.zone_or_sort", quest::zone_or_sort::observe),
    observer!("quest.name", quest::name::observe),
    observer!("quest.quest_level", quest::quest_level::observe),
    observer!("quest.objectives_text", quest::objectives_text::observe),
    observer!("quest.objective_count", quest::objective_count::observe),
    observer!("npc.name", npc::name::observe),
    observer!("item.name", item::name::observe),
    observer!("object.name", object::name::observe),
    observer!("quest.required_level", quest::required_level::observe),
    observer!("quest.objectives", quest::objectives::observe),
    observer!("quest.trigger_end", quest::trigger_end::observe),
    observer!("quest.started_by", quest::started_by::observe),
    observer!("quest.finished_by", quest::finished_by::observe),
    observer!("quest.reputation_reward", quest::reputation_reward::observe),
    observer!("quest.quest_xp", quest::quest_xp::observe),
    observer!("quest.factions", quest::required_races::observe_quest_factions),
    observer!("quest.turn_in_poi", quest::turn_in_poi::observe),
    observer!("npc.spawns", npc::spawns::observe),
    observer!("npc.level", npc::level::observe),
    observer!("npc.friendly_to_faction", npc::friendly_to_faction::observe),
    observer!("npc.rank", npc::rank::observe),
    observer!("item.npc_drops", item::npc_drops::observe),
    observer!("item.object_drops", item::object_drops::observe),
    observer!("object.spawns", object::spawns::observe),
    observer!("lookup.npc_name", npc::name::observe_localized),
    observer!("lookup.quest_name", quest::name::observe_localized),
    observer!("lookup.objectives_text", quest::objectives_text::observe_localized),
    observer!("lookup.item_name", item::name::observe_localized),
    observer!("lookup.object_name", object::name::observe_localized),
];

fn main() {
    let mut inflater = Inflater::new();
    let (mut read, mut decode) = (Duration::ZERO, Duration::ZERO);
    let mut first: BTreeMap<&str, Duration> = BTreeMap::new();
    let mut again: BTreeMap<&str, Duration> = BTreeMap::new();
    let mut sessions = 0;
    for path in std::env::args().skip(1) {
        let started = Instant::now();
        let submission = Submission::read(path.as_ref(), &mut inflater);
        read += started.elapsed();
        let started = Instant::now();
        let decoded = submission.decode();
        decode += started.elapsed();
        for trace in &decoded.traces {
            for session in trace.sessions() {
                if !is_forever_session(&session) {
                    continue;
                }
                sessions += 1;
                for (times, _) in [(&mut first, ()), (&mut again, ())] {
                    for (name, observe) in OBSERVERS {
                        let started = Instant::now();
                        std::hint::black_box(observe(&session));
                        *times.entry(name).or_default() += started.elapsed();
                    }
                }
            }
        }
    }
    println!(
        "{sessions} sessions; read+inflate {:.2}s, decode {:.2}s",
        read.as_secs_f64(),
        decode.as_secs_f64()
    );
    let mut rows: Vec<_> = OBSERVERS.iter().map(|(name, _)| (*name, first[name], again[name])).collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.1));
    println!("{:<26} {:>10} {:>10}", "observer", "first (s)", "again (s)");
    for (name, f, a) in rows {
        println!("{name:<26} {:>10.2} {:>10.2}", f.as_secs_f64(), a.as_secs_f64());
    }
}
