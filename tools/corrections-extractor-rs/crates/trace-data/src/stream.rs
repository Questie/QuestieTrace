//! Function streams: what an API returned over a session, recorded as entries `{t, tp, v}`.
//! Ports `core/emulator.ts` and the time correlation of `extract/correlate.ts`.

use crate::session::Session;
use crate::value::{KeyRef, Value};

/// A leaf stream: entries in recording order.
pub type Stream<'v, 'a> = &'v [Value<'a>];

/// emulator.ts `getStream`: `session.functions[name][param1][param2]...` when that is a leaf
/// stream (an array of entries); None for a missing or non-leaf node.
pub fn get_stream<'v, 'a>(session: &Session<'v, 'a>, name: &str, params: &[KeyRef]) -> Option<Stream<'v, 'a>> {
    let mut node = session.function(name)?;
    for &param in params {
        if node.is_array() {
            return None;
        }
        node = node.get(param)?;
    }
    node.as_array()
}

/// emulator.ts `getParamKeys`: the parameter keys of a parameterized stream node, in JS
/// property order; none for a leaf stream.
pub fn param_keys(node: &Value) -> Vec<String> {
    if node.is_array() {
        return Vec::new();
    }
    node.keys()
}

/// An entry's time `t`; NaN when missing, so comparisons fail like JS `undefined` does.
pub fn entry_t(entry: &Value) -> f64 {
    entry.get("t").and_then(Value::as_num).unwrap_or(f64::NAN)
}

/// An entry's value `v`; None stands for nil.
pub fn entry_v<'v, 'a>(entry: &'v Value<'a>) -> Option<&'v Value<'a>> {
    entry.get("v")
}

/// emulator.ts `valueAt`: `v` of the latest entry with `t <= target`.
pub fn value_at<'v, 'a>(stream: Stream<'v, 'a>, target: f64) -> Option<&'v Value<'a>> {
    let (mut lo, mut hi) = (0isize, stream.len() as isize - 1);
    let mut result = None;
    while lo <= hi {
        let mid = ((lo + hi) as usize) >> 1;
        if entry_t(&stream[mid]) <= target {
            result = Some(mid);
            lo = mid as isize + 1;
        } else {
            hi = mid as isize - 1;
        }
    }
    entry_v(&stream[result?])
}

/// correlate.ts `DEFAULT_CORRELATION_WINDOW_SECONDS`.
pub const DEFAULT_CORRELATION_WINDOW_SECONDS: f64 = 5.0;

/// correlate.ts `nearestByTime`: `v` of the entry closest to `target`, before or after it,
/// within `window` seconds. The earlier entry wins a tie.
pub fn nearest_by_time<'v, 'a>(stream: Option<Stream<'v, 'a>>, target: f64, window: f64) -> Option<&'v Value<'a>> {
    let stream = stream.filter(|s| !s.is_empty())?;
    let (mut lo, mut hi) = (0usize, stream.len());
    while lo < hi {
        let mid = (lo + hi) >> 1;
        if entry_t(&stream[mid]) < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let mut best = None;
    let mut best_diff = f64::INFINITY;
    for index in [lo.checked_sub(1), Some(lo)].into_iter().flatten() {
        let Some(entry) = stream.get(index) else { continue };
        let diff = (entry_t(entry) - target).abs();
        if diff <= window && diff < best_diff {
            best = Some(entry);
            best_diff = diff;
        }
    }
    entry_v(best?)
}

/// emulator.ts `emulate` for packed args `{1: a, 2: b, n: 2}`: the values 1..n (None for nil).
/// Any other value is not packed: None.
pub fn unpack<'v, 'a>(value: &'v Value<'a>) -> Option<Vec<Option<&'v Value<'a>>>> {
    let object = value.as_object()?;
    let count = object.get(KeyRef::Str("n"))?.as_num()?;
    let mut values = Vec::new();
    let mut i = 1.0;
    while i <= count {
        values.push(object.get(KeyRef::Num(i)));
        i += 1.0;
    }
    Some(values)
}

/// emulator.ts `emulate` followed by `Array.isArray`: the values of packed args, or the items of
/// an array (decoded arrays hold no nil, so their items are all Some). None for any other value.
pub fn emulated_list<'v, 'a>(value: Option<&'v Value<'a>>) -> Option<Vec<Option<&'v Value<'a>>>> {
    let value = value?;
    unpack(value).or_else(|| value.as_array().map(|items| items.iter().map(Some).collect()))
}
