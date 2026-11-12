//! Quest relationship inference for WoW Forever from QuestieTrace recordings: which quests unlock,
//! hide or lead to which. A Rust version of the Exporters pipeline (`src/relations` there): same
//! inputs, same steps, same files.
//!
//! ```text
//! trace-data --ingest--> episodes --+
//! QuestieDB --catalog--> catalog ---+--signals--> candidates --+
//! scraper --wowhead--> context candidates ---------------------+--combine--> Lua, review, meta
//! ```
//!
//! Every step is a function from typed inputs to a typed output that knows how to write itself
//! (see [`model`] for the contracts). `pipeline` runs them in order in one process; each step is
//! also a command of its own, reading its inputs from the files the previous step wrote.

// The steps and contracts `main.rs` drives are public; the helpers they share are crate-internal.
pub mod catalog;
pub mod combine;
pub mod config;
mod eligibility;
pub mod ingest;
pub mod io;
pub mod model;
mod race_masks;
pub mod score;
pub mod signals;
pub mod sources;
mod text;
mod timeline;

#[cfg(test)]
mod test_support;

/// `unix_millis` as an ISO 8601 UTC timestamp with milliseconds, like JS `Date.toISOString()`.
fn iso_timestamp(unix_millis: i64) -> String {
    let days = unix_millis.div_euclid(86_400_000);
    let ms_of_day = unix_millis.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000
    )
}

/// The current time as `iso_timestamp`.
pub fn now_iso() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    iso_timestamp(millis)
}
