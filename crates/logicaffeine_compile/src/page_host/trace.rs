//! A scenario's run on one tier, as the page looked after every statement — and the comparison
//! of two such runs.

use super::snapshot::{Axis, DocSnapshot};
use serde::Serialize;

/// The page after one statement of a scenario.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceStep {
    /// Which top-level statement this follows: `0` is the page the program's own statements left
    /// (before the scenario's first statement), `k` the page after its `k`-th.
    pub statement: usize,
    /// Whether this tier executed the statement. A statement that only READS the page (an
    /// `Expect`, a `Let` of a page query) is executed by the reference tier; a tier that drives the
    /// page from outside has nothing to do for it, and says so here.
    pub executed: bool,
    pub snapshot: DocSnapshot,
}

/// Where a scenario stopped: the statement whose failure ended the run, and the failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stop {
    pub statement: usize,
    pub message: String,
}

/// One scenario's run on one tier: a snapshot per top-level statement, until it stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PageTrace {
    pub steps: Vec<TraceStep>,
    pub stopped: Option<Stop>,
}

/// Two tiers disagreeing about one axis, first at `statement`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AxisDivergence {
    pub axis: Axis,
    pub statement: usize,
    pub reference: String,
    pub candidate: String,
}

/// Everything two runs of one scenario disagree about, one entry per axis, each at the first
/// statement where that axis differs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Divergence {
    pub axes: Vec<AxisDivergence>,
}

impl Divergence {
    pub fn is_empty(&self) -> bool {
        self.axes.is_empty()
    }

    /// The first statement at which the runs disagree about anything.
    pub fn first_statement(&self) -> Option<usize> {
        self.axes.iter().map(|a| a.statement).min()
    }

    /// A report a person can act on: each axis, where it first differs, and how.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for a in &self.axes {
            out.push_str(&format!(
                "{} differs after statement {}:\n{}",
                a.axis.name(),
                a.statement,
                describe_difference(&a.reference, &a.candidate)
            ));
        }
        out
    }
}

impl PageTrace {
    /// The last step at or before `statement`.
    pub fn at(&self, statement: usize) -> Option<&TraceStep> {
        self.steps.iter().rev().find(|s| s.statement <= statement)
    }

    /// Where `candidate` disagrees with `self`, the reference.
    ///
    /// The runs are compared statement by statement, over the statements both reached. A
    /// reference stop at a statement the candidate did not execute (a page query that found no
    /// element, say) ends the comparison there: the candidate would have read the same page and
    /// failed the same way, since every earlier statement already agreed.
    pub fn compare(&self, candidate: &PageTrace) -> Divergence {
        let mut limit = usize::MAX;
        let mut axes: Vec<AxisDivergence> = Vec::new();
        if let Some(stop) = &self.stopped {
            let candidate_ran_it = candidate
                .steps
                .iter()
                .any(|s| s.statement == stop.statement && s.executed);
            if !candidate_ran_it {
                limit = stop.statement.saturating_sub(1);
            }
        }
        for axis in Axis::SNAPSHOT {
            let first = self
                .steps
                .iter()
                .filter(|s| s.statement <= limit)
                .filter_map(|r| {
                    let c = candidate
                        .steps
                        .iter()
                        .find(|c| c.statement == r.statement)?;
                    let (rv, cv) = (r.snapshot.axis(axis), c.snapshot.axis(axis));
                    (rv != cv).then_some((r.statement, rv, cv))
                })
                .next();
            if let Some((statement, reference, candidate)) = first {
                axes.push(AxisDivergence {
                    axis,
                    statement,
                    reference,
                    candidate,
                });
            }
        }
        let reference_stop = self.stopped.as_ref().filter(|s| s.statement <= limit);
        let candidate_stop = candidate.stopped.as_ref().filter(|s| s.statement <= limit);
        if reference_stop != candidate_stop {
            let render = |s: Option<&Stop>| match s {
                Some(s) => format!("stopped at statement {}: {}", s.statement, s.message),
                None => "ran to the end".to_string(),
            };
            let statement = [reference_stop, candidate_stop]
                .into_iter()
                .flatten()
                .map(|s| s.statement)
                .min()
                .unwrap_or(0);
            axes.push(AxisDivergence {
                axis: Axis::Stopped,
                statement,
                reference: render(reference_stop),
                candidate: render(candidate_stop),
            });
        }
        axes.sort_by_key(|a| (a.statement, a.axis));
        Divergence { axes }
    }
}

/// Two renderings of one axis, with the first difference pointed at — a document is long, and a
/// person looking for what differs should not have to find it.
pub fn describe_difference(reference: &str, candidate: &str) -> String {
    let common = reference
        .char_indices()
        .zip(candidate.chars())
        .take_while(|((_, a), b)| a == b)
        .last()
        .map(|((i, a), _)| i + a.len_utf8())
        .unwrap_or(0);
    let window = |s: &str| -> String {
        let start = s[..common.min(s.len())]
            .char_indices()
            .rev()
            .nth(40)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let rest: String = s[start..].chars().take(160).collect();
        let lead = if start > 0 { "…" } else { "" };
        format!("{lead}{rest}")
    };
    format!(
        "  reference: {:?}\n  candidate: {:?}\n  (first difference at byte {common})\n",
        window(reference),
        window(candidate)
    )
}
