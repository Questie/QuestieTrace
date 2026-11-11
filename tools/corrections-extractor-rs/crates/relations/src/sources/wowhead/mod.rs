//! Wowhead relations from the scraper's databases, one candidates file per page surface
//! (`wowhead-series`, `wowhead-storyline`, ...). Context only: see combine.
//!
//! Reads English quest pages from scraper-questie's Forever raw cache (data/raw/forever.db), and
//! the Era cache (data/raw/classic.db) only to tell which claims Forever pages inherited from Era.
//! Both are opened read-only. Steps: `parse` (HTML -> facts), `interpret` (facts -> claims),
//! `scores` (provenance, base scores), `aggregate` (claims -> candidates).
//!
//! What the Forever cache holds (2026-10 snapshot): pages fetched from Wowhead's Forever
//! environment only render Series and Storyline panels. Requires / Requires Any / Requires In
//! Progress / Unlocks / Disables appear only on Era pages the scraper inherited and never
//! refetched, so those surfaces are Era data and are only "covered" on those pages.

mod aggregate;
mod interpret;
mod pages;
mod parse;
mod scores;

use crate::config::Config;
use crate::io;
use crate::model::{CandidateFile, QuestId, RelationField};
use aggregate::{aggregate_claims, PageClaim, SeriesPositions};
use anyhow::{bail, Result};
use interpret::{claims_from_page, Surface};
use parse::{is_placeholder_name, merge_quest_records, parse_quest_page, PageEnvironment, QuestLookup, QuestPageFacts};
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use scores::{provenance_of, ClaimKey, Provenance};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

pub struct WowheadOutput {
    /// One per surface, in [`Surface::ALL`] order.
    pub files: Vec<CandidateFile>,
    /// reports/wowhead-inventory.json
    pub inventory: WowheadInventory,
}

impl WowheadOutput {
    pub fn write(&self, config: &Config) -> Result<()> {
        for file in &self.files {
            let path = config.candidates(&file.signal);
            io::write_json(&path, file)?;
            println!(
                "{:<32} {:>6} candidates, {:>5} covered -> {}",
                file.signal,
                file.candidates.len(),
                file.covered_quest_ids.len(),
                path.display()
            );
        }
        io::write_json(&config.report("wowhead-inventory"), &self.inventory)?;
        let pages = serde_json::to_string(&self.inventory.pages)?;
        let dropped = self.inventory.dropped_claims_for_unknown_or_placeholder_quests;
        println!("pages: {pages}; {dropped} claims dropped (quest not in Forever or a placeholder)");
        Ok(())
    }
}

/// Counts per page environment, surface, field or provenance.
type Counts<K> = BTreeMap<K, usize>;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WowheadInventory {
    pub generated_at: String,
    pub forever_db: String,
    pub era_db: String,
    pub pages: Counts<PageEnvironment>,
    pub dropped_claims_for_unknown_or_placeholder_quests: usize,
    /// Series rows whose markup did not parse; they break adjacency, so check this stays empty.
    pub unparsed_series_rows: Counts<PageEnvironment>,
    /// `<environment>:<envChange status, or none>`
    pub env_change: Counts<String>,
    pub pages_with_surface: BTreeMap<Surface, Counts<PageEnvironment>>,
    pub candidates: BTreeMap<Surface, SurfaceSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceSummary {
    pub covered: usize,
    pub candidates: usize,
    pub by_field: Counts<RelationField>,
    pub by_provenance: Counts<Provenance>,
}

pub fn run(config: &Config) -> Result<WowheadOutput> {
    let scraper_dir = std::path::absolute(config.scraper_dir()?)?;
    let forever_path = scraper_dir.join("data/raw/forever.db");
    let era_path = scraper_dir.join("data/raw/classic.db");

    let pages = read_pages(&forever_path)?;
    if pages.is_empty() {
        bail!("{} has no English quest pages.", forever_path.display());
    }
    let quests = merge_quest_records(&pages);
    let era_claim_keys = read_era_claim_keys(&era_path, &quests)?;

    // Inherited Era pages link Era-only quests (e.g. Season of Discovery ids) that Forever does not
    // have; every registered Forever quest has a page, so a page is the existence check.
    // Unshipped placeholders (<NYI>, <TXT>) have pages but can never be taken.
    let forever_quests: FxHashSet<QuestId> = pages
        .iter()
        .filter(|page| !is_placeholder_name(quests.get(&page.quest_id).and_then(|quest| quest.name.as_deref())))
        .map(|page| page.quest_id)
        .collect();
    let mut dropped = 0;
    let mut claims = Vec::new();
    let mut series_positions = SeriesPositions::new();
    for page in &pages {
        for claim in claims_from_page(page, &quests) {
            if !forever_quests.contains(&claim.quest_id) || !forever_quests.contains(&claim.target) {
                dropped += 1;
                continue;
            }
            let provenance = provenance_of(page.environment, &ClaimKey::of(&claim), &era_claim_keys);
            claims.push(PageClaim {
                claim,
                provenance,
                environment: page.environment,
                env_change: page.env_change.clone(),
            });
        }
        if !page.series.is_empty() {
            series_positions.insert(page.quest_id, row_positions(page));
        }
    }
    let mut candidates = aggregate_claims(claims, &series_positions);

    let page_counts = count_by(pages.iter().map(|page| page.environment));
    let mut files = Vec::new();
    let mut summary = BTreeMap::new();
    for surface in Surface::ALL {
        let covered: Vec<QuestId> = pages
            .iter()
            .filter(|page| forever_quests.contains(&page.quest_id) && covers_surface(page, surface))
            .map(|page| page.quest_id)
            .collect();
        let list = candidates.remove(&surface).unwrap_or_default();
        summary.insert(
            surface,
            SurfaceSummary {
                covered: covered.len(),
                candidates: list.len(),
                by_field: count_by(list.iter().map(|c| c.candidate.field)),
                by_provenance: count_by(list.iter().map(|c| c.provenance)),
            },
        );
        files.push(CandidateFile {
            signal: format!("wowhead-{}", surface.name()),
            generated_at: config.generated_at.clone(),
            input_count: pages.len(),
            params: json!({
                "foreverDb": forever_path.display().to_string(),
                "eraDb": era_path.display().to_string(),
                "pages": page_counts,
                "coverage": coverage(surface),
                "support": "Wowhead pages showing the edge (renderings of the same data, not independent observations)",
                "contradict": "Series only: pages whose Series shows both quests, but not directly one after the other",
                // Worded as the TypeScript pipeline writes it; the table is scores.rs here.
                "note": "note = surface, field, shape and provenance of the best claim (see sources/wowhead/scores.ts)",
            }),
            covered_quest_ids: covered,
            candidates: list.into_iter().map(|c| c.candidate).collect(),
        });
    }

    let inventory = WowheadInventory {
        generated_at: config.generated_at.clone(),
        forever_db: forever_path.display().to_string(),
        era_db: era_path.display().to_string(),
        pages: page_counts,
        dropped_claims_for_unknown_or_placeholder_quests: dropped,
        unparsed_series_rows: count_by(
            pages
                .iter()
                .flat_map(|page| page.series.iter().filter(|row| row.unparsed).map(|_| page.environment)),
        ),
        env_change: count_by(
            pages
                .iter()
                .map(|page| format!("{}:{}", page.environment.name(), page.env_change.as_deref().unwrap_or("none"))),
        ),
        pages_with_surface: Surface::ALL
            .into_iter()
            .map(|surface| {
                (
                    surface,
                    count_by(pages.iter().filter(|page| has_surface(page, surface)).map(|page| page.environment)),
                )
            })
            .collect(),
        candidates: summary,
    };
    Ok(WowheadOutput { files, inventory })
}

fn coverage(surface: Surface) -> &'static str {
    match surface {
        Surface::Series => "every parsed page (Forever and inherited Era pages both render Series)",
        Surface::Storyline => "pages fetched from Wowhead Forever (Era pages have no Storyline panel)",
        Surface::Requires | Surface::RequiresAny | Surface::RequiresInProgress => "inherited Era pages only (Forever pages never render this panel)",
        Surface::Unlocks => "inherited Era pages only; candidates sit on the unlocked quests, so recall is not meaningful",
        Surface::Disables => "inherited Era pages only; candidates sit on the disabled quests, so recall is not meaningful",
    }
}

fn covers_surface(page: &QuestPageFacts, surface: Surface) -> bool {
    match surface {
        Surface::Series => true,
        Surface::Storyline => page.environment == PageEnvironment::Forever,
        _ => page.environment != PageEnvironment::Forever,
    }
}

fn has_surface(page: &QuestPageFacts, surface: Surface) -> bool {
    match surface {
        Surface::Series => !page.series.is_empty(),
        Surface::Storyline => page.storyline.is_some(),
        _ => surface.panel().is_some_and(|panel| page.panels.contains_key(&panel)),
    }
}

/// Every quest page of the cache at `path`, parsed on all cores, in id order.
fn read_pages(path: &Path) -> Result<Vec<QuestPageFacts>> {
    let stored = pages::read_quest_pages(path)?;
    Ok(stored.par_iter().map(|page| parse_quest_page(page.quest_id, &page.body)).collect())
}

/// Every claim the Era cache's pages produce, read with the Forever quest facts.
fn read_era_claim_keys(path: &Path, quests: &QuestLookup) -> Result<FxHashSet<ClaimKey>> {
    let stored = pages::read_quest_pages(path)?;
    let keys: FxHashSet<ClaimKey> = stored
        .par_iter()
        .flat_map_iter(|page| {
            claims_from_page(&parse_quest_page(page.quest_id, &page.body), quests)
                .into_iter()
                .map(|claim| ClaimKey::of(&claim))
        })
        .collect();
    if keys.is_empty() {
        bail!("{} produced no claims; is it the Era raw cache?", path.display());
    }
    Ok(keys)
}

/// Quest id -> Series row position on this page; a quest listed twice keeps its last row.
fn row_positions(page: &QuestPageFacts) -> FxHashMap<QuestId, usize> {
    let mut positions = FxHashMap::default();
    for (position, row) in page.series.iter().enumerate() {
        for member in &row.members {
            positions.insert(member.quest_id, position);
        }
    }
    positions
}

fn count_by<K: Ord>(keys: impl IntoIterator<Item = K>) -> Counts<K> {
    let mut counts = Counts::new();
    for key in keys {
        *counts.entry(key).or_default() += 1;
    }
    counts
}
