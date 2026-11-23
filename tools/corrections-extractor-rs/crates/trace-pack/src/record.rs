//! One submission as a pack holds it: what a frame decompresses to.
//!
//! A submission file is JSON around one long export string. A record keeps the JSON around
//! the string byte for byte, and in place of the string the exports it held, inflated. A file
//! that does not fit that shape is kept whole.

use crate::values::{self, ValueEncoded};
use crate::wire::{self, Reader, WireError};
use std::borrow::Cow;
use std::path::Path;
use trace_data::submission::{submission_id_from_path, Submission};
use trace_data::Inflater;

/// How a pack holds one export, or a whole file. The numbers are the format's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// The submission file as it was received.
    Raw = 0,
    /// The export's CBOR as it was inflated.
    Exact = 1,
    /// The export's CBOR in the value encoding.
    Value = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Export {
    Exact(Vec<u8>),
    Value(ValueEncoded),
}

impl Export {
    pub fn encoding(&self) -> Encoding {
        match self {
            Export::Exact(_) => Encoding::Exact,
            Export::Value(_) => Encoding::Value,
        }
    }

    /// The export's CBOR as a reader gets it.
    pub fn cbor(&self) -> Result<Cow<'_, [u8]>, String> {
        match self {
            Export::Exact(cbor) => Ok(Cow::Borrowed(cbor)),
            Export::Value(encoded) => values::decode(encoded).map(Cow::Owned),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// A file kept whole: it is not UTF-8, not the expected JSON, or one of its exports could
    /// not be inflated. Reading it goes through the same code as reading the file itself, so
    /// it fails in the same way with the same message.
    File { name: String, bytes: Vec<u8> },
    /// `head` is the file up to and including the quote that opens its export string, `tail`
    /// the file from the closing quote on. Together they are the file with an empty export
    /// string.
    Exports {
        name: String,
        head: Vec<u8>,
        tail: Vec<u8>,
        exports: Vec<Export>,
    },
}

const KIND_FILE: u64 = 0;
const KIND_EXPORTS: u64 = 1;

impl Record {
    /// The key the submission had where it was read from, such as `2026-11/<stamp>_<id>.json`.
    pub fn name(&self) -> &str {
        match self {
            Record::File { name, .. } | Record::Exports { name, .. } => name,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Record::File { name, bytes } => {
                wire::write_array_head(&mut out, 3);
                wire::write_uint(&mut out, KIND_FILE);
                wire::write_text(&mut out, name);
                wire::write_bytes(&mut out, bytes);
            }
            Record::Exports { name, head, tail, exports } => {
                wire::write_array_head(&mut out, 5);
                wire::write_uint(&mut out, KIND_EXPORTS);
                wire::write_text(&mut out, name);
                wire::write_bytes(&mut out, head);
                wire::write_bytes(&mut out, tail);
                wire::write_array_head(&mut out, exports.len());
                exports.iter().for_each(|export| write_export(&mut out, export));
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Record, WireError> {
        let mut reader = Reader::new(bytes);
        let fields = reader.array()?;
        let record = match (reader.uint()?, fields) {
            (KIND_FILE, 3) => Record::File {
                name: reader.text()?.to_owned(),
                bytes: reader.bytes()?.to_vec(),
            },
            (KIND_EXPORTS, 5) => Record::Exports {
                name: reader.text()?.to_owned(),
                head: reader.bytes()?.to_vec(),
                tail: reader.bytes()?.to_vec(),
                exports: (0..reader.array()?).map(|_| read_export(&mut reader)).collect::<Result<_, _>>()?,
            },
            _ => return Err(WireError("unknown kind of pack record".into())),
        };
        if !reader.is_done() {
            return Err(WireError("data after the pack record".into()));
        }
        Ok(record)
    }

    /// The submission for the extractors, as `Submission::read` gives it for the file.
    pub fn submission(&self, inflater: &mut Inflater) -> Result<Submission, String> {
        let fallback_id = submission_id_from_path(Path::new(self.name()));
        match self {
            Record::File { bytes, .. } => Ok(Submission::from_json(bytes, fallback_id, inflater)),
            Record::Exports { head, tail, exports, .. } => {
                let exports = exports.iter().map(|export| export.cbor().map(Cow::into_owned)).collect::<Result<_, _>>()?;
                Ok(Submission::from_inflated(&[head.as_slice(), tail.as_slice()].concat(), fallback_id, exports))
            }
        }
    }
}

fn write_export(out: &mut Vec<u8>, export: &Export) {
    match export {
        Export::Exact(cbor) => {
            wire::write_array_head(out, 2);
            wire::write_uint(out, Encoding::Exact as u64);
            wire::write_bytes(out, cbor);
        }
        Export::Value(encoded) => {
            wire::write_array_head(out, 7);
            wire::write_uint(out, Encoding::Value as u64);
            wire::write_bytes(out, &encoded.skeleton);
            wire::write_array_head(out, encoded.sessions.len());
            for &(started_at, t_count) in &encoded.sessions {
                wire::write_array_head(out, 2);
                wire::write_uint(out, started_at);
                wire::write_uint(out, t_count);
            }
            encoded.streams.iter().for_each(|stream| write_stream(out, stream));
        }
    }
}

fn read_export(reader: &mut Reader) -> Result<Export, WireError> {
    let fields = reader.array()?;
    match (reader.uint()?, fields) {
        (1, 2) => Ok(Export::Exact(reader.bytes()?.to_vec())),
        (2, 7) => {
            let skeleton = reader.bytes()?.to_vec();
            let sessions = (0..reader.array()?)
                .map(|_| match reader.array()? {
                    2 => Ok((reader.uint()?, reader.uint()?)),
                    _ => Err(WireError("a session row must hold a start time and a count".into())),
                })
                .collect::<Result<_, _>>()?;
            let streams = [read_stream(reader)?, read_stream(reader)?, read_stream(reader)?, read_stream(reader)?];
            Ok(Export::Value(ValueEncoded { skeleton, streams, sessions }))
        }
        _ => Err(WireError("unknown encoding of a packed export".into())),
    }
}

/// A value stream is stored as differences: each value minus the one before it, the first
/// minus zero. Neighbouring times are close, so the differences are small and repeat.
fn write_stream(out: &mut Vec<u8>, values: &[i64]) {
    wire::write_array_head(out, values.len());
    let mut previous = 0i64;
    for &value in values {
        wire::write_int(out, value.wrapping_sub(previous));
        previous = value;
    }
}

fn read_stream(reader: &mut Reader) -> Result<Vec<i64>, WireError> {
    let mut value = 0i64;
    (0..reader.array()?)
        .map(|_| {
            value = value.checked_add(reader.int()?).ok_or_else(|| WireError("a value stream overflows".into()))?;
            Ok(value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::Field;

    fn value_export() -> Export {
        let mut streams: [Vec<i64>; 4] = Default::default();
        streams[Field::T as usize] = vec![8_559_178, 8_559_178, 8_559_378, 100];
        streams[Field::X as usize] = vec![4733, -12];
        Export::Value(ValueEncoded {
            skeleton: vec![0xa1, 0x61, b't', 0xe1],
            streams,
            sessions: vec![(8551.46f64.to_bits(), 4)],
        })
    }

    #[test]
    fn records_of_both_kinds_survive_encoding() {
        let file = Record::File {
            name: "2026-11/a_1.json".into(),
            bytes: vec![0xff, 0x00, b'{'],
        };
        let exports = Record::Exports {
            name: "2026-11/b_2.json".into(),
            head: br#"{"id":"2","export_string":""#.to_vec(),
            tail: br#"","nickname":"x"}"#.to_vec(),
            exports: vec![Export::Exact(vec![0xa0]), value_export()],
        };
        for record in [file, exports] {
            assert_eq!(Record::decode(&record.encode()).unwrap(), record);
        }
    }

    #[test]
    fn a_stream_is_written_as_differences() {
        let mut out = Vec::new();
        write_stream(&mut out, &[1000, 1000, 1003, 990]);
        // An array of four: 1000, 0, 3, -13.
        assert_eq!(out, [0x84, 0x19, 0x03, 0xe8, 0x00, 0x03, 0x2c]);
        assert_eq!(read_stream(&mut Reader::new(&out)).unwrap(), [1000, 1000, 1003, 990]);
    }

    #[test]
    fn a_stream_whose_sum_leaves_the_integer_range_is_rejected() {
        let mut out = Vec::new();
        wire::write_array_head(&mut out, 2);
        wire::write_int(&mut out, i64::MAX);
        wire::write_int(&mut out, 1);
        assert_eq!(read_stream(&mut Reader::new(&out)), Err(WireError("a value stream overflows".into())));
    }

    #[test]
    fn trailing_data_and_unknown_kinds_are_rejected() {
        let mut bytes = Record::File {
            name: "a".into(),
            bytes: vec![1],
        }
        .encode();
        bytes.push(0);
        assert_eq!(Record::decode(&bytes), Err(WireError("data after the pack record".into())));
        // [9, "a", h'01']
        assert_eq!(
            Record::decode(&[0x83, 0x09, 0x61, b'a', 0x41, 0x01]),
            Err(WireError("unknown kind of pack record".into()))
        );
    }

    #[test]
    fn a_record_is_written_as_the_format_lays_it_out() {
        let file = Record::File {
            name: "a".into(),
            bytes: vec![0xff],
        };
        assert_eq!(file.encode(), [0x83, 0x00, 0x61, b'a', 0x41, 0xff]);

        let value = ValueEncoded {
            skeleton: vec![0xe1],
            streams: [vec![7], vec![1000, 1002], vec![], vec![-1]],
            sessions: vec![(1, 2)],
        };
        let exports = Record::Exports {
            name: "a".into(),
            head: b"{".to_vec(),
            tail: b"}".to_vec(),
            exports: vec![Export::Exact(vec![0xa0]), Export::Value(value)],
        };
        #[rustfmt::skip]
        let expected = [
            0x85, 0x01, 0x61, b'a', 0x41, b'{', 0x41, b'}',
            0x82,                         // two exports
            0x82, 0x01, 0x41, 0xa0,       // exact: the CBOR
            0x87, 0x02, 0x41, 0xe1,       // value: the skeleton
            0x81, 0x82, 0x01, 0x02,       // one session row: start time bits 1, two t values
            0x81, 0x07,                   // tp: 7
            0x82, 0x19, 0x03, 0xe8, 0x02, // t: 1000, then 2 more
            0x80,                         // x: none
            0x81, 0x20,                   // y: -1
        ];
        assert_eq!(exports.encode(), expected);
    }
}
