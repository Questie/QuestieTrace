// reports/combine-review.md: what a person reads before trusting the Lua output. Ordered by
// value: Forever-new quests nobody has authored come first, most confident first; then authored
// quests where the evidence adds or disagrees; then Classic disagreements; then the numbers.
// Wowhead claims appear only as labelled context lines (see inputs.ts for why).

import type { CatalogQuest, RelationField } from "../core/types";
import type { FittedKind, ScoredEdge } from "./crossval";
import type { Claim } from "./edges";
import type { EdgeView, HintView, QuestView } from "./views";

export interface ReviewInput {
  views: ReadonlyMap<number, QuestView>;
  quests: Readonly<Record<string, CatalogQuest>>;
  minScore: number;
  reviewScore: number;
  candidateFiles: Array<{ signal: string; candidates: number; covered: number }>;
  /** Context-only files (Wowhead): shown, never scored. */
  contextFiles: Array<{ signal: string; candidates: number }>;
  andSources: string[];
  metricsMarkdown: string;
  /** Full model vs the model without signal-score columns. */
  labelFreeMarkdown: string;
  caveats: readonly string[];
  fitted: FittedKind[];
  luaPath: string;
  luaQuests: number;
  /** Classic edges left out despite broad signal agreement, with the inherited values they disagree with. */
  classicMisses: Array<{ edge: ScoredEdge; inherited: Array<{ field: RelationField; target: number }> }>;
  generatedAt: Date;
}

const WRITTEN = new Set(["accepted", "clique", "derived"]);
const isWritten = (edge: EdgeView) => WRITTEN.has(edge.outcome);

function questLabel(questId: number, quests: ReviewInput["quests"]): string {
  const name = quests[String(questId)]?.name;
  return name ? `${questId} "${name}"` : `${questId} (not in catalog)`;
}

function giverOf(questId: number, quests: ReviewInput["quests"]): string {
  const starters = quests[String(questId)]?.starters;
  if (!starters) return "giver unknown";
  const all = [...starters.npcs.map((id) => `npc ${id}`), ...starters.objects.map((id) => `object ${id}`), ...starters.items.map((id) => `item ${id}`)];
  if (all.length === 0) return "giver unknown";
  return all.length > 3 ? `${all.slice(0, 3).join(", ")} +${all.length - 3}` : all.join(", ");
}

function describeKey(key: string): string {
  const [kind, quest, target] = key.split(":");
  return `${kind} ${quest} -> ${target}`;
}

function clip(text: string, length = 170): string {
  return text.length > length ? `${text.slice(0, length - 1)}…` : text;
}

function outcomeText(edge: EdgeView): string {
  switch (edge.outcome) {
    case "accepted":
      return edge.status ?? "";
    case "clique":
      return `${edge.status} · completes an accepted exclusive group`;
    case "derived":
      return `${edge.status} · implied by ${describeKey(edge.because!)}`;
    case "below-threshold":
      return "near miss";
    default:
      return `dropped: ${edge.outcome}${edge.because ? ` (${describeKey(edge.because)})` : ""}`;
  }
}

/** "wowhead-requires-any" -> "Wowhead Requires Any". */
function contextLabel(claim: Claim): string {
  const words = claim.signal.split("-").map((word) => word.charAt(0).toUpperCase() + word.slice(1));
  return `${words.join(" ")} (untrusted, context only)`;
}

function contextLine(claim: Claim): string {
  const example = claim.evidence[0]?.text;
  return `${contextLabel(claim)} ${claim.score} (support ${claim.support})${example ? `: ${clip(example, 140)}` : ""}`;
}

function hintLine(hint: HintView, input: ReviewInput): string {
  return `- hint: **${hint.field}** ${questLabel(hint.target, input.quests)} · ${hint.claims.map(contextLine).join(" · ")}`;
}

function edgeBlock(edge: EdgeView, input: ReviewInput, evidenceLines: boolean): string[] {
  const score = `${edge.probability.toFixed(3)}${edge.heldOut ? " held-out" : ""}`;
  const lines = [`- **${edge.field}** ${questLabel(edge.target, input.quests)} · ${score} · ${outcomeText(edge)}`];
  if (!evidenceLines) return lines;
  for (const claim of edge.claims) {
    const example = claim.evidence[0]?.text;
    const against = claim.contradict > 0 ? `, ${claim.contradict} against` : "";
    lines.push(`  - ${claim.signal} ${claim.score} (support ${claim.support}${against})${example ? `: ${clip(example)}` : ""}`);
  }
  if (edge.contributions.length > 0) {
    const claimed = new Set(edge.claims.map((claim) => claim.signal));
    const terms = edge.contributions.map(
      (term) =>
        `${term.signal} ${term.logOdds >= 0 ? "+" : ""}${term.logOdds.toFixed(1)}${term.signal !== "base" && !claimed.has(term.signal) ? " silent" : ""}`,
    );
    lines.push(`  - log-odds: ${terms.join(" · ")}`);
  }
  for (const claim of edge.context) lines.push(`  - ${contextLine(claim)}`);
  return lines;
}

function questBlock(view: QuestView, input: ReviewInput, options: { referenceOnly: boolean }): string[] {
  const lines = [`### ${questLabel(view.questId, input.quests)} · ${giverOf(view.questId, input.quests)}`, ""];
  for (const note of view.notes) lines.push(`_${note}_`, "");
  for (const edge of view.edges) lines.push(...edgeBlock(edge, input, isWritten(edge) || edge.outcome === "below-threshold"));
  for (const hint of view.hints) lines.push(hintLine(hint, input));
  if (options.referenceOnly && view.referenceOnly.length > 0) {
    const listed = view.referenceOnly.map(({ field, target }) => `${field} ${questLabel(target, input.quests)}`);
    lines.push(`- reference only (no accepted evidence): ${listed.join("; ")}`);
  }
  lines.push("");
  return lines;
}

const minWritten = (view: QuestView) => Math.min(...view.edges.filter(isWritten).map((edge) => edge.probability));
const maxShown = (view: QuestView) => Math.max(0, ...view.edges.map((edge) => edge.probability));

export function renderReview(input: ReviewInput): string {
  const views = [...input.views.values()];
  const forever = views.filter((view) => view.domain === "forever");
  const uncovered = forever.filter((view) => view.reference === "none");
  const uncoveredAccepted = uncovered.filter((view) => view.edges.some(isWritten)).sort((a, b) => minWritten(b) - minWritten(a) || a.questId - b.questId);
  const uncoveredNearMiss = uncovered
    .filter((view) => !view.edges.some(isWritten) && (view.edges.length > 0 || view.hints.length > 0))
    .sort((a, b) => maxShown(b) - maxShown(a) || a.questId - b.questId);
  const authored = forever.filter((view) => view.reference === "authored");
  const differs = (view: QuestView) => view.edges.some((edge) => isWritten(edge) && edge.status !== "agree");
  const authoredDiffering = authored
    .filter(differs)
    .sort(
      (a, b) =>
        b.edges.filter((edge) => edge.status === "conflict").length - a.edges.filter((edge) => edge.status === "conflict").length || a.questId - b.questId,
    );
  const authoredAgreeing = authored.filter((view) => !differs(view) && view.edges.some(isWritten));
  const authoredSilent = authored.filter((view) => !view.edges.some(isWritten));
  const classicDisagreeing = views
    .filter((view) => view.domain === "classic" && view.edges.some((edge) => isWritten(edge) && edge.status !== "agree"))
    .sort((a, b) => disagreement(b) - disagreement(a) || a.questId - b.questId);

  const writtenEdges = (list: QuestView[]) => list.reduce((sum, view) => sum + view.edges.filter(isWritten).length, 0);
  const count = (list: QuestView[], status: string) =>
    list.reduce((sum, view) => sum + view.edges.filter((edge) => isWritten(edge) && edge.status === status).length, 0);
  const authoredOnly = authored.reduce((sum, view) => sum + view.referenceOnly.length, 0);

  const lines: string[] = [
    "# Quest relation review",
    "",
    `Generated ${input.generatedAt.toISOString()} by \`npm run relations -- combine\`. Relations at or above **${input.minScore}** are accepted ` +
      `(relations.json and the Lua output); near misses from ${input.reviewScore} are listed for review.`,
    "",
    "A score is the combined model's probability that the relation is right. **held-out** marks scores for quests with reference data " +
      "(authored or inherited relations): the combine model that produced them never saw that quest's references (5-fold cross-validation " +
      "by quest group). The signals did: each calibrated its own scores on the same references, so held-out scores and metrics are " +
      "optimistic (see section 4). Each relation lists the evidence of every signal that claimed it, then the log-odds each signal added; " +
      "`silent` means the signal covered the quest but did not claim the relation.",
    "",
    "**Wowhead is context only.** Questie's own relation data was scraped from Wowhead Series rows, so Wowhead agreeing with the " +
      "references is circular. Its claims add nothing to any score, decision or metric; they appear as lines marked " +
      "`(untrusted, context only)`, and as `hint:` lines on Forever-new quests where only Wowhead claims something.",
    "",
    "Status against reference data: **agree**; **new** (the reference field is empty, so the output fills it); **conflict** " +
      "(the reference has other values; for authored quests the authored value wins at load time).",
    "",
    "## Summary",
    "",
    `- Forever-new quests without authored relations: **${uncoveredAccepted.length}** with accepted relations ` +
      `(${writtenEdges(uncoveredAccepted)} relations), ` +
      `${uncoveredNearMiss.length} with near misses or hints only.`,
    `- Forever-new quests with authored relations: ${authored.length}; accepted relations agree ${count(authored, "agree")}, new ${count(authored, "new")}, ` +
      `conflict ${count(authored, "conflict")}; ${authoredOnly} authored relations have no accepted evidence.`,
    `- Classic quests whose accepted relations disagree with inherited data: ${classicDisagreeing.length} (review only, never in the Lua output).`,
    `- Lua: \`${input.luaPath}\`, ${input.luaQuests} Forever-new quests.`,
    `- Inputs: ${input.candidateFiles.map((file) => `${file.signal} (${file.candidates})`).join(", ")}.`,
    `- Context only, untrusted: ${input.contextFiles.map((file) => `${file.signal} (${file.candidates})`).join(", ") || "none"}.`,
    `- AND (preQuestGroup) evidence trusted from: ${input.andSources.join(", ") || "none"}.`,
    "",
    "## 1. Forever-new quests without authored relations",
    "",
    "Sorted by each quest's weakest accepted relation, strongest first.",
    "",
  ];
  for (const view of uncoveredAccepted) lines.push(...questBlock(view, input, { referenceOnly: false }));

  lines.push("## 1b. Forever-new quests without authored relations: near misses and hints only", "");
  for (const view of uncoveredNearMiss) lines.push(...questBlock(view, input, { referenceOnly: false }));

  lines.push(
    "## 2. Forever-new quests with authored relations",
    "",
    "Quests where an accepted relation is new or conflicts with the authored data. Authored corrections load after the generated module " +
      "and override per field, so a conflict here changes nothing in game until someone acts on it. Prerequisites are one slot: Questie " +
      "ignores preQuestGroup when preQuestSingle is set, so quests with authored prerequisites get no prerequisites from the Lua output.",
    "",
  );
  for (const view of authoredDiffering) lines.push(...questBlock(view, input, { referenceOnly: true }));
  lines.push(
    `**In full agreement** (${authoredAgreeing.length}): ${authoredAgreeing.map((view) => view.questId).join(", ") || "none"}.`,
    "",
    `**No accepted relation** (${authoredSilent.length}): ${authoredSilent.map((view) => view.questId).join(", ") || "none"}.`,
    "",
    "## 3. Classic quests that disagree with inherited data",
    "",
    "Review only. Forever may have re-wired these (often into Forever-new content), or the inherited data may be wrong; either way a person decides. " +
      "This includes the Classic side of exclusive pairs and breadcrumbs with a Forever-new quest: the Lua output carries only the Forever side. " +
      "Sorted by the strongest disagreeing relation.",
    "",
  );
  for (const view of classicDisagreeing) {
    const shown = { ...view, edges: view.edges.filter((edge) => isWritten(edge) && edge.status !== "agree") };
    lines.push(...questBlock(shown, input, { referenceOnly: false }));
    const fields = new Set(shown.edges.map((edge) => edge.field));
    const reference = view.referenceOnly.filter(
      (entry) => fields.has(entry.field) || (entry.field.startsWith("preQuest") && [...fields].some((f) => f.startsWith("preQuest"))),
    );
    if (reference.length > 0)
      lines.push(`- inherited instead: ${reference.map(({ field, target }) => `${field} ${questLabel(target, input.quests)}`).join("; ")}`, "");
  }

  lines.push(
    "### 3b. Classic near misses with broad agreement",
    "",
    "Not accepted, but at least two signals claim them at 0.8 or more and the inherited data lacks them. The model learns Classic from the " +
      "inherited data, so where Forever re-wired a chain (often by inserting a new quest the Classic quest now also needs) it learns to " +
      "distrust exactly this evidence. Worth a look; a preQuestGroup claim suggests the new quest is needed in addition to the inherited one.",
    "",
  );
  for (const { edge, inherited } of input.classicMisses) {
    const signals = edge.claims.map((claim) => `${claim.signal} ${claim.score}${claim.field === "preQuestGroup" ? " (AND)" : ""}`).join(", ");
    const instead = inherited.map(({ field, target }) => `${field} ${questLabel(target, input.quests)}`).join("; ") || "nothing";
    const relation = `${questLabel(edge.quest, input.quests)} ${edge.kind} ${questLabel(edge.target, input.quests)}`;
    lines.push(`- ${relation} · ${edge.probability.toFixed(3)} · ${signals} · inherited: ${instead}`);
  }
  lines.push(
    "",
    "## 4. Held-out metrics",
    "",
    "Per Questie field, scored like `npm run relations -- score` over the quests any trace signal covered. Combined scores are out of fold. " +
      "The best single signal is the trace signal with the highest recall at no less than the combined precision (else the most precise one), " +
      "over the same quests and any of its own score thresholds 0 / 0.5 / 0.8 / 0.9 / 0.95. Authored data is incomplete (many authored quests " +
      "lack nextQuestInChain even where the authored next quest requires them), so authored precision is a lower bound on that side.",
    "",
    "How far held out goes:",
    "",
    ...input.caveats.map((caveat) => `- ${caveat}`),
    "",
    input.metricsMarkdown,
    "#### Without signal-score columns",
    "",
    "The same pipeline refitted on claimed, support, contradiction rate and silence only, as a check on how much the in-sample signal " +
      "calibration flatters the numbers above.",
    "",
    input.labelFreeMarkdown,
    "",
    "## 5. Model",
    "",
    "One L2-regularized logistic regression per relation kind, fitted on every judged edge. Columns: per source `claimed`, `score` and `support` " +
      "(centered log terms), `contradict` (rate); per signal `silent`. Weights are log-odds; the largest by magnitude are shown.",
    "",
  );
  for (const fitted of input.fitted) {
    if (!Number.isFinite(fitted.model.intercept)) {
      lines.push(`- **${fitted.kind}** (${fitted.judged} judged edges, ${fitted.positives} true): too little ground truth to calibrate; never accepted`);
      continue;
    }
    const weights = fitted.columns
      .map((column, j) => ({ name: column.name, weight: fitted.model.weights[j] }))
      .filter((entry) => Math.abs(entry.weight) >= 0.05)
      .sort((a, b) => Math.abs(b.weight) - Math.abs(a.weight))
      .slice(0, 12)
      .map((entry) => `${entry.name} ${entry.weight >= 0 ? "+" : ""}${entry.weight.toFixed(2)}`);
    lines.push(
      `- **${fitted.kind}** (${fitted.judged} judged edges, ${fitted.positives} true): intercept ${fitted.model.intercept.toFixed(2)}; ${weights.join(", ")}`,
    );
  }
  lines.push("");
  return lines.join("\n");
}

function disagreement(view: QuestView): number {
  return Math.max(0, ...view.edges.filter((edge) => isWritten(edge) && edge.status !== "agree").map((edge) => edge.probability));
}
