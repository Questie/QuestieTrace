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
//! always decodes later.

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
        match key.js_key() {
            JsKey::Index(i) => {
                let indexed = &self.entries[..self.index_count];
                indexed
                    .binary_search_by_key(&i, |(key, _)| match key {
                        JsKey::Index(k) => *k,
                        JsKey::Name(_) => unreachable!("index keys come first"),
                    })
                    .ok()
                    .map(|at| &indexed[at].1)
            }
            name @ JsKey::Name(_) => self.entries[self.index_count..].iter().find(|(key, _)| *key == name).map(|(_, v)| v),
        }
    }

    /// Builds an object from entries in insertion order (unique keys).
    fn from_insertion_order(mut entries: Vec<(JsKey<'a>, Value<'a>)>) -> Self {
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
                .value(self.depth, Context::Plain)
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
    /// Validated now, decoded on first read.
    Lazy,
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
            Context::Functions => Context::Lazy,
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
    /// One value; None stands for nil.
    fn value(&mut self, depth: u32, context: Context) -> Result<Option<Value<'a>>> {
        if context == Context::Lazy {
            return self.lazy(depth);
        }
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
        let mut items = Vec::new();
        let mut first_integer_keyed = false;
        // Set once an item is nil: the table has a hole and is no plain array any more.
        let mut raw: Option<Vec<(JsKey<'a>, Value<'a>)>> = None;
        let mut index = 1.0;
        while index <= length {
            let key = JsKey::Index(index as u32);
            let item = self.value(depth + 1, context.child(&key))?;
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
        let mut table = TableBuilder::default();
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
            return Ok(Value::Object(Object::from_insertion_order(table.entries)));
        }
        Ok(self.finish(table.entries, first_integer_keyed))
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
    fn lazy(&mut self, depth: u32) -> Result<Option<Value<'a>>> {
        let start = self.cursor.pos();
        let shape = self.skip(depth, true)?;
        let raw = self.cursor.slice_from(start);
        Ok(match shape {
            Shape::Nil => None,
            Shape::Scalar => Some(Value::Lazy(Lazy {
                raw,
                depth,
                decoded: OnceCell::new(),
            })),
            Shape::Table { integer_keyed } => {
                self.last_table_integer_keyed = integer_keyed;
                Some(Value::Lazy(Lazy {
                    raw,
                    depth,
                    decoded: OnceCell::new(),
                }))
            }
        })
    }

    /// Walks one value, reporting every error `value` would. With `want_shape`, also works out
    /// whether a table would have only positive integer keys; nested values only validate.
    fn skip(&mut self, depth: u32, want_shape: bool) -> Result<Shape> {
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
                let mut keys: Vec<JsKey<'a>> = Vec::new();
                let mut has_other_key = false;
                let mut has_deleted = false;
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
                        keys.retain(|k| *k != key);
                        has_deleted = true;
                    } else {
                        if !has_other_key && !key.is_positive_integer() {
                            has_other_key = true;
                        }
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                }
                let integer_keyed = if has_other_key && !has_deleted {
                    false
                } else {
                    !keys.is_empty() && keys.iter().all(JsKey::is_positive_integer)
                };
                Ok(Shape::Table { integer_keyed })
            }
            _ => unreachable!("scalars returned above"),
        }
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
/// position, deleting removes it. (Lua keys are unique; they only collide when a number and a
/// string name the same JS property, or a nil value deletes a key.)
#[derive(Default)]
struct TableBuilder<'a> {
    entries: Vec<(JsKey<'a>, Value<'a>)>,
    index: Option<HashMap<JsKey<'a>, usize>>,
}

impl<'a> TableBuilder<'a> {
    fn position(&self, key: &JsKey<'a>) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(key).copied(),
            None => self.entries.iter().position(|(k, _)| k == key),
        }
    }

    fn set(&mut self, key: JsKey<'a>, value: Value<'a>) {
        if let Some(at) = self.position(&key) {
            self.entries[at].1 = value;
            return;
        }
        if let Some(index) = &mut self.index {
            index.insert(key.clone(), self.entries.len());
        } else if self.entries.len() == SCAN_LIMIT {
            let mut index: HashMap<JsKey<'a>, usize> = self.entries.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect();
            index.insert(key.clone(), self.entries.len());
            self.index = Some(index);
        }
        self.entries.push((key, value));
    }

    fn remove(&mut self, key: &JsKey<'a>) {
        if let Some(at) = self.position(key) {
            self.entries.remove(at);
            if self.index.is_some() {
                self.index = Some(self.entries.iter().enumerate().map(|(i, (k, _))| (k.clone(), i)).collect());
            }
        }
    }
}
