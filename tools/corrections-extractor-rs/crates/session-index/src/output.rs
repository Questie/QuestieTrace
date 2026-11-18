//! Writes an [`Index`] for scripts to read:
//!
//! - `sessions.jsonl`: one line per session, with the copy kept and every superseded copy.
//! - `submissions.jsonl`: one line per submission file, with its [`SubmissionStatus`].
//! - `summary.json`: the totals, written last: it marks a complete index.
//!
//! [`SubmissionStatus`]: crate::index::SubmissionStatus

use crate::index::{Index, SessionCopy, Summary};
use serde::Serialize;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionLine<'a> {
    /// The session's capture-start clocks; null for a session without both.
    started_at: Option<f64>,
    started_at_precise: Option<f64>,
    kept: &'a SessionCopy,
    superseded: &'a [SessionCopy],
}

pub fn write_index(index: &Index, dir: &Path) -> io::Result<Summary> {
    std::fs::create_dir_all(dir)?;
    // A summary left by an earlier run must not vouch for files this run is replacing.
    match std::fs::remove_file(dir.join("summary.json")) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    write_lines(
        &dir.join("sessions.jsonl"),
        index.sessions.iter().map(|session| {
            let (started_at, started_at_precise) = session.kept.identity.map(|identity| identity.key.clocks()).unzip();
            SessionLine {
                started_at,
                started_at_precise,
                kept: &session.kept,
                superseded: &session.superseded,
            }
        }),
    )?;
    write_lines(&dir.join("submissions.jsonl"), index.submissions.iter())?;
    let summary = index.summary();
    let mut json = serde_json::to_string_pretty(&summary)?;
    json.push('\n');
    std::fs::write(dir.join("summary.json"), json)?;
    Ok(summary)
}

fn write_lines<T: Serialize>(path: &Path, lines: impl Iterator<Item = T>) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    for line in lines {
        serde_json::to_writer(&mut out, &line)?;
        out.write_all(b"\n")?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn a_write_that_fails_leaves_no_summary_of_an_earlier_run_behind() {
        let output = TempDir::new("output");
        std::fs::write(output.0.join("summary.json"), "{}").unwrap();
        // A directory where sessions.jsonl goes makes writing it fail.
        std::fs::create_dir(output.0.join("sessions.jsonl")).unwrap();

        let result = write_index(&Index::build(Vec::new()), &output.0);

        assert!(result.is_err());
        assert!(!output.0.join("summary.json").exists());
    }
}
