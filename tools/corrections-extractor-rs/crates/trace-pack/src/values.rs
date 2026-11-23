//! The value encoding: an export's time and position floats stored as integers.
//!
//! A compressed trace is mostly `tp`, `t`, `x` and `y` values. Each is a float whose digits are
//! close to random, so it hardly compresses. Here the floats that can be restored are replaced
//! in the export's CBOR by a one-byte placeholder, and their integer form is collected in one
//! stream per field, in document order. See FORMAT.md for the rules and what they guarantee.
//!
//! [`encode`] decides what to convert; [`decode`] puts the floats back. `decode` is driven by
//! the placeholders alone: it never decides again which values were converted.

use crate::wire::{shortest_float_width, write_float_shortest};
use trace_data::cbor::{Cursor, Item};

/// The four fields whose floats are converted. The discriminant is the field's stream index
/// and the low bits of its placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Tp = 0,
    T = 1,
    X = 2,
    Y = 3,
}

/// CBOR simple values 0 to 3. The addon's serializer never writes them and
/// `trace_data::cbor::Cursor` rejects them, so an export that decodes cannot hold one.
const PLACEHOLDER_BASE: u8 = 0xe0;

impl Field {
    fn placeholder(self) -> u8 {
        PLACEHOLDER_BASE + self as u8
    }

    fn from_key(key: &[u8]) -> Option<Field> {
        match key {
            b"tp" => Some(Field::Tp),
            b"t" => Some(Field::T),
            b"x" => Some(Field::X),
            b"y" => Some(Field::Y),
            _ => None,
        }
    }
}

/// `tp` is stored in whole microseconds, `t` in whole milliseconds, positions in 1/10,000.
const TP_STEPS_PER_SECOND: f64 = 1_000_000.0;
const T_STEPS_PER_SECOND: f64 = 1000.0;
const POSITION_STEPS: f64 = 10_000.0;

/// Only integers up to this size are stored. Trace contents are untrusted, and with this bound
/// the difference between two stream values always fits an `i64`.
const MAX_STORED: f64 = 9_007_199_254_740_992.0; // 2^53

/// Deeper nesting than any export that decodes (`trace_data` stops at 100).
const MAX_DEPTH: u32 = 128;

/// Which sessions may have their `tp` values rounded. `t`, `x` and `y` are always converted,
/// because they come back bit for bit.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum TpPolicy {
    /// `tp` stays a float everywhere.
    #[default]
    Off,
    /// Only sessions recorded with one of these `schemaVersion`s: the versions an extractor
    /// parity run has covered.
    SchemaVersions(Vec<u32>),
}

impl TpPolicy {
    /// Whether `tp` may be rounded in a session recorded with this `schemaVersion`.
    pub fn allows(&self, schema_version: Option<f64>) -> bool {
        match (self, schema_version) {
            (TpPolicy::SchemaVersions(versions), Some(version)) => versions.iter().any(|&allowed| allowed as f64 == version),
            _ => false,
        }
    }
}

/// An export in the value encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueEncoded {
    /// The export's CBOR with a placeholder byte where each converted float was.
    pub skeleton: Vec<u8>,
    /// Per [`Field`], the converted values in document order.
    pub streams: [Vec<i64>; 4],
    /// Per session with converted `t` values, in document order: the bits of its `startedAt`
    /// and how many `t` values it holds.
    pub sessions: Vec<(u64, u64)>,
}

/// Why a session's `tp` values were left as floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TpKept {
    /// The policy does not cover the session's `schemaVersion`.
    Policy,
    /// An entry has a `t` without a numeric `tp`, or a float `tp` sits outside such an entry.
    Shape,
    /// A `tp` is not finite or too large to store.
    Range,
    /// Rounding would change how two values of the session compare.
    Order,
}

/// What [`encode`] did, for the per-pack report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EncodeStats {
    /// Floats found under each field's key, inside sessions.
    pub floats: [u64; 4],
    /// Of those, how many were converted.
    pub converted: [u64; 4],
    pub sessions: u64,
    /// Sessions holding float `tp` values that were converted.
    pub tp_sessions_converted: u64,
    /// Sessions holding float `tp` values that were kept, by reason:
    /// `[Policy, Shape, Range, Order]`.
    pub tp_sessions_kept: [u64; 4],
}

impl EncodeStats {
    pub fn add(&mut self, other: &EncodeStats) {
        for field in 0..4 {
            self.floats[field] += other.floats[field];
            self.converted[field] += other.converted[field];
            self.tp_sessions_kept[field] += other.tp_sessions_kept[field];
        }
        self.sessions += other.sessions;
        self.tp_sessions_converted += other.tp_sessions_converted;
    }
}

// ---- Encoding: scan the export ----

/// A value in the export: where its item is and what it holds.
#[derive(Clone, Copy)]
struct Slot {
    start: usize,
    end: usize,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    Float(f64),
    Integer(f64),
    Other,
}

impl Slot {
    fn number(&self) -> Option<f64> {
        match self.kind {
            Kind::Float(value) | Kind::Integer(value) => Some(value),
            Kind::Other => None,
        }
    }

    /// The payload width of a float item: its length without the initial byte.
    fn float_width(&self) -> usize {
        self.end - self.start - 1
    }
}

/// The `t`, `tp`, `x` and `y` values of one map.
#[derive(Default)]
struct MapFields {
    slots: [Option<Slot>; 4],
}

/// Everything in one session that the rules need.
#[derive(Default)]
struct SessionScan {
    started_at: Option<Slot>,
    duration_precise: Option<Slot>,
    schema_version: Option<Slot>,
    /// Float values under `t`, `x` and `y`, in document order.
    t: Vec<Slot>,
    x: Vec<Slot>,
    y: Vec<Slot>,
    /// Every numeric `tp`, integers included: the order rule compares them all.
    tp: Vec<Slot>,
    /// An entry (a map with a numeric `t`) without a numeric `tp`.
    entry_without_tp: bool,
    /// A float `tp` in a map that is no entry.
    tp_outside_entry: bool,
}

impl SessionScan {
    fn note_map(&mut self, fields: &MapFields) {
        let [tp, t, x, y] = fields.slots;
        let is_entry = t.is_some_and(|slot| slot.number().is_some());
        if is_entry && tp.is_none_or(|slot| slot.number().is_none()) {
            self.entry_without_tp = true;
        }
        if let Some(slot) = tp.filter(|slot| slot.number().is_some()) {
            if matches!(slot.kind, Kind::Float(_)) && !is_entry {
                self.tp_outside_entry = true;
            }
            self.tp.push(slot);
        }
        for (slot, list) in [(t, &mut self.t), (x, &mut self.x), (y, &mut self.y)] {
            if let Some(slot) = slot.filter(|slot| matches!(slot.kind, Kind::Float(_))) {
                list.push(slot);
            }
        }
    }
}

/// The export could not be scanned for the value encoding. It is then stored exact.
struct Unsuitable;

struct Scanner<'a> {
    cursor: Cursor<'a>,
}

impl<'a> Scanner<'a> {
    /// Reads a map key and returns its name when it is a string, text or bytes, however its
    /// length is written. A key of another kind has no name; what it contains is read past.
    fn key(&mut self, depth: u32) -> Result<Option<&'a [u8]>, Unsuitable> {
        if depth > MAX_DEPTH {
            return Err(Unsuitable);
        }
        let contained = match self.cursor.item().map_err(|_| Unsuitable)? {
            Item::Str(name) => return Ok(Some(name)),
            Item::Array(length) => length as u64,
            Item::Map(length) => (length as u64).saturating_mul(2),
            _ => 0,
        };
        for _ in 0..contained {
            self.value(None, depth + 1)?;
        }
        Ok(None)
    }

    /// Reads one value and everything in it. Inside a session, every map reports its fields.
    fn value(&mut self, mut session: Option<&mut SessionScan>, depth: u32) -> Result<Slot, Unsuitable> {
        if depth > MAX_DEPTH {
            return Err(Unsuitable);
        }
        let start = self.cursor.pos();
        let initial = self.cursor.peek_byte().ok_or(Unsuitable)?;
        let kind = match self.cursor.item().map_err(|_| Unsuitable)? {
            Item::Map(length) => {
                self.map(length, session.as_deref_mut(), depth, false)?;
                Kind::Other
            }
            Item::Array(length) => {
                for _ in 0..length as u64 {
                    self.value(session.as_deref_mut(), depth + 1)?;
                }
                Kind::Other
            }
            // The cursor returns every number as a double; the initial byte tells which were
            // written as floats (major type 7) and which as integers (0 and 1).
            Item::Num(value) if initial >> 5 == 7 => Kind::Float(value),
            Item::Num(value) => Kind::Integer(value),
            _ => Kind::Other,
        };
        Ok(Slot {
            start,
            end: self.cursor.pos(),
            kind,
        })
    }

    /// The pairs of a map whose header was just read. `is_session` marks a session's own map,
    /// whose `startedAt`, `durationPrecise` and `schemaVersion` are kept for the rules.
    fn map(&mut self, length: f64, mut session: Option<&mut SessionScan>, depth: u32, is_session: bool) -> Result<(), Unsuitable> {
        let mut fields = MapFields::default();
        for _ in 0..length as u64 {
            let name = self.key(depth + 1)?;
            let value = self.value(session.as_deref_mut(), depth + 1)?;
            if let Some(field) = name.and_then(Field::from_key) {
                // A repeated key: which value a reader sees depends on the reader.
                if fields.slots[field as usize].replace(value).is_some() {
                    return Err(Unsuitable);
                }
            }
            if let (true, Some(scan), Some(name)) = (is_session, session.as_deref_mut(), name) {
                let kept = match name {
                    b"startedAt" => Some(&mut scan.started_at),
                    b"durationPrecise" => Some(&mut scan.duration_precise),
                    b"schemaVersion" => Some(&mut scan.schema_version),
                    _ => None,
                };
                if kept.is_some_and(|kept| kept.replace(value).is_some()) {
                    return Err(Unsuitable);
                }
            }
        }
        if let Some(scan) = session {
            scan.note_map(&fields);
        }
        Ok(())
    }

    /// The export's sessions: the maps in the root's `sessions` array.
    fn sessions(&mut self) -> Result<Vec<SessionScan>, Unsuitable> {
        let Item::Map(length) = self.cursor.item().map_err(|_| Unsuitable)? else {
            return Ok(Vec::new());
        };
        let mut sessions = Vec::new();
        let mut seen_sessions = false;
        for _ in 0..length as u64 {
            if self.key(1)? != Some(b"sessions".as_slice()) {
                self.value(None, 1)?;
                continue;
            }
            if std::mem::replace(&mut seen_sessions, true) {
                return Err(Unsuitable);
            }
            // `sessions` that is no array holds no sessions: nothing to convert.
            let Item::Array(count) = self.cursor.item().map_err(|_| Unsuitable)? else {
                return Err(Unsuitable);
            };
            for _ in 0..count as u64 {
                if self.cursor.peek_major() == Some(5) {
                    let Item::Map(length) = self.cursor.item().map_err(|_| Unsuitable)? else {
                        return Err(Unsuitable);
                    };
                    let mut scan = SessionScan::default();
                    self.map(length, Some(&mut scan), 2, true)?;
                    sessions.push(scan);
                } else {
                    self.value(None, 2)?;
                }
            }
        }
        if !self.cursor.is_done() {
            return Err(Unsuitable);
        }
        Ok(sessions)
    }
}

// ---- Encoding: decide what to convert ----

/// One float to replace: where it is, which stream it goes to and its integer form.
struct Conversion {
    start: usize,
    end: usize,
    field: Field,
    value: i64,
}

/// The integer for `value` when it is whole and small enough to store.
fn storable(value: f64) -> Option<i64> {
    (value.is_finite() && value.abs() <= MAX_STORED && value.fract() == 0.0).then_some(value as i64)
}

/// `tp` in whole microseconds, and the float the reader hands back for it. None for a value
/// that cannot be stored, and for a negative one that would round to zero: an integer zero
/// has no sign, so it would come back as positive zero.
fn round_tp(tp: f64) -> Option<(i64, f64)> {
    let steps = storable((tp * TP_STEPS_PER_SECOND).round())?;
    if steps == 0 && tp.is_sign_negative() {
        return None;
    }
    Some((steps, steps as f64 / TP_STEPS_PER_SECOND))
}

/// A position as an integer, when dividing it gives back the same bits in the same width.
fn position_steps(slot: &Slot) -> Option<i64> {
    let Kind::Float(value) = slot.kind else { return None };
    let steps = storable((value * POSITION_STEPS).round())?;
    let restored = steps as f64 / POSITION_STEPS;
    (restored.to_bits() == value.to_bits() && shortest_float_width(restored) == slot.float_width()).then_some(steps)
}

/// The game clock's millisecond count for `t`, when [`restore_t`] gives back the same bits in
/// the same width.
fn t_steps(slot: &Slot, started_at: f64) -> Option<i64> {
    let Kind::Float(value) = slot.kind else { return None };
    let steps = storable(((value + started_at) * T_STEPS_PER_SECOND).round())?;
    let restored = restore_t(steps, started_at);
    (restored.to_bits() == value.to_bits() && shortest_float_width(restored) == slot.float_width()).then_some(steps)
}

/// `t` from the game clock's millisecond count and the session's `startedAt`: the arithmetic
/// the addon did, `GetTime() - startedAt`, with `GetTime()` a count of milliseconds times 0.001.
/// Multiply, then subtract: the two must stay separate operations for the bits to match.
fn restore_t(steps: i64, started_at: f64) -> f64 {
    steps as f64 * 0.001 - started_at
}

/// Whether rounding the float `tp` values leaves every comparison in the session as it was:
/// between any two `tp` values, integers included, and against `durationPrecise`.
fn tp_order_survives(tp: &[Slot], duration_precise: Option<f64>) -> Result<(), TpKept> {
    let mut pairs = Vec::with_capacity(tp.len() + 1);
    for slot in tp {
        match slot.kind {
            Kind::Float(value) => pairs.push((value, round_tp(value).ok_or(TpKept::Range)?.1)),
            Kind::Integer(value) if value.is_finite() => pairs.push((value, value)),
            _ => return Err(TpKept::Range),
        }
    }
    if let Some(duration) = duration_precise {
        if !duration.is_finite() {
            return Err(TpKept::Range);
        }
        pairs.push((duration, duration));
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Sorted by the original value, so neighbours decide every pair: equal must stay equal,
    // smaller must stay smaller.
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
        Err(TpKept::Order)
    }
}

/// Why a session's `tp` values cannot be converted, or `Ok` when they can.
fn tp_verdict(scan: &SessionScan, policy: &TpPolicy) -> Result<(), TpKept> {
    if !policy.allows(scan.schema_version.and_then(|slot| slot.number())) {
        return Err(TpKept::Policy);
    }
    if scan.entry_without_tp || scan.tp_outside_entry {
        return Err(TpKept::Shape);
    }
    tp_order_survives(&scan.tp, scan.duration_precise.and_then(|slot| slot.number()))
}

/// The conversions of one session, appended to `conversions`.
fn plan_session(scan: &SessionScan, policy: &TpPolicy, conversions: &mut Vec<Conversion>, stats: &mut EncodeStats) -> Option<(u64, u64)> {
    stats.sessions += 1;
    let mut convert = |slot: &Slot, field: Field, value: Option<i64>| {
        stats.floats[field as usize] += 1;
        if let Some(value) = value {
            stats.converted[field as usize] += 1;
            conversions.push(Conversion {
                start: slot.start,
                end: slot.end,
                field,
                value,
            });
        }
        value.is_some()
    };

    // Positions: each value on its own.
    for slot in &scan.x {
        convert(slot, Field::X, position_steps(slot));
    }
    for slot in &scan.y {
        convert(slot, Field::Y, position_steps(slot));
    }

    // t: each value on its own, against the session's start time.
    let started_at = scan.started_at.and_then(|slot| slot.number()).filter(|value| value.is_finite());
    let mut t_count = 0;
    for slot in &scan.t {
        let steps = started_at.and_then(|started_at| t_steps(slot, started_at));
        t_count += convert(slot, Field::T, steps) as u64;
    }

    // tp: all of the session's values or none.
    let float_tp: Vec<&Slot> = scan.tp.iter().filter(|slot| matches!(slot.kind, Kind::Float(_))).collect();
    let verdict = tp_verdict(scan, policy);
    for slot in &float_tp {
        let steps = verdict.ok().and_then(|()| slot.number()).and_then(round_tp).map(|(steps, _)| steps);
        convert(slot, Field::Tp, steps);
    }
    if !float_tp.is_empty() {
        match verdict {
            Ok(()) => stats.tp_sessions_converted += 1,
            Err(reason) => stats.tp_sessions_kept[reason as usize] += 1,
        }
    }

    started_at.filter(|_| t_count > 0).map(|started_at| (started_at.to_bits(), t_count))
}

/// The value encoding of an export's CBOR, or None when nothing in it can be converted. The
/// caller is expected to have checked that the export decodes.
pub fn encode(cbor: &[u8], policy: &TpPolicy) -> Option<(ValueEncoded, EncodeStats)> {
    let mut scanner = Scanner { cursor: Cursor::new(cbor) };
    let sessions = scanner.sessions().ok()?;

    let mut stats = EncodeStats::default();
    let mut conversions = Vec::new();
    let session_table = sessions
        .iter()
        .filter_map(|scan| plan_session(scan, policy, &mut conversions, &mut stats))
        .collect();
    if conversions.is_empty() {
        return None;
    }

    // Each session lists its fields separately; the streams and the skeleton need document order.
    conversions.sort_by_key(|conversion| conversion.start);
    let mut skeleton = Vec::with_capacity(cbor.len());
    let mut streams: [Vec<i64>; 4] = Default::default();
    let mut copied = 0;
    for conversion in &conversions {
        skeleton.extend_from_slice(&cbor[copied..conversion.start]);
        skeleton.push(conversion.field.placeholder());
        streams[conversion.field as usize].push(conversion.value);
        copied = conversion.end;
    }
    skeleton.extend_from_slice(&cbor[copied..]);

    let encoded = ValueEncoded {
        skeleton,
        streams,
        sessions: session_table,
    };
    Some((encoded, stats))
}

// ---- Decoding ----

/// The export's CBOR, with every placeholder replaced by its float.
///
/// One pass over the skeleton's items, no nesting to track: strings are skipped by their
/// length, and every other item is a header whose contents follow as further items.
pub fn decode(encoded: &ValueEncoded) -> Result<Vec<u8>, String> {
    let skeleton = &encoded.skeleton;
    let mut out = Vec::with_capacity(skeleton.len() + 8 * encoded.streams.iter().map(Vec::len).sum::<usize>());
    let mut next = [0usize; 4];
    let mut sessions = encoded.sessions.iter();
    // The session the current `t` values belong to: its start time and how many are left.
    let mut session: Option<(f64, u64)> = None;

    let mut pos = 0;
    let mut copied = 0;
    while pos < skeleton.len() {
        let initial = skeleton[pos];
        let (major, info) = (initial >> 5, initial & 31);

        // A placeholder: copy what came before it, then write its float.
        if major == 7 && info < 4 {
            let field = [Field::Tp, Field::T, Field::X, Field::Y][info as usize];
            let index = &mut next[field as usize];
            let value = *encoded.streams[field as usize]
                .get(*index)
                .ok_or("a value stream ended before its placeholders did")?;
            *index += 1;
            let float = match field {
                Field::Tp => value as f64 / TP_STEPS_PER_SECOND,
                Field::X | Field::Y => value as f64 / POSITION_STEPS,
                Field::T => {
                    if session.is_none_or(|(_, left)| left == 0) {
                        let &(started_at, count) = sessions.next().ok_or("more t values than the session table lists")?;
                        if count == 0 {
                            return Err("a session row lists no t values".into());
                        }
                        session = Some((f64::from_bits(started_at), count));
                    }
                    let (started_at, left) = session.as_mut().expect("just set");
                    *left -= 1;
                    restore_t(value, *started_at)
                }
            };
            out.extend_from_slice(&skeleton[copied..pos]);
            write_float_shortest(&mut out, float);
            pos += 1;
            copied = pos;
            continue;
        }

        // Any other item: step over its header, and over a string's contents.
        let argument_width = match info {
            0..=23 => 0,
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            _ => return Err("unsupported item in a value-encoded export".into()),
        };
        let header_end = pos + 1 + argument_width;
        let argument = skeleton.get(pos + 1..header_end).ok_or("truncated value-encoded export")?;
        pos = header_end;
        if matches!(major, 2 | 3) {
            let length = if argument_width == 0 {
                info as u64
            } else {
                argument.iter().fold(0u64, |length, &byte| (length << 8) | byte as u64)
            };
            pos = usize::try_from(length)
                .ok()
                .and_then(|length| pos.checked_add(length))
                .filter(|&end| end <= skeleton.len())
                .ok_or("truncated value-encoded export")?;
        }
    }
    out.extend_from_slice(&skeleton[copied..]);

    let all_used = (0..4).all(|field| next[field] == encoded.streams[field].len());
    let sessions_used = sessions.next().is_none() && session.is_none_or(|(_, left)| left == 0);
    if !all_used || !sessions_used {
        return Err("a value stream holds more values than the export has placeholders".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    const ANY_VERSION: u32 = 17;

    fn policy() -> TpPolicy {
        TpPolicy::SchemaVersions(vec![ANY_VERSION])
    }

    /// A session with the given start time and entries, recorded with [`ANY_VERSION`].
    fn session(started_at: f64, entries: Vec<Vec<u8>>) -> Vec<u8> {
        map(&[
            (text("schemaVersion"), uint(ANY_VERSION as u64)),
            (text("startedAt"), float(started_at)),
            (text("events"), array(&entries)),
        ])
    }

    fn entry(t: f64, tp: f64) -> Vec<u8> {
        map(&[(text("t"), float(t)), (text("tp"), float(tp))])
    }

    fn export(sessions: &[Vec<u8>]) -> Vec<u8> {
        map(&[(text("sessions"), array(sessions))])
    }

    fn encoded(cbor: &[u8]) -> ValueEncoded {
        encode(cbor, &policy()).expect("something to convert").0
    }

    /// A `t` as the addon computes it: the game clock, a millisecond count times 0.001, minus
    /// the session's start time.
    fn game_t(clock_ms: i64, started_at: f64) -> f64 {
        clock_ms as f64 * 0.001 - started_at
    }

    const STARTED_AT: f64 = 8551.460000000001;

    #[test]
    fn a_t_from_the_game_clock_becomes_its_millisecond_count_and_comes_back_with_the_same_bits() {
        let t = game_t(8_559_178, STARTED_AT);
        // The value the addon stores is not a clean decimal: it carries the subtraction's error.
        assert_ne!(t, 7.718);
        let cbor = export(&[session(STARTED_AT, vec![entry(t, 7.370141)])]);
        let encoded = encoded(&cbor);
        assert_eq!(encoded.streams[Field::T as usize], [8_559_178]);
        assert_eq!(encoded.sessions, [(STARTED_AT.to_bits(), 1)]);
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn a_t_that_the_formula_does_not_reproduce_stays_a_float() {
        // One unit in the last place away from a game clock value.
        let t = game_t(8_559_178, STARTED_AT).next_up();
        let cbor = export(&[session(STARTED_AT, vec![entry(t, 7.370141)])]);
        let (encoded, stats) = encode(&cbor, &policy()).unwrap();
        assert!(encoded.streams[Field::T as usize].is_empty());
        assert_eq!((stats.floats[Field::T as usize], stats.converted[Field::T as usize]), (1, 0));
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn a_position_with_four_decimals_becomes_an_integer_and_one_with_more_stays_a_float() {
        let position = map(&[(text("x"), double(0.4733)), (text("y"), double(0.3619576692581177))]);
        let cbor = export(&[session(STARTED_AT, vec![map(&[(text("v"), position)])])]);
        let encoded = encoded(&cbor);
        assert_eq!(encoded.streams[Field::X as usize], [4733]);
        assert!(encoded.streams[Field::Y as usize].is_empty());
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn floats_written_in_two_and_four_bytes_come_back_in_the_same_width() {
        // 0.5 is a half float; 4126.25 - 0 needs a single. Both pass the bit check.
        let t = 4126.25;
        let position = map(&[(text("x"), half(0x3800)), (text("y"), double(0.25))]);
        let entry = map(&[(text("t"), single(t as f32)), (text("tp"), double(4126.250001)), (text("v"), position)]);
        let cbor = export(&[session(0.0, vec![entry])]);
        let encoded = encoded(&cbor);
        assert_eq!(encoded.streams[Field::T as usize], [4_126_250]);
        assert_eq!(encoded.streams[Field::X as usize], [5000]);
        // 0.25 was written as a double although a half float holds it: restoring it would
        // write two bytes, so it is left alone.
        assert!(encoded.streams[Field::Y as usize].is_empty());
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn values_already_integers_and_negative_zero_are_never_converted() {
        let baseline = map(&[(text("t"), uint(0)), (text("tp"), uint(0))]);
        let position = map(&[(text("x"), uint(0)), (text("y"), half(0x8000))]);
        let cbor = export(&[session(STARTED_AT, vec![baseline, map(&[(text("v"), position)])])]);
        assert_eq!(encode(&cbor, &policy()), None);
    }

    #[test]
    fn tp_becomes_whole_microseconds_and_comes_back_rounded() {
        let cbor = export(&[session(STARTED_AT, vec![entry(game_t(8_559_178, STARTED_AT), 7.370140500001071)])]);
        let encoded = encoded(&cbor);
        assert_eq!(encoded.streams[Field::Tp as usize], [7_370_141]);
        let expected = export(&[session(STARTED_AT, vec![entry(game_t(8_559_178, STARTED_AT), 7.370141)])]);
        assert_eq!(decode(&encoded).unwrap(), expected);
    }

    fn tp_stats(cbor: &[u8], policy: &TpPolicy) -> (u64, [u64; 4]) {
        let stats = encode(cbor, policy).map(|(_, stats)| stats).unwrap_or_default();
        (stats.tp_sessions_converted, stats.tp_sessions_kept)
    }

    #[test]
    fn a_session_keeps_float_tp_when_two_values_would_merge() {
        // 0.4 microseconds apart: both round to 1,000,001 microseconds.
        let close = vec![entry(game_t(8_559_178, STARTED_AT), 1.0000006), entry(game_t(8_559_178, STARTED_AT), 1.000001)];
        assert_eq!(tp_stats(&export(&[session(STARTED_AT, close)]), &policy()), (0, [0, 0, 0, 1]));
        // 2 microseconds apart: converted.
        let apart = vec![entry(game_t(8_559_178, STARTED_AT), 1.0000006), entry(game_t(8_559_178, STARTED_AT), 1.0000026)];
        assert_eq!(tp_stats(&export(&[session(STARTED_AT, apart)]), &policy()), (1, [0; 4]));
    }

    #[test]
    fn a_float_tp_that_would_round_onto_an_integer_tp_keeps_the_session_as_floats() {
        // A capture-start baseline carries tp = 0 as an integer; 0.3 microseconds rounds to 0.
        let baseline = map(&[(text("t"), uint(0)), (text("tp"), uint(0))]);
        let cbor = export(&[session(STARTED_AT, vec![baseline, entry(game_t(8_559_178, STARTED_AT), 3e-7)])]);
        assert_eq!(tp_stats(&cbor, &policy()), (0, [0, 0, 0, 1]));
    }

    #[test]
    fn a_tp_that_would_cross_duration_precise_keeps_the_session_as_floats() {
        // The relations pipeline tests `durationPrecise >= last tp`: 100.0000006 would round
        // up past a duration of 100.0000008.
        let with_duration = |duration: f64| {
            map(&[
                (text("schemaVersion"), uint(ANY_VERSION as u64)),
                (text("startedAt"), double(STARTED_AT)),
                (text("durationPrecise"), double(duration)),
                (text("events"), array(&[entry(game_t(8_559_178, STARTED_AT), 100.0000006)])),
            ])
        };
        assert_eq!(tp_stats(&export(&[with_duration(100.0000008)]), &policy()), (0, [0, 0, 0, 1]));
        assert_eq!(tp_stats(&export(&[with_duration(100.5)]), &policy()), (1, [0; 4]));
    }

    #[test]
    fn equal_tp_values_stay_equal_so_one_sampling_pass_still_shares_its_time() {
        let shared = vec![entry(game_t(8_559_178, STARTED_AT), 5.0000004), entry(game_t(8_559_178, STARTED_AT), 5.0000004)];
        let cbor = export(&[session(STARTED_AT, shared)]);
        assert_eq!(encoded(&cbor).streams[Field::Tp as usize], [5_000_000, 5_000_000]);
    }

    #[test]
    fn tp_stays_a_float_when_an_entry_lacks_it_or_it_sits_outside_an_entry() {
        let no_tp = map(&[(text("t"), float(game_t(8_559_178, STARTED_AT)))]);
        let lacking = export(&[session(STARTED_AT, vec![no_tp, entry(game_t(8_559_178, STARTED_AT), 2.5000001)])]);
        assert_eq!(tp_stats(&lacking, &policy()), (0, [0, 1, 0, 0]));
        // A float under a key named tp in a map without t: some API's own table, not a sample.
        let stray = map(&[(text("tp"), double(2.5000001))]);
        let outside = export(&[session(STARTED_AT, vec![stray, entry(game_t(8_559_178, STARTED_AT), 3.5)])]);
        assert_eq!(tp_stats(&outside, &policy()), (0, [0, 1, 0, 0]));
    }

    #[test]
    fn tp_is_rounded_only_for_schema_versions_the_policy_names() {
        let cbor = export(&[session(STARTED_AT, vec![entry(game_t(8_559_178, STARTED_AT), 7.370140500001071)])]);
        assert_eq!(tp_stats(&cbor, &TpPolicy::Off), (0, [1, 0, 0, 0]));
        assert_eq!(tp_stats(&cbor, &TpPolicy::SchemaVersions(vec![ANY_VERSION + 1])), (0, [1, 0, 0, 0]));
        assert_eq!(tp_stats(&cbor, &policy()), (1, [0; 4]));
        // t is still converted when tp is not.
        let (encoded, _) = encode(&cbor, &TpPolicy::Off).unwrap();
        assert_eq!((encoded.streams[Field::T as usize].len(), encoded.streams[Field::Tp as usize].len()), (1, 0));
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn values_too_large_or_not_finite_are_left_alone() {
        let position = map(&[(text("x"), double(9.0e14)), (text("y"), double(f64::INFINITY))]);
        let huge = map(&[(text("t"), double(1.0e300)), (text("tp"), double(f64::NAN)), (text("v"), position)]);
        let cbor = export(&[session(STARTED_AT, vec![huge])]);
        assert_eq!(encode(&cbor, &policy()), None);
        // 9e11 times 10,000 is at the 2^53 limit's edge: 9.0e11 is whole and converts.
        let large = export(&[session(STARTED_AT, vec![map(&[(text("x"), double(9.0e11))])])]);
        assert_eq!(encoded(&large).streams[Field::X as usize], [9_000_000_000_000_000]);
    }

    #[test]
    fn each_session_uses_its_own_start_time() {
        let (first, second) = (100.25, 99_000.125);
        let cbor = export(&[
            session(first, vec![entry(game_t(101_000, first), 0.75), entry(game_t(102_500, first), 2.25)]),
            // A session with nothing to convert between them leaves no row in the table.
            session(50.0, vec![]),
            session(second, vec![entry(game_t(99_000_500, second), 0.375)]),
        ]);
        let encoded = encoded(&cbor);
        assert_eq!(encoded.sessions, [(first.to_bits(), 2), (second.to_bits(), 1)]);
        assert_eq!(encoded.streams[Field::T as usize], [101_000, 102_500, 99_000_500]);
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn a_string_holding_placeholder_bytes_is_copied_not_expanded() {
        let name = bytes(&[0xe0, 0xe1, 0xe2, 0xe3]);
        let cbor = export(&[session(STARTED_AT, vec![map(&[(text("e"), name), (text("x"), double(0.5001))])])]);
        assert_eq!(decode(&encoded(&cbor)).unwrap(), cbor);
    }

    #[test]
    fn a_repeated_field_key_or_values_outside_sessions_leave_the_export_exact() {
        let repeated = map(&[(text("x"), double(0.4733)), (text("x"), double(0.4734))]);
        assert_eq!(encode(&export(&[session(STARTED_AT, vec![repeated])]), &policy()), None);
        // Positions outside any session are not converted.
        let loose = map(&[(text("sessions"), array(&[])), (text("x"), double(0.4733))]);
        assert_eq!(encode(&loose, &policy()), None);
    }

    #[test]
    fn a_string_of_any_length_holding_placeholder_bytes_is_copied() {
        // 24, 256 and 65,536 bytes: the three ways a longer length is written.
        for length in [24, 256, 65_536] {
            let filler = bytes(&vec![0xe1; length]);
            let cbor = export(&[session(STARTED_AT, vec![map(&[(text("e"), filler), (text("x"), double(0.5001))])])]);
            assert_eq!(decode(&encoded(&cbor)).unwrap(), cbor, "{length} bytes");
        }
    }

    #[test]
    fn a_key_with_its_length_written_long_is_still_the_same_key() {
        // 78 01 "x": the key x with its length in a separate byte. It repeats the key x.
        let long_x = [vec![0x78, 0x01, b'x'], double(0.9001)].concat();
        let repeated = [vec![0xa2], text("x"), double(0.4733), long_x.clone()].concat();
        assert_eq!(encode(&export(&[session(STARTED_AT, vec![repeated])]), &policy()), None);
        // On its own it is converted like any x.
        let alone = [vec![0xa1], long_x].concat();
        let cbor = export(&[session(STARTED_AT, vec![alone])]);
        assert_eq!(encoded(&cbor).streams[Field::X as usize], [9001]);
        assert_eq!(decode(&encoded(&cbor)).unwrap(), cbor);
    }

    #[test]
    fn t_needs_a_numeric_start_time_and_an_integer_one_serves() {
        let entries = || vec![entry(1.5, 2.000001)];
        let with_start = |started_at: Option<Vec<u8>>| {
            let mut pairs = vec![(text("schemaVersion"), uint(ANY_VERSION as u64)), (text("events"), array(&entries()))];
            pairs.extend(started_at.map(|value| (text("startedAt"), value)));
            export(&[map(&pairs)])
        };
        // No start time, or one that is no number: t stays, tp is still converted.
        for missing in [None, Some(text("soon"))] {
            let encoded = encoded(&with_start(missing));
            assert_eq!((encoded.streams[Field::T as usize].len(), encoded.streams[Field::Tp as usize].len()), (0, 1));
            assert!(encoded.sessions.is_empty());
        }
        // A start time written as the integer 8551: the game clock stood at 8552.5 s.
        let cbor = with_start(Some(uint(8551)));
        let encoded = encoded(&cbor);
        assert_eq!(encoded.streams[Field::T as usize], [8_552_500]);
        assert_eq!(encoded.sessions, [(8551f64.to_bits(), 1)]);
        assert_eq!(decode(&encoded).unwrap(), cbor);
    }

    #[test]
    fn a_tp_that_would_come_back_with_another_sign_keeps_the_session_as_floats() {
        for tp in [-0.0, -1e-7] {
            let cbor = export(&[session(STARTED_AT, vec![entry(game_t(8_559_178, STARTED_AT), tp)])]);
            assert_eq!(tp_stats(&cbor, &policy()), (0, [0, 0, 1, 0]), "{tp}");
        }
    }

    #[test]
    fn a_session_row_without_values_is_rejected() {
        let cbor = export(&[session(STARTED_AT, vec![entry(game_t(8_559_178, STARTED_AT), 2.000001)])]);
        let mut empty_row = encoded(&cbor);
        empty_row.sessions.insert(0, (0, 0));
        assert_eq!(decode(&empty_row).unwrap_err(), "a session row lists no t values");
    }

    /// Small deterministic generator: the same traces on every run.
    struct Random(u64);

    impl Random {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % bound
        }

        fn number(&mut self) -> Vec<u8> {
            match self.below(8) {
                0 => uint(self.below(5)),
                1 => half([0x0000, 0x8000, 0x3c00, 0x7c00, 0x7e00][self.below(5) as usize]),
                2 => single(self.below(1_000_000) as f32 / 64.0),
                3 => double(self.below(1_000_000) as f64 / 10_000.0),
                4 => float(self.below(100_000) as f64 * 0.001 - 12.3),
                5 => float(self.below(1 << 40) as f64 / 1e7),
                6 => text("x"),
                _ => float(self.below(10_000) as f64 / 1e4),
            }
        }

        /// A map holding each of the keys at most once: a repeated key leaves an export exact.
        fn entry(&mut self) -> Vec<u8> {
            let mut pairs = Vec::new();
            for key in ["t", "tp", "x", "y", "v", "e"] {
                if self.below(2) == 0 {
                    let value = if key == "v" && self.below(2) == 0 { self.entry() } else { self.number() };
                    pairs.push((text(key), value));
                }
            }
            map(&pairs)
        }
    }

    #[test]
    fn random_traces_come_back_byte_for_byte_when_tp_is_not_rounded() {
        let mut random = Random(2026);
        let mut converted = 0;
        for _ in 0..3000 {
            let sessions: Vec<_> = (0..random.below(3))
                .map(|_| {
                    let entries = (0..random.below(6)).map(|_| random.entry()).collect();
                    session(random.below(100_000) as f64 * 0.001, entries)
                })
                .collect();
            let cbor = export(&sessions);
            if let Some((encoded, _)) = encode(&cbor, &TpPolicy::Off) {
                converted += 1;
                assert_eq!(decode(&encoded).unwrap(), cbor);
            }
        }
        // The generator must actually reach the encoder for the test to mean anything.
        assert!(converted > 500, "only {converted} traces had something to convert");
    }

    #[test]
    fn decoding_rejects_streams_that_do_not_match_the_placeholders() {
        let cbor = export(&[session(STARTED_AT, vec![map(&[(text("x"), double(0.4733))])])]);
        let good = encoded(&cbor);
        let mut short = good.clone();
        short.streams[Field::X as usize].clear();
        assert_eq!(decode(&short).unwrap_err(), "a value stream ended before its placeholders did");
        let mut long = good.clone();
        long.streams[Field::Y as usize].push(1);
        assert_eq!(decode(&long).unwrap_err(), "a value stream holds more values than the export has placeholders");
    }
}
