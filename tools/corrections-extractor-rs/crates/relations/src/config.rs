//! Where the pipeline reads its inputs and writes its files. The environment variables are the
//! ones the TypeScript pipeline and the Exporters dump use, so the binary is a drop-in.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    /// TRACE_DATA_DIR: a trace-data checkout (`submissions/<yyyy-mm>/<stamp>_<id>.json`).
    pub trace_data_dir: Option<PathBuf>,
    /// QUESTIEDB_DIR: a QuestieDB checkout (data/ and src/corrections/).
    pub questie_db_dir: Option<PathBuf>,
    /// RELATIONS_TRACE_MODULES_DIR: a dump's own forever*Traces.lua modules, used by the catalog in
    /// place of the trace modules imported into QuestieDB.
    pub trace_modules_dir: Option<PathBuf>,
    /// RELATIONS_DIR: the pipeline's working files (episodes, catalog, candidates, reports).
    pub relations_dir: PathBuf,
    /// RELATIONS_OUTPUT_DIR: the deliverables; `<relations dir>/output` by default.
    pub output_dir: PathBuf,
    /// RELATIONS_REQUIRE_FOREVER_VERSION=1: only sessions with a Forever interface version.
    pub require_forever_version: bool,
    /// RELATIONS_BUCKET_BYTES: where the episode store ends a bucket (see
    /// [`Config::bucket_bytes`]); small values make tests cross many bucket boundaries.
    pub bucket_bytes: Option<String>,
    /// When this run started (ISO 8601), written as `generatedAt` everywhere.
    pub generated_at: String,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from)
}

impl Config {
    pub fn from_env(generated_at: String) -> Config {
        let relations_dir = env_path("RELATIONS_DIR").unwrap_or_else(|| PathBuf::from(".relations"));
        Config {
            trace_data_dir: env_path("TRACE_DATA_DIR"),
            questie_db_dir: env_path("QUESTIEDB_DIR"),
            trace_modules_dir: env_path("RELATIONS_TRACE_MODULES_DIR"),
            output_dir: env_path("RELATIONS_OUTPUT_DIR").unwrap_or_else(|| relations_dir.join("output")),
            relations_dir,
            require_forever_version: std::env::var("RELATIONS_REQUIRE_FOREVER_VERSION").is_ok_and(|v| v == "1"),
            bucket_bytes: std::env::var("RELATIONS_BUCKET_BYTES").ok().filter(|value| !value.is_empty()),
            generated_at,
        }
    }

    /// Bytes of encoded episodes after which the store ends a bucket at the next character.
    pub fn bucket_bytes(&self) -> Result<usize> {
        match &self.bucket_bytes {
            None => Ok(crate::episode_store::BUCKET_BYTES),
            Some(value) => value
                .parse::<usize>()
                .ok()
                .filter(|&bytes| bytes > 0)
                .with_context(|| format!("RELATIONS_BUCKET_BYTES={value:?} is not a positive number of bytes")),
        }
    }

    pub fn trace_data_dir(&self) -> Result<&Path> {
        required(&self.trace_data_dir, "TRACE_DATA_DIR", "a trace-data checkout")
    }

    pub fn questie_db_dir(&self) -> Result<&Path> {
        required(&self.questie_db_dir, "QUESTIEDB_DIR", "a QuestieDB checkout")
    }

    pub fn episodes(&self) -> PathBuf {
        self.relations_dir.join("episodes.jsonl")
    }

    /// The episodes as the steps read them, one bucket of characters at a time (see
    /// [`crate::episode_store`]).
    pub fn episode_store(&self) -> PathBuf {
        self.relations_dir.join("episodes")
    }

    pub fn catalog(&self) -> PathBuf {
        self.relations_dir.join("catalog.json")
    }

    pub fn ground_truth(&self) -> PathBuf {
        self.relations_dir.join("groundtruth.json")
    }

    pub fn candidates_dir(&self) -> PathBuf {
        self.relations_dir.join("candidates")
    }

    pub fn candidates(&self, signal: &str) -> PathBuf {
        self.candidates_dir().join(format!("{signal}.json"))
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.relations_dir.join("reports")
    }

    pub fn report(&self, name: &str) -> PathBuf {
        self.reports_dir().join(format!("{name}.json"))
    }
}

fn required<'a>(value: &'a Option<PathBuf>, variable: &str, what: &str) -> Result<&'a Path> {
    match value {
        Some(path) => Ok(path),
        None => bail!("set {variable} to {what}"),
    }
}
