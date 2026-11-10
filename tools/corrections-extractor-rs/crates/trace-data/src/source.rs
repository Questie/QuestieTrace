//! The submissions of a local trace-data checkout: `<dir>/submissions/<yyyy-mm>/*.json`.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One submission file.
#[derive(Clone, Debug)]
pub struct SubmissionFile {
    /// `<yyyy-mm>/<file>.json`, the key the TypeScript store and meta.json use.
    pub key: String,
    pub path: PathBuf,
}

/// Every submission of the checkout, newest first (keys start with the receive time).
pub fn list_submissions(trace_data_dir: &Path) -> std::io::Result<Vec<SubmissionFile>> {
    let dir = trace_data_dir.join("submissions");
    let mut files = Vec::new();
    for month in std::fs::read_dir(&dir)? {
        let month = month?;
        if !month.file_type()?.is_dir() {
            continue;
        }
        let month_name = month.file_name().to_string_lossy().into_owned();
        for file in std::fs::read_dir(month.path())? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") {
                files.push(SubmissionFile { key: format!("{month_name}/{name}"), path: file.path() });
            }
        }
    }
    files.sort_by(|a, b| b.key.cmp(&a.key));
    Ok(files)
}

/// Whether `key` is in a `percent` sample: the first 4 bytes of SHA-256(key) as a fraction
/// below it. The same submissions every run, and a sample only grows as `percent` rises.
pub fn in_sample(key: &str, percent: f64) -> bool {
    let digest = Sha256::digest(key.as_bytes());
    let head = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]);
    (head as f64) / 4_294_967_296.0 < percent / 100.0
}
