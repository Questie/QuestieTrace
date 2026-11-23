//! Checks that an export rebuilt from a pack is the export that went in.
//!
//! This is the gate before a pack is uploaded and its source objects deleted, so it does not
//! rest on the encoder's account of what it changed. It reads both exports with
//! `trace_data::cbor::Cursor` and applies FORMAT.md's rules itself:
//!
//! 1. Every item is byte-identical, except a float directly under a `tp` key inside a session,
//!    which may instead be the original rounded to a whole microsecond.
//! 2. A session in which any `tp` differs must be one whose `tp` may be rounded at all: its
//!    `schemaVersion` is allowed, it has the shape the rule requires, every one of its float
//!    `tp` values is rounded, and its `tp` values and `durationPrecise` compare with each
//!    other exactly as they did.
//!
//! The rules are written out here a second time on purpose. Sharing the encoder's functions
//! would let one mistake pass on both sides.

use crate::values::TpPolicy;
use crate::wire::write_float_shortest;
use trace_data::cbor::{Cursor, Item};

/// Deeper nesting than any export that decodes (`trace_data` stops at 100).
const MAX_DEPTH: u32 = 128;

const MICROSECONDS: f64 = 1_000_000.0;

/// The largest whole number of microseconds a `tp` may be stored as: 2^53.
const MAX_STEPS: f64 = 9_007_199_254_740_992.0;

/// Ok when `rebuilt` is `original`, up to the rounding of `tp` that `policy` permits.
pub fn verify_export(original: &[u8], rebuilt: &[u8], policy: &TpPolicy) -> Result<(), String> {
    if original == rebuilt {
        return Ok(());
    }

    // Item by item: finds every difference and which sessions they are in.
    let mut pair = Lockstep {
        original: Side::new(original),
        rebuilt: Side::new(rebuilt),
        rounded_sessions: Vec::new(),
    };
    pair.root()?;
    if !pair.original.cursor.is_done() || !pair.rebuilt.cursor.is_done() {
        return Err("data after the export's root item".into());
    }

    // Session by session: was rounding allowed there, and was it done to all of it.
    let before = sessions_of(original)?;
    let after = sessions_of(rebuilt)?;
    if before.len() != after.len() || before.len() != pair.rounded_sessions.len() {
        return Err("the rebuilt export has a different number of sessions".into());
    }
    for (index, (before, after)) in before.iter().zip(&after).enumerate() {
        if pair.rounded_sessions[index] {
            rounding_was_permitted(before, after, policy).map_err(|message| format!("session {}: {message}", index + 1))?;
        }
    }
    Ok(())
}

// ---- Item by item ----

struct Side<'a> {
    bytes: &'a [u8],
    cursor: Cursor<'a>,
}

impl<'a> Side<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Side {
            bytes,
            cursor: Cursor::new(bytes),
        }
    }

    /// Reads the next item: its own bytes (a container's header only) and what it is.
    fn item(&mut self) -> Result<(&'a [u8], Item<'a>), String> {
        let start = self.cursor.pos();
        let item = self.cursor.item().map_err(|error| error.0)?;
        Ok((&self.bytes[start..self.cursor.pos()], item))
    }
}

struct Lockstep<'a> {
    original: Side<'a>,
    rebuilt: Side<'a>,
    /// Per session, in order: whether any of its `tp` values differs.
    rounded_sessions: Vec<bool>,
}

fn is_float(item: &[u8]) -> bool {
    matches!(item.first(), Some(0xf9..=0xfb))
}

/// What a `tp` reads back as after rounding, or None when it may not be rounded at all.
fn rounded_tp(tp: f64) -> Option<f64> {
    let steps = (tp * MICROSECONDS).round();
    // A negative value that rounds to zero would come back as positive zero: a changed sign.
    let loses_sign = steps == 0.0 && tp.is_sign_negative();
    (steps.is_finite() && steps.abs() <= MAX_STEPS && !loses_sign).then_some(steps / MICROSECONDS)
}

impl<'a> Lockstep<'a> {
    /// The root, and in it the sessions: the maps in the array under the root's `sessions`.
    fn root(&mut self) -> Result<(), String> {
        let Item::Map(length) = self.same_item()? else {
            return Ok(());
        };
        let mut seen_sessions = false;
        for _ in 0..length as u64 {
            if self.key(1)? != Some(b"sessions".as_slice()) {
                self.value(false, None, 1)?;
                continue;
            }
            if std::mem::replace(&mut seen_sessions, true) {
                return Err("the export has two sessions keys".into());
            }
            if self.original.cursor.peek_major() != Some(4) {
                self.value(false, None, 1)?;
                continue;
            }
            let Item::Array(count) = self.same_item()? else {
                unreachable!("the next item was seen to be an array");
            };
            for _ in 0..count as u64 {
                let session = (self.original.cursor.peek_major() == Some(5)).then(|| {
                    self.rounded_sessions.push(false);
                    self.rounded_sessions.len() - 1
                });
                self.value(false, session, 2)?;
            }
        }
        Ok(())
    }

    /// Reads the next item of both exports; it must be the same bytes.
    fn same_item(&mut self) -> Result<Item<'a>, String> {
        let (before, item) = self.original.item()?;
        let (after, _) = self.rebuilt.item()?;
        if before != after {
            return Err(self.differs(before));
        }
        Ok(item)
    }

    fn differs(&self, item: &[u8]) -> String {
        format!("an item differs at byte {} of the original", self.original.cursor.pos() - item.len())
    }

    /// Reads a map key of both exports. A key must be identical, whatever it holds. Its name
    /// when it is a string.
    fn key(&mut self, depth: u32) -> Result<Option<&'a [u8]>, String> {
        let start = self.original.cursor.pos();
        let peeked = Cursor::new(&self.original.bytes[start..]).item().map_err(|error| error.0)?;
        self.value(false, None, depth)?;
        Ok(match peeked {
            Item::Str(name) => Some(name),
            _ => None,
        })
    }

    /// Compares the next value of both exports. `under_tp` is set for the value of a `tp` key,
    /// `session` inside a session.
    fn value(&mut self, under_tp: bool, session: Option<usize>, depth: u32) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err("nested too deeply".into());
        }
        let (before, item) = self.original.item()?;
        let (after, rebuilt_item) = self.rebuilt.item()?;
        if before != after {
            // The one permitted difference: a float tp of a session, rounded.
            let (Item::Num(tp), Item::Num(read_back), Some(session)) = (item, rebuilt_item, session) else {
                return Err(self.differs(before));
            };
            let rounded = rounded_tp(tp).filter(|_| under_tp && is_float(before) && is_float(after));
            let mut shortest = Vec::with_capacity(9);
            rounded.iter().for_each(|&rounded| write_float_shortest(&mut shortest, rounded));
            // The value as the cursor reads it, and the bytes as the reader must write them.
            if rounded.map(f64::to_bits) != Some(read_back.to_bits()) || shortest != after {
                return Err(self.differs(before));
            }
            self.rounded_sessions[session] = true;
            return Ok(());
        }
        match item {
            Item::Array(length) => {
                for _ in 0..length as u64 {
                    self.value(false, session, depth + 1)?;
                }
            }
            Item::Map(length) => {
                for _ in 0..length as u64 {
                    let is_tp = self.key(depth + 1)? == Some(b"tp".as_slice());
                    self.value(is_tp, session, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

// ---- Session by session ----

/// What the rounding rule looks at in one session.
#[derive(Default)]
struct SessionFacts {
    /// Every numeric `tp` in document order, and whether it was written as a float.
    tp: Vec<(f64, bool)>,
    duration_precise: Option<f64>,
    schema_version: Option<f64>,
    /// An entry with a `t` but no numeric `tp`, a float `tp` outside an entry, or a field key
    /// that appears twice in one map.
    misshapen: bool,
}

fn next<'a>(cursor: &mut Cursor<'a>) -> Result<Item<'a>, String> {
    cursor.item().map_err(|error| error.0)
}

/// The next item when it is a number, else None; either way it is read.
fn next_number(cursor: &mut Cursor) -> Result<Option<f64>, String> {
    Ok(match next(cursor)? {
        Item::Num(value) => Some(value),
        _ => None,
    })
}

/// Steps over what follows a container's header; a scalar has nothing to step over. The
/// item-by-item comparison ran first, so the data is known to be well formed.
fn skip_contents(cursor: &mut Cursor, header: &Item) {
    match *header {
        Item::Array(count) => cursor.skip_trusted(count as u64),
        Item::Map(count) => cursor.skip_trusted((count as u64).saturating_mul(2)),
        _ => {}
    }
}

/// The sessions of an export, in the order [`Lockstep::root`] numbers them.
fn sessions_of(cbor: &[u8]) -> Result<Vec<SessionFacts>, String> {
    let mut cursor = Cursor::new(cbor);
    let mut sessions = Vec::new();
    let Item::Map(length) = next(&mut cursor)? else {
        return Ok(sessions);
    };
    for _ in 0..length as u64 {
        let key = next(&mut cursor)?;
        skip_contents(&mut cursor, &key);
        match next(&mut cursor)? {
            Item::Array(count) if matches!(key, Item::Str(b"sessions")) => {
                for _ in 0..count as u64 {
                    let mut facts = SessionFacts::default();
                    let is_session = cursor.peek_major() == Some(5);
                    collect(&mut cursor, &mut facts, is_session, 2)?;
                    if is_session {
                        sessions.push(facts);
                    }
                }
            }
            other => skip_contents(&mut cursor, &other),
        }
    }
    Ok(sessions)
}

/// Reads one value into `facts`. `is_session` is set for a session's own map.
fn collect(cursor: &mut Cursor, facts: &mut SessionFacts, is_session: bool, depth: u32) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("nested too deeply".into());
    }
    match next(cursor)? {
        Item::Array(length) => {
            for _ in 0..length as u64 {
                collect(cursor, facts, false, depth + 1)?;
            }
        }
        Item::Map(length) => {
            // This map's own t and tp: is it an entry, and does it hold a float tp.
            let (mut numeric_t, mut numeric_tp, mut float_tp) = (false, false, false);
            let (mut seen_t, mut seen_tp) = (false, false);
            for _ in 0..length as u64 {
                let key = next(cursor)?;
                skip_contents(cursor, &key);
                let name: &[u8] = match key {
                    Item::Str(name) => name,
                    _ => b"",
                };
                // A value that is a container is walked like any other, whatever its key.
                let is_scalar = !matches!(cursor.peek_major(), Some(4 | 5));
                let written_as_float = cursor.peek_major() == Some(7);
                match name {
                    b"tp" => {
                        facts.misshapen |= std::mem::replace(&mut seen_tp, true);
                        if !is_scalar {
                            collect(cursor, facts, false, depth + 1)?;
                        } else if let Some(tp) = next_number(cursor)? {
                            facts.tp.push((tp, written_as_float));
                            numeric_tp = true;
                            float_tp = written_as_float;
                        }
                    }
                    b"t" => {
                        facts.misshapen |= std::mem::replace(&mut seen_t, true);
                        if !is_scalar {
                            collect(cursor, facts, false, depth + 1)?;
                        } else {
                            numeric_t = next_number(cursor)?.is_some();
                        }
                    }
                    b"durationPrecise" | b"schemaVersion" if is_session && is_scalar => {
                        let field = if name == b"schemaVersion" {
                            &mut facts.schema_version
                        } else {
                            &mut facts.duration_precise
                        };
                        let value = next_number(cursor)?;
                        facts.misshapen |= field.is_some();
                        *field = value.or(*field);
                    }
                    _ => collect(cursor, facts, false, depth + 1)?,
                }
            }
            facts.misshapen |= (numeric_t && !numeric_tp) || (float_tp && !numeric_t);
        }
        _ => {}
    }
    Ok(())
}

/// Whether a session whose `tp` values differ was rounded as FORMAT.md allows.
fn rounding_was_permitted(before: &SessionFacts, after: &SessionFacts, policy: &TpPolicy) -> Result<(), String> {
    if !policy.allows(before.schema_version) {
        return Err("tp was rounded in a session whose schemaVersion may not be".into());
    }
    if before.misshapen {
        return Err("tp was rounded in a session that does not have the required shape".into());
    }
    if before.tp.len() != after.tp.len() {
        return Err("a different number of tp values".into());
    }

    // All or none: every float tp must read back rounded, every integer tp untouched.
    for (&(tp, was_float), &(read_back, _)) in before.tp.iter().zip(&after.tp) {
        let expected = if was_float { rounded_tp(tp) } else { Some(tp) };
        if expected.map(f64::to_bits) != Some(read_back.to_bits()) {
            return Err("not every tp of the session was rounded to a whole microsecond".into());
        }
    }

    // Every comparison between the session's times must come out as before.
    let mut pairs: Vec<(f64, f64)> = before.tp.iter().zip(&after.tp).map(|(before, after)| (before.0, after.0)).collect();
    match before.duration_precise {
        Some(duration) if duration.is_finite() => pairs.push((duration, duration)),
        Some(_) => return Err("tp was rounded in a session whose durationPrecise is not finite".into()),
        None => {}
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Sorted by the original value, so neighbours decide every pair.
    let unchanged = pairs.windows(2).all(|pair| {
        let ((before_a, after_a), (before_b, after_b)) = (pair[0], pair[1]);
        if before_a == before_b {
            after_a == after_b
        } else {
            after_a < after_b
        }
    });
    if unchanged {
        Ok(())
    } else {
        Err("two tp values compare differently after rounding".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    const VERSION: u64 = 17;

    fn allowed() -> TpPolicy {
        TpPolicy::SchemaVersions(vec![VERSION as u32])
    }

    fn entry(tp: Vec<u8>) -> Vec<u8> {
        map(&[(text("t"), float(1.5)), (text("tp"), tp)])
    }

    fn session(entries: &[Vec<u8>]) -> Vec<u8> {
        map(&[(text("schemaVersion"), uint(VERSION)), (text("events"), array(entries))])
    }

    fn export(entries: &[Vec<u8>]) -> Vec<u8> {
        map(&[(text("sessions"), array(&[session(entries)]))])
    }

    fn verify(original: &[u8], rebuilt: &[u8]) -> Result<(), String> {
        verify_export(original, rebuilt, &allowed())
    }

    #[test]
    fn a_tp_rounded_to_a_whole_microsecond_is_accepted_and_nothing_coarser_or_finer() {
        let original = export(&[entry(float(7.370140500001071))]);
        assert_eq!(verify(&original, &export(&[entry(float(7.370141))])), Ok(()));
        // The neighbouring microsecond, and a value rounded to 100 ns.
        assert!(verify(&original, &export(&[entry(float(7.370140))])).is_err());
        assert!(verify(&original, &export(&[entry(float(7.3701405))])).is_err());
    }

    #[test]
    fn a_rounded_tp_must_be_written_in_its_shortest_width() {
        // 2.5000004 rounds to 2.5, which the reader writes as a half float.
        let original = export(&[entry(float(2.5000004))]);
        assert_eq!(verify(&original, &export(&[entry(half(0x4100))])), Ok(()));
        assert!(verify(&original, &export(&[entry(double(2.5))])).is_err());
    }

    #[test]
    fn any_other_changed_value_is_rejected_even_by_one_bit() {
        let with_t = |t: f64| export(&[map(&[(text("t"), float(t)), (text("tp"), float(1.25))])]);
        let error = verify(&with_t(7.717999999998938), &with_t(7.718)).unwrap_err();
        assert!(error.starts_with("an item differs at byte"), "{error}");
        // A float under tp that became an integer, and an integer tp that became a float.
        assert!(verify(&export(&[entry(float(3.0000001))]), &export(&[entry(uint(3))])).is_err());
        assert!(verify(&export(&[entry(uint(3))]), &export(&[entry(float(3.0))])).is_err());
    }

    #[test]
    fn only_the_value_directly_under_tp_may_differ_not_values_nested_in_it() {
        let nested = |value: f64| export(&[map(&[(text("tp"), map(&[(text("v"), float(value))]))])]);
        assert!(verify(&nested(7.370140500001071), &nested(7.370141)).is_err());
    }

    #[test]
    fn a_rounded_tp_outside_a_session_or_inside_a_key_is_rejected() {
        let (exact, rounded) = (float(7.370140500001071), float(7.370141));
        // At the root, beside the sessions.
        let at_root = |tp: &Vec<u8>| map(&[(text("sessions"), array(&[])), (text("tp"), tp.clone())]);
        assert!(verify(&at_root(&exact), &at_root(&rounded)).is_err());
        // In an export whose root is no map: there are no sessions at all.
        let in_array = |tp: &Vec<u8>| array(&[entry(tp.clone())]);
        assert!(verify(&in_array(&exact), &in_array(&rounded)).is_err());
        // Inside a map that is used as a key.
        let in_key = |tp: &Vec<u8>| export(&[map(&[(entry(tp.clone()), uint(1))])]);
        assert!(verify(&in_key(&exact), &in_key(&rounded)).is_err());
    }

    #[test]
    fn rounding_only_some_of_a_sessions_tp_values_is_rejected() {
        let original = export(&[entry(float(1.0000004)), entry(float(2.0000004))]);
        let partly = export(&[entry(float(1.0)), entry(float(2.0000004))]);
        assert_eq!(
            verify(&original, &partly).unwrap_err(),
            "session 1: not every tp of the session was rounded to a whole microsecond"
        );
        assert_eq!(verify(&original, &export(&[entry(float(1.0)), entry(float(2.0))])), Ok(()));
    }

    #[test]
    fn rounding_is_rejected_where_the_policy_or_the_sessions_shape_does_not_allow_it() {
        let original = export(&[entry(float(7.370140500001071))]);
        let rounded = export(&[entry(float(7.370141))]);
        assert_eq!(
            verify_export(&original, &rounded, &TpPolicy::Off).unwrap_err(),
            "session 1: tp was rounded in a session whose schemaVersion may not be"
        );
        assert!(verify_export(&original, &rounded, &TpPolicy::SchemaVersions(vec![VERSION as u32 + 1])).is_err());

        // An entry with a t and no tp, and a float tp in a map without t.
        let lacking = |tp: f64| export(&[map(&[(text("t"), float(1.5))]), entry(float(tp))]);
        assert_eq!(
            verify(&lacking(7.370140500001071), &lacking(7.370141)).unwrap_err(),
            "session 1: tp was rounded in a session that does not have the required shape"
        );
        let stray = |tp: f64| export(&[map(&[(text("tp"), float(tp))])]);
        assert!(verify(&stray(7.370140500001071), &stray(7.370141)).is_err());
    }

    #[test]
    fn a_tp_too_large_to_store_or_one_that_would_lose_its_sign_may_not_change() {
        // Above 2^53 microseconds: about 285 years.
        let huge = |tp: f64| export(&[entry(double(tp))]);
        assert!(verify(&huge(10000000000.300005), &huge(10000000000.300003)).is_err());
        // Negative zero must not come back as zero.
        assert!(verify(&export(&[entry(half(0x8000))]), &export(&[entry(half(0x0000))])).is_err());
        assert!(verify(&export(&[entry(float(-1e-7))]), &export(&[entry(float(0.0))])).is_err());
    }

    #[test]
    fn rounding_that_merges_two_values_or_crosses_the_duration_is_rejected() {
        let merged = verify(
            &export(&[entry(float(1.0000006)), entry(float(1.000001))]),
            &export(&[entry(float(1.000001)), entry(float(1.000001))]),
        );
        assert_eq!(merged.unwrap_err(), "session 1: two tp values compare differently after rounding");

        let with_duration = |tp: f64| {
            let session = map(&[
                (text("schemaVersion"), uint(VERSION)),
                (text("durationPrecise"), float(100.0000008)),
                (text("events"), array(&[entry(float(tp))])),
            ]);
            map(&[(text("sessions"), array(&[session]))])
        };
        let crossed = verify(&with_duration(100.0000006), &with_duration(100.000001));
        assert_eq!(crossed.unwrap_err(), "session 1: two tp values compare differently after rounding");
    }

    #[test]
    fn a_rounded_tp_landing_on_an_integer_tp_is_rejected() {
        let original = export(&[entry(uint(0)), entry(float(3e-7))]);
        let rebuilt = export(&[entry(uint(0)), entry(float(0.0))]);
        assert_eq!(
            verify(&original, &rebuilt).unwrap_err(),
            "session 1: two tp values compare differently after rounding"
        );
    }

    #[test]
    fn a_tp_key_written_with_a_longer_length_counts_like_any_other() {
        // 78 02 "tp": the key tp with its length in a separate byte.
        let long_key = |tp: f64| [vec![0x78, 0x02, b't', b'p'], float(tp)].concat();
        let pair = |first: f64, second: f64| {
            let odd = [vec![0xa2], text("t"), float(1.5), long_key(second)].concat();
            export(&[entry(float(first)), odd])
        };
        // Both round to the same microsecond: the second, oddly keyed one is seen too.
        assert_eq!(
            verify(&pair(1.0000006, 1.000001), &pair(1.000001, 1.000001)).unwrap_err(),
            "session 1: two tp values compare differently after rounding"
        );
    }

    #[test]
    fn each_session_is_judged_on_its_own() {
        let two = |first: f64, second: f64| map(&[(text("sessions"), array(&[session(&[entry(float(first))]), session(&[entry(float(second))])]))]);
        // Only the second session changes; the first holds a tp that could not be rounded.
        assert_eq!(verify(&two(-1e-7, 7.370140500001071), &two(-1e-7, 7.370141)), Ok(()));
    }

    #[test]
    fn a_missing_or_extra_item_is_rejected() {
        let original = export(&[entry(float(1.25)), entry(float(2.25))]);
        assert!(verify(&original, &export(&[entry(float(1.25))])).is_err());
        let mut trailing = original.clone();
        trailing.push(0xf6);
        assert!(verify(&original, &trailing).is_err());
    }
}
