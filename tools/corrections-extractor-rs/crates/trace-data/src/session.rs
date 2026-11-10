//! A recorded session, read through the decoded value tree. See specs/TRACE_FILE_SPEC.md
//! for the shape; `core/types.ts` types it for the TypeScript tool.

use crate::value::{KeyRef, Object, Value};

/// One valid session of an export.
#[derive(Clone)]
pub struct Session<'v, 'a> {
    record: &'v Object<'a>,
    /// Every event's name and times, read once: observers walk all events many times, and
    /// reading them from the decoded event objects missed the cache on every event, every pass.
    events: Vec<Event<'v, 'a>>,
    functions: &'v Object<'a>,
}

impl<'v, 'a> Session<'v, 'a> {
    /// submissions.ts `isSessionRecord`: an object with an `events` array and a `functions`
    /// object. Anything else is no session.
    pub fn new(value: &'v Value<'a>) -> Option<Self> {
        let record = value.as_object()?;
        let events = record.get(KeyRef::Str("events"))?.as_array()?;
        let functions = record.get(KeyRef::Str("functions"))?.as_object()?;
        let events = events.iter().map(Event::read).collect();
        Some(Session { record, events, functions })
    }

    /// A top-level field such as `startedAt` or `buildInfo`.
    pub fn field(&self, name: &str) -> Option<&'v Value<'a>> {
        self.record.get(KeyRef::Str(name))
    }

    pub fn started_at(&self) -> Option<f64> {
        self.field("startedAt").and_then(Value::as_num)
    }

    pub fn started_at_precise(&self) -> Option<f64> {
        self.field("startedAtPrecise").and_then(Value::as_num)
    }

    /// The recorded events, in order. Every event counts toward indices, read or not.
    pub fn events(&self) -> impl ExactSizeIterator<Item = Event<'v, 'a>> + Clone + '_ {
        self.events.iter().copied()
    }

    pub fn event(&self, index: usize) -> Option<Event<'v, 'a>> {
        self.events.get(index).copied()
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// `session.functions`.
    pub fn functions(&self) -> &'v Object<'a> {
        self.functions
    }

    /// `session.functions[name]`, decoded on first read.
    pub fn function(&self, name: &str) -> Option<&'v Value<'a>> {
        self.functions.get(KeyRef::Str(name))
    }

    /// `session.functionsDelta[name]`.
    pub fn function_delta(&self, name: &str) -> Option<&'v Value<'a>> {
        self.field("functionsDelta")?.get(name)
    }

    /// Stable identity of the session across trace files and submissions: its two
    /// capture-start clocks. None when either is missing; such a session cannot be matched
    /// with its copies.
    pub fn key(&self) -> Option<SessionKey> {
        let started_at = self.started_at().filter(|n| n.is_finite())?;
        let precise = self.started_at_precise().filter(|n| n.is_finite())?;
        // +0.0 turns -0 into 0: String(-0) is "0", so TS keys them alike.
        Some(SessionKey((started_at + 0.0).to_bits(), (precise + 0.0).to_bits()))
    }
}

/// See [`Session::key`]. Compared for equality only.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SessionKey(u64, u64);

/// One recorded event: `{ t, tp, e, a }`, with its name and times already read.
#[derive(Clone, Copy)]
pub struct Event<'v, 'a> {
    value: &'v Value<'a>,
    name: Option<&'v str>,
    t: f64,
    tp: f64,
}

impl<'v, 'a> Event<'v, 'a> {
    fn read(value: &'v Value<'a>) -> Self {
        if let Some(header) = value.event_header() {
            return Event {
                value,
                name: header.name,
                t: header.t,
                tp: header.tp,
            };
        }
        let num = |key: &str| value.get(key).and_then(Value::as_num).unwrap_or(f64::NAN);
        Event {
            value,
            name: value.get("e").and_then(Value::as_str),
            t: num("t"),
            tp: num("tp"),
        }
    }

    /// The event name (`e`).
    pub fn name(&self) -> Option<&'v str> {
        self.name
    }

    pub fn is(&self, name: &str) -> bool {
        self.name == Some(name)
    }

    /// Session time (`t`); NaN when missing, so comparisons fail like JS `undefined` does.
    pub fn t(&self) -> f64 {
        self.t
    }

    pub fn tp(&self) -> f64 {
        self.tp
    }

    /// The packed arguments (`a`): `{ [1] = ..., n = count }`, decoded on first read.
    pub fn args(&self) -> Option<&'v Value<'a>> {
        self.value.get("a")
    }

    /// `a.n`, the argument count.
    pub fn arg_count(&self) -> Option<f64> {
        self.args()?.get("n").and_then(Value::as_num)
    }

    /// Argument `index` (1-based, like Lua and the packed table).
    pub fn arg(&self, index: u32) -> Option<&'v Value<'a>> {
        self.args()?.get(index)
    }

    pub fn value(&self) -> &'v Value<'a> {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{decode_trace, Arena};

    fn text(s: &str) -> Vec<u8> {
        let mut out = vec![0x60 | s.len() as u8];
        out.extend(s.as_bytes());
        out
    }

    /// `{"sessions": [{"events": [event], "functions": {"F": 1}}]}`
    fn session_bytes(event: &[u8]) -> Vec<u8> {
        let mut out = vec![0xa1];
        out.extend(text("sessions"));
        out.extend([0x81, 0xa2]);
        out.extend(text("events"));
        out.push(0x81);
        out.extend(event);
        out.extend(text("functions"));
        out.push(0xa1);
        out.extend(text("F"));
        out.push(0x01);
        out
    }

    #[test]
    fn an_events_header_read_from_its_bytes_matches_the_decoded_event() {
        let f64_bytes = |n: f64| -> Vec<u8> { std::iter::once(0xfb).chain(n.to_be_bytes()).collect() };
        let mut plain = vec![0xa4];
        plain.extend(text("t"));
        plain.extend(f64_bytes(1.25));
        plain.extend(text("tp"));
        plain.extend(f64_bytes(1.5));
        plain.extend(text("e"));
        plain.extend(text("QUEST_ACCEPTED"));
        plain.extend(text("a"));
        plain.extend([0xa2, 0x01, 0x07, 0x61, b'n', 0x01]);
        let events: Vec<(&str, Vec<u8>)> = vec![
            ("plain", plain),
            // "e" deleted by a nil and set again: decoding decides.
            ("deleted then set", [vec![0xa2], text("e"), vec![0xf6], text("e"), text("X")].concat()),
            ("a number name", [vec![0xa1], text("e"), vec![0x05]].concat()),
            ("a string time", [vec![0xa2], text("t"), text("1"), text("e"), text("X")].concat()),
            ("a table time", [vec![0xa2], text("t"), vec![0x81, 0x01], text("e"), text("X")].concat()),
            ("a numeric key", [vec![0xa2, 0x01], text("x"), text("e"), text("X")].concat()),
            ("invalid UTF-8 name", [vec![0xa1], text("e"), vec![0x62, 0xff, 0xfe]].concat()),
            // Not a map at all (an integer-keyed array here would make the events table an
            // object, so no session).
            ("not a map", text("x")),
        ];
        for (case, event) in events {
            let bytes = session_bytes(&event);
            let arena = Arena::new();
            let root = decode_trace(&bytes, &arena).unwrap().unwrap();
            let sessions = root.get("sessions").and_then(Value::as_array).unwrap();
            let session = Session::new(&sessions[0]).unwrap_or_else(|| panic!("{case}: no session"));
            let header = session.event(0).unwrap();
            // Decoding the event afterwards is the reference.
            let decoded = &session.events[0].value;
            let num = |key: &str| decoded.get(key).and_then(Value::as_num).unwrap_or(f64::NAN);
            assert_eq!(header.name(), decoded.get("e").and_then(Value::as_str), "{case}: name");
            assert_eq!(header.t().to_bits(), num("t").to_bits(), "{case}: t");
            assert_eq!(header.tp().to_bits(), num("tp").to_bits(), "{case}: tp");
        }
    }
}
