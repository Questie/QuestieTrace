//! The archive bucket as the compactor uses it, and where things are in it.

use anyhow::Result;
use jiff::Timestamp;
use md5::{Digest, Md5};

/// Accepted submissions the intake copied here, one JSON object each, waiting to be packed.
/// What follows the prefix is the submission's name, which a pack's index keeps.
pub const LOOSE: &str = "loose/";

/// One object per submission the intake wants removed, keyed by submission id.
pub const REMOVAL_REQUESTS: &str = "removals/requested/";

/// Where a writer's packs go. Each writer has its own prefix and its own intent, so the
/// compactor on the server and a backfill elsewhere cannot write the same key.
pub fn packs_prefix(writer: &str) -> String {
    format!("packs/{writer}/")
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

/// What the provider says about one object.
#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
    pub size: u64,
    pub etag: String,
}

/// One bucket. Every request can be repeated: a `put` replaces, a `delete` of an absent key
/// succeeds. An `Err` means the request got no usable answer and may or may not have been
/// carried out.
pub trait Bucket: Sync {
    /// Every object whose key starts with `prefix`, or an error: never part of them.
    fn list(&self, prefix: &str) -> Result<Vec<Listed>>;
    /// None when the key is absent.
    fn stat(&self, key: &str) -> Result<Option<Stat>>;
    /// The whole object. None when the key is absent.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    /// Stores `bytes` under `key` in one request, with a checksum the provider enforces.
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
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
