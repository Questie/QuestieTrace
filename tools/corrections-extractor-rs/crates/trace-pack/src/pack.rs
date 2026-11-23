//! A pack: one frame per submission, then an index of the frames. See FORMAT.md.
//!
//! Writing is two steps so the first can run on many threads: [`build_frame`] turns one
//! submission file into a compressed frame and checks that it reads back, [`PackWriter`] puts
//! frames together. [`PackReader`] reads a pack that is held in memory.

use crate::record::{Encoding, Export, Record};
use crate::values::{self, EncodeStats, TpPolicy};
use crate::verify::verify_export;
use crate::wire::{self, Reader, WireError};
use sha2::{Digest, Sha256};
use std::path::Path;
use trace_data::submission::{export_string_span, inflate_exports, submission_id_from_path, Submission};
use trace_data::value::decode_trace;
use trace_data::{Arena, Inflater};

pub const FORMAT_VERSION: u32 = 1;

/// A pack starts and ends with a zstd skippable frame, so `zstd -d` reads a whole pack and
/// simply steps over them.
const HEADER_MAGIC: [u8; 4] = 0x184D_2A50u32.to_le_bytes();
const TRAILER_MAGIC: [u8; 4] = 0x184D_2A51u32.to_le_bytes();
const PACK_MAGIC: &[u8; 4] = b"QTPK";
const HEADER_LEN: usize = 16;
const TRAILER_LEN: usize = 24;

/// The most a frame may decompress to. A record holds a submission's exports, each at most
/// 512 MiB of CBOR (`trace_data`'s limit); the index of a pack is far smaller. A crafted frame
/// that claims more is refused before anything is allocated for it.
const MAX_RECORD_BYTES: u64 = 1 << 30;
const MAX_INDEX_BYTES: u64 = 256 << 20;

pub type Sha256Digest = [u8; 32];

/// One zstd frame with a content checksum: a damaged frame then fails to decompress, where
/// without one it would mostly decompress to other bytes.
fn compress(bytes: &[u8], level: i32) -> Result<Vec<u8>, String> {
    let mut compressor = zstd::bulk::Compressor::new(level).map_err(|error| error.to_string())?;
    compressor.include_checksum(true).map_err(|error| error.to_string())?;
    compressor.compress(bytes).map_err(|error| error.to_string())
}

/// Decompresses one frame. The frame must state its size, as every frame of a pack does, and
/// the size must be within `limit`.
fn decompress(frame: &[u8], limit: u64) -> Result<Vec<u8>, String> {
    let size = match zstd::zstd_safe::get_frame_content_size(frame) {
        Ok(Some(size)) if size <= limit => size as usize,
        Ok(Some(size)) => return Err(format!("frame of {size} bytes is over the limit of {limit}")),
        _ => return Err("frame does not state its size".into()),
    };
    zstd::bulk::decompress(frame, size).map_err(|error| error.to_string())
}

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256::digest(bytes).into()
}

#[derive(Clone, Debug)]
pub struct PackOptions {
    /// zstd compression level.
    pub level: i32,
    /// Off stores every export exact: no value encoding at all.
    pub value_encoding: bool,
    pub tp: TpPolicy,
}

impl Default for PackOptions {
    fn default() -> Self {
        PackOptions {
            level: 9,
            value_encoding: true,
            tp: TpPolicy::Off,
        }
    }
}

/// One submission in the index.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Where the submission's frame is in the pack.
    pub offset: u64,
    pub length: u64,
    pub submission_id: String,
    pub contributor_id: String,
    pub received_at: String,
    /// Per export, how it is stored and the SHA-256 of its CBOR as a reader gets it. For a
    /// file kept whole: one `Raw` entry with the SHA-256 of the file.
    pub exports: Vec<(Encoding, Sha256Digest)>,
}

/// A submission ready to be added to a pack.
pub struct Frame {
    pub compressed: Vec<u8>,
    /// `offset` is set when the frame is added to a pack.
    pub entry: Entry,
    pub report: FrameReport,
}

/// What happened to one submission, for the pack's report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameReport {
    pub file_bytes: u64,
    pub cbor_bytes: u64,
    /// Exports the value encoding was tried on and that did not read back as required, so
    /// they were stored exact. Should be none; a pack with many is a sign of a fault.
    pub value_fallbacks: Vec<String>,
    pub values: EncodeStats,
}

/// Buffers one worker thread reuses between submissions.
pub struct Scratch {
    inflater: Inflater,
    arena: Arena,
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Scratch {
    pub fn new() -> Self {
        Scratch {
            inflater: Inflater::new(),
            arena: Arena::new(),
        }
    }
}

// ---- Building a frame ----

/// The frame for one submission file. `name` is the key the file has in its source.
///
/// The frame is read back before it is returned: what a reader gets from it must be the file,
/// or its exports up to the `tp` rounding the format permits. `Err` means a frame that does
/// not read back could not be avoided; the pack must then not be written.
pub fn build_frame(name: &str, file: &[u8], options: &PackOptions, scratch: &mut Scratch) -> Result<Frame, String> {
    let mut report = FrameReport {
        file_bytes: file.len() as u64,
        ..FrameReport::default()
    };

    // The exports as the extractors read them today. A file they cannot split is kept whole.
    let Some(SplitFile {
        head,
        tail,
        exports: originals,
    }) = split_file(file, &mut scratch.inflater)
    else {
        let record = Record::File {
            name: name.to_owned(),
            bytes: file.to_vec(),
        };
        return finish_frame(record, &[], Vec::new(), file, options, scratch, report);
    };
    report.cbor_bytes = originals.iter().map(|cbor| cbor.len() as u64).sum();

    // First with the value encoding where it applies.
    let mut exports = Vec::with_capacity(originals.len());
    let mut stats = Vec::with_capacity(originals.len());
    for cbor in &originals {
        let encoded = (options.value_encoding && decodes(cbor, &mut scratch.arena))
            .then(|| values::encode(cbor, &options.tp))
            .flatten();
        let (export, export_stats) = match encoded {
            Some((encoded, stats)) => (Export::Value(encoded), Some(stats)),
            None => (Export::Exact(cbor.clone()), None),
        };
        exports.push(export);
        stats.push(export_stats);
    }
    let record = Record::Exports {
        name: name.to_owned(),
        head,
        tail,
        exports,
    };
    finish_frame(record, &originals, stats, file, options, scratch, report)
}

/// A submission file taken apart: the text around its export string, and its exports inflated.
struct SplitFile {
    head: Vec<u8>,
    tail: Vec<u8>,
    exports: Vec<Vec<u8>>,
}

/// None when the file is not UTF-8, has no plain export string, or any of its exports cannot
/// be inflated.
fn split_file(file: &[u8], inflater: &mut Inflater) -> Option<SplitFile> {
    let text = std::str::from_utf8(file).ok()?;
    let span = export_string_span(text)?;
    let exports = inflate_exports(&text[span.clone()], inflater).ok()?;
    let all_inflated = exports.iter().all(Result::is_ok);
    let cbor: Vec<Vec<u8>> = exports.iter().flatten().map(|inflated| inflated.bytes().to_vec()).collect();
    inflater.recycle(exports.into_iter().flatten());
    all_inflated.then(|| SplitFile {
        head: file[..span.start].to_vec(),
        tail: file[span.end..].to_vec(),
        exports: cbor,
    })
}

/// Whether the extractors' decoder accepts the export. The value encoding is tried only then;
/// an export it rejects is stored exact, to be rejected again with the same message.
fn decodes(cbor: &[u8], arena: &mut Arena) -> bool {
    let accepted = decode_trace(cbor, arena).is_ok();
    arena.reset();
    accepted
}

/// Compresses the record and reads it back. A value-encoded export that does not read back as
/// required is stored exact and the frame is built once more. `stats` holds, per export, what
/// the value encoding converted; only exports that stay value-encoded are counted.
fn finish_frame(
    mut record: Record,
    originals: &[Vec<u8>],
    mut stats: Vec<Option<EncodeStats>>,
    file: &[u8],
    options: &PackOptions,
    scratch: &mut Scratch,
    mut report: FrameReport,
) -> Result<Frame, String> {
    loop {
        let compressed = compress(&record.encode(), options.level).map_err(|error| format!("{}: {error}", record.name()))?;
        match read_back(&compressed, &record, originals, file, &options.tp) {
            Ok(exports) => {
                stats.iter().flatten().for_each(|stats| report.values.add(stats));
                let entry = entry_for(&record, exports, compressed.len() as u64, &mut scratch.inflater);
                return Ok(Frame { compressed, entry, report });
            }
            Err(ReadBack::Exports(failed)) => {
                let name = record.name().to_owned();
                let Record::Exports { exports, .. } = &mut record else {
                    unreachable!("only a record of exports reports failed exports");
                };
                for (index, message) in failed {
                    // An exact export that fails cannot be made more exact.
                    if exports[index].encoding() != Encoding::Value {
                        return Err(format!("{name}: export {} does not read back: {message}", index + 1));
                    }
                    exports[index] = Export::Exact(originals[index].clone());
                    stats[index] = None;
                    report.value_fallbacks.push(format!("{name} export {}: {message}", index + 1));
                }
            }
            Err(ReadBack::Frame(message)) => return Err(format!("{}: frame does not read back: {message}", record.name())),
        }
    }
}

enum ReadBack {
    /// These exports do not read back as required: their index and why.
    Exports(Vec<(usize, String)>),
    Frame(String),
}

/// Reads a frame the way a reader does, from its compressed bytes, and compares what comes
/// out with what went in. Ok carries the index's line for each export.
fn read_back(compressed: &[u8], written: &Record, originals: &[Vec<u8>], file: &[u8], policy: &TpPolicy) -> Result<Vec<(Encoding, Sha256Digest)>, ReadBack> {
    let frame = |message: String| ReadBack::Frame(message);
    let bytes = decompress(compressed, MAX_RECORD_BYTES).map_err(frame)?;
    let record = Record::decode(&bytes).map_err(|error| frame(error.0))?;
    if record.name() != written.name() {
        return Err(frame("the name changed".into()));
    }
    match (&record, written) {
        (Record::File { bytes, .. }, Record::File { .. }) if bytes.as_slice() == file => Ok(vec![(Encoding::Raw, sha256(bytes))]),
        (Record::File { .. }, _) => Err(frame("the file changed".into())),
        (
            Record::Exports { head, tail, exports, .. },
            Record::Exports {
                head: written_head,
                tail: written_tail,
                ..
            },
        ) => {
            if head != written_head || tail != written_tail || exports.len() != originals.len() {
                return Err(frame("the text around the exports changed".into()));
            }
            let mut lines = Vec::with_capacity(exports.len());
            let mut failed = Vec::new();
            for (index, (export, original)) in exports.iter().zip(originals).enumerate() {
                match check_export(export, original, policy) {
                    Ok(digest) => lines.push((export.encoding(), digest)),
                    Err(message) => failed.push((index, message)),
                }
            }
            if failed.is_empty() {
                Ok(lines)
            } else {
                Err(ReadBack::Exports(failed))
            }
        }
        (Record::Exports { .. }, Record::File { .. }) => Err(frame("the kind of record changed".into())),
    }
}

/// What a reader gets for `export` must be `original`: byte for byte when stored exact, up to
/// the permitted `tp` rounding when value-encoded. Ok carries the SHA-256 of what is read.
fn check_export(export: &Export, original: &[u8], policy: &TpPolicy) -> Result<Sha256Digest, String> {
    let rebuilt = export.cbor()?;
    match export {
        Export::Exact(_) if *rebuilt != *original => return Err("an export stored exact is not the original".into()),
        Export::Exact(_) => {}
        Export::Value(_) => verify_export(original, &rebuilt, policy)?,
    }
    Ok(sha256(&rebuilt))
}

/// Compares a record read from a pack with the submission file it was made from, and its
/// index entry with both. For checking a finished pack against its source. `policy` is the
/// one the pack was built with: `tp` rounded anywhere it does not allow is an error.
pub fn check_against_file(record: &Record, entry: &Entry, file: &[u8], policy: &TpPolicy, scratch: &mut Scratch) -> Result<(), String> {
    // The index entry's ids must be the ones the extractors read from the file itself.
    let from_file = Submission::from_json(file, submission_id_from_path(Path::new(&entry.name)), &mut scratch.inflater);
    let ids_match = record.name() == entry.name
        && from_file.submission_id == entry.submission_id
        && from_file.contributor_id == entry.contributor_id
        && from_file.received_at == entry.received_at;
    if !ids_match {
        return Err("the index entry's ids are not the file's".into());
    }

    let lines = match record {
        Record::File { bytes, .. } if bytes.as_slice() == file => vec![(Encoding::Raw, sha256(bytes))],
        Record::File { .. } => return Err("the file differs from the one kept whole".into()),
        Record::Exports { head, tail, exports, .. } => {
            let Some(SplitFile {
                head: file_head,
                tail: file_tail,
                exports: originals,
            }) = split_file(file, &mut scratch.inflater)
            else {
                return Err("the file cannot be split into exports, but the pack holds exports".into());
            };
            if *head != file_head || *tail != file_tail || exports.len() != originals.len() {
                return Err("the text around the exports differs".into());
            }
            let mut lines = Vec::with_capacity(exports.len());
            for (index, (export, original)) in exports.iter().zip(&originals).enumerate() {
                let digest = check_export(export, original, policy).map_err(|message| format!("export {}: {message}", index + 1))?;
                lines.push((export.encoding(), digest));
            }
            lines
        }
    };
    if lines != entry.exports {
        return Err("the index entry does not match the frame".into());
    }
    Ok(())
}

/// The index entry: the ids as the extractors read them from the record.
fn entry_for(record: &Record, exports: Vec<(Encoding, Sha256Digest)>, length: u64, inflater: &mut Inflater) -> Entry {
    // Only the ids are needed here, so a record of exports is read without its exports.
    let ids = match record {
        Record::File { .. } => record.submission(inflater),
        Record::Exports { name, head, tail, .. } => Record::Exports {
            name: name.clone(),
            head: head.clone(),
            tail: tail.clone(),
            exports: Vec::new(),
        }
        .submission(inflater),
    };
    let ids = ids.expect("a record without value-encoded exports always reads");
    Entry {
        name: record.name().to_owned(),
        offset: 0,
        length,
        submission_id: ids.submission_id.clone(),
        contributor_id: ids.contributor_id.clone(),
        received_at: ids.received_at.clone(),
        exports,
    }
}

// ---- Writing a pack ----

pub struct PackWriter {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
    /// The SHA-256 of each frame as it was handed in, to check the finished pack against.
    frame_digests: Vec<Sha256Digest>,
}

impl Default for PackWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PackWriter {
    pub fn new() -> Self {
        let mut bytes = Vec::new();
        bytes.extend(HEADER_MAGIC);
        bytes.extend(8u32.to_le_bytes());
        bytes.extend(PACK_MAGIC);
        bytes.extend(FORMAT_VERSION.to_le_bytes());
        PackWriter {
            bytes,
            entries: Vec::new(),
            frame_digests: Vec::new(),
        }
    }

    pub fn add(&mut self, frame: Frame) {
        let mut entry = frame.entry;
        entry.offset = self.bytes.len() as u64;
        self.frame_digests.push(sha256(&frame.compressed));
        self.bytes.extend_from_slice(&frame.compressed);
        self.entries.push(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The finished pack: the frames, the index, and the trailer that says where the index is.
    ///
    /// The pack is opened again before it is returned: its index must name every frame that
    /// was added, where it was put, and each frame must still be the bytes that were added.
    pub fn finish(self, level: i32) -> Result<Vec<u8>, String> {
        let PackWriter {
            mut bytes,
            entries,
            frame_digests,
        } = self;
        let index = compress(&encode_index(&entries), level)?;
        let index_offset = bytes.len() as u64;
        bytes.extend_from_slice(&index);
        bytes.extend(TRAILER_MAGIC);
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(index_offset.to_le_bytes());
        bytes.extend((index.len() as u64).to_le_bytes());

        let reader = PackReader::open(&bytes).map_err(|message| format!("the finished pack does not open: {message}"))?;
        if reader.entries() != entries {
            return Err("the finished pack's index is not the one written".into());
        }
        let frames_intact = entries.iter().zip(&frame_digests).all(|(entry, digest)| sha256(reader.frame(entry)) == *digest);
        if !frames_intact {
            return Err("a frame of the finished pack is not the one added".into());
        }
        Ok(bytes)
    }
}

fn encode_index(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    wire::write_array_head(&mut out, entries.len());
    for entry in entries {
        wire::write_array_head(&mut out, 7);
        wire::write_text(&mut out, &entry.name);
        wire::write_uint(&mut out, entry.offset);
        wire::write_uint(&mut out, entry.length);
        wire::write_text(&mut out, &entry.submission_id);
        wire::write_text(&mut out, &entry.contributor_id);
        wire::write_text(&mut out, &entry.received_at);
        wire::write_array_head(&mut out, entry.exports.len());
        for (encoding, digest) in &entry.exports {
            wire::write_array_head(&mut out, 2);
            wire::write_uint(&mut out, *encoding as u64);
            wire::write_bytes(&mut out, digest);
        }
    }
    out
}

fn decode_index(bytes: &[u8]) -> Result<Vec<Entry>, WireError> {
    let malformed = || WireError("malformed pack index".into());
    let mut reader = Reader::new(bytes);
    let mut entries = Vec::new();
    for _ in 0..reader.array()? {
        if reader.array()? != 7 {
            return Err(malformed());
        }
        let mut entry = Entry {
            name: reader.text()?.to_owned(),
            offset: reader.uint()?,
            length: reader.uint()?,
            submission_id: reader.text()?.to_owned(),
            contributor_id: reader.text()?.to_owned(),
            received_at: reader.text()?.to_owned(),
            exports: Vec::new(),
        };
        for _ in 0..reader.array()? {
            if reader.array()? != 2 {
                return Err(malformed());
            }
            let encoding = match reader.uint()? {
                0 => Encoding::Raw,
                1 => Encoding::Exact,
                2 => Encoding::Value,
                _ => return Err(malformed()),
            };
            entry.exports.push((encoding, reader.bytes()?.try_into().map_err(|_| malformed())?));
        }
        entries.push(entry);
    }
    if !reader.is_done() {
        return Err(malformed());
    }
    Ok(entries)
}

// ---- Reading a pack ----

pub struct PackReader<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

impl<'a> PackReader<'a> {
    /// Reads the index of a pack held in memory. Every frame the index names must lie inside
    /// the pack, before the index.
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        let header = bytes.get(..HEADER_LEN).ok_or("not a pack: too short")?;
        if header[..4] != HEADER_MAGIC || header[4..8] != 8u32.to_le_bytes() || header[8..12] != *PACK_MAGIC {
            return Err("not a pack".into());
        }
        let version = u32::from_le_bytes(header[12..16].try_into().expect("four bytes"));
        if version != FORMAT_VERSION {
            return Err(format!("pack format version {version} is not supported"));
        }
        let trailer = bytes
            .len()
            .checked_sub(TRAILER_LEN)
            .map(|start| &bytes[start..])
            .ok_or("not a pack: too short")?;
        if trailer[..4] != TRAILER_MAGIC || trailer[4..8] != 16u32.to_le_bytes() {
            return Err("pack has no trailer: it was cut off".into());
        }
        let number = |range: std::ops::Range<usize>| u64::from_le_bytes(trailer[range].try_into().expect("eight bytes"));
        let (index_offset, index_length) = (number(8..16), number(16..24));
        let index_end = index_offset.checked_add(index_length).filter(|&end| end == (bytes.len() - TRAILER_LEN) as u64);
        if index_offset < HEADER_LEN as u64 || index_end.is_none() {
            return Err("pack trailer does not point at the index".into());
        }
        let index =
            decompress(&bytes[index_offset as usize..bytes.len() - TRAILER_LEN], MAX_INDEX_BYTES).map_err(|message| format!("pack index: {message}"))?;
        let entries = decode_index(&index).map_err(|error| error.0)?;

        // The frames must fill the space between the header and the index, back to back and in
        // index order: one entry for every frame, and no bytes the index does not account for.
        let mut expected_offset = HEADER_LEN as u64;
        for entry in &entries {
            if entry.offset != expected_offset {
                return Err("pack index does not list the frames back to back".into());
            }
            expected_offset = entry
                .offset
                .checked_add(entry.length)
                .filter(|&end| end <= index_offset)
                .ok_or("pack index names a frame outside the pack")?;
        }
        if expected_offset != index_offset {
            return Err("pack holds bytes its index does not list".into());
        }
        Ok(PackReader { bytes, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The compressed frame of an entry of this pack.
    fn frame(&self, entry: &Entry) -> &'a [u8] {
        &self.bytes[entry.offset as usize..(entry.offset + entry.length) as usize]
    }

    /// The record of the submission at `index` in [`Self::entries`]. A frame that was damaged
    /// fails its checksum here.
    pub fn record(&self, index: usize) -> Result<Record, String> {
        let entry = &self.entries[index];
        let bytes = decompress(self.frame(entry), MAX_RECORD_BYTES).map_err(|message| format!("{}: {message}", entry.name))?;
        let record = Record::decode(&bytes).map_err(|error| format!("{}: {}", entry.name, error.0))?;
        if record.name() != entry.name {
            return Err(format!("{}: the frame holds another submission", entry.name));
        }
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use crate::values::Field;

    /// An export string as the addon builds it: print-encoded raw deflate of the CBOR.
    fn export_string(cbor: &[u8]) -> String {
        const ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()";
        let mut compressor = libdeflater::Compressor::new(libdeflater::CompressionLvl::default());
        let mut deflated = vec![0; compressor.deflate_compress_bound(cbor.len())];
        let size = compressor.deflate_compress(cbor, &mut deflated).unwrap();
        let mut out = String::from("!QuestieTrace:1!");
        // Three bytes make four characters, least significant six bits first.
        for chunk in deflated[..size].chunks(3) {
            let bits = chunk.iter().enumerate().fold(0u32, |bits, (index, &byte)| bits | (byte as u32) << (8 * index));
            let characters = if chunk.len() == 3 { 4 } else { chunk.len() + 1 };
            (0..characters).for_each(|index| out.push(ALPHABET[(bits >> (6 * index)) as usize & 63] as char));
        }
        out.push_str("!End:QuestieTrace:1!");
        out
    }

    const STARTED_AT: f64 = 8551.460000000001;

    fn trace(tp: f64) -> Vec<u8> {
        let entry = map(&[
            (text("t"), float(8_559_178.0 * 0.001 - STARTED_AT)),
            (text("tp"), float(tp)),
            (text("e"), text("QUEST_ACCEPTED")),
        ]);
        let session = map(&[
            (text("schemaVersion"), uint(17)),
            (text("startedAt"), float(STARTED_AT)),
            (text("events"), array(&[entry])),
            (text("functions"), map(&[])),
        ]);
        map(&[(text("sessions"), array(&[session]))])
    }

    fn file(export_string: &str) -> Vec<u8> {
        format!(r#"{{"schema_version": 2, "id": "abc", "contributor_id": "c0ffee", "received_at": "2026-11-08T18:00:00Z", "nickname": "Thrall", "export_string": "{export_string}"}}"#).into_bytes()
    }

    fn options() -> PackOptions {
        PackOptions {
            tp: TpPolicy::SchemaVersions(vec![17]),
            ..PackOptions::default()
        }
    }

    fn pack_of(files: &[(&str, Vec<u8>)], options: &PackOptions) -> (Vec<u8>, Vec<FrameReport>) {
        let mut scratch = Scratch::new();
        let mut writer = PackWriter::new();
        let mut reports = Vec::new();
        for (name, file) in files {
            let frame = build_frame(name, file, options, &mut scratch).unwrap();
            reports.push(frame.report.clone());
            writer.add(frame);
        }
        (writer.finish(options.level).unwrap(), reports)
    }

    /// The traces a submission decodes to, as JSON, with its ids and its errors.
    fn read(submission: &Submission) -> (String, String, String, Option<String>, Vec<String>, Vec<String>) {
        let arena = Arena::new();
        let decoded = submission.decode(&arena);
        (
            submission.submission_id.clone(),
            submission.contributor_id.clone(),
            submission.received_at.clone(),
            decoded.error.clone(),
            decoded.dropped_exports.clone(),
            decoded.traces.iter().map(|trace| trace_data::value::to_json(trace.root())).collect(),
        )
    }

    #[test]
    fn a_submission_read_from_a_pack_decodes_like_the_file_it_came_from() {
        // Whole microseconds in tp, so rounding changes nothing and the traces must be equal.
        let original = file(&export_string(&trace(7.370141)));
        let (pack, reports) = pack_of(&[("2026-11/2026-11-08_180000Z_abc.json", original.clone())], &options());
        let reader = PackReader::open(&pack).unwrap();
        let entry = &reader.entries()[0];
        assert_eq!(
            (
                entry.name.as_str(),
                entry.submission_id.as_str(),
                entry.contributor_id.as_str(),
                entry.received_at.as_str()
            ),
            ("2026-11/2026-11-08_180000Z_abc.json", "abc", "c0ffee", "2026-11-08T18:00:00Z")
        );
        assert_eq!(entry.exports.iter().map(|(encoding, _)| *encoding).collect::<Vec<_>>(), [Encoding::Value]);
        assert_eq!(reports[0].values.converted[Field::T as usize], 1);

        let mut inflater = Inflater::new();
        let from_pack = reader.record(0).unwrap().submission(&mut inflater).unwrap();
        let from_file = Submission::from_json(&original, "abc".into(), &mut inflater);
        assert_eq!(read(&from_pack), read(&from_file));
        assert_eq!(read(&from_pack).5.len(), 1);
    }

    #[test]
    fn the_text_around_the_export_string_is_kept_byte_for_byte() {
        let original = file(&export_string(&trace(7.370141)));
        let (pack, _) = pack_of(&[("a_1.json", original.clone())], &options());
        let Record::Exports { head, tail, .. } = PackReader::open(&pack).unwrap().record(0).unwrap() else {
            panic!("expected a record of exports");
        };
        let text = String::from_utf8(original).unwrap();
        let (before, after) = text.split_once("!QuestieTrace:1!").unwrap();
        assert_eq!(String::from_utf8(head).unwrap(), before);
        assert_eq!(String::from_utf8(tail).unwrap(), after.rsplit_once("!End:QuestieTrace:1!").unwrap().1);
    }

    #[test]
    fn a_tp_between_microseconds_reads_back_rounded_and_the_index_digest_is_of_what_is_read() {
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&trace(7.370140500001071))))], &options());
        let reader = PackReader::open(&pack).unwrap();
        let Record::Exports { exports, .. } = reader.record(0).unwrap() else {
            panic!("expected a record of exports");
        };
        let rebuilt = exports[0].cbor().unwrap();
        assert_eq!(*rebuilt, trace(7.370141));
        assert_eq!(reader.entries()[0].exports[0].1, sha256(&rebuilt));
    }

    #[test]
    fn with_the_value_encoding_off_every_export_is_stored_exact() {
        let cbor = trace(7.370140500001071);
        let exact = PackOptions {
            value_encoding: false,
            ..options()
        };
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&cbor)))], &exact);
        let reader = PackReader::open(&pack).unwrap();
        assert_eq!(reader.entries()[0].exports, [(Encoding::Exact, sha256(&cbor))]);
        assert_eq!(reader.record(0).unwrap(), {
            let text = String::from_utf8(file(&export_string(&cbor))).unwrap();
            let (head, rest) = text.split_once("!QuestieTrace:1!").unwrap();
            Record::Exports {
                name: "a_1.json".into(),
                head: head.as_bytes().to_vec(),
                tail: rest.rsplit_once("!End:QuestieTrace:1!").unwrap().1.as_bytes().to_vec(),
                exports: vec![Export::Exact(cbor)],
            }
        });
    }

    #[test]
    fn files_the_extractors_cannot_split_are_kept_whole_and_fail_the_same_way() {
        let cut_off = file("!QuestieTrace:1!abcdef");
        let not_json = b"\xff\xfe not json".to_vec();
        let two = format!("{} {}", export_string(&trace(1.25)), "!QuestieTrace:1!zz!End:QuestieTrace:1!");
        let one_bad_export = file(&two);
        let files = [("a_1.json", cut_off), ("b_2.json", not_json), ("c_3.json", one_bad_export)];
        let (pack, _) = pack_of(&files, &options());
        let reader = PackReader::open(&pack).unwrap();
        let mut inflater = Inflater::new();
        for (index, (name, original)) in files.iter().enumerate() {
            assert_eq!(reader.entries()[index].exports, [(Encoding::Raw, sha256(original))], "{name}");
            let from_pack = reader.record(index).unwrap().submission(&mut inflater).unwrap();
            let from_file = Submission::from_json(original, trace_data::submission::submission_id_from_path(name.as_ref()), &mut inflater);
            assert_eq!(read(&from_pack), read(&from_file), "{name}");
        }
        // The third file still yields its good export, with the reason the other was dropped.
        let third = read(&reader.record(2).unwrap().submission(&mut inflater).unwrap());
        assert_eq!((third.4.len(), third.5.len()), (1, 1));
    }

    #[test]
    fn an_export_that_inflates_but_is_not_a_trace_is_stored_exact_and_rejected_as_before() {
        // A CBOR tag: the decoder does not support it.
        let original = file(&export_string(&[0xc0, 0x00]));
        let (pack, _) = pack_of(&[("a_1.json", original.clone())], &options());
        let reader = PackReader::open(&pack).unwrap();
        assert_eq!(reader.entries()[0].exports[0].0, Encoding::Exact);
        let mut inflater = Inflater::new();
        let from_pack = read(&reader.record(0).unwrap().submission(&mut inflater).unwrap());
        assert_eq!(from_pack, read(&Submission::from_json(&original, "1".into(), &mut inflater)));
        assert_eq!(from_pack.3.as_deref(), Some("export 1: CBOR tags are not supported"));
    }

    #[test]
    fn each_submission_is_found_through_the_index_among_many() {
        let files: Vec<(String, Vec<u8>)> = (0..5)
            .map(|index| (format!("s_{index}.json"), file(&export_string(&trace(index as f64 + 0.250001)))))
            .collect();
        let borrowed: Vec<(&str, Vec<u8>)> = files.iter().map(|(name, file)| (name.as_str(), file.clone())).collect();
        let (pack, _) = pack_of(&borrowed, &options());
        let reader = PackReader::open(&pack).unwrap();
        assert_eq!(reader.entries().len(), 5);
        for index in [3, 0, 4] {
            let Record::Exports { exports, .. } = reader.record(index).unwrap() else {
                panic!("expected a record of exports");
            };
            assert_eq!(*exports[0].cbor().unwrap(), trace(index as f64 + 0.250001));
        }
    }

    #[test]
    fn a_pack_that_was_cut_off_or_altered_is_refused() {
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&trace(1.25))))], &options());
        assert_eq!(
            PackReader::open(&pack[..pack.len() - 1]).err().as_deref(),
            Some("pack has no trailer: it was cut off")
        );
        assert_eq!(PackReader::open(&pack[..10]).err().as_deref(), Some("not a pack: too short"));
        // The trailer's index offset moved by one.
        let mut moved = pack.clone();
        let at = moved.len() - 16;
        moved[at] ^= 1;
        assert_eq!(PackReader::open(&moved).err().as_deref(), Some("pack trailer does not point at the index"));
        let mut other_format = pack.clone();
        other_format[12] = 2;
        assert_eq!(PackReader::open(&other_format).err().as_deref(), Some("pack format version 2 is not supported"));
    }

    #[test]
    fn plain_zstd_reads_a_whole_pack_and_steps_over_its_header_and_trailer() {
        let cbor = trace(1.25);
        let exact = PackOptions {
            value_encoding: false,
            ..options()
        };
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&cbor)))], &exact);
        let everything = zstd::stream::decode_all(pack.as_slice()).unwrap();
        // The record, then the index: the record alone decodes from the front.
        let reader = PackReader::open(&pack).unwrap();
        let record = reader.record(0).unwrap().encode();
        assert_eq!(&everything[..record.len()], record);
        assert!(everything.len() > record.len());
    }

    #[test]
    fn a_pack_starts_and_ends_with_the_bytes_the_format_names() {
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&trace(1.25))))], &options());
        assert_eq!(pack[..16], [0x50, 0x2a, 0x4d, 0x18, 8, 0, 0, 0, b'Q', b'T', b'P', b'K', 1, 0, 0, 0]);
        let trailer = &pack[pack.len() - 24..];
        assert_eq!(trailer[..8], [0x51, 0x2a, 0x4d, 0x18, 16, 0, 0, 0]);
        // The one frame starts right after the header, the index right after the frame.
        let length = PackReader::open(&pack).unwrap().entries()[0].length;
        assert_eq!(PackReader::open(&pack).unwrap().entries()[0].offset, 16);
        assert_eq!(u64::from_le_bytes(trailer[8..16].try_into().unwrap()), 16 + length);
        assert_eq!(u64::from_le_bytes(trailer[16..24].try_into().unwrap()), pack.len() as u64 - 24 - 16 - length);
    }

    #[test]
    fn the_index_lists_name_position_ids_and_one_line_per_export() {
        let cbor = trace(1.25);
        let exact = PackOptions {
            value_encoding: false,
            ..options()
        };
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&cbor)))], &exact);
        let length = PackReader::open(&pack).unwrap().entries()[0].length;
        let index_frame = &pack[16 + length as usize..pack.len() - 24];
        let line = array(&[uint(1), bytes(&sha256(&cbor))]);
        let entry = array(&[
            text("a_1.json"),
            uint(16),
            uint(length),
            text("abc"),
            text("c0ffee"),
            text("2026-11-08T18:00:00Z"),
            array(&[line]),
        ]);
        assert_eq!(zstd::stream::decode_all(index_frame).unwrap(), array(&[entry]));
    }

    /// The file for `cbor` and the record a writer would make of it, with `export` in place of
    /// the export it would choose.
    fn record_with(cbor: &[u8], export: Export) -> (Vec<u8>, Record) {
        let original = file(&export_string(cbor));
        let text = String::from_utf8(original.clone()).unwrap();
        let (head, rest) = text.split_once("!QuestieTrace:1!").unwrap();
        let record = Record::Exports {
            name: "a_1.json".into(),
            head: head.as_bytes().to_vec(),
            tail: rest.rsplit_once("!End:QuestieTrace:1!").unwrap().1.as_bytes().to_vec(),
            exports: vec![export],
        };
        (original, record)
    }

    #[test]
    fn a_value_export_that_does_not_read_back_is_stored_exact_and_reported() {
        // What a faulty encoder could hand over: a t that is one millisecond off.
        let cbor = trace(7.370141);
        let (mut encoded, stats) = values::encode(&cbor, &options().tp).unwrap();
        encoded.streams[Field::T as usize][0] += 1;
        let (original, record) = record_with(&cbor, Export::Value(encoded));

        let frame = finish_frame(
            record,
            std::slice::from_ref(&cbor),
            vec![Some(stats)],
            &original,
            &options(),
            &mut Scratch::new(),
            FrameReport::default(),
        )
        .unwrap();
        assert_eq!(frame.entry.exports, [(Encoding::Exact, sha256(&cbor))]);
        assert_eq!(frame.report.value_fallbacks.len(), 1);
        assert!(
            frame.report.value_fallbacks[0].starts_with("a_1.json export 1: an item differs at byte"),
            "{:?}",
            frame.report.value_fallbacks
        );
        // Nothing of the abandoned encoding is counted as converted.
        assert_eq!(frame.report.values, EncodeStats::default());
    }

    #[test]
    fn an_exact_export_that_is_not_the_original_fails_the_frame() {
        let cbor = trace(7.370141);
        let (original, record) = record_with(&cbor, Export::Exact(trace(7.370142)));
        let error = finish_frame(record, &[cbor], vec![None], &original, &options(), &mut Scratch::new(), FrameReport::default()).err();
        assert_eq!(
            error.as_deref(),
            Some("a_1.json: export 1 does not read back: an export stored exact is not the original")
        );
    }

    #[test]
    fn a_finished_pack_is_checked_against_the_file_and_the_policy_it_was_built_with() {
        let name = "2026-11/2026-11-08_180000Z_abc.json";
        let original = file(&export_string(&trace(7.370140500001071)));
        let (pack, _) = pack_of(&[(name, original.clone())], &options());
        let reader = PackReader::open(&pack).unwrap();
        let (record, entry) = (reader.record(0).unwrap(), reader.entries()[0].clone());
        let mut scratch = Scratch::new();
        let mut check = |entry: &Entry, file: &[u8], policy: &TpPolicy| check_against_file(&record, entry, file, policy, &mut scratch);

        assert_eq!(check(&entry, &original, &options().tp), Ok(()));
        // The same pack, held against a policy that rounds no tp.
        assert_eq!(
            check(&entry, &original, &TpPolicy::Off).unwrap_err(),
            "export 1: session 1: tp was rounded in a session whose schemaVersion may not be"
        );
        // Another trace under the same name.
        assert!(check(&entry, &file(&export_string(&trace(9.5))), &options().tp).is_err());
        // An index entry whose ids are not the file's, and one whose digest is not the frame's.
        let mut other_contributor = entry.clone();
        other_contributor.contributor_id = "someone else".into();
        assert_eq!(
            check(&other_contributor, &original, &options().tp).unwrap_err(),
            "the index entry's ids are not the file's"
        );
        let mut other_digest = entry.clone();
        other_digest.exports[0].1[0] ^= 1;
        assert_eq!(
            check(&other_digest, &original, &options().tp).unwrap_err(),
            "the index entry does not match the frame"
        );
    }

    #[test]
    fn a_damaged_frame_never_reads_as_another_record() {
        let (pack, _) = pack_of(&[("a_1.json", file(&export_string(&trace(1.25))))], &options());
        let reader = PackReader::open(&pack).unwrap();
        let (entry, intact) = (reader.entries()[0].clone(), reader.record(0).unwrap());
        let mut refused = 0;
        for position in entry.offset as usize..(entry.offset + entry.length) as usize {
            let mut damaged = pack.clone();
            damaged[position] ^= 0x10;
            match PackReader::open(&damaged).and_then(|reader| reader.record(0)) {
                Ok(record) => assert_eq!(record, intact, "byte {position} changed what is read"),
                Err(_) => refused += 1,
            }
        }
        // Nearly every byte is covered by the frame's checksum.
        assert!(
            refused as u64 > entry.length * 9 / 10,
            "only {refused} of {} damaged frames were refused",
            entry.length
        );
    }

    #[test]
    fn a_frame_stating_more_than_the_limit_is_refused() {
        let frame = compress(&[7u8; 4096], 3).unwrap();
        assert_eq!(decompress(&frame, 4096).unwrap(), [7u8; 4096]);
        assert_eq!(decompress(&frame, 4095).unwrap_err(), "frame of 4096 bytes is over the limit of 4095");
    }

    /// A pack put together by hand from frames and whatever index is given.
    fn assemble(frames: &[&[u8]], entries: &[Entry]) -> Vec<u8> {
        let mut pack = PackWriter::new().bytes;
        frames.iter().for_each(|frame| pack.extend_from_slice(frame));
        let index = compress(&encode_index(entries), 3).unwrap();
        let index_offset = pack.len() as u64;
        pack.extend_from_slice(&index);
        pack.extend(TRAILER_MAGIC);
        pack.extend(16u32.to_le_bytes());
        pack.extend(index_offset.to_le_bytes());
        pack.extend((index.len() as u64).to_le_bytes());
        pack
    }

    #[test]
    fn an_index_that_does_not_account_for_every_byte_between_header_and_index_is_refused() {
        let mut scratch = Scratch::new();
        let frames: Vec<Frame> = ["a_1.json", "b_2.json"]
            .iter()
            .map(|name| build_frame(name, &file(&export_string(&trace(1.25))), &options(), &mut scratch).unwrap())
            .collect();
        let bytes: Vec<&[u8]> = frames.iter().map(|frame| frame.compressed.as_slice()).collect();
        let (mut first, mut second) = (frames[0].entry.clone(), frames[1].entry.clone());
        first.offset = 16;
        second.offset = 16 + first.length;
        let open = |entries: &[Entry]| PackReader::open(&assemble(&bytes, entries)).err();

        assert_eq!(open(&[first.clone(), second.clone()]), None);
        // The second frame is not listed, then listed first, then listed one byte late.
        assert_eq!(open(std::slice::from_ref(&first)).as_deref(), Some("pack holds bytes its index does not list"));
        assert_eq!(
            open(&[second.clone(), first.clone()]).as_deref(),
            Some("pack index does not list the frames back to back")
        );
        let mut late = second.clone();
        late.offset += 1;
        assert_eq!(open(&[first, late]).as_deref(), Some("pack index does not list the frames back to back"));
    }
}
