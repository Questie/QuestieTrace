//! One trace-data submission file: `submissions/<yyyy-mm>/<stamp>_<id>.json`, holding one or
//! more addon exports in `export_string`.
//!
//! Only the submission id and the export are read. Every other field (the pseudonymous
//! contributor id, free text such as `nickname`) is never deserialized.
//!
//! Reading happens in two steps, so decoded values can borrow the inflated bytes:
//! [`Submission::read`] inflates every export, [`Submission::decode`] decodes them.

use crate::export::{decode_for_print, find_exports, is_js_whitespace, Export, Inflater};
use crate::session::Session;
use crate::value::{decode_trace, Value};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::path::Path;

/// A submission with its exports inflated.
pub struct Submission {
    /// The file's `id`, else the id in its file name.
    pub submission_id: String,
    /// Why the file could not be read at all.
    error: Option<String>,
    /// Per export: its CBOR bytes, or why it was dropped.
    exports: Vec<Result<Vec<u8>, String>>,
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

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    id: Option<&'a RawValue>,
    #[serde(borrow)]
    export_string: Option<&'a RawValue>,
}

fn json_string(raw: Option<&RawValue>) -> Option<String> {
    raw.and_then(|raw| serde_json::from_str::<String>(raw.get()).ok())
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
        let mut submission = Submission {
            submission_id: submission_id_from_path(path),
            error: None,
            exports: Vec::new(),
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => {
                submission.error = Some("unreadable submission JSON".into());
                return submission;
            }
        };
        let envelope = match serde_json::from_slice::<Envelope>(&bytes) {
            Ok(envelope) => envelope,
            // An array is a JS object too: it parses, but has no export_string.
            Err(_) if serde_json::from_slice::<Vec<&RawValue>>(&bytes).is_ok() => {
                submission.error = Some("no export_string".into());
                return submission;
            }
            // Never pass serde's message on: it can quote the input, which holds free text.
            Err(_) => {
                submission.error = Some("unreadable submission JSON".into());
                return submission;
            }
        };
        if let Some(id) = json_string(envelope.id).filter(|id| !id.is_empty()) {
            submission.submission_id = id;
        }
        let Some(export_string) = json_string(envelope.export_string) else {
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
                Ok(cbor) => match decode_root(cbor) {
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

fn inflate_export(index: usize, export: &Export, inflater: &mut Inflater) -> Result<Vec<u8>, String> {
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

    #[test]
    fn the_id_comes_from_the_file_name_after_the_last_underscore() {
        assert_eq!(submission_id_from_path(Path::new("2026-11/2026-11-05_204720Z_0b9d.json")), "0b9d");
        assert_eq!(submission_id_from_path(Path::new("plain.json")), "plain");
    }
}
