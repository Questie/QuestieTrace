//! Delay distributions around turn-ins at a known giver (reports/handoff-timing.json), to choose
//! the hand-off windows from data rather than by guess.

use super::detect::{HandoffParams, TurnIn};
use indexmap::IndexMap;
use serde::Serialize;

/// Upper bin edges in seconds; one more open bin follows the last.
const EDGES: [f64; 15] = [0.1, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0];

#[derive(Default)]
pub struct TimingHistogram {
    turn_ins: usize,
    with_giver: usize,
    next_detail: Vec<f64>,
    previous_list: Vec<f64>,
    next_list: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingReport<'a> {
    generated_at: &'a str,
    params: HandoffParams,
    turn_ins: usize,
    turn_ins_with_giver: usize,
    next_detail_at_same_giver: Distribution,
    previous_list_at_same_giver: Distribution,
    next_list_at_same_giver: Distribution,
}

/// Quantiles are absent when there are no values.
#[derive(Serialize)]
struct Distribution {
    count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    p10: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p50: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p90: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    p99: Option<f64>,
    /// "<edge" -> values from the previous edge (or 0) up to this one.
    bins: IndexMap<String, usize>,
}

impl TimingHistogram {
    pub fn add(&mut self, turn_in: &TurnIn) {
        self.turn_ins += 1;
        if turn_in.giver.is_none() {
            return;
        }
        self.with_giver += 1;
        self.next_detail.extend(turn_in.timing.next_detail);
        self.previous_list.extend(turn_in.timing.previous_list);
        self.next_list.extend(turn_in.timing.next_list);
    }

    pub fn report<'a>(&self, generated_at: &'a str, params: HandoffParams) -> TimingReport<'a> {
        TimingReport {
            generated_at,
            params,
            turn_ins: self.turn_ins,
            turn_ins_with_giver: self.with_giver,
            next_detail_at_same_giver: describe(&self.next_detail),
            previous_list_at_same_giver: describe(&self.previous_list),
            next_list_at_same_giver: describe(&self.next_list),
        }
    }
}

fn describe(values: &[f64]) -> Distribution {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let quantile = |q: f64| {
        let last = sorted.len().checked_sub(1)?;
        let value = sorted[((q * sorted.len() as f64).floor() as usize).min(last)];
        // Rounded to milliseconds through the decimal text, like the TS `Number(x.toFixed(3))`.
        Some(format!("{value:.3}").parse::<f64>().expect("a fixed-point number parses"))
    };
    let mut bins = IndexMap::new();
    let mut lower = 0.0;
    for edge in EDGES.into_iter().chain([f64::INFINITY]) {
        let label = if edge.is_finite() { format!("<{edge}") } else { "<Infinity".to_owned() };
        bins.insert(label, sorted.iter().filter(|&&value| value >= lower && value < edge).count());
        lower = edge;
    }
    Distribution {
        count: sorted.len(),
        p10: quantile(0.1),
        p50: quantile(0.5),
        p90: quantile(0.9),
        p99: quantile(0.99),
        bins,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_take_the_value_at_the_floor_rank_rounded_to_milliseconds_and_bins_are_half_open() {
        // Unsorted on purpose; ascending they are 0.1, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6, 0.7, 1.23456, 700.
        let values = [0.7, 0.1, 1.23456, 0.25, 0.3, 700.0, 0.4, 0.2, 0.5, 0.6];
        let distribution = describe(&values);
        // Ranks floor(q * 10): 1, 5, 9, 9.
        let quantiles = (distribution.p10, distribution.p50, distribution.p90, distribution.p99);
        assert_eq!(quantiles, (Some(0.2), Some(0.5), Some(700.0), Some(700.0)));
        let bins: Vec<(&str, usize)> = distribution
            .bins
            .iter()
            .filter(|(_, &count)| count > 0)
            .map(|(label, &count)| (label.as_str(), count))
            .collect();
        assert_eq!(bins, [("<0.25", 2), ("<0.5", 3), ("<1", 3), ("<1.5", 1), ("<Infinity", 1)]);
        assert_eq!(describe(&[1.23456]).p50, Some(1.235));
        assert_eq!(describe(&[]).p50, None);
    }
}
