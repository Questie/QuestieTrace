//! Exclusion: X and Y are each commonly completed, yet no character ever had both.
//!
//! In practice this is the rare signature: most exclusive groups are co-flagged instead (see
//! [`super::coflag`]), and on the 2026-10 data no pair clears the thresholds below. It stays as the
//! check for true choose-one quests, scored low until a human confirms an edge
//! ([`super::calibration`]).
//!
//! "Never together" only means something relative to how often they would meet by chance, so
//! each pair gets the number of characters expected to complete both if completions were
//! independent. That expectation is stratified by race/class cell and gated on level: two
//! starting-zone quests of different races are never done together, but not because the server
//! forbids it. A pair qualifies when the expectation is high enough that zero is surprising and
//! no character ever held both at once (in the log together, or one in the log while the other
//! was completed). Quests gated by skills, reputation or spells we cannot see are only compared
//! with quests behind the same gate: "Camping 101: Alchemy" and "Camping 101: Mining" are never
//! done together because few characters have both professions, not because the server forbids it.
//!
//! Choose-one-of-N groups (profession variants, reward choices) are rarely completed per member,
//! so pairs inside one structural cluster (shared starter, or shared name stem) are judged
//! together: every pair of a clique must be clean, and the clique's summed expectation must clear
//! a lower bar than an unrelated pair needs.

use super::history::held_together;
use super::population::{Character, Population};
use crate::model::{Evidence, QuestId};
use crate::text::js_trim;
use indexmap::IndexMap;
use rayon::prelude::*;
use rustc_hash::{FxBuildHasher, FxHashSet};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExclusionParams {
    /// Completers a quest needs before it takes part in pairwise tests.
    pub min_completers: usize,
    /// Expected co-completions (under independence) an unrelated pair needs.
    pub min_expected: f64,
    /// Summed expected co-completions a structural clique needs.
    pub min_group_expected: f64,
    /// Members of a structural clique need at least this many completers.
    pub min_group_member_completers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PairStats {
    /// The lower quest id of the pair.
    pub a: QuestId,
    pub b: QuestId,
    /// The quest indexes of `a` and `b`.
    indexes: (usize, usize),
    /// Completers of a / b who could also have taken the other.
    pub completers_a: usize,
    pub completers_b: usize,
    pub expected: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExclusionKind {
    /// Cleared `min_expected` alone.
    Pair,
    /// Member of a structural clique of `size` quests whose pairs expected `expected` together.
    Group { size: usize, expected: f64 },
}

pub struct ExclusionEdge {
    pub pair: PairStats,
    pub kind: ExclusionKind,
    /// Characters who took both at different times (abandoned one, then took the other).
    pub switched: usize,
    pub evidence: Vec<Evidence>,
}

pub struct ExclusionResult {
    pub edges: Vec<ExclusionEdge>,
    pub tested_pairs: usize,
    /// Pairs that cleared the expectation but some character held both at once.
    pub conflicted: usize,
}

/// What the pairwise pass found for one quest against every later frequent quest.
struct PairPass {
    tested: usize,
    conflicted: usize,
    clean: Vec<PairStats>,
}

pub fn infer_exclusions(population: &Population, params: &ExclusionParams) -> ExclusionResult {
    let stats = PairCounter::new(population);
    let mut result = ExclusionResult {
        edges: Vec::new(),
        tested_pairs: 0,
        conflicted: 0,
    };
    let mut seen: FxHashSet<(QuestId, QuestId)> = FxHashSet::default();
    let mut emit = |result: &mut ExclusionResult, pair: PairStats, kind: ExclusionKind| {
        if seen.insert((pair.a, pair.b)) {
            result.edges.push(ExclusionEdge {
                pair,
                kind,
                switched: stats.switched(pair.indexes.0, pair.indexes.1),
                evidence: evidence_for(population, &pair),
            });
        }
    };

    let completers = |index: usize| population.completers[index].len();
    let frequent: Vec<usize> = (0..population.quests.len())
        .filter(|&index| completers(index) >= params.min_completers)
        .collect();
    let passes: Vec<PairPass> = (0..frequent.len())
        .into_par_iter()
        .map(|i| {
            let mut pass = PairPass {
                tested: 0,
                conflicted: 0,
                clean: Vec::new(),
            };
            let a = frequent[i];
            for &b in &frequent[i + 1..] {
                // E <= min(completers), so small quests can never clear the bar alone.
                if (completers(a).min(completers(b)) as f64) < params.min_expected {
                    continue;
                }
                if !same_hidden_gate(population, a, b) || stats.co_completed(a, b) {
                    continue;
                }
                pass.tested += 1;
                let pair = stats.pair(a, b);
                if pair.expected < params.min_expected {
                    continue;
                }
                if stats.conflicted(a, b) {
                    pass.conflicted += 1;
                    continue;
                }
                pass.clean.push(pair);
            }
            pass
        })
        .collect();
    for pass in passes {
        result.tested_pairs += pass.tested;
        result.conflicted += pass.conflicted;
        for pair in pass.clean {
            emit(&mut result, pair, ExclusionKind::Pair);
        }
    }

    for cluster in structural_clusters(population, params.min_group_member_completers) {
        let clean =
            |a: usize, b: usize| same_hidden_gate(population, a, b) && !stats.co_completed(a, b) && stats.pair(a, b).expected > 0.0 && !stats.conflicted(a, b);
        // Every pair of the cluster, once: a clean pair is clean either way round.
        let mut adjacent = vec![vec![false; cluster.len()]; cluster.len()];
        for x in 0..cluster.len() {
            for y in x + 1..cluster.len() {
                let edge = clean(cluster[x], cluster[y]);
                adjacent[x][y] = edge;
                adjacent[y][x] = edge;
            }
        }
        for clique in maximal_cliques(cluster.len(), |x, y| adjacent[x][y]) {
            let members: Vec<usize> = clique.iter().map(|&vertex| cluster[vertex]).collect();
            let pairs: Vec<PairStats> = members
                .iter()
                .enumerate()
                .flat_map(|(i, &a)| members[i + 1..].iter().map(move |&b| (a, b)))
                .map(|(a, b)| stats.pair(a, b))
                .collect();
            let group_expected: f64 = pairs.iter().map(|pair| pair.expected).sum();
            if group_expected < params.min_group_expected {
                continue;
            }
            for pair in pairs {
                let kind = ExclusionKind::Group {
                    size: members.len(),
                    expected: group_expected,
                };
                emit(&mut result, pair, kind);
            }
        }
    }
    result
}

/// Both quests sit behind the same untracked requirement (skill, reputation, spell), or neither
/// does.
fn same_hidden_gate(population: &Population, a: usize, b: usize) -> bool {
    let gate = |index: usize| {
        let quest = population.quests[index];
        (
            quest.required_skill.map(|(skill, _)| skill),
            quest.required_min_rep.map(|(faction, _)| faction),
            quest.required_max_rep.map(|(faction, _)| faction),
            quest.required_spell,
        )
    };
    gate(a) == gate(b)
}

/// Pair statistics over the population, with the completer bitsets they need. Quests are quest
/// indexes.
struct PairCounter<'p, 'e> {
    population: &'p Population<'e>,
    /// Per quest index, its completers as a bitset over character indexes.
    completed_by: Vec<Vec<u64>>,
    /// Per cell, its characters.
    cell_members: Vec<Vec<usize>>,
}

impl<'p, 'e> PairCounter<'p, 'e> {
    fn new(population: &'p Population<'e>) -> Self {
        let words = population.characters.len().div_ceil(64);
        let completed_by = population
            .completers
            .iter()
            .map(|completers| {
                let mut set = vec![0u64; words];
                for &character in completers {
                    set[character / 64] |= 1 << (character % 64);
                }
                set
            })
            .collect();
        let mut cell_members = vec![Vec::new(); population.cells.len()];
        for (index, character) in population.characters.iter().enumerate() {
            cell_members[character.cell].push(index);
        }
        PairCounter {
            population,
            completed_by,
            cell_members,
        }
    }

    fn co_completed(&self, a: usize, b: usize) -> bool {
        self.completed_by[a].iter().zip(&self.completed_by[b]).any(|(x, y)| x & y != 0)
    }

    /// Completers of each quest who could take the other, and the stratified expected overlap.
    fn pair(&self, a: usize, b: usize) -> PairStats {
        let (a, b) = (a.min(b), a.max(b));
        let population = self.population;
        // Per cell, in the order cells are met: completers of a who could take b, and of b who could take a.
        let mut per_cell: IndexMap<usize, (usize, usize), FxBuildHasher> = IndexMap::default();
        for &character in &population.completers[a] {
            let character = &population.characters[character];
            if population.could_take(character, b) {
                per_cell.entry(character.cell).or_default().0 += 1;
            }
        }
        for &character in &population.completers[b] {
            let character = &population.characters[character];
            if population.could_take(character, a) {
                per_cell.entry(character.cell).or_default().1 += 1;
            }
        }

        let mut stats = PairStats {
            a: population.quests[a].id,
            b: population.quests[b].id,
            indexes: (a, b),
            completers_a: 0,
            completers_b: 0,
            expected: 0.0,
        };
        for (&cell, &(count_a, count_b)) in &per_cell {
            stats.completers_a += count_a;
            stats.completers_b += count_b;
            if count_a == 0 || count_b == 0 {
                continue;
            }
            let eligible = self.cell_members[cell]
                .iter()
                .filter(|&&member| {
                    let character = &population.characters[member];
                    population.could_take(character, a) && population.could_take(character, b)
                })
                .count();
            if eligible > 0 {
                stats.expected += (count_a * count_b) as f64 / eligible as f64;
            }
        }
        stats
    }

    /// Some character had both active at once: both in the log, or one in the log while the other
    /// was completed.
    fn conflicted(&self, a: usize, b: usize) -> bool {
        let (id_a, id_b) = (self.population.quests[a].id, self.population.quests[b].id);
        self.both_taken(a, id_b).any(|character| held_together(&character.history, id_a, id_b))
    }

    /// Characters who took both quests without ever holding them together.
    fn switched(&self, a: usize, b: usize) -> usize {
        let (id_a, id_b) = (self.population.quests[a].id, self.population.quests[b].id);
        self.both_taken(a, id_b)
            .filter(|character| !held_together(&character.history, id_a, id_b))
            .count()
    }

    fn both_taken(&self, a: usize, id_b: QuestId) -> impl Iterator<Item = &'p Character<'e>> + '_ {
        let population = self.population;
        population.takers[a]
            .iter()
            .map(move |&character| &population.characters[character])
            .filter(move |character| character.history.taken.contains_key(&id_b))
    }
}

/// What makes quests plausibly one choice.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ClusterKey {
    StarterNpc(i64),
    StarterObject(i64),
    StarterItem(i64),
    NameStem(String),
    Name(String),
}

/// Quest-index clusters that plausibly form one choice: quests sharing a starter, or sharing a name
/// stem ("Camping 101: Alchemy" and "Camping 101: Herbalism"). Only members with enough
/// completers. Clusters come in the order their key is first met, walking quests by ascending id.
fn structural_clusters(population: &Population, min_completers: usize) -> Vec<Vec<usize>> {
    let mut groups: IndexMap<ClusterKey, Vec<usize>, FxBuildHasher> = IndexMap::default();
    let mut add = |key: ClusterKey, index: usize| {
        let members = groups.entry(key).or_default();
        // A starter listed twice on one quest must not make the quest its own cluster-mate.
        if members.last() != Some(&index) {
            members.push(index);
        }
    };
    for (index, quest) in population.quests.iter().enumerate() {
        if population.completers[index].len() < min_completers {
            continue;
        }
        for &id in &quest.starters.npcs {
            add(ClusterKey::StarterNpc(id), index);
        }
        for &id in &quest.starters.objects {
            add(ClusterKey::StarterObject(id), index);
        }
        for &id in &quest.starters.items {
            add(ClusterKey::StarterItem(id), index);
        }
        if let Some(stem) = name_stem(quest.name.as_deref()) {
            add(ClusterKey::NameStem(stem), index);
        }
        if let Some(name) = quest.name.as_deref().filter(|name| !name.is_empty()) {
            add(ClusterKey::Name(name.to_lowercase()), index);
        }
    }
    groups.into_values().filter(|members| members.len() >= 2).collect()
}

/// "Camping 101: Alchemy" -> "camping 101". Names without a separator are their own stem.
pub fn name_stem(name: Option<&str>) -> Option<String> {
    let name = name.filter(|name| !name.is_empty())?;
    let head = name.split(':').next().unwrap_or(name).split(" - ").next().unwrap_or(name);
    let stem = js_trim(head).to_lowercase();
    (stem.chars().count() >= 4).then_some(stem)
}

/// Bron-Kerbosch (without pivoting) over vertices `0..count`. Cliques of one are not returned.
/// Cliques come in the order the recursion finds them, which decides which clique an edge shared
/// by two cliques is reported with.
fn maximal_cliques(count: usize, adjacent: impl Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
    fn expand(
        clique: Vec<usize>,
        mut candidates: Vec<usize>,
        mut excluded: Vec<usize>,
        adjacent: &dyn Fn(usize, usize) -> bool,
        cliques: &mut Vec<Vec<usize>>,
    ) {
        if candidates.is_empty() && excluded.is_empty() {
            if clique.len() >= 2 {
                cliques.push(clique);
            }
            return;
        }
        for vertex in candidates.clone() {
            let neighbours = |list: &[usize]| -> Vec<usize> { list.iter().copied().filter(|&other| other != vertex && adjacent(vertex, other)).collect() };
            let mut grown = clique.clone();
            grown.push(vertex);
            expand(grown, neighbours(&candidates), neighbours(&excluded), adjacent, cliques);
            candidates.retain(|&other| other != vertex);
            excluded.push(vertex);
        }
    }
    let mut cliques = Vec::new();
    expand(Vec::new(), (0..count).collect(), Vec::new(), &adjacent, &mut cliques);
    cliques
}

/// Up to three completers per side who could have taken the other quest and never did.
fn evidence_for(population: &Population, pair: &PairStats) -> Vec<Evidence> {
    let side = |completed_index: usize, other_index: usize| {
        let (completed, other) = (population.quests[completed_index].id, population.quests[other_index].id);
        population.completers[completed_index]
            .iter()
            .map(|&character| &population.characters[character])
            .filter(move |character| population.could_take(character, other_index) && !character.history.taken.contains_key(&other))
            .take(3)
            .map(move |character| {
                let history = &character.history;
                let player = &history.player;
                Evidence {
                    reference: history.episode_key(&history.done[&completed]).to_owned(),
                    t: None,
                    text: format!(
                        "character {} (race {}, class {}, level {}): completed {completed}, never took {other}",
                        history.key,
                        js_template(player.race_id),
                        js_template(player.class_id),
                        js_template(history.max_level)
                    ),
                }
            })
    };
    let (a, b) = pair.indexes;
    side(a, b).chain(side(b, a)).collect()
}

/// A possibly missing number as the TS template literal wrote it: "undefined" when missing. Kept
/// so evidence text matches the TS pipeline's.
fn js_template(value: Option<i64>) -> String {
    value.map_or_else(|| "undefined".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CatalogQuest, QuestEpisode};
    use crate::signals::cooccurrence::fixtures::{population_of, session, SessionSpec};
    use crate::test_support::{catalog, catalog_quest};

    // Twelve characters of one race and class, half completing 51 and half 52: 3 expected together
    // (2.8 once a thirteenth character joins), against a bar of 2.5.
    const PARAMS: ExclusionParams = ExclusionParams {
        min_completers: 2,
        min_expected: 2.5,
        min_group_expected: 3.0,
        min_group_member_completers: 2,
    };

    fn split(race_of_b: i64) -> Vec<QuestEpisode> {
        let completing = |prefix: &str, race: i64, quest_id: QuestId| {
            (0..6)
                .map(|i| {
                    session(
                        &format!("{prefix}{i}"),
                        SessionSpec {
                            race,
                            initial: vec![8],
                            turn_ins: vec![(100.0, quest_id)],
                            ..SessionSpec::default()
                        },
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut episodes = completing("a", 1, 51);
        episodes.extend(completing("b", race_of_b, 52));
        episodes
    }

    fn pairs(episodes: &[QuestEpisode], quests: Vec<CatalogQuest>) -> Vec<(QuestId, QuestId)> {
        let catalog = catalog(quests);
        infer_exclusions(&population_of(episodes, &catalog), &PARAMS)
            .edges
            .iter()
            .map(|edge| (edge.pair.a, edge.pair.b))
            .collect()
    }

    fn plain_quests() -> Vec<CatalogQuest> {
        [8, 51, 52].map(catalog_quest).into()
    }

    #[test]
    fn two_quests_eligible_characters_complete_often_but_never_together_are_flagged() {
        let catalog = catalog(plain_quests());
        let episodes = split(1);
        let result = infer_exclusions(&population_of(&episodes, &catalog), &PARAMS);

        assert_eq!(result.edges.len(), 1);
        let edge = &result.edges[0];
        assert_eq!((edge.pair.a, edge.pair.b, edge.pair.completers_a, edge.pair.completers_b), (51, 52, 6, 6));
        assert!((edge.pair.expected - 3.0).abs() < 1e-12, "6 * 6 / 12 eligible, got {}", edge.pair.expected);
        assert_eq!(edge.evidence.len(), 6, "three completers per side");
        assert!(
            edge.evidence[0].text.ends_with("(race 1, class 1, level 5): completed 51, never took 52"),
            "{}",
            edge.evidence[0].text
        );
    }

    #[test]
    fn quests_behind_different_hidden_gates_are_not_compared() {
        // Different profession skills explain the split on their own.
        let quests = vec![
            catalog_quest(8),
            catalog_quest(51),
            CatalogQuest {
                required_skill: Some((171, 1)),
                ..catalog_quest(52)
            },
        ];
        assert_eq!(pairs(&split(1), quests), []);
    }

    #[test]
    fn races_that_each_did_their_own_quest_are_expected_to_have_nothing_in_common() {
        assert_eq!(pairs(&split(3), plain_quests()), []);
    }

    #[test]
    fn the_pair_is_dropped_once_a_character_held_both_at_the_same_time() {
        let mut episodes = split(1);
        episodes.push(session(
            "both",
            SessionSpec {
                initial: vec![8],
                accepts: vec![(10.0, 51), (20.0, 52)],
                ..SessionSpec::default()
            },
        ));
        assert_eq!(pairs(&episodes, plain_quests()), []);
    }

    #[test]
    fn name_stems_cut_at_the_first_separator_and_need_four_characters() {
        assert_eq!(name_stem(Some("Camping 101: Alchemy")).as_deref(), Some("camping 101"));
        assert_eq!(name_stem(Some("The Tome - Part 2: Ink")).as_deref(), Some("the tome"));
        assert_eq!(name_stem(Some("Ore: Mining")), None);
        assert_eq!(name_stem(None), None);
    }

    #[test]
    fn maximal_cliques_are_found_in_recursion_order() {
        // 0-1-2 form a triangle, 2-3 an edge, 4 is isolated.
        let edges = [(0, 1), (0, 2), (1, 2), (2, 3)];
        let adjacent = |a: usize, b: usize| edges.contains(&(a.min(b), a.max(b)));

        assert_eq!(maximal_cliques(5, adjacent), [vec![0, 1, 2], vec![2, 3]]);
    }
}
