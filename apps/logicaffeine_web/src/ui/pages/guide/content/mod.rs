//! Embedded guide content for the Programmer's Guide page.
//!
//! Every section of the Programmer's Guide as Rust constants: WASM cannot read files at runtime,
//! so the content is embedded at compile time.

use crate::ui::components::code_editor::Language;

/// Mode for code examples - determines how "Run" executes them
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExampleMode {
    /// Logic mode: compile to First-Order Logic (FOL)
    Logic,
    /// Imperative mode: execute via WASM interpreter
    Imperative,
    /// Theorem mode: run the proof engine (`compile_theorem_for_ui`) — the
    /// output is the kernel-certified verdict.
    Theorem,
    /// Web mode: an LGX page compiled to WebAssembly in the browser and shown
    /// live in a sandboxed frame — the preview the Studio's Web tab runs
    /// (`web_host::assemble_preview`). A web example proves itself with its
    /// own `## Test` blocks rather than with printed output.
    Web,
}

impl ExampleMode {
    /// The editor language the example is read in: LOGOS code (a program or a page), or the
    /// English of sentences and theorems — as the Studio reads a `.logos` and a `.logic` file.
    pub fn language(self) -> Language {
        match self {
            ExampleMode::Imperative | ExampleMode::Web => Language::Logos,
            ExampleMode::Logic | ExampleMode::Theorem => Language::Logic,
        }
    }
}

/// How an example behaves in the browser playground. The single source of
/// truth shared by the Run-button affordance, the badge, AND the test harness
/// partition (`tests/guide_examples.rs`) — the three can never disagree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Runnability {
    /// Executes in the browser playground.
    Playground,
    /// The runtime only exists in compiled Rust; the UI renders a badge with
    /// the reason instead of a Run button that would fail.
    CompiledOnly { reason: &'static str },
    /// The example fails ON PURPOSE — the displayed Socratic error is the
    /// content. The full diagnostic text is locked exactly.
    ExpectedError { message: &'static str },
}

/// A code example within a section
#[derive(Clone, Debug, PartialEq)]
pub struct CodeExample {
    pub id: &'static str,
    pub label: &'static str,
    pub mode: ExampleMode,
    pub runnability: Runnability,
    pub expected_output: Option<&'static str>,
    /// Predict-before-reveal (Muller/POE): the block renders with its locked
    /// expectation HIDDEN and asks the reader to commit first. Only legal with
    /// something locked to predict (enforced by `guide_pedagogy.rs`).
    pub predict_first: bool,
    pub code: &'static str,
}

/// A section of the guide
#[derive(Clone, Debug)]
pub struct Section {
    pub id: &'static str,
    pub number: u8,
    pub title: &'static str,
    pub part: &'static str,
    /// The headache before the aspirin (Meyer/Dewey): a concrete problem the
    /// reader can't yet solve, rendered as the section's opener — before any
    /// definition gets to speak. Empty until authored; teaching sections are
    /// floor-ratcheted toward full coverage.
    pub hook: &'static str,
    /// "After this section you can …" — observable Bloom-verb behaviors
    /// (Mager). 2..=5 once authored.
    pub objectives: &'static [&'static str],
    pub content: &'static str,
    /// Imported beliefs confronted in the reader's voice, each backed by a
    /// runnable evidence example in this same section (Muller).
    pub misconceptions: &'static [Misconception],
    pub examples: &'static [CodeExample],
    pub exercises: &'static [Exercise],
    /// End-of-section "Check yourself" free-recall questions, reveal-style
    /// (Roediger & Karpicke). Self-marks feed the SM-2 review deck.
    pub recap: &'static [RecapQuestion],
}

/// "Coming from X you'd expect A; LOGOS does B because C" — stated first in
/// the reader's voice, then proven by a runnable example.
#[derive(Clone, Debug, PartialEq)]
pub struct Misconception {
    pub id: &'static str,
    /// The belief the reader arrives with, in their voice.
    pub expectation: &'static str,
    /// What LOGOS does and WHY (the because is not optional).
    pub reality: &'static str,
    /// Id of the evidence example IN THIS SECTION — predict-first or
    /// ExpectedError, so the reader commits before the proof lands.
    pub evidence: &'static str,
}

/// One end-of-section retrieval prompt: question → recall → reveal →
/// self-mark. Free recall, never multiple-choice (recognition ≠ production).
#[derive(Clone, Debug, PartialEq)]
pub struct RecapQuestion {
    pub id: &'static str,
    pub question: &'static str,
    pub answer: &'static str,
}

/// The rung an exercise occupies on the worked→completion→free ladder
/// (Sweller/Renkl fading). Within a section, support only ever fades.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExerciseKind {
    /// The worked solution minus 1–3 `HOLE_MARKER` holes — the faded middle
    /// rung between reading an example and writing from scratch.
    Completion,
    /// Free practice of one (max two) concepts.
    Practice,
    /// Multi-concept stretch — also the checkpoint interleaving vehicle.
    Challenge,
}

/// The canonical hole token for Completion starters: greppable, never valid
/// LOGOS, unmistakably a blank to fill.
pub const HOLE_MARKER: &str = "???";

/// A "now you try" challenge: the reader writes code (Imperative — graded by
/// exact program output), a page (Web — graded by its own `## Test` blocks)
/// or FOL (Logic — graded structurally by the Learn grader), right in the guide.
#[derive(Clone, Debug, PartialEq)]
pub struct Exercise {
    pub id: &'static str,
    /// Where this sits on the fading ladder.
    pub kind: ExerciseKind,
    /// The LANGUAGE_SURFACE features this practices (deliberate practice
    /// targets the hard part; checkpoints interleave via these).
    pub concepts: &'static [&'static str],
    /// The task, stated to the reader.
    pub prompt: &'static str,
    pub mode: ExampleMode,
    /// The graded target: exact output lines (Imperative) or the expected FOL. Empty for a Web
    /// exercise, which is graded by the `## Test` blocks its page carries.
    pub expected: &'static str,
    /// Pre-filled editor contents.
    pub starter: &'static str,
    /// 1–3 progressive hints, disclosed on demand: orient, then narrow —
    /// never the answer (Bloom's corrective tutor).
    pub hints: &'static [&'static str],
    /// The hidden answer key — the self-check lock proves it grades correct.
    pub reference: &'static str,
}

/// All guide sections organized by part
mod part1;
mod part2;
mod part3;
mod part4;
mod part5;
mod part6;
mod part7;
mod part8;

use std::sync::LazyLock;

/// All guide sections in display order, concatenated from the per-part modules.
pub static SECTIONS: LazyLock<Vec<Section>> = LazyLock::new(|| {
    [
        part1::PART_1,
        part2::PART_2,
        part3::PART_3,
        part4::PART_4,
        part5::PART_5,
        part6::PART_6,
        part7::PART_7,
        part8::PART_8,
    ]
    .concat()
});
