//! relations-review.md: what a person reads before trusting the Lua output. Ordered by value:
//! Forever-new quests nobody has authored come first, most confident first; then authored quests
//! where the evidence adds or disagrees; then Classic disagreements; then the numbers. Wowhead
//! claims appear only as labelled context lines (see [`super::inputs`] for why).

use super::crossval::{FittedKind, ScoredEdge};
use super::edges::{Claim, Domain, EdgeKey, Reference};
use super::fixed;
use super::meta::InputSummary;
use super::resolve::Outcome;
use super::views::{EdgeView, HintView, QuestView, Status};
use crate::model::{QuestCatalog, QuestId, RelationField};
use indexmap::IndexMap;
use rustc_hash::FxHashSet;
use std::cmp::Ordering;

/// A Classic edge left out despite broad signal agreement, with the inherited values it
/// disagrees with.
pub struct ClassicMiss<'s> {
    pub edge: &'s ScoredEdge<'s>,
    pub inherited: Vec<(RelationField, QuestId)>,
}

pub struct ReviewInput<'r, 's> {
    pub views: &'r IndexMap<QuestId, QuestView<'s>>,
    pub catalog: &'r QuestCatalog,
    pub min_score: f64,
    pub review_score: f64,
    pub candidate_files: &'r [InputSummary],
    /// Context-only files (Wowhead): shown, never scored. (signal, candidates)
    pub context_files: &'r [(&'r str, usize)],
    pub and_sources: &'r [String],
    pub metrics_markdown: &'r str,
    /// Full model vs the model without signal-score columns.
    pub label_free_markdown: &'r str,
    pub caveats: &'r [&'r str],
    pub fitted: &'r [FittedKind<'s>],
    /// The Lua module's file name; it ships next to this report.
    pub lua_file: &'r str,
    pub lua_quests: usize,
    pub classic_misses: &'r [ClassicMiss<'s>],
    pub generated_at: &'r str,
}

fn quest_label(quest_id: QuestId, catalog: &QuestCatalog) -> String {
    match catalog.quest(quest_id).and_then(|quest| quest.name.as_deref()).filter(|name| !name.is_empty()) {
        Some(name) => format!("{quest_id} \"{name}\""),
        None => format!("{quest_id} (not in catalog)"),
    }
}

fn giver_of(quest_id: QuestId, catalog: &QuestCatalog) -> String {
    let Some(quest) = catalog.quest(quest_id) else {
        return "giver unknown".to_owned();
    };
    let starters = &quest.starters;
    let all: Vec<String> = starters
        .npcs
        .iter()
        .map(|id| format!("npc {id}"))
        .chain(starters.objects.iter().map(|id| format!("object {id}")))
        .chain(starters.items.iter().map(|id| format!("item {id}")))
        .collect();
    match all.len() {
        0 => "giver unknown".to_owned(),
        1..=3 => all.join(", "),
        n => format!("{} +{}", all[..3].join(", "), n - 3),
    }
}

fn describe_key(key: EdgeKey) -> String {
    format!("{} {} -> {}", key.kind.name(), key.quest, key.target)
}

/// At most `length` UTF-16 code units (the TS pipeline's string length), ending in "…" when cut.
/// Never splits a character: where TS would cut a surrogate pair in half (and write U+FFFD), the
/// whole character goes. Evidence texts are ASCII in practice.
fn clip(text: &str, length: usize) -> String {
    if text.encode_utf16().count() <= length {
        return text.to_owned();
    }
    let mut units = 0;
    let kept: String = text
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units < length
        })
        .collect();
    format!("{kept}…")
}

fn status_name(status: Option<Status>) -> &'static str {
    status.map_or("", Status::name)
}

fn outcome_text(edge: &EdgeView) -> String {
    match edge.outcome {
        Outcome::Accepted => status_name(edge.status).to_owned(),
        Outcome::Clique => format!("{} · completes an accepted exclusive group", status_name(edge.status)),
        Outcome::Derived => {
            let because = edge.because.map(describe_key).unwrap_or_default();
            format!("{} · implied by {because}", status_name(edge.status))
        }
        Outcome::BelowThreshold => "near miss".to_owned(),
        outcome => match edge.because {
            Some(because) => format!("dropped: {} ({})", outcome.name(), describe_key(because)),
            None => format!("dropped: {}", outcome.name()),
        },
    }
}

/// "wowhead-requires-any" -> "Wowhead Requires Any".
fn context_label(claim: &Claim) -> String {
    let words: Vec<String> = claim
        .signal()
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
        })
        .collect();
    format!("{} (untrusted, context only)", words.join(" "))
}

/// The first evidence text, when there is one.
fn example<'c>(claim: &Claim<'c>) -> Option<&'c str> {
    claim.evidence.first().map(|evidence| evidence.text.as_str()).filter(|text| !text.is_empty())
}

fn context_line(claim: &Claim) -> String {
    let example = example(claim).map_or_else(String::new, |text| format!(": {}", clip(text, 140)));
    format!("{} {} (support {}){example}", context_label(claim), claim.score, claim.support)
}

fn hint_line(hint: &HintView, input: &ReviewInput) -> String {
    let context: Vec<String> = hint.claims.iter().map(context_line).collect();
    format!(
        "- hint: **{}** {} · {}",
        hint.field,
        quest_label(hint.target, input.catalog),
        context.join(" · ")
    )
}

fn signed(value: f64, decimals: usize) -> String {
    format!("{}{}", if value >= 0.0 { "+" } else { "" }, fixed(value, decimals))
}

fn edge_block(edge: &EdgeView, input: &ReviewInput, evidence_lines: bool) -> Vec<String> {
    let score = format!("{}{}", fixed(edge.probability, 3), if edge.held_out { " held-out" } else { "" });
    let mut lines = vec![format!(
        "- **{}** {} · {score} · {}",
        edge.field,
        quest_label(edge.target, input.catalog),
        outcome_text(edge)
    )];
    if !evidence_lines {
        return lines;
    }
    for claim in edge.claims {
        let example = example(claim).map_or_else(String::new, |text| format!(": {}", clip(text, 170)));
        let against = if claim.contradict > 0 {
            format!(", {} against", claim.contradict)
        } else {
            String::new()
        };
        lines.push(format!("  - {} {} (support {}{against}){example}", claim.signal(), claim.score, claim.support));
    }
    if !edge.contributions.is_empty() {
        let claimed: FxHashSet<&str> = edge.claims.iter().map(Claim::signal).collect();
        let terms: Vec<String> = edge
            .contributions
            .iter()
            .map(|term| {
                let silent = if term.signal != "base" && !claimed.contains(term.signal) {
                    " silent"
                } else {
                    ""
                };
                format!("{} {}{silent}", term.signal, signed(term.log_odds, 1))
            })
            .collect();
        lines.push(format!("  - log-odds: {}", terms.join(" · ")));
    }
    lines.extend(edge.context.iter().map(|claim| format!("  - {}", context_line(claim))));
    lines
}

fn quest_block(view: &QuestView, edges: &[&EdgeView], input: &ReviewInput, reference_only: bool) -> Vec<String> {
    let mut lines = vec![
        format!("### {} · {}", quest_label(view.quest_id, input.catalog), giver_of(view.quest_id, input.catalog)),
        String::new(),
    ];
    if let Some(note) = view.note {
        lines.extend([format!("_{note}_"), String::new()]);
    }
    for edge in edges {
        lines.extend(edge_block(edge, input, edge.is_written() || edge.outcome == Outcome::BelowThreshold));
    }
    lines.extend(view.hints.iter().map(|hint| hint_line(hint, input)));
    if reference_only && !view.reference_only.is_empty() {
        let listed: Vec<String> = view
            .reference_only
            .iter()
            .map(|&(field, target)| format!("{field} {}", quest_label(target, input.catalog)))
            .collect();
        lines.push(format!("- reference only (no accepted evidence): {}", listed.join("; ")));
    }
    lines.push(String::new());
    lines
}

fn all_edges<'v>(view: &'v QuestView) -> Vec<&'v EdgeView<'v>> {
    view.edges.iter().collect()
}

fn written<'v>(view: &'v QuestView) -> impl Iterator<Item = &'v EdgeView<'v>> {
    view.edges.iter().filter(|edge| edge.is_written())
}

fn min_written(view: &QuestView) -> f64 {
    written(view).map(|edge| edge.probability).fold(f64::INFINITY, f64::min)
}

fn max_shown(view: &QuestView) -> f64 {
    view.edges.iter().map(|edge| edge.probability).fold(0.0, f64::max)
}

fn differs(edge: &EdgeView) -> bool {
    edge.is_written() && edge.status != Some(Status::Agree)
}

/// The strongest written relation that disagrees with the inherited data.
fn disagreement(view: &QuestView) -> f64 {
    view.edges.iter().filter(|edge| differs(edge)).map(|edge| edge.probability).fold(0.0, f64::max)
}

/// Descending by `key`, then by quest id.
fn by_descending(key: impl Fn(&QuestView) -> f64) -> impl Fn(&&QuestView, &&QuestView) -> Ordering {
    move |a, b| key(b).partial_cmp(&key(a)).unwrap_or(Ordering::Equal).then(a.quest_id.cmp(&b.quest_id))
}

pub fn render_review(input: &ReviewInput) -> String {
    let views: Vec<&QuestView> = input.views.values().collect();
    let forever: Vec<&QuestView> = views.iter().copied().filter(|view| view.domain == Domain::Forever).collect();
    let uncovered: Vec<&QuestView> = forever.iter().copied().filter(|view| view.reference == Reference::None).collect();
    let mut uncovered_accepted: Vec<&QuestView> = uncovered.iter().copied().filter(|view| written(view).next().is_some()).collect();
    uncovered_accepted.sort_by(by_descending(min_written));
    let mut uncovered_near_miss: Vec<&QuestView> = uncovered
        .iter()
        .copied()
        .filter(|view| written(view).next().is_none() && (!view.edges.is_empty() || !view.hints.is_empty()))
        .collect();
    uncovered_near_miss.sort_by(by_descending(max_shown));
    let authored: Vec<&QuestView> = forever.iter().copied().filter(|view| view.reference == Reference::Authored).collect();
    let mut authored_differing: Vec<&QuestView> = authored.iter().copied().filter(|view| view.edges.iter().any(differs)).collect();
    authored_differing.sort_by(by_descending(|view| {
        view.edges.iter().filter(|edge| edge.status == Some(Status::Conflict)).count() as f64
    }));
    let authored_agreeing: Vec<&QuestView> = authored
        .iter()
        .copied()
        .filter(|view| !view.edges.iter().any(differs) && written(view).next().is_some())
        .collect();
    let authored_silent: Vec<&QuestView> = authored.iter().copied().filter(|view| written(view).next().is_none()).collect();
    let mut classic_disagreeing: Vec<&QuestView> = views
        .iter()
        .copied()
        .filter(|view| view.domain == Domain::Classic && view.edges.iter().any(differs))
        .collect();
    classic_disagreeing.sort_by(by_descending(disagreement));

    let written_edges = |list: &[&QuestView]| list.iter().map(|view| written(view).count()).sum::<usize>();
    let count = |list: &[&QuestView], status: Status| {
        list.iter()
            .map(|view| written(view).filter(|edge| edge.status == Some(status)).count())
            .sum::<usize>()
    };
    let authored_only: usize = authored.iter().map(|view| view.reference_only.len()).sum();
    let ids = |list: &[&QuestView]| {
        let ids: Vec<String> = list.iter().map(|view| view.quest_id.to_string()).collect();
        if ids.is_empty() {
            "none".to_owned()
        } else {
            ids.join(", ")
        }
    };
    let or_none = |text: String| if text.is_empty() { "none".to_owned() } else { text };
    let inputs: Vec<String> = input
        .candidate_files
        .iter()
        .map(|file| format!("{} ({})", file.signal, file.candidates))
        .collect();
    let context: Vec<String> = input
        .context_files
        .iter()
        .map(|(signal, candidates)| format!("{signal} ({candidates})"))
        .collect();

    let mut lines: Vec<String> = vec![
        "# Quest relation review".into(),
        String::new(),
        format!(
            "Generated {} by `npm run relations -- combine`. Relations at or above **{}** are accepted (relations.json and the Lua output); \
             near misses from {} are listed for review.",
            input.generated_at, input.min_score, input.review_score
        ),
        String::new(),
        "A score is the combined model's probability that the relation is right. **held-out** marks scores for quests with reference data \
         (authored or inherited relations): the combine model that produced them never saw that quest's references (5-fold cross-validation \
         by quest group). The signals did: each calibrated its own scores on the same references, so held-out scores and metrics are \
         optimistic (see section 4). Each relation lists the evidence of every signal that claimed it, then the log-odds each signal added; \
         `silent` means the signal covered the quest but did not claim the relation."
            .into(),
        String::new(),
        "**Wowhead is context only.** Questie's own relation data was scraped from Wowhead Series rows, so Wowhead agreeing with the \
         references is circular. Its claims add nothing to any score, decision or metric; they appear as lines marked \
         `(untrusted, context only)`, and as `hint:` lines on Forever-new quests where only Wowhead claims something."
            .into(),
        String::new(),
        "Status against reference data: **agree**; **new** (the reference field is empty, so the output fills it); **conflict** \
         (the reference has other values; for authored quests the authored value wins at load time)."
            .into(),
        String::new(),
        "## Summary".into(),
        String::new(),
        format!(
            "- Forever-new quests without authored relations: **{}** with accepted relations ({} relations), {} with near misses or hints only.",
            uncovered_accepted.len(),
            written_edges(&uncovered_accepted),
            uncovered_near_miss.len()
        ),
        format!(
            "- Forever-new quests with authored relations: {}; accepted relations agree {}, new {}, conflict {}; {authored_only} authored relations \
             have no accepted evidence.",
            authored.len(),
            count(&authored, Status::Agree),
            count(&authored, Status::New),
            count(&authored, Status::Conflict)
        ),
        format!(
            "- Classic quests whose accepted relations disagree with inherited data: {} (review only, never in the Lua output).",
            classic_disagreeing.len()
        ),
        format!(
            "- Lua: `{}` (next to this report; `npm run dump --relation` merges its fields into `foreverQuestTraces.lua`), {} Forever-new quests.",
            input.lua_file, input.lua_quests
        ),
        format!("- Inputs: {}.", inputs.join(", ")),
        format!("- Context only, untrusted: {}.", or_none(context.join(", "))),
        format!("- AND (preQuestGroup) evidence trusted from: {}.", or_none(input.and_sources.join(", "))),
        String::new(),
        "## 1. Forever-new quests without authored relations".into(),
        String::new(),
        "Sorted by each quest's weakest accepted relation, strongest first.".into(),
        String::new(),
    ];
    for view in &uncovered_accepted {
        lines.extend(quest_block(view, &all_edges(view), input, false));
    }

    lines.extend([
        "## 1b. Forever-new quests without authored relations: near misses and hints only".into(),
        String::new(),
    ]);
    for view in &uncovered_near_miss {
        lines.extend(quest_block(view, &all_edges(view), input, false));
    }

    lines.extend([
        "## 2. Forever-new quests with authored relations".into(),
        String::new(),
        "Quests where an accepted relation is new or conflicts with the authored data. Authored corrections load after the generated module \
         and override per field, so a conflict here changes nothing in game until someone acts on it. Prerequisites are one slot: Questie \
         ignores preQuestGroup when preQuestSingle is set, so quests with authored prerequisites get no prerequisites from the Lua output."
            .into(),
        String::new(),
    ]);
    for view in &authored_differing {
        lines.extend(quest_block(view, &all_edges(view), input, true));
    }
    lines.extend([
        format!("**In full agreement** ({}): {}.", authored_agreeing.len(), ids(&authored_agreeing)),
        String::new(),
        format!("**No accepted relation** ({}): {}.", authored_silent.len(), ids(&authored_silent)),
        String::new(),
        "## 3. Classic quests that disagree with inherited data".into(),
        String::new(),
        "Review only. Forever may have re-wired these (often into Forever-new content), or the inherited data may be wrong; either way a person decides. \
         This includes the Classic side of exclusive pairs and breadcrumbs with a Forever-new quest: the Lua output carries only the Forever side. \
         Sorted by the strongest disagreeing relation."
            .into(),
        String::new(),
    ]);
    for view in &classic_disagreeing {
        let shown: Vec<&EdgeView> = view.edges.iter().filter(|edge| differs(edge)).collect();
        lines.extend(quest_block(view, &shown, input, false));
        let fields: Vec<RelationField> = shown.iter().map(|edge| edge.field).collect();
        let is_prerequisite = |field: &RelationField| matches!(field, RelationField::PreQuestSingle | RelationField::PreQuestGroup);
        let reference: Vec<String> = view
            .reference_only
            .iter()
            .filter(|(field, _)| fields.contains(field) || (is_prerequisite(field) && fields.iter().any(is_prerequisite)))
            .map(|&(field, target)| format!("{field} {}", quest_label(target, input.catalog)))
            .collect();
        if !reference.is_empty() {
            lines.extend([format!("- inherited instead: {}", reference.join("; ")), String::new()]);
        }
    }

    lines.extend([
        "### 3b. Classic near misses with broad agreement".into(),
        String::new(),
        "Not accepted, but at least two signals claim them at 0.8 or more and the inherited data lacks them. The model learns Classic from the \
         inherited data, so where Forever re-wired a chain (often by inserting a new quest the Classic quest now also needs) it learns to \
         distrust exactly this evidence. Worth a look; a preQuestGroup claim suggests the new quest is needed in addition to the inherited one."
            .into(),
        String::new(),
    ]);
    for miss in input.classic_misses {
        let edge = miss.edge;
        let signals: Vec<String> = edge
            .edge
            .claims
            .iter()
            .map(|claim| {
                format!(
                    "{} {}{}",
                    claim.signal(),
                    claim.score,
                    if claim.field == RelationField::PreQuestGroup { " (AND)" } else { "" }
                )
            })
            .collect();
        let instead: Vec<String> = miss
            .inherited
            .iter()
            .map(|&(field, target)| format!("{field} {}", quest_label(target, input.catalog)))
            .collect();
        let key = edge.key();
        let relation = format!(
            "{} {} {}",
            quest_label(key.quest, input.catalog),
            key.kind.name(),
            quest_label(key.target, input.catalog)
        );
        lines.push(format!(
            "- {relation} · {} · {} · inherited: {}",
            fixed(edge.probability, 3),
            signals.join(", "),
            if instead.is_empty() { "nothing".to_owned() } else { instead.join("; ") }
        ));
    }
    lines.extend([
        String::new(),
        "## 4. Held-out metrics".into(),
        String::new(),
        "Per Questie field, scored like `npm run relations -- score` over the quests any trace signal covered. Combined scores are out of fold. \
         The best single signal is the trace signal with the highest recall at no less than the combined precision (else the most precise one), \
         over the same quests and any of its own score thresholds 0 / 0.5 / 0.8 / 0.9 / 0.95. Authored data is incomplete (many authored quests \
         lack nextQuestInChain even where the authored next quest requires them), so authored precision is a lower bound on that side."
            .into(),
        String::new(),
        "How far held out goes:".into(),
        String::new(),
    ]);
    lines.extend(input.caveats.iter().map(|caveat| format!("- {caveat}")));
    lines.extend([
        String::new(),
        input.metrics_markdown.to_owned(),
        "#### Without signal-score columns".into(),
        String::new(),
        "The same pipeline refitted on claimed, support, contradiction rate and silence only, as a check on how much the in-sample signal \
         calibration flatters the numbers above."
            .into(),
        String::new(),
        input.label_free_markdown.to_owned(),
        String::new(),
        "## 5. Model".into(),
        String::new(),
        "One L2-regularized logistic regression per relation kind, fitted on every judged edge. Columns: per source `claimed`, `score` and `support` \
         (centered log terms), `contradict` (rate); per signal `silent`. Weights are log-odds; the largest by magnitude are shown."
            .into(),
        String::new(),
    ]);
    for fitted in input.fitted {
        let kind = fitted.kind.name();
        if !fitted.model.intercept.is_finite() {
            lines.push(format!(
                "- **{kind}** ({} judged edges, {} true): too little ground truth to calibrate; never accepted",
                fitted.judged, fitted.positives
            ));
            continue;
        }
        let mut weights: Vec<(&str, f64)> = fitted
            .columns
            .iter()
            .zip(&fitted.model.weights)
            .map(|(column, &weight)| (column.name.as_str(), weight))
            .filter(|(_, weight)| weight.abs() >= 0.05)
            .collect();
        weights.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        let weights: Vec<String> = weights.iter().take(12).map(|(name, weight)| format!("{name} {}", signed(*weight, 2))).collect();
        lines.push(format!(
            "- **{kind}** ({} judged edges, {} true): intercept {}; {}",
            fitted.judged,
            fitted.positives,
            fixed(fitted.model.intercept, 2),
            weights.join(", ")
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_by_utf16_length_without_splitting_characters() {
        assert_eq!(clip("abcdef", 6), "abcdef");
        assert_eq!(clip("abcdefg", 6), "abcde…");
        // "😀" is two UTF-16 code units: it does not fit in the 5 kept units after "abcd".
        assert_eq!(clip("abcd😀fg", 6), "abcd…");
    }
}
