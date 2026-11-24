//! The archive bucket as the compactor uses it, and where things are in it.

use anyhow::{bail, Result};
use jiff::Timestamp;
use md5::{Digest, Md5};
use std::ops::Range;
use std::path::Path;
use trace_data::submission::submission_id_from_path;

/// Accepted submissions the intake copied here, one JSON object each, waiting to be packed.
/// What follows the prefix is the submission's name, which a pack's index keeps.
pub const LOOSE: &str = "loose/";

/// One object per submission the intake wants removed, keyed by submission id. Written and
/// kept by the intake; the compactor only reads them.
pub const REMOVAL_REQUESTS: &str = "removals/requested/";

/// The compactor's record that it began removing a submission, keyed by submission id.
pub const REMOVAL_INTENTS: &str = "removals/started/";

/// The compactor's answer to a request, keyed by submission id. The intake purges its receipt
/// of the submission when it lists one, which cannot be undone, so nothing else is ever
/// written here and one is written only when the submission is gone.
pub const REMOVAL_RECEIPTS: &str = "removals/done/";

/// Every writer's packs.
pub const PACKS: &str = "packs/";

/// Where a writer's packs go. Each writer has its own prefix and its own intent, so the
/// compactor on the server and a backfill elsewhere cannot write the same key.
pub fn packs_prefix(writer: &str) -> String {
    format!("{PACKS}{writer}/")
}

/// The submission name in a loose key: what follows `loose/`. A pack's index keeps it. None
/// for a key that names no object of its own, such as the folder marker some tools create.
pub fn submission_name(key: &str) -> Option<&str> {
    key.strip_prefix(LOOSE).filter(|name| !name.is_empty() && !name.ends_with('/'))
}

/// The id of a stored submission, from its name: `<month>/<time received>_<id>.json`. This is
/// the id the intake names in a removal request. A submission is matched by it wherever it
/// is stored, and never by its content: the same export sent by another contributor is
/// another submission.
pub fn submission_id(name: &str) -> String {
    submission_id_from_path(Path::new(name))
}

/// The record of the pack a writer is storing. At most one exists per writer.
pub fn pack_intent_key(writer: &str) -> String {
    format!("intents/{writer}/pack.json")
}

/// One object of a listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub key: String,
    pub size: u64,
    /// The provider's checksum of the content. Every object in the archive bucket is stored
    /// in one part and unencrypted, and for those it is the MD5 in hex: see [`md5_hex`].
    pub etag: String,
    /// When the provider stored the object.
    pub stored_at: Timestamp,
}

/// One bucket. Every request can be repeated: a `put` replaces, a `delete` of an absent key
/// succeeds. An `Err` means the request got no usable answer and may or may not have been
/// carried out.
pub trait Bucket: Sync {
    /// Every object whose key starts with `prefix`, or an error: never part of them.
    fn list(&self, prefix: &str) -> Result<Vec<Listed>>;
    /// The object's ETag as the provider holds it now. None when the key is absent.
    fn etag(&self, key: &str) -> Result<Option<String>>;
    /// The whole object. None when the key is absent.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    /// These bytes of the object, which must hold them all. None when the key is absent.
    fn get_range(&self, key: &str, range: Range<u64>) -> Result<Option<Vec<u8>>>;
    /// Stores `bytes` under `key` in one request, with a checksum the provider enforces.
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// Deletes an object only while it is still the one with this ETag. An absent one counts as
/// deleted: a run that finishes an earlier one meets what that one already deleted.
pub fn delete_unchanged(bucket: &dyn Bucket, key: &str, etag: &str) -> Result<()> {
    match bucket.etag(key)? {
        None => Ok(()),
        Some(stored) if stored == etag => bucket.delete(key),
        Some(_) => bail!("{key}: changed since it was listed, so it is not deleted"),
    }
}

/// The ETag a provider gives an object with these bytes when it is stored in one part.
pub fn md5_hex(bytes: &[u8]) -> String {
    Md5::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_hex_is_the_etag_a_provider_reports() {
        // RFC 1321's own examples.
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }
}
