//! Read-only access to scraper-questie's raw page caches (data/raw/<version>.db). The scraper may
//! be running while we read, so connections are opened read-only and only ever SELECT; never
//! write to these files.

use crate::model::QuestId;
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub struct StoredQuestPage {
    pub quest_id: QuestId,
    pub body: String,
}

/// English quest pages for registered, non-excluded quests, in id order. Excluded entries (junk,
/// placeholder, unused) can still have a cached page from before they were excluded.
pub fn read_quest_pages(path: &Path) -> Result<Vec<StoredQuestPage>> {
    if !path.exists() {
        bail!(
            "{} does not exist. Point SCRAPER_DIR at a scraper-questie checkout with raw caches.",
            path.display()
        );
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("opening {} read-only", path.display()))?;
    let mut statement = db.prepare(
        "SELECT p.entity_id, p.body
           FROM pages p
           JOIN entries e ON e.entity_type = p.entity_type AND e.entity_id = p.entity_id
          WHERE p.entity_type = 'quest' AND p.locale = 'enUS' AND e.excluded = 0
          ORDER BY p.entity_id",
    )?;
    let pages = statement.query_map([], |row| {
        Ok(StoredQuestPage {
            quest_id: row.get(0)?,
            // Lossy like the TypeScript reader's UTF-8 decoding: a stray byte must not drop a page.
            body: String::from_utf8_lossy(row.get_ref(1)?.as_bytes()?).into_owned(),
        })
    })?;
    pages
        .collect::<rusqlite::Result<_>>()
        .with_context(|| format!("reading quest pages from {}", path.display()))
}
