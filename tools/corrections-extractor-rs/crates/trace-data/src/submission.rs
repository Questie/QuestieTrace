//! One trace-data submission file: `submissions/<yyyy-mm>/<stamp>_<id>.json`, holding one or
//! more addon exports in `export_string`.
//!
//! Only the submission id and the export are read. Every other field (the pseudonymous
//! contributor id, free text such as `nickname`) is never deserialized.
//!
//! Reading happens in two steps, so decoded values can borrow the inflated bytes:
//! [`Submission::read`] inflates every export, [`Submission::decode`] decodes them.

use crate::export::{decode_for_print, find_exports, is_js_whitespace, Export, Inflated, Inflater};
use crate::session::Session;
use crate::value::{decode_trace, Value};
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

/// A submission with its exports inflated.
pub struct Submission {
    /// The file's `id`, else the id in its file name.
    pub submission_id: String,
    /// Why the file could not be read at all.
    error: Option<String>,
    /// Per export: its CBOR bytes, or why it was dropped.
    exports: Vec<Result<Inflated, String>>,
}

/// A decoded submission: one trace per export that decoded.
pub struct Decoded<'a> {
    pub traces: Vec<Trace<'a>>,
    /// Why each export that could not be decoded was dropped, as decode.ts words it.
    pub dropped_exports: Vec<String>,
    /// Why the submission could not be read; `traces` is then empty.
    pub error: Option<String>,
}

/// One decoded export.
pub struct Trace<'a> {
    root: Value<'a>,
}

impl<'a> Trace<'a> {
    /// The decoded export, as the TypeScript decoder returns it.
    pub fn root(&self) -> &Value<'a> {
        &self.root
    }

    /// The export's valid sessions, in order: `sessions` must be an array, and each session an
    /// object with an `events` array and a `functions` object (as submissions.ts filters them).
    pub fn sessions(&self) -> Vec<Session<'_, 'a>> {
        match self.root.get("sessions").and_then(Value::as_array) {
            Some(sessions) => sessions.iter().filter_map(Session::new).collect(),
            None => Vec::new(),
        }
    }
}

/// The top-level fields of a submission file, as raw JSON: only `id` and `export_string` are
/// ever parsed. A key that appears twice keeps its last value, as with `JSON.parse`.
type Envelope<'a> = HashMap<String, &'a RawValue>;

/// A JSON string field's value. A lone UTF-16 surrogate (which JS strings allow, Rust strings
/// do not) makes it unreadable: "no export_string" where the TS tool would decode on.
/// Borrowed when the string has no escapes, as an export string never has: the raw JSON was
/// validated, so the text between the quotes is the value.
fn json_string(raw: Option<&RawValue>) -> Option<Cow<'_, str>> {
    let json = raw?.get();
    let inner = json.strip_prefix('"').and_then(|rest| rest.strip_suffix('"'));
    match inner {
        Some(inner) if !inner.contains('\\') => Some(Cow::Borrowed(inner)),
        _ => serde_json::from_str::<String>(json).ok().map(Cow::Owned),
    }
}

/// trace-data names files `<received stamp>_<submission id>.json`.
fn submission_id_from_path(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let name = name.strip_suffix(".json").unwrap_or(name);
    name.rsplit_once('_').map_or(name, |(_, id)| id).to_owned()
}

impl Submission {
    /// Reads and inflates the file at `path`; never fails, errors are kept for [`Self::decode`].
    pub fn read(path: &Path, inflater: &mut Inflater) -> Submission {
        match std::fs::read(path) {
            Ok(bytes) => Self::from_json(&bytes, submission_id_from_path(path), inflater),
            Err(_) => Submission {
                submission_id: submission_id_from_path(path),
                error: Some("unreadable submission JSON".into()),
                exports: Vec::new(),
            },
        }
    }

    /// A submission file's content; `fallback_id` is used when the file has no `id`.
    pub fn from_json(bytes: &[u8], fallback_id: String, inflater: &mut Inflater) -> Submission {
        let mut submission = Submission {
            submission_id: fallback_id,
            error: None,
            exports: Vec::new(),
        };
        // Read as UTF-8 like Node does: invalid bytes become U+FFFD instead of failing the file.
        let text = String::from_utf8_lossy(bytes);
        let envelope = match serde_json::from_str::<Envelope>(&text) {
            Ok(envelope) => envelope,
            // An array is a JS object too: it parses, but has no export_string.
            Err(_) if serde_json::from_str::<Vec<&RawValue>>(&text).is_ok() => {
                submission.error = Some("no export_string".into());
                return submission;
            }
            // Never pass serde's message on: it can quote the input, which holds free text.
            Err(_) => {
                submission.error = Some("unreadable submission JSON".into());
                return submission;
            }
        };
        if let Some(id) = json_string(envelope.get("id").copied()).filter(|id| !id.is_empty()) {
            submission.submission_id = id.into_owned();
        }
        let Some(export_string) = json_string(envelope.get("export_string").copied()) else {
            submission.error = Some("no export_string".into());
            return submission;
        };
        match find_exports(&export_string) {
            Err(message) => submission.error = Some(message),
            Ok(exports) => {
                submission.exports = exports
                    .iter()
                    .enumerate()
                    .map(|(index, export)| inflate_export(index, export, inflater))
                    .collect();
            }
        }
        submission
    }

    /// Hands the inflated exports' buffers back to `inflater` for the next submission.
    pub fn recycle(self, inflater: &mut Inflater) {
        inflater.recycle(self.exports.into_iter().filter_map(Result::ok));
    }

    /// Decodes every inflated export. When none decodes, the submission fails with every
    /// export's reason, as decode.ts does.
    pub fn decode(&self) -> Decoded<'_> {
        if let Some(error) = &self.error {
            return Decoded {
                traces: Vec::new(),
                dropped_exports: Vec::new(),
                error: Some(error.clone()),
            };
        }
        let mut traces = Vec::new();
        let mut dropped_exports = Vec::new();
        for (index, export) in self.exports.iter().enumerate() {
            match export {
                Err(message) => dropped_exports.push(message.clone()),
                Ok(inflated) => match decode_root(inflated.bytes()) {
                    Ok(root) => traces.push(Trace { root }),
                    Err(message) => dropped_exports.push(format!("export {}: {message}", index + 1)),
                },
            }
        }
        if traces.is_empty() {
            return Decoded {
                traces,
                dropped_exports: Vec::new(),
                error: Some(dropped_exports.join("; ")),
            };
        }
        Decoded {
            traces,
            dropped_exports,
            error: None,
        }
    }
}

fn inflate_export(index: usize, export: &Export, inflater: &mut Inflater) -> Result<Inflated, String> {
    let number = index + 1;
    let payload = match export {
        Export::Unterminated => return Err(format!("export {number} has no matching end marker")),
        Export::Payload(payload) if payload.chars().all(is_js_whitespace) => {
            return Err(format!("export {number}: export markers found but payload is empty"));
        }
        Export::Payload(payload) => payload,
    };
    let compressed = decode_for_print(payload).map_err(|message| format!("export {number}: {message}"))?;
    inflater.inflate(&compressed).map_err(|message| format!("export {number}: {message}"))
}

/// decode.ts `decodePayload` after inflating: the payload must decode to a table.
fn decode_root(cbor: &[u8]) -> Result<Value<'_>, String> {
    match decode_trace(cbor).map_err(|e| e.0)? {
        Some(root) if root.is_table() => Ok(root),
        Some(Value::Num(_)) => Err("decoded payload is not a table (got number)".into()),
        Some(Value::Str(_)) => Err("decoded payload is not a table (got string)".into()),
        Some(Value::Bool(_)) => Err("decoded payload is not a table (got boolean)".into()),
        Some(_) => Err("decoded payload is not a table (got object)".into()),
        None => Err("decoded payload is not a table (got undefined)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::to_json;

    /// The TS tests' TINY_EXPORT (core/trace-data/fixtures.ts), built by the addon's own pipeline:
    /// one session with packed args, a nil hole, nil stream values, index-keyed streams, floats
    /// and UTF-8. Read from there so both tools test the same bytes.
    pub(crate) fn tiny_export() -> &'static str {
        let fixtures = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../corrections-extractor/src/core/trace-data/fixtures.ts"
        ));
        let declaration = fixtures.find("export const TINY_EXPORT =").expect("TINY_EXPORT is declared");
        let open = declaration + fixtures[declaration..].find('"').expect("a string literal") + 1;
        let close = open + fixtures[open..].find('"').expect("a closing quote");
        &fixtures[open..close]
    }

    fn submission(export_string: &str) -> Submission {
        let json = serde_json::json!({ "id": "test", "export_string": export_string, "nickname": "never read" });
        Submission::from_json(json.to_string().as_bytes(), "fallback".into(), &mut Inflater::new())
    }

    fn traces_json(decoded: &Decoded) -> Vec<String> {
        decoded.traces.iter().map(|trace| to_json(trace.root())).collect()
    }

    #[test]
    fn the_id_comes_from_the_file_name_after_the_last_underscore() {
        assert_eq!(submission_id_from_path(Path::new("2026-11/2026-11-05_204720Z_0b9d.json")), "0b9d");
        assert_eq!(submission_id_from_path(Path::new("plain.json")), "plain");
    }

    #[test]
    fn decodes_packed_args_holes_nil_values_and_index_keyed_streams_like_the_addon_recorded_them() {
        let submission = submission(tiny_export());
        let decoded = submission.decode();
        assert_eq!((decoded.error.as_deref(), decoded.dropped_exports.len(), decoded.traces.len()), (None, 0, 1));
        let sessions = decoded.traces[0].sessions();
        let session = &sessions[0];
        let args: Vec<String> = session.events().map(|event| to_json(event.args().expect("args"))).collect();
        // Packed args keep their `n` and stay objects; nil args are simply absent.
        assert_eq!(args, [r#"{"1":7,"2":92579,"n":2}"#, r#"{"n":1}"#]);
        let function = |name: &str| session.function(name).expect(name);
        // A nil stream value leaves no `v` key; a nil hole in an array leaves a sparse object.
        assert_eq!(to_json(function("UnitGUID").get("npc").and_then(|s| s.get(1)).unwrap()), r#"{"t":2,"tp":2}"#);
        assert_eq!(to_json(function("Sparse").get(0).and_then(|e| e.get("v")).unwrap()), r#"{"1":10,"3":30}"#);
        // Index-keyed streams stay maps so a stream can be looked up by index. Keys keep the
        // order the addon wrote them in (the TS test's toEqual ignores order; JSON does not).
        assert_eq!(
            to_json(function("GetAvailableTitle")),
            r#"{"1":[{"t":1,"v":"Ünïcode Title","tp":1.5}],"2":[{"t":1,"v":"Second","tp":1.5}]}"#
        );
        assert_eq!(
            to_json(function("Numbers").get(0).and_then(|e| e.get("v")).unwrap()),
            "[-1,-1.5,65504,1e+300,9007199254740992]"
        );
        assert_eq!(
            to_json(session.function_delta("C_QuestLog.GetAllCompletedQuestIDs").unwrap()),
            r#"{"t":0,"tp":0,"delta":[{"t":5,"tp":5.25,"add":[4]}],"initial":[1,2,3]}"#
        );
    }

    #[test]
    fn decodes_every_export_of_a_submission_even_when_line_wrapped() {
        let tiny = tiny_export();
        let wrapped: String = tiny
            .chars()
            .collect::<Vec<_>>()
            .chunks(76)
            .map(|line| line.iter().collect::<String>() + "\n")
            .collect();
        let submission = submission(&format!("{tiny}\n{wrapped}"));
        let decoded = submission.decode();
        let traces = traces_json(&decoded);
        assert_eq!(traces.len(), 2);
        assert_eq!(traces[1], traces[0]);
        assert!(decoded.dropped_exports.is_empty());
    }

    #[test]
    fn decodes_an_export_pasted_into_another_and_drops_the_one_it_cut_off() {
        let tiny = tiny_export();
        let (cut_off, tail) = (&tiny[..200], &tiny[tiny.len() - 60..]);
        let alone = submission(tiny);
        let pasted = submission(&format!("{cut_off}{tiny}{tail}"));
        let decoded = pasted.decode();
        assert_eq!(traces_json(&decoded), traces_json(&alone.decode()));
        assert_eq!(decoded.dropped_exports, ["export 1 has no matching end marker"]);
    }

    #[test]
    fn keeps_the_exports_that_decode_when_another_does_not() {
        let submission = submission(&format!("!QuestieTrace:1!abc$def!End:QuestieTrace:1!\n{}", tiny_export()));
        let decoded = submission.decode();
        assert_eq!(decoded.traces.len(), 1);
        assert_eq!(decoded.dropped_exports, ["export 1: payload is not valid print-encoded data (character \"$\")"]);
    }

    #[test]
    fn fails_when_no_export_decodes() {
        let error = |export_string: &str| submission(export_string).decode().error.expect("fails");
        assert!(error("not an export").contains("markers"));
        assert_eq!(
            error("!QuestieTrace:1!abc$def!End:QuestieTrace:1!"),
            "export 1: payload is not valid print-encoded data (character \"$\")"
        );
        assert_eq!(error(&tiny_export()[..200]), "export 1 has no matching end marker");
    }

    #[test]
    fn a_repeated_key_keeps_its_last_value_and_invalid_utf8_is_replaced_like_node_does() {
        let tiny = tiny_export();
        let repeated = format!(r#"{{"export_string": "not an export", "export_string": "{tiny}"}}"#);
        let read = |bytes: &[u8]| Submission::from_json(bytes, "fallback".into(), &mut Inflater::new());
        assert_eq!(read(repeated.as_bytes()).decode().traces.len(), 1);
        // An invalid byte in a free-text field must not make the file unreadable.
        let mut invalid = br#"{"nickname": "x"#.to_vec();
        invalid.push(0xff);
        invalid.extend(format!(r#"", "export_string": "{tiny}"}}"#).into_bytes());
        assert_eq!(read(&invalid).decode().traces.len(), 1);
    }

    #[test]
    fn json_strings_with_escapes_are_unescaped_and_others_borrowed() {
        let raw = |json: &str| serde_json::from_str::<&RawValue>(json).unwrap().get().to_owned();
        let plain = raw(r#""abc""#);
        let escaped = raw(r#""a\u0062\"c""#);
        assert!(matches!(json_string(Some(serde_json::from_str(&plain).unwrap())), Some(Cow::Borrowed("abc"))));
        assert_eq!(json_string(Some(serde_json::from_str(&escaped).unwrap())).as_deref(), Some("ab\"c"));
        assert_eq!(json_string(Some(serde_json::from_str("5").unwrap())), None);
    }

    #[test]
    fn a_file_that_is_no_json_object_names_why() {
        let read = |json: &str| Submission::from_json(json.as_bytes(), "fallback".into(), &mut Inflater::new()).decode().error;
        assert_eq!(read("not json").as_deref(), Some("unreadable submission JSON"));
        assert_eq!(read("[1, 2]").as_deref(), Some("no export_string"));
        assert_eq!(read(r#"{"export_string": 5}"#).as_deref(), Some("no export_string"));
    }
}
