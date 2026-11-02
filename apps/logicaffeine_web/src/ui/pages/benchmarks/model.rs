//! The Programming Language tab's model: which bars a chart draws, in what order, and the headline figures. Pure, so
//! the page's tests drive it without rendering.

use super::*;

/// The order groups appear in: the LOGOS family first, then its competition by kind.
pub(super) const TIER_ORDER: [&str; 4] = ["logos", "systems", "managed", "transpiled"];

/// Why a language has no time at a size.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Missing {
    /// It ran into the run's timeout, in milliseconds.
    TimedOut(f64),
    /// It cannot run at this size — a runtime limit, and the reason.
    NotRun(String),
}

/// One bar: a language's measurement (a median time, a speedup), or why it has none.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Bar {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) value: Option<f64>,
    pub(super) missing: Option<Missing>,
}

/// The bars of one tier.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChartGroup {
    pub(super) tier: String,
    pub(super) bars: Vec<Bar>,
}

/// Which way a chart sorts the competition within a group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Better {
    /// Smaller is better (a time): fastest first.
    Lower,
    /// Larger is better (a speedup): fastest first.
    Higher,
}

/// Bars grouped by tier: the LOGOS family first in registry order (Rust, C, JIT — the family reads as one, whatever
/// each tier scored), then Systems, Managed and Transpiled, each best first. A bar without a value comes last in its
/// group. A tier the page does not know goes to an `other` group at the end instead of vanishing.
pub(super) fn group_bars(
    languages: &[Language],
    bars: impl Fn(&Language) -> Option<Bar>,
    better: Better,
) -> Vec<ChartGroup> {
    let mut groups = Vec::new();
    for tier in TIER_ORDER.iter().copied().chain(std::iter::once("other")) {
        let mut members: Vec<Bar> = languages
            .iter()
            .filter(|l| group_of(l) == tier)
            .filter_map(&bars)
            .collect();
        if tier != "logos" {
            members.sort_by(|a, b| match (a.value, b.value) {
                (Some(x), Some(y)) => {
                    let order = x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
                    if better == Better::Lower {
                        order
                    } else {
                        order.reverse()
                    }
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
        }
        if !members.is_empty() {
            groups.push(ChartGroup {
                tier: tier.to_string(),
                bars: members,
            });
        }
    }
    groups
}

/// Every language's bar at one size: its median time, or — when it has none — the timeout it ran into or the limit that
/// keeps it from running at this size, drawn last in its group with the reason. A language with none of the three did
/// not run at this size for another reason and is not drawn.
pub(super) fn chart_groups(
    languages: &[Language],
    timings: Option<&HashMap<String, TimingResult>>,
    timed_out: Option<&HashMap<String, f64>>,
    limits: Option<&HashMap<String, String>>,
) -> Vec<ChartGroup> {
    let bar = |lang: &Language| -> Option<Bar> {
        let median = timings.and_then(|t| t.get(&lang.id)).map(|t| t.median_ms);
        let missing = if median.is_some() {
            None
        } else if let Some(ms) = timed_out.and_then(|t| t.get(&lang.id)) {
            Some(Missing::TimedOut(*ms))
        } else {
            limits
                .and_then(|l| l.get(&lang.id))
                .map(|why| Missing::NotRun(why.clone()))
        };
        (median.is_some() || missing.is_some()).then(|| Bar {
            id: lang.id.clone(),
            label: lang.label.clone(),
            value: median,
            missing,
        })
    };
    group_bars(languages, bar, Better::Lower)
}

/// The summary chart: every language's geometric-mean speed against C, the LOGOS family first.
pub(super) fn summary_groups(data: &BenchmarkData) -> Vec<ChartGroup> {
    let bar = |lang: &Language| -> Option<Bar> {
        data.summary
            .geometric_mean_speedup_vs_c
            .get(&lang.id)
            .map(|&v| Bar {
                id: lang.id.clone(),
                label: lang.label.clone(),
                value: Some(v),
                missing: None,
            })
    };
    group_bars(&data.languages, bar, Better::Higher)
}

/// The chart group a language is drawn in: its tier, or `other` for a tier the page does not know.
fn group_of(lang: &Language) -> &str {
    if TIER_ORDER.contains(&lang.tier.as_str()) {
        lang.tier.as_str()
    } else {
        "other"
    }
}

/// One headline card: a LOGOS tier's geometric-mean speed against C over every benchmark, the same over the benchmarks
/// where every language runs the same algorithm, and how many benchmarks each covers.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Headline {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) vs_c: Option<f64>,
    pub(super) same_algorithm: Option<f64>,
    pub(super) coverage: Option<(u32, u32)>,
}

/// One headline per LOGOS tier, in registry order — never one number chosen across tiers.
pub(super) fn tier_headlines(data: &BenchmarkData) -> Vec<Headline> {
    data.languages
        .iter()
        .filter(|l| l.is_logos())
        .map(|l| Headline {
            id: l.id.clone(),
            label: l.label.clone(),
            vs_c: data.summary.geometric_mean_speedup_vs_c.get(&l.id).copied(),
            same_algorithm: data.summary.same_algorithm_speedup_vs_c.get(&l.id).copied(),
            coverage: data
                .summary
                .coverage
                .get(&l.id)
                .map(|c| (c.benchmarks, c.of)),
        })
        .collect()
}
