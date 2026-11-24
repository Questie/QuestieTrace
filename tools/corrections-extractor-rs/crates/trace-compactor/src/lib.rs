//! The compactor: turns the loose submissions of the archive bucket into packs.
//!
//! It keeps no state outside the bucket. What it is about to destroy it first records there,
//! so a run that stops anywhere is finished by the next one. How this fits the intake is in
//! the `railway-forms` repository: `OVH_MIGRATION_PLAN.md` and `docs/storage.md`.

pub mod bucket;
pub mod packing;
pub mod s3;

#[cfg(test)]
mod memory_bucket;
