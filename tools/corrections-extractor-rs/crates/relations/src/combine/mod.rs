//! Combine: every signal's candidates -> one resolved set of relations, and the deliverables:
//! `foreverQuestRelationTraces.lua`, `relations-review.md` and `relations-meta.json`.
//!
//! `inputs` sets Wowhead aside as context only; `pipeline` scores and resolves the rest. This
//! module renders the outputs. Deliverables go to the output dir (next to the field extractor's
//! modules): foreverQuestRelationTraces.lua (every accepted relation; see [`lua`] for which apply),
//! relations-review.md and relations-meta.json. Intermediates stay in the relations dir: relations.json (every quest with
//! accepted or near-miss relations) and reports/combine-metrics.json.
//!
//! The acceptance threshold is `pipeline::MIN_SCORE`; the TS `--min-score` override is not
//! ported (the pipeline never passes it).

mod assumptions;
mod crossval;
mod edges;
mod features;
mod foldwise;
mod inputs;
mod logistic;
mod lua;
mod meta;
mod metrics;
mod pipeline;
mod resolve;
mod review;
mod views;

use crate::config::Config;
use crate::io;
use crate::model::{CandidateFile, GroundTruth, QuestCatalog, QuestEpisode, QuestId, RelationField, RelationSet};
use anyhow::{Context, Result};
use crossval::{ModelParams, ScoredEdge, DEFAULT_MODEL_PARAMS};
use edges::{Claim, Domain, EdgeKey, EdgeKind, QuestDomains, Reference, Truth};
use indexmap::IndexMap;
use lua::{forever_relations_lua, relation_records, LuaHeader, RecordSources};
use meta::{authored_status_counts, count_episodes, lua_quest_count, lua_relation_count, InputSummary, LuaCounts, RelationsMeta, LEAKAGE_CAVEAT};
use metrics::{format_comparison, format_metrics, held_out_metrics, TierMetrics, METRIC_CAVEATS};
use pipeline::{combine, Combined, CLIQUE_MIN_SCORE, MIN_SCORE, REVIEW_SCORE};
use resolve::{Decision, Outcome};
use review::{render_review, ClassicMiss, ReviewInput};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use views::{build_quest_views, QuestView, Status, ViewSources};

const LUA_FILE: &str = "foreverQuestRelationTraces.lua";
const REVIEW_FILE: &str = "relations-review.md";
const META_FILE: &str = "relations-meta.json";

pub struct CombineInput<'a> {
    /// Every candidates file (signals and sources), in any order: combine orders them by signal
    /// name itself (see `inputs::prepare_inputs`).
    pub candidate_files: Vec<CandidateFile>,
    pub catalog: &'a QuestCatalog,
    pub ground_truth: &'a GroundTruth,
    /// For counts only.
    pub episodes: &'a [QuestEpisode],
}

/// Every file combine writes, rendered.
pub struct CombineOutput {
    relations: RelationsFile,
    metrics: MetricsFile,
    lua: String,
    review: String,
    meta: RelationsMeta,
    /// What the run printed in TS: counts, the files and the metrics table.
    summary: String,
}

impl CombineOutput {
    /// Writes the intermediates, then the deliverables, then prints the summary.
    pub fn write(&self, config: &Config) -> Result<()> {
        io::write_json(&config.report("combine-metrics"), &self.metrics)?;
        io::write_json(&relations_path(config), &self.relations)?;
        // relations-meta.json marks a complete run: remove it until the module and the review are written.
        let meta_path = config.output_dir.join(META_FILE);
        match std::fs::remove_file(&meta_path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error).with_context(|| format!("removing {}", meta_path.display())),
            _ => {}
        }
        io::write_text(&config.output_dir.join(LUA_FILE), &self.lua)?;
        io::write_text(&config.output_dir.join(REVIEW_FILE), &self.review)?;
        io::write_text(&meta_path, &serde_json::to_string_pretty(&self.meta)?)?;
        println!("{}", self.summary);
        Ok(())
    }
}

fn relations_path(config: &Config) -> PathBuf {
    config.relations_dir.join("relations.json")
}

pub fn run(config: &Config, input: CombineInput) -> Result<CombineOutput> {
    let min_score = MIN_SCORE;
    let inputs = inputs::prepare_inputs(&input.candidate_files, input.catalog, input.ground_truth)?;
    // The same pipeline twice: once as shipped, once without signal-score columns (see METRIC_CAVEATS).
    let shipped = combine(&inputs, min_score, &DEFAULT_MODEL_PARAMS);
    // Single-signal comparisons use the trace files only: Wowhead's agreement with the ground truth is circular.
    let metrics_of = |run: &Combined| held_out_metrics(&run.accepted.relations, &run.at_review_score.relations, &inputs.files, input.ground_truth);
    let metrics = metrics_of(&shipped);
    let label_free_params = ModelParams {
        score_features: false,
        ..DEFAULT_MODEL_PARAMS
    };
    let label_free = metrics_of(&combine(&inputs, min_score, &label_free_params));
    let and_sources: Vec<String> = shipped.and_sources.iter().map(ToString::to_string).collect();

    let views = build_quest_views(
        &ViewSources {
            scored: &shipped.scored,
            resolution: &shipped.accepted,
            context: &inputs.context,
            truth: &inputs.truth,
            domains: &inputs.domains,
            catalog: input.catalog,
        },
        REVIEW_SCORE,
    );
    let input_summary: Vec<InputSummary> = inputs.files.iter().map(|file| InputSummary::of(file)).collect();
    let context_only: Vec<String> = inputs.context_files.iter().map(|file| file.signal.clone()).collect();
    let relations = RelationsFile {
        generated_at: config.generated_at.clone(),
        params: RunParams {
            min_score,
            review_score: REVIEW_SCORE,
            clique_min_score: CLIQUE_MIN_SCORE,
            model: DEFAULT_MODEL_PARAMS,
        },
        inputs: input_summary.clone(),
        context_only: context_only.clone(),
        and_sources: and_sources.clone(),
        models: shipped.fitted.iter().map(ModelJson::of).collect(),
        quests: views
            .values()
            .filter(|view| !view.edges.is_empty())
            .map(|view| (view.quest_id, QuestJson::of(view)))
            .collect(),
    };

    let records = relation_records(&RecordSources {
        relations: &shipped.accepted.relations,
        truth: &inputs.truth,
        authored_fields: &input.ground_truth.authored_fields,
        dynamic_fields: &input.ground_truth.dynamic_fields,
        views: &views,
    });
    let candidate_files: Vec<&str> = inputs.files.iter().map(|file| file.signal.as_str()).collect();
    let lua = forever_relations_lua(
        &records,
        &LuaHeader {
            candidate_files: &candidate_files,
            session_count: inputs.files.iter().map(|file| file.input_count).max().unwrap_or(0),
            min_score,
            generated_at: &config.generated_at,
            review_file: REVIEW_FILE,
        },
        &views,
        input.catalog,
    );

    let metrics_markdown = format_metrics(&metrics, min_score, REVIEW_SCORE);
    let context_files: Vec<(&str, usize)> = inputs.context_files.iter().map(|file| (file.signal.as_str(), file.candidates.len())).collect();
    let misses = classic_misses(&shipped.scored, &shipped.accepted.decisions, &inputs.truth, &inputs.domains);
    let review = render_review(&ReviewInput {
        views: &views,
        catalog: input.catalog,
        min_score,
        review_score: REVIEW_SCORE,
        candidate_files: &input_summary,
        context_files: &context_files,
        and_sources: &and_sources,
        metrics_markdown: &metrics_markdown,
        label_free_markdown: &format_comparison(&metrics, &label_free, min_score),
        caveats: &METRIC_CAVEATS,
        fitted: &shipped.fitted,
        lua_file: LUA_FILE,
        lua_quests: lua_quest_count(&records),
        classic_misses: &misses,
        generated_at: &config.generated_at,
    });

    let (episode_count, character_count) = count_episodes(input.episodes);
    let meta = RelationsMeta {
        generated_at: config.generated_at.clone(),
        min_score,
        episode_count,
        character_count,
        lua: LuaCounts {
            quests: lua_quest_count(&records),
            relations: lua_relation_count(&records),
        },
        authored: authored_status_counts(views.values()),
        inputs: input_summary,
        context_only,
        caveat: LEAKAGE_CAVEAT,
    };
    let summary = summary(config, &views, &shipped.accepted.relations, &meta, &metrics_markdown, inputs.files.len());
    let metrics = MetricsFile {
        generated_at: config.generated_at.clone(),
        min_score,
        review_score: REVIEW_SCORE,
        caveats: METRIC_CAVEATS,
        and_sources,
        metrics,
        label_free,
    };
    Ok(CombineOutput {
        relations,
        metrics,
        lua,
        review,
        meta,
        summary,
    })
}

/// Classic edges the model leaves out although at least two signals claim them at 0.8 or more,
/// and the inherited data lacks them. The model's Classic weights come from that inherited data,
/// so a chain Forever re-wired is distrusted precisely because the reference is stale; a person
/// decides. Most probable first.
fn classic_misses<'s>(scored: &'s [ScoredEdge<'s>], decisions: &FxHashMap<EdgeKey, Decision>, truth: &Truth, domains: &QuestDomains) -> Vec<ClassicMiss<'s>> {
    let inherited_fields = |kind: EdgeKind| -> &'static [RelationField] {
        match kind {
            EdgeKind::Prerequisite => &[RelationField::PreQuestSingle, RelationField::PreQuestGroup],
            EdgeKind::Next => &[RelationField::NextQuestInChain],
            EdgeKind::Breadcrumb => &[RelationField::BreadcrumbForQuestId],
            EdgeKind::Exclusive => &[RelationField::ExclusiveTo],
            _ => &[],
        }
    };
    let mut misses: Vec<&ScoredEdge> = scored
        .iter()
        .filter(|edge| {
            domains.domain_of(edge.key()) == Domain::Classic
                && edge.label == Some(false)
                && decisions.get(&edge.key()).is_some_and(|decision| decision.outcome == Outcome::BelowThreshold)
        })
        .filter(|edge| {
            edge.edge
                .claims
                .iter()
                .filter(|claim| claim.score >= 0.8)
                .map(Claim::signal)
                .collect::<FxHashSet<_>>()
                .len()
                >= 2
        })
        .collect();
    misses.sort_by(|a, b| b.probability.total_cmp(&a.probability));
    misses
        .into_iter()
        .map(|edge| {
            let reference = truth.get(&edge.key().quest);
            let inherited = inherited_fields(edge.key().kind)
                .iter()
                .flat_map(|field| reference.and_then(|set| set.get(field)).into_iter().flatten().map(|&target| (*field, target)))
                .collect();
            ClassicMiss { edge, inherited }
        })
        .collect()
}

fn summary(
    config: &Config,
    views: &IndexMap<QuestId, QuestView>,
    relations: &BTreeMap<QuestId, RelationSet>,
    meta: &RelationsMeta,
    metrics_markdown: &str,
    file_count: usize,
) -> String {
    let written = |view: &QuestView| view.edges.iter().filter(|edge| edge.is_written()).count();
    let uncovered: Vec<&QuestView> = views
        .values()
        .filter(|view| view.domain == Domain::Forever && view.reference == Reference::None && written(view) > 0)
        .collect();
    let classic = views
        .values()
        .filter(|view| view.domain == Domain::Classic && view.edges.iter().any(|edge| edge.is_written() && edge.status != Some(Status::Agree)))
        .count();
    let output = |name: &str| config.output_dir.join(name).display().to_string();
    format!(
        "combine: {file_count} candidate files, {} quests with accepted relations\n  \
         Forever-new without authored relations: {} quests, {} relations\n  \
         Forever-new authored: agree {}, new {}, conflict {}\n  \
         Classic quests disagreeing with inherited data: {classic}\n  \
         -> {} ({} quests, {} relations)\n  -> {}\n  -> {}\n  -> {}\n\nHeld-out metrics:\n{metrics_markdown}",
        relations.len(),
        uncovered.len(),
        uncovered.iter().map(|view| written(view)).sum::<usize>(),
        meta.authored.agree,
        meta.authored.new,
        meta.authored.conflict,
        output(LUA_FILE),
        meta.lua.quests,
        meta.lua.relations,
        output(REVIEW_FILE),
        output(META_FILE),
        relations_path(config).display(),
    )
}

/// `value` rounded to `decimals` digits, like the TS pipeline's `Number(value.toFixed(n))`.
fn rounded(value: f64, decimals: usize) -> f64 {
    if !value.is_finite() {
        return value;
    }
    crate::text::to_fixed(value, decimals).parse().expect("a formatted number parses")
}

// ---------------------------------------------------------------------------
// relations.json and reports/combine-metrics.json
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationsFile {
    generated_at: String,
    params: RunParams,
    inputs: Vec<InputSummary>,
    context_only: Vec<String>,
    and_sources: Vec<String>,
    models: Vec<ModelJson>,
    /// Every quest with accepted or near-miss relations.
    quests: BTreeMap<QuestId, QuestJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunParams {
    min_score: f64,
    review_score: f64,
    clique_min_score: f64,
    #[serde(flatten)]
    model: ModelParams,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelJson {
    kind: EdgeKind,
    judged: usize,
    positives: usize,
    calibrated: bool,
    /// null when the kind had too little ground truth to calibrate.
    intercept: Option<f64>,
    weights: IndexMap<String, f64>,
}

impl ModelJson {
    fn of(fitted: &crossval::FittedKind) -> ModelJson {
        let calibrated = fitted.model.intercept.is_finite();
        ModelJson {
            kind: fitted.kind,
            judged: fitted.judged,
            positives: fitted.positives,
            calibrated,
            intercept: calibrated.then_some(fitted.model.intercept),
            weights: fitted
                .columns
                .iter()
                .zip(&fitted.model.weights)
                .map(|(column, &weight)| (column.name.clone(), rounded(weight, 4)))
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct ClaimJson {
    source: String,
    score: f64,
    support: u32,
    contradict: u32,
}

impl ClaimJson {
    fn of(claim: &Claim) -> ClaimJson {
        ClaimJson {
            source: claim.source.to_string(),
            score: claim.score,
            support: claim.support,
            contradict: claim.contradict,
        }
    }

    fn all(claims: &[Claim]) -> Vec<ClaimJson> {
        claims.iter().map(ClaimJson::of).collect()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EdgeJson {
    field: RelationField,
    target: QuestId,
    probability: f64,
    held_out: bool,
    outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    because: Option<EdgeKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<Status>,
    signals: Vec<ClaimJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<Vec<ClaimJson>>,
    /// Per signal, and "base" for the intercept; null for an uncalibrated kind's intercept.
    log_odds: IndexMap<String, f64>,
}

#[derive(Serialize)]
struct FieldTarget {
    field: RelationField,
    target: QuestId,
}

#[derive(Serialize)]
struct HintJson {
    field: RelationField,
    target: QuestId,
    context: Vec<ClaimJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuestJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    domain: Domain,
    reference: Reference,
    relations: RelationSet,
    edges: Vec<EdgeJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_only: Option<Vec<FieldTarget>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hints: Option<Vec<HintJson>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<Vec<String>>,
}

impl QuestJson {
    fn of(view: &QuestView) -> QuestJson {
        QuestJson {
            name: view.name.map(str::to_owned),
            domain: view.domain,
            reference: view.reference,
            relations: view.relations.clone(),
            edges: view
                .edges
                .iter()
                .map(|edge| EdgeJson {
                    field: edge.field,
                    target: edge.target,
                    probability: rounded(edge.probability, 4),
                    held_out: edge.held_out,
                    outcome: edge.outcome,
                    because: edge.because,
                    status: edge.status,
                    signals: ClaimJson::all(edge.claims),
                    context: (!edge.context.is_empty()).then(|| ClaimJson::all(edge.context)),
                    log_odds: edge
                        .contributions
                        .iter()
                        .map(|term| (term.signal.to_owned(), rounded(term.log_odds, 3)))
                        .collect(),
                })
                .collect(),
            reference_only: non_empty(view.reference_only.iter().map(|&(field, target)| FieldTarget { field, target }).collect()),
            hints: non_empty(
                view.hints
                    .iter()
                    .map(|hint| HintJson {
                        field: hint.field,
                        target: hint.target,
                        context: ClaimJson::all(hint.claims),
                    })
                    .collect(),
            ),
            notes: view.note.map(|note| vec![note.to_owned()]),
        }
    }
}

fn non_empty<T>(list: Vec<T>) -> Option<Vec<T>> {
    (!list.is_empty()).then_some(list)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricsFile {
    generated_at: String,
    min_score: f64,
    review_score: f64,
    caveats: [&'static str; 5],
    and_sources: Vec<String>,
    metrics: Vec<TierMetrics>,
    label_free: Vec<TierMetrics>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_reads_back_what_to_fixed_prints() {
        assert_eq!(rounded(0.123456, 4), 0.1235);
        assert_eq!(rounded(0.8125, 3), 0.813, "an exact tie rounds up, as Number(x.toFixed(3)) does");
    }

    #[test]
    fn families_trim_what_js_trims() {
        let named = |name: &str| {
            crate::test_support::catalog([crate::model::CatalogQuest {
                name: Some(name.into()),
                ..crate::test_support::catalog_quest(1)
            }])
        };
        assert_eq!(inputs::family_of(&named("\u{feff} The Test\u{3000}"), 1), "name:the test");
        assert_eq!(inputs::family_of(&named("The Test\u{85}"), 1), "name:the test\u{85}");
        assert_eq!(inputs::family_of(&named(""), 1), "id:1");
    }
}
