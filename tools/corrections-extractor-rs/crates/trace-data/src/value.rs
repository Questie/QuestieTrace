//! Decoded Lua values, in the shape `core/trace-data/cbor.ts` gives every trace consumer:
//!
//! - nil never appears; keys other than numbers and strings are dropped; a nil value deletes
//!   its key
//! - a table whose keys are exactly 1..n is an array, unless its first value is itself a
//!   table with only positive integer keys (index-keyed streams stay objects)
//! - every other table is an object; an empty table is an empty array
//!
//! Objects iterate their keys in JS property order (array-index keys ascending, then the
//! others in insertion order), and look keys up the way JS does: `100` and `"100"` are the
//! same key.
//!
//! Function streams, `functionsDelta` and event arguments are most of a session and most of
//! them are never read, so they are only validated while decoding and stay [`Value::Lazy`]
//! until first read. Validation reports every error a full decode would, so a lazy value
//! always decodes later. A function stream, once read, keeps its entries lazy in turn: most are
//! only ever probed by a binary search over time.

use crate::cbor::{CborError, Cursor, Item, Result, MAX_NESTING_DEPTH};
use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;

pub enum Value<'a> {
    Bool(bool),
    Num(f64),
    Str(Cow<'a, str>),
    Array(Vec<Value<'a>>),
    Object(Object<'a>),
    /// Not decoded yet; every accessor decodes it first.
    Lazy(Lazy<'a>),
}

impl fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.resolved() {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Num(n) => f.write_str(&format_number(*n)),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Array(items) => f.debug_list().entries(items).finish(),
            Value::Object(object) => f.debug_map().entries(object.entries().map(|(k, v)| (k.to_string(), v))).finish(),
            Value::Lazy(_) => unreachable!("resolved() never returns a lazy value"),
        }
    }
}

impl<'a> Value<'a> {
    /// The decoded value: itself, or a lazy value decoded on first call.
    pub fn resolved(&self) -> &Value<'a> {
        match self {
            Value::Lazy(lazy) => lazy.get(),
            other => other,
        }
    }

    /// JS `Array.isArray`.
    pub fn is_array(&self) -> bool {
        matches!(self.resolved(), Value::Array(_))
    }

    /// JS `typeof v === "object"` for a decoded value: an array or an object.
    pub fn is_table(&self) -> bool {
        matches!(self.resolved(), Value::Array(_) | Value::Object(_))
    }

    pub fn as_array(&self) -> Option<&[Value<'a>]> {
        match self.resolved() {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Object<'a>> {
        match self.resolved() {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<f64> {
        match self.resolved() {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self.resolved() {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.resolved() {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// JS `table[key]` on an array or object; None for anything else or a missing key.
    pub fn get<'k>(&self, key: impl Into<KeyRef<'k>>) -> Option<&Value<'a>> {
        let key = key.into();
        match self.resolved() {
            Value::Object(object) => object.get(key),
            Value::Array(items) => match key.js_key() {
                // JS arrays are 0-based; Lua's 1..n land at 0..n-1.
                JsKey::Index(i) => items.get(i as usize),
                JsKey::Name(_) => None,
            },
            _ => None,
        }
    }

    /// JS `Object.keys(v)` for an object; array indices as strings for an array.
    pub fn keys(&self) -> Vec<String> {
        match self.resolved() {
            Value::Object(object) => object.entries().map(|(key, _)| key.to_string()).collect(),
            Value::Array(items) => (0..items.len()).map(|i| i.to_string()).collect(),
            _ => Vec::new(),
        }
    }
}

/// The value as `JSON.stringify` prints the TS decoder's result: for comparing the two decoders
/// and in tests. Lazy values are decoded.
pub fn to_json(value: &Value) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json_string(out: &mut String, s: &str) {
    use std::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => write!(out, "\\u{:04x}", c as u32).expect("writing to a String"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_json(out: &mut String, value: &Value) {
    match value.resolved() {
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) if n.is_finite() => out.push_str(&format_number(*n)),
        Value::Num(_) => out.push_str("null"),
        Value::Str(s) => write_json_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Value::Object(object) => {
            out.push('{');
            for (i, (key, item)) in object.entries().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(out, &key.to_string());
                out.push(':');
                write_json(out, item);
            }
            out.push('}');
        }
        Value::Lazy(_) => unreachable!("resolved() never returns a lazy value"),
    }
}

/// A key to look up: a number or a string, compared as JS property keys.
#[derive(Clone, Copy)]
pub enum KeyRef<'k> {
    Num(f64),
    Str(&'k str),
}

impl From<f64> for KeyRef<'_> {
    fn from(n: f64) -> Self {
        KeyRef::Num(n)
    }
}

impl From<i64> for KeyRef<'_> {
    fn from(n: i64) -> Self {
        KeyRef::Num(n as f64)
    }
}

impl From<u32> for KeyRef<'_> {
    fn from(n: u32) -> Self {
        KeyRef::Num(n as f64)
    }
}

impl From<i32> for KeyRef<'_> {
    fn from(n: i32) -> Self {
        KeyRef::Num(n as f64)
    }
}

impl<'k> From<&'k str> for KeyRef<'k> {
    fn from(s: &'k str) -> Self {
        KeyRef::Str(s)
    }
}

impl<'k> KeyRef<'k> {
    fn js_key(self) -> JsKey<'k> {
        match self {
            KeyRef::Num(n) => JsKey::from_num(n),
            KeyRef::Str(s) => JsKey::from_str(Cow::Borrowed(s)),
        }
    }
}

/// A property key as JS sees it: an array index (canonical integer 0 ..= 2^32 - 2) or a name.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum JsKey<'a> {
    Index(u32),
    Name(Cow<'a, str>),
}

const MAX_ARRAY_INDEX: f64 = 4_294_967_294.0;

impl<'a> JsKey<'a> {
    fn from_num(n: f64) -> JsKey<'static> {
        // -0 is the index 0: String(-0) === "0".
        if n.fract() == 0.0 && (0.0..=MAX_ARRAY_INDEX).contains(&n) {
            JsKey::Index(n as u32)
        } else {
            JsKey::Name(Cow::Owned(format_number(n)))
        }
    }

    fn from_str(s: Cow<'a, str>) -> JsKey<'a> {
        match canonical_index(&s) {
            Some(i) => JsKey::Index(i),
            None => JsKey::Name(s),
        }
    }

    /// cbor.ts `isIntegerKey`: the key reads back as a positive integer.
    fn is_positive_integer(&self) -> bool {
        match self {
            JsKey::Index(i) => *i >= 1,
            JsKey::Name(name) => {
                let first = name.as_bytes().first().copied().unwrap_or(b'0');
                if first.is_ascii_alphabetic() || first == b'_' {
                    return false;
                }
                // Only canonical forms: String(Number(name)) must give the name back.
                match name.parse::<f64>() {
                    Ok(n) => n.fract() == 0.0 && n >= 1.0 && n.is_finite() && format_number(n) == **name,
                    Err(_) => false,
                }
            }
        }
    }
}

impl fmt::Display for JsKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsKey::Index(i) => write!(f, "{i}"),
            JsKey::Name(name) => f.write_str(name),
        }
    }
}

/// The array index a canonical decimal string names: "0" or digits without a leading zero.
fn canonical_index(s: &str) -> Option<u32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 10 || !bytes.iter().all(u8::is_ascii_digit) || (bytes[0] == b'0' && bytes.len() > 1) {
        return None;
    }
    let n: u64 = s.parse().ok()?;
    (n as f64 <= MAX_ARRAY_INDEX).then_some(n as u32)
}

/// JS `String(n)`.
pub fn format_number(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

/// A table that is not an array, in JS property order.
pub struct Object<'a> {
    /// Array-index keys ascending, then the other keys in insertion order.
    entries: Vec<(JsKey<'a>, Value<'a>)>,
    /// How many leading entries have an index key.
    index_count: usize,
}

impl<'a> Object<'a> {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// JS `Object.entries`.
    pub fn entries(&self) -> impl Iterator<Item = (&JsKey<'a>, &Value<'a>)> {
        self.entries.iter().map(|(key, value)| (key, value))
    }

    pub fn get(&self, key: KeyRef<'_>) -> Option<&Value<'a>> {
        match key {
            // The hot paths: a name that cannot be an array index (every `t`, `tp`, `v`, every
            // function name) is compared as it is, and an integral number is an index.
            KeyRef::Str(name) if !name.as_bytes().first().is_some_and(u8::is_ascii_digit) => self.get_name(name),
            KeyRef::Num(n) if n.fract() == 0.0 && (0.0..=MAX_ARRAY_INDEX).contains(&n) => self.get_index(n as u32),
            _ => match key.js_key() {
                JsKey::Index(i) => self.get_index(i),
                JsKey::Name(name) => self.get_name(&name),
            },
        }
    }

    fn get_index(&self, i: u32) -> Option<&Value<'a>> {
        let indexed = &self.entries[..self.index_count];
        indexed
            .binary_search_by_key(&i, |(key, _)| match key {
                JsKey::Index(k) => *k,
                JsKey::Name(_) => unreachable!("index keys come first"),
            })
            .ok()
            .map(|at| &indexed[at].1)
    }

    fn get_name(&self, name: &str) -> Option<&Value<'a>> {
        self.entries[self.index_count..]
            .iter()
            .find(|(key, _)| matches!(key, JsKey::Name(n) if n.as_ref() == name))
            .map(|(_, v)| v)
    }

    /// Builds an object from entries in insertion order (unique keys).
    fn from_insertion_order(mut entries: Vec<(JsKey<'a>, Value<'a>)>) -> Self {
        // Most objects (every stream entry {t, tp, v}) have no index key: nothing to reorder.
        if entries.iter().all(|(key, _)| matches!(key, JsKey::Name(_))) {
            return Object { entries, index_count: 0 };
        }
        // Stable: names keep their insertion order behind the sorted indices.
        entries.sort_by(|(a, _), (b, _)| match (a, b) {
            (JsKey::Index(a), JsKey::Index(b)) => a.cmp(b),
            (JsKey::Index(_), JsKey::Name(_)) => std::cmp::Ordering::Less,
            (JsKey::Name(_), JsKey::Index(_)) => std::cmp::Ordering::Greater,
            (JsKey::Name(_), JsKey::Name(_)) => std::cmp::Ordering::Equal,
        });
        let index_count = entries.iter().take_while(|(key, _)| matches!(key, JsKey::Index(_))).count();
        Object { entries, index_count }
    }
}

/// A validated, undecoded value.
pub struct Lazy<'a> {
    raw: &'a [u8],
    depth: u32,
    /// How to decode it when read: in full, or as a function stream with lazy entries.
    decode_as: Context,
    decoded: OnceCell<Box<Value<'a>>>,
}

impl<'a> Lazy<'a> {
    fn get(&self) -> &Value<'a> {
        self.decoded.get_or_init(|| {
            let mut decoder = Decoder {
                cursor: Cursor::new(self.raw),
                last_table_integer_keyed: false,
            };
            let value = decoder
                .value(self.depth, self.decode_as)
                .expect("a lazy value was validated when it was skipped");
            Box::new(value.expect("a nil value is never kept lazy"))
        })
    }
}

/// Where a value sits in a trace file, deciding which values stay lazy.
#[derive(Clone, Copy, PartialEq)]
enum Context {
    Root,
    Sessions,
    Session,
    Functions,
    Events,
    Event,
    /// Decoded in full.
    Plain,
    /// Validated now, decoded in full on first read.
    Lazy,
    /// A function stream: validated now, decoded as [`Context::Stream`] on first read.
    LazyStream,
    /// Inside a function stream: tables are decoded, but array items (the stream entries
    /// `{t, tp, v}`) stay lazy, so a binary search over a stream decodes only the entries it
    /// probes.
    Stream,
}

impl Context {
    fn child(self, key: &JsKey) -> Context {
        let name = |expected: &str| matches!(key, JsKey::Name(n) if n == expected);
        match self {
            Context::Root if name("sessions") => Context::Sessions,
            Context::Sessions => Context::Session,
            Context::Session if name("functions") => Context::Functions,
            Context::Session if name("functionsDelta") => Context::Lazy,
            Context::Session if name("events") => Context::Events,
            Context::Functions => Context::LazyStream,
            Context::Stream => Context::Stream,
            Context::Events => Context::Event,
            Context::Event if name("a") => Context::Lazy,
            _ => Context::Plain,
        }
    }
}

/// Decodes a trace file payload: exactly one CBOR item, nothing after it. None for nil.
pub fn decode_trace(bytes: &[u8]) -> Result<Option<Value<'_>>> {
    let mut decoder = Decoder {
        cursor: Cursor::new(bytes),
        last_table_integer_keyed: false,
    };
    let value = decoder.value(1, Context::Root)?;
    if !decoder.cursor.is_done() {
        return Err(CborError("trailing data after CBOR item".into()));
    }
    Ok(value)
}

/// Decodes one complete CBOR item in full (no lazy values). None for nil.
pub fn decode_value(bytes: &[u8]) -> Result<Option<Value<'_>>> {
    let mut decoder = Decoder {
        cursor: Cursor::new(bytes),
        last_table_integer_keyed: false,
    };
    let value = decoder.value(1, Context::Plain)?;
    if !decoder.cursor.is_done() {
        return Err(CborError("trailing data after CBOR item".into()));
    }
    Ok(value)
}

struct Decoder<'a> {
    cursor: Cursor<'a>,
    /// Whether the table decoded last had only positive integer keys (and at least one).
    last_table_integer_keyed: bool,
}

fn depth_error() -> CborError {
    CborError(format!("CBOR deserialization exceeded maximum table depth of {MAX_NESTING_DEPTH}"))
}

fn text(bytes: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(bytes)
}

impl<'a> Decoder<'a> {
    /// cbor.ts checks a table's depth before reading its length, so a table too deep reports
    /// that, even when its length is invalid too.
    fn check_container_depth(&self, depth: u32) -> Result<()> {
        match self.cursor.peek_major() {
            Some(4 | 5) if depth > MAX_NESTING_DEPTH => Err(depth_error()),
            _ => Ok(()),
        }
    }

    /// One value; None stands for nil.
    fn value(&mut self, depth: u32, context: Context) -> Result<Option<Value<'a>>> {
        match context {
            Context::Lazy => return self.lazy(depth, Context::Plain),
            Context::LazyStream => return self.lazy(depth, Context::Stream),
            _ => {}
        }
        self.check_container_depth(depth)?;
        Ok(match self.cursor.item()? {
            Item::Num(n) => Some(Value::Num(n)),
            Item::Str(bytes) => Some(Value::Str(text(bytes))),
            Item::Bool(b) => Some(Value::Bool(b)),
            Item::Nil => None,
            Item::Array(length) => Some(self.array(length, depth, context)?),
            Item::Map(length) => Some(self.map(length, depth, context)?),
        })
    }

    fn array(&mut self, length: f64, depth: u32, context: Context) -> Result<Value<'a>> {
        if depth > MAX_NESTING_DEPTH {
            return Err(depth_error());
        }
        let mut items = Vec::with_capacity(self.cursor.capacity_for(length));
        let mut first_integer_keyed = false;
        // Set once an item is nil: the table has a hole and is no plain array any more.
        let mut raw: Option<Vec<(JsKey<'a>, Value<'a>)>> = None;
        let mut index = 1.0;
        while index <= length {
            let key = JsKey::Index(index as u32);
            // Stream entries stay lazy; other arrays follow their context.
            let item_context = if context == Context::Stream { Context::Lazy } else { context.child(&key) };
            let item = self.value(depth + 1, item_context)?;
            if index == 1.0 {
                first_integer_keyed = item.as_ref().is_some_and(Value::is_table_shape) && self.last_table_integer_keyed;
            }
            match (item, &mut raw) {
                (None, None) => {
                    raw = Some(items.drain(..).enumerate().map(|(i, v)| (JsKey::Index(i as u32 + 1), v)).collect());
                }
                (None, Some(_)) => {}
                (Some(item), Some(raw)) => raw.push((key, item)),
                (Some(item), None) => items.push(item),
            }
            index += 1.0;
        }
        if let Some(raw) = raw {
            return Ok(self.finish(raw, first_integer_keyed));
        }
        self.last_table_integer_keyed = !items.is_empty();
        if first_integer_keyed {
            let entries = items.into_iter().enumerate().map(|(i, v)| (JsKey::Index(i as u32 + 1), v)).collect();
            return Ok(Value::Object(Object::from_insertion_order(entries)));
        }
        Ok(Value::Array(items))
    }

    fn map(&mut self, length: f64, depth: u32, context: Context) -> Result<Value<'a>> {
        if depth > MAX_NESTING_DEPTH {
            return Err(depth_error());
        }
        let mut table = TableBuilder::with_capacity(self.cursor.capacity_for(length));
        let mut first_integer_keyed = false;
        let mut has_other_key = false;
        let mut has_deleted = false;
        let mut i = 0.0;
        while i < length {
            i += 1.0;
            let key = self.value(depth + 1, Context::Plain)?;
            let Some(key) = key else {
                return Err(CborError("CBOR map keys that decode to nil are not supported".into()));
            };
            let key = match key {
                Value::Num(n) => Some(JsKey::from_num(n).with_nan_check(n)),
                Value::Str(s) => Some(Ok(JsKey::from_str(s))),
                _ => None,
            };
            let item_context = match &key {
                Some(Ok(key)) => context.child(key),
                _ => Context::Plain,
            };
            let item = self.value(depth + 1, item_context)?;
            let item_integer_keyed = item.as_ref().is_some_and(Value::is_table_shape) && self.last_table_integer_keyed;
            // Only number and string keys are kept; boolean and table keys are skipped.
            let Some(key) = key else { continue };
            let key = key?;
            let is_one = key == JsKey::Index(1);
            match item {
                None => {
                    table.remove(&key);
                    has_deleted = true;
                }
                Some(item) => {
                    if !has_other_key && !key.is_positive_integer() {
                        has_other_key = true;
                    }
                    table.set(key, item);
                }
            }
            if is_one {
                first_integer_keyed = item_integer_keyed;
            }
        }
        // Fast path for the common case: a key that is no positive integer makes it an object.
        if has_other_key && !has_deleted {
            self.last_table_integer_keyed = false;
            return Ok(Value::Object(Object::from_insertion_order(table.into_entries())));
        }
        Ok(self.finish(table.into_entries(), first_integer_keyed))
    }

    /// cbor.ts `finish`: a table with string keys becomes an array or an object.
    fn finish(&mut self, raw: Vec<(JsKey<'a>, Value<'a>)>, first_integer_keyed: bool) -> Value<'a> {
        let integer_keyed = raw.iter().all(|(key, _)| key.is_positive_integer());
        self.last_table_integer_keyed = integer_keyed && !raw.is_empty();
        if raw.is_empty() {
            return Value::Array(Vec::new());
        }
        let object = Object::from_insertion_order(raw);
        if !integer_keyed || first_integer_keyed {
            return Value::Object(object);
        }
        let sequential =
            object.index_count == object.entries.len() && object.entries.iter().enumerate().all(|(i, (key, _))| *key == JsKey::Index(i as u32 + 1));
        if !sequential {
            return Value::Object(object);
        }
        Value::Array(object.entries.into_iter().map(|(_, v)| v).collect())
    }

    /// Validates one value without decoding it; records its shape for the parent.
    fn lazy(&mut self, depth: u32, decode_as: Context) -> Result<Option<Value<'a>>> {
        let start = self.cursor.pos();
        let shape = self.skip(depth, true)?;
        let raw = self.cursor.slice_from(start);
        Ok(match shape {
            Shape::Nil => None,
            Shape::Scalar => Some(Value::Lazy(Lazy {
                raw,
                depth,
                decode_as,
                decoded: OnceCell::new(),
            })),
            Shape::Table { integer_keyed } => {
                self.last_table_integer_keyed = integer_keyed;
                Some(Value::Lazy(Lazy {
                    raw,
                    depth,
                    decode_as,
                    decoded: OnceCell::new(),
                }))
            }
        })
    }

    /// Walks one value, reporting every error `value` would. With `want_shape`, also works out
    /// whether a table would have only positive integer keys; nested values only validate.
    fn skip(&mut self, depth: u32, want_shape: bool) -> Result<Shape> {
        self.check_container_depth(depth)?;
        let header = self.cursor.item()?;
        match header {
            Item::Num(_) | Item::Str(_) | Item::Bool(_) => return Ok(Shape::Scalar),
            Item::Nil => return Ok(Shape::Nil),
            Item::Array(_) | Item::Map(_) if !want_shape => {
                self.skip_rest(header, depth)?;
                // Nobody reads the shape of a nested table.
                return Ok(Shape::Table { integer_keyed: false });
            }
            _ => {}
        }
        if depth > MAX_NESTING_DEPTH {
            return Err(depth_error());
        }
        match header {
            Item::Array(length) => {
                let mut present = 0usize;
                let mut index = 1.0;
                while index <= length {
                    if self.skip(depth + 1, false)? != Shape::Nil {
                        present += 1;
                    }
                    index += 1.0;
                }
                // With or without holes, the remaining keys are all integers.
                Ok(Shape::Table { integer_keyed: present > 0 })
            }
            Item::Map(length) => {
                // Without deleted keys the shape follows from flags alone, so no key list is
                // built; a nil value (rare) rewinds and replays the map with set semantics.
                let first_key = self.cursor.pos();
                let mut has_other_key = false;
                let mut has_kept_key = false;
                let mut i = 0.0;
                while i < length {
                    i += 1.0;
                    let key = self.skip_key(depth + 1)?;
                    let item = self.skip(depth + 1, false)?;
                    let key = match key {
                        None => continue,
                        Some(SkippedKey::Num(n)) if n.is_nan() => return Err(CborError("table index is NaN".into())),
                        Some(key) => key,
                    };
                    if item == Shape::Nil {
                        self.cursor.set_pos(first_key);
                        return self.map_shape_with_deletes(length, depth);
                    }
                    has_kept_key = true;
                    if !has_other_key && !key.is_positive_integer() {
                        has_other_key = true;
                    }
                }
                Ok(Shape::Table {
                    integer_keyed: has_kept_key && !has_other_key,
                })
            }
            _ => unreachable!("scalars returned above"),
        }
    }

    /// The shape of a map that deletes keys: replays it from its first key with JS set/delete
    /// semantics over the kept keys, validating the items the first pass had not reached.
    fn map_shape_with_deletes(&mut self, length: f64, depth: u32) -> Result<Shape> {
        // Only which keys remain matters for the shape, not their order.
        let mut keys: std::collections::HashSet<JsKey<'a>> = std::collections::HashSet::new();
        let mut i = 0.0;
        while i < length {
            i += 1.0;
            let key = self.skip_key(depth + 1)?;
            let item = self.skip(depth + 1, false)?;
            let key = match key {
                None => continue,
                Some(SkippedKey::Num(n)) if n.is_nan() => return Err(CborError("table index is NaN".into())),
                Some(SkippedKey::Num(n)) => JsKey::from_num(n),
                Some(SkippedKey::Str(bytes)) => JsKey::from_str(text(bytes)),
            };
            if item == Shape::Nil {
                keys.remove(&key);
            } else {
                keys.insert(key);
            }
        }
        // A deleted key always takes the full rule (cbor.ts skips its fast path then).
        Ok(Shape::Table {
            integer_keyed: !keys.is_empty() && keys.iter().all(JsKey::is_positive_integer),
        })
    }

    /// Validates the items of a container whose header was read.
    fn skip_rest(&mut self, header: Item<'a>, depth: u32) -> Result<()> {
        if depth > MAX_NESTING_DEPTH {
            return Err(depth_error());
        }
        let mut i = 0.0;
        match header {
            Item::Array(length) => {
                while i < length {
                    i += 1.0;
                    self.skip(depth + 1, false)?;
                }
            }
            Item::Map(length) => {
                while i < length {
                    i += 1.0;
                    let key = self.skip_key(depth + 1)?;
                    self.skip(depth + 1, false)?;
                    if matches!(key, Some(SkippedKey::Num(n)) if n.is_nan()) {
                        return Err(CborError("table index is NaN".into()));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Reads a map key; None for keys the decoder drops (booleans and tables).
    fn skip_key(&mut self, depth: u32) -> Result<Option<SkippedKey<'a>>> {
        self.check_container_depth(depth)?;
        match self.cursor.item()? {
            Item::Nil => Err(CborError("CBOR map keys that decode to nil are not supported".into())),
            Item::Num(n) => Ok(Some(SkippedKey::Num(n))),
            Item::Str(bytes) => Ok(Some(SkippedKey::Str(bytes))),
            Item::Bool(_) => Ok(None),
            container @ (Item::Array(_) | Item::Map(_)) => {
                self.skip_rest(container, depth)?;
                Ok(None)
            }
        }
    }
}

enum SkippedKey<'a> {
    Num(f64),
    Str(&'a [u8]),
}

impl SkippedKey<'_> {
    /// cbor.ts `isIntegerKey` without building the key: names starting with a letter or an
    /// underscore never read back as numbers.
    fn is_positive_integer(&self) -> bool {
        match self {
            SkippedKey::Num(n) => n.is_finite() && n.fract() == 0.0 && *n >= 1.0,
            SkippedKey::Str(bytes) => match bytes.first() {
                Some(b) if b.is_ascii_alphabetic() || *b == b'_' => false,
                _ => JsKey::from_str(text(bytes)).is_positive_integer(),
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Shape {
    Nil,
    Scalar,
    Table { integer_keyed: bool },
}

impl JsKey<'_> {
    fn with_nan_check(self, n: f64) -> Result<Self> {
        if n.is_nan() {
            Err(CborError("table index is NaN".into()))
        } else {
            Ok(self)
        }
    }
}

impl Value<'_> {
    /// Whether this is a table, without decoding a lazy value (its shape was recorded).
    fn is_table_shape(&self) -> bool {
        match self {
            Value::Array(_) | Value::Object(_) => true,
            Value::Lazy(lazy) => !matches!(lazy.raw.first().map(|b| b >> 5), Some(0..=3) | Some(7)),
            _ => false,
        }
    }
}

/// Tables up to this size find keys by scanning; larger ones keep a hash index.
const SCAN_LIMIT: usize = 16;

/// An insertion-ordered table with JS set/delete semantics: setting an existing key keeps its
/// position, deleting removes it (a key set again afterwards goes last). Lua keys are unique;
/// they only collide when a number and a string name the same JS property, or a nil value
/// deletes a key. Deleting leaves a tombstone, so any mix of sets and deletes stays linear.
#[derive(Default)]
struct TableBuilder<'a> {
    /// Entries in insertion order; None where an entry was deleted.
    slots: Vec<Option<(JsKey<'a>, Value<'a>)>>,
    /// Live key -> slot, once the table has more than SCAN_LIMIT slots.
    index: Option<HashMap<JsKey<'a>, usize>>,
}

impl<'a> TableBuilder<'a> {
    fn with_capacity(capacity: usize) -> Self {
        TableBuilder {
            slots: Vec::with_capacity(capacity),
            index: None,
        }
    }

    fn position(&self, key: &JsKey<'a>) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(key).copied(),
            None => self.slots.iter().position(|slot| slot.as_ref().is_some_and(|(k, _)| k == key)),
        }
    }

    fn set(&mut self, key: JsKey<'a>, value: Value<'a>) {
        if let Some(at) = self.position(&key) {
            if let Some(slot) = &mut self.slots[at] {
                slot.1 = value;
            }
            return;
        }
        if let Some(index) = &mut self.index {
            index.insert(key.clone(), self.slots.len());
        } else if self.slots.len() == SCAN_LIMIT {
            let mut index: HashMap<JsKey<'a>, usize> = self
                .slots
                .iter()
                .enumerate()
                .filter_map(|(i, slot)| slot.as_ref().map(|(k, _)| (k.clone(), i)))
                .collect();
            index.insert(key.clone(), self.slots.len());
            self.index = Some(index);
        }
        self.slots.push(Some((key, value)));
    }

    fn remove(&mut self, key: &JsKey<'a>) {
        if let Some(at) = self.position(key) {
            self.slots[at] = None;
            if let Some(index) = &mut self.index {
                index.remove(key);
            }
        }
    }

    /// The live entries, in insertion order.
    fn into_entries(self) -> Vec<(JsKey<'a>, Value<'a>)> {
        self.slots.into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    //! The cases of the TS decoder's tests (core/trace-data/cbor.test.ts), each decoded three
    //! ways: in full, as a lazily decoded function stream and as lazily decoded event
    //! arguments. All three must give the TS result; errors must surface while decoding, never
    //! later from a lazy value.

    use super::*;

    fn text(s: &str) -> Vec<u8> {
        let mut out = vec![0x60 | s.len() as u8];
        out.extend(s.as_bytes());
        out
    }

    /// `{"sessions": [{"events": [{"e": "E", "a": item}], "functions": {"F": item}, "functionsDelta": item}]}`
    fn in_a_session(item: &[u8]) -> Vec<u8> {
        let mut out = vec![0xa1];
        out.extend(text("sessions"));
        out.extend([0x81, 0xa3]);
        out.extend(text("events"));
        out.extend([0x81, 0xa2]);
        out.extend(text("e"));
        out.extend(text("E"));
        out.extend(text("a"));
        out.extend(item);
        out.extend(text("functions"));
        out.push(0xa1);
        out.extend(text("F"));
        out.extend(item);
        out.extend(text("functionsDelta"));
        out.extend(item);
        out
    }

    /// The item's JSON as decoded in full, as a function stream and as event arguments.
    fn decoded_three_ways(item: &[u8]) -> [String; 3] {
        let full = to_json(&decode_value(item).expect("decodes").expect("not nil"));
        let bytes = in_a_session(item);
        let root = decode_trace(&bytes).expect("decodes").expect("not nil");
        let session = &root.get("sessions").and_then(Value::as_array).expect("sessions")[0];
        let stream = to_json(session.get("functions").and_then(|f| f.get("F")).expect("F"));
        let args = to_json(session.get("events").and_then(|e| e.get(0)).and_then(|e| e.get("a")).expect("a"));
        [full, stream, args]
    }

    #[track_caller]
    fn decodes_to(item: &[u8], json: &str) {
        let [full, stream, args] = decoded_three_ways(item);
        assert_eq!(full, json, "decoded in full");
        assert_eq!(stream, json, "decoded lazily as a function stream");
        assert_eq!(args, json, "decoded lazily as event arguments");
    }

    #[track_caller]
    fn fails_with(item: &[u8], message: &str) {
        let full = decode_value(item).map(|_| ()).expect_err("the full decode fails").0;
        assert!(full.contains(message), "{full}");
        let lazy = decode_trace(&in_a_session(item))
            .map(|_| ())
            .expect_err("validation fails while decoding, not later")
            .0;
        assert_eq!(lazy, full);
    }

    fn ascii(s: &str) -> impl Iterator<Item = u8> + '_ {
        s.bytes()
    }

    #[test]
    fn integers_are_doubles_like_the_lua_reference() {
        assert_eq!(decode_value(&[0x38, 0x63]).unwrap().unwrap().as_num(), Some(-100.0));
        // 2^53 + 1 cannot be represented; Lua and the TS decoder both round to 2^53.
        decodes_to(&[0x81, 0x1b, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01], "[9007199254740992]");
    }

    #[test]
    fn half_single_and_double_floats() {
        let num = |bytes: &[u8]| decode_value(bytes).unwrap().unwrap().as_num().unwrap();
        assert_eq!(num(&[0xf9, 0xc4, 0x00]), -4.0);
        assert_eq!(num(&[0xf9, 0x00, 0x01]), 2f64.powi(-24));
        assert_eq!(num(&[0xf9, 0x7c, 0x00]), f64::INFINITY);
        assert_eq!(num(&[0xfa, 0x47, 0xc3, 0x50, 0x00]), 100000.0);
        assert_eq!(num(&[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a]), 1.1);
    }

    #[test]
    fn arrays_stay_arrays_and_arrays_with_holes_become_objects_keyed_from_1() {
        decodes_to(&[0x82, 0x01, 0x02], "[1,2]");
        decodes_to(&[0x83, 0x01, 0xf6, 0x03], r#"{"1":1,"3":3}"#);
        decodes_to(&[0x83, 0x01, 0x02, 0xf6], "[1,2]");
        decodes_to(&[0x80], "[]");
        decodes_to(&[0xa0], "[]");
    }

    #[test]
    fn maps_keyed_1_to_n_become_arrays() {
        decodes_to(&[0xa2, 0x02, 0x61, b'b', 0x01, 0x61, b'a'], r#"["a","b"]"#);
        decodes_to(&[0xa1, 0x02, 0x61, b'b'], r#"{"2":"b"}"#);
        decodes_to(&[0xa2, 0x61, b'1', 0x0a, 0x61, b'2', 0x14], "[10,20]");
        decodes_to(&[0xa3, 0x61, b'a', 0x01, 0x61, b'a', 0xf6, 0x01, 0x05], "[5]");
    }

    #[test]
    fn packed_args_and_tables_with_string_keys_stay_objects() {
        decodes_to(&[0xa2, 0x01, 0x07, 0x61, b'n', 0x01], r#"{"1":7,"n":1}"#);
        decodes_to(&[0xa1, 0x61, b'x', 0x82, 0x01, 0x02], r#"{"x":[1,2]}"#);
    }

    #[test]
    fn a_table_whose_first_value_is_integer_keyed_stays_an_object_like_index_keyed_streams() {
        decodes_to(&[0x82, 0x81, 0x01, 0x81, 0x02], r#"{"1":[1],"2":[2]}"#);
        decodes_to(&[0x82, 0xa1, 0x02, 0x01, 0x81, 0x02], r#"{"1":{"2":1},"2":[2]}"#);
        decodes_to(&[0x82, 0x80, 0x81, 0x02], "[[],[2]]");
        decodes_to(&[0x82, 0xa1, 0x61, b'x', 0x01, 0x81, 0x02], r#"[{"x":1},[2]]"#);
    }

    #[test]
    fn only_number_and_string_keys_are_kept_and_a_nil_value_removes_a_key() {
        decodes_to(&[0xa1, 0x01, 0x61, b'x'], r#"["x"]"#);
        decodes_to(&[0xa2, 0x61, b'a', 0x01, 0x61, b'a', 0xf6], "[]");
        decodes_to(&[0xa2, 0xf5, 0x01, 0x80, 0x02], "[]");
    }

    #[test]
    fn non_canonical_numeric_string_keys_stay_object_keys() {
        decodes_to(&[0xa1, 0x62, b'0', b'1', 0x05], r#"{"01":5}"#);
        decodes_to(&[0xa1, 0x63, b'1', b'.', b'0', 0x05], r#"{"1.0":5}"#);
    }

    #[test]
    fn a_proto_key_is_an_ordinary_entry() {
        let item: Vec<u8> = [0xa1, 0x69].into_iter().chain(ascii("__proto__")).chain([0x01]).collect();
        decodes_to(&item, r#"{"__proto__":1}"#);
    }

    #[test]
    fn numbers_and_strings_find_the_same_keys() {
        // {"b": 1, 10: 2, "007": 3, -1: 4}
        let item = [0xa4, 0x61, b'b', 0x01, 0x0a, 0x02, 0x63, b'0', b'0', b'7', 0x03, 0x20, 0x04];
        let value = decode_value(&item).unwrap().unwrap();
        let num = |key: KeyRef| value.get(key).and_then(Value::as_num);
        assert_eq!((num(KeyRef::Num(10.0)), num(KeyRef::Str("10"))), (Some(2.0), Some(2.0)));
        assert_eq!(
            (num(KeyRef::Str("b")), num(KeyRef::Str("007")), num(KeyRef::Num(7.0))),
            (Some(1.0), Some(3.0), None)
        );
        assert_eq!((num(KeyRef::Num(-1.0)), num(KeyRef::Str("-1"))), (Some(4.0), Some(4.0)));
        assert_eq!((num(KeyRef::Num(-0.0)), num(KeyRef::Str("")), num(KeyRef::Num(10.5))), (None, None, None));
    }

    #[test]
    fn index_keys_iterate_ascending_before_names_in_insertion_order() {
        // {"b": 1, 10: 2, "a": 3, 2: 4}: JS iterates 2, 10, b, a.
        decodes_to(
            &[0xa4, 0x61, b'b', 0x01, 0x0a, 0x02, 0x61, b'a', 0x03, 0x02, 0x04],
            r#"{"2":4,"10":2,"b":1,"a":3}"#,
        );
    }

    #[test]
    fn byte_and_text_strings_read_as_utf8() {
        decodes_to(&[0x81, 0x42, 0xc3, 0xa9], r#"["é"]"#);
        decodes_to(&[0x81, 0x62, 0xc3, 0xa9], r#"["é"]"#);
    }

    #[test]
    fn rejects_what_blizzard_cbor_rejects_while_decoding_lazy_values_too() {
        fails_with(&[0x81, 0xc0, 0x00], "tags");
        fails_with(&[0x81, 0x9f, 0xff], "indefinite");
        fails_with(&[0x81, 0x19, 0x01], "truncated");
        fails_with(&[0xa1, 0xf6, 0x01], "nil");
        fails_with(&[0xa1, 0xfb, 0x7f, 0xf8, 0, 0, 0, 0, 0, 0, 0x01], "NaN");
        assert!(decode_value(&[0x01, 0x02]).unwrap_err().0.contains("trailing"));
    }

    #[test]
    fn a_nan_key_after_a_deleted_key_is_still_an_error() {
        // {"a": nil, NaN: 1}: the deleted key replays the map, which must still check the rest.
        fails_with(&[0xa2, 0x61, b'a', 0xf6, 0xfb, 0x7f, 0xf8, 0, 0, 0, 0, 0, 0, 0x01], "NaN");
    }

    #[test]
    fn a_table_too_deep_reports_its_depth_even_when_its_length_is_invalid() {
        // 100 arrays, then an indefinite-length array at depth 101: TS checks depth first.
        let item: Vec<u8> = std::iter::repeat_n(0x81, 100).chain([0x9f, 0xff]).collect();
        assert!(decode_value(&item).unwrap_err().0.contains("depth"));
    }

    #[test]
    fn allows_100_nested_tables_and_rejects_101() {
        let nested = |depth: usize| -> Vec<u8> { std::iter::repeat_n(0x81, depth).chain([0x80]).collect() };
        assert!(decode_value(&nested(99)).is_ok());
        assert!(decode_value(&nested(100)).unwrap_err().0.contains("depth"));
    }

    #[test]
    fn many_deleted_keys_decode_in_linear_time() {
        // {k0: 1, ..., kN: 1, k0: nil, ..., kN: nil}: quadratic before deletes left tombstones.
        let keys = 40_000u32;
        let key = |i: u32| -> Vec<u8> { std::iter::once(0x78).chain(std::iter::once(8)).chain(format!("k{i:07}").into_bytes()).collect() };
        let mut item = vec![0xba];
        item.extend((keys * 2).to_be_bytes());
        for i in 0..keys {
            item.extend(key(i));
            item.push(0x01);
        }
        for i in 0..keys {
            item.extend(key(i));
            item.push(0xf6);
        }
        let started = std::time::Instant::now();
        decodes_to(&item, "[]");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
    }
}
