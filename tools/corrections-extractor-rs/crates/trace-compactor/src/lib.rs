//! The compactor: turns the loose submissions of the archive bucket into packs, and removes
//! the submissions the intake asks it to.
//!
//! It keeps no state outside the bucket. What it is about to destroy it first records there,
//! so a run that stops anywhere is finished by the next one. How this fits the intake is in
//! the `railway-forms` repository: `OVH_MIGRATION_PLAN.md` and `docs/storage.md`.

pub mod backfill;
pub mod bucket;
pub mod packing;
pub mod removal;
pub mod s3;

#[cfg(test)]
mod memory_bucket;
#[cfg(test)]
mod test_support;

use anyhow::Result;
use bucket::Bucket;
use jiff::{SignedDuration, Timestamp};
use trace_pack::pack::PackOptions;

pub struct Config {
    /// Names this writer's packs and its pack intent: `live` for the compactor on the server.
    /// The names keep two writers' keys apart. They do not make two processes safe together:
    /// only one may run against a bucket at a time, whatever their names, because a removal
    /// rewrites packs of every writer.
    pub writer: String,
    /// How long a loose object waits before it is packed, counted from when the bucket
    /// stored it.
    pub settle: SignedDuration,
    /// The most loose JSON one pack is built from, in bytes. The finished frames and the pack
    /// are held in memory until it is uploaded: between a quarter and two thirds of this. A
    /// single larger object gets a pack of its own.
    pub pack_input_bytes: u64,
    /// The most loose JSON that is being built into frames at one moment, in bytes. Building
    /// takes about 22 times this in memory. A single larger object is built alone.
    pub build_input_bytes: u64,
    pub options: PackOptions,
}

/// What one run did.
#[derive(Debug, Default, PartialEq)]
pub struct RunReport {
    pub removals: removal::Removals,
    pub packed: packing::Packed,
}

/// One run at `now`. An `Err` means the run stopped and the next one continues from what it
/// left. A removal request that could not be worked on, or a loose object that could not be
/// packed, does not stop the run: it is in the report, and is left as it is.
pub fn run(bucket: &dyn Bucket, config: &Config, now: Timestamp) -> Result<RunReport> {
    // First what an earlier run left: until its pack is finished, loose objects that are
    // already in a pack are still listed.
    packing::finish_interrupted(bucket, config)?;

    // Removals before packing, so a submission that is to go is deleted from `loose/` and not
    // packed first. A removal step that cannot go on does not hold up the packing: a pack
    // whose index cannot be read says nothing against packing what is in `loose/`.
    let requested = removal::requested(bucket)?;
    let mut removals = removal::Removals::default();
    if let Err(error) = removal::answer(bucket, &requested, config.options.level, &mut removals) {
        removals.stopped = Some(format!("{error:#}"));
    }

    let packed = packing::pack_settled(bucket, config, now, &requested)?;
    Ok(RunReport { removals, packed })
}
