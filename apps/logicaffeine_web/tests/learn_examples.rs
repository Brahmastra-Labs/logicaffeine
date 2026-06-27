//! Exhaustive integrity audit of EVERY example in the Learn Logic curriculum.
//!
//! The learn section (`assets/<era>/<module>/{ex_*,sec_*}.json`, loaded by
//! [`ContentEngine`]) ships hundreds of hand-authored teaching examples:
//!
//!   * **Exercises** (`ex_*.json`): `translation`, `multiple_choice`, `ambiguity`.
//!   * **Section content** (`sec_*.json`): `paragraph`, `definition`, `example`,
//!     `symbols`, `quiz` blocks.
//!
//! These are the IP of the learning product. A single wrong `correct` index, a
//! duplicated distractor, a colliding id, or a translation that no longer
//! compiles is a user-visible embarrassment — and before this harness NOTHING
//! asserted these invariants. This file pins them, exhaustively, so any drift
//! (a lexicon edit, a parser tweak, a careless content paste) fails CI.
//!
//! Every test accumulates ALL violations and reports them together, so one run
//! tells you the full blast radius rather than failing on the first finding.
//!
//! Layers (cheapest → most semantic):
//!   1. `every_exercise_id_is_unique`     — ids are honest identifiers (progress + SM-2 key on them).
//!   2. `every_exercise_is_structurally_sound` — per-type field/option/index invariants.
//!   3. `every_section_block_is_sound`     — lesson prose/quiz/example/symbol invariants.
//!   4. `every_exercise_generates_a_challenge` — the SAME engine path the UI runs:
//!      templates fill, translations `compile()`, ambiguities scope, MC indices land in range.

use std::collections::HashMap;

use logicaffeine_web::content::{ContentBlock, ContentEngine, ExerciseConfig, ExerciseType};
use logicaffeine_web::generator::{AnswerType, Generator};
use rand::rngs::StdRng;
use rand::SeedableRng;

/// Walk every exercise as `(era_id, module_id, &ExerciseConfig)`.
fn for_each_exercise(engine: &ContentEngine, mut f: impl FnMut(&str, &str, &ExerciseConfig)) {
    for era in engine.eras() {
        for module in &era.modules {
            for ex in &module.exercises {
                f(&era.meta.id, &module.meta.id, ex);
            }
        }
    }
}

fn loc(era: &str, module: &str, id: &str) -> String {
    format!("{era}/{module}/{id}")
}

// ---------------------------------------------------------------------------
// Layer 1 — IDs are unique. Progress (`progress.rs`) and SM-2 review
// (`review.rs`) key by the BARE exercise id, so a reused id silently merges two
// distinct exercises' completion + scheduling. Ids must be unique both within a
// module (navigation / `get_exercise`) and globally (the progress keyspace).
// ---------------------------------------------------------------------------

#[test]
fn every_exercise_id_is_unique() {
    let engine = ContentEngine::new();

    let mut per_module: HashMap<(String, String, String), usize> = HashMap::new();
    let mut global: HashMap<String, Vec<String>> = HashMap::new();

    for_each_exercise(&engine, |era, module, ex| {
        *per_module
            .entry((era.to_string(), module.to_string(), ex.id.clone()))
            .or_insert(0) += 1;
        global
            .entry(ex.id.clone())
            .or_default()
            .push(format!("{era}/{module}"));
    });

    let mut violations = Vec::new();

    let mut within: Vec<_> = per_module
        .iter()
        .filter(|(_, &n)| n > 1)
        .map(|((era, module, id), n)| format!("  WITHIN-MODULE: {era}/{module} id `{id}` used {n}×"))
        .collect();
    within.sort();
    violations.extend(within);

    let mut cross: Vec<_> = global
        .iter()
        .filter_map(|(id, mods)| {
            // De-dup module list: a within-module dup already counted above.
            let mut uniq: Vec<&String> = mods.iter().collect();
            uniq.sort();
            uniq.dedup();
            if uniq.len() > 1 {
                Some(format!(
                    "  CROSS-MODULE: id `{id}` appears in {} modules: {:?}",
                    uniq.len(),
                    uniq
                ))
            } else {
                None
            }
        })
        .collect();
    cross.sort();
    violations.extend(cross);

    assert!(
        violations.is_empty(),
        "\n{} exercise id collision(s) — these corrupt progress + spaced repetition:\n{}\n",
        violations.len(),
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Layer 2 — structural soundness of every exercise.
// ---------------------------------------------------------------------------

#[test]
fn every_exercise_is_structurally_sound() {
    let engine = ContentEngine::new();
    let mut v = Vec::new();

    for_each_exercise(&engine, |era, module, ex| {
        let at = loc(era, module, &ex.id);

        if ex.id.trim().is_empty() {
            v.push(format!("{at}: empty id"));
        }
        if ex.prompt.trim().is_empty() {
            v.push(format!("{at}: empty prompt"));
        }

        match ex.exercise_type {
            ExerciseType::MultipleChoice => {
                match &ex.options {
                    None => v.push(format!("{at}: multiple_choice with no options")),
                    Some(opts) => {
                        if opts.len() < 2 {
                            v.push(format!("{at}: only {} option(s) (need ≥2)", opts.len()));
                        }
                        for (i, o) in opts.iter().enumerate() {
                            if o.trim().is_empty() {
                                v.push(format!("{at}: option {i} is empty/blank"));
                            }
                        }
                        // Distinct AFTER trimming — a distractor that equals
                        // another option (even modulo whitespace) shrinks the
                        // real choice count and can duplicate the answer text.
                        let mut seen = std::collections::HashSet::new();
                        for o in opts {
                            let key = o.trim();
                            if !seen.insert(key) {
                                v.push(format!(
                                    "{at}: duplicate option {:?} in {:?}",
                                    key, opts
                                ));
                            }
                        }
                        match ex.correct {
                            None => v.push(format!("{at}: multiple_choice with no `correct` index")),
                            Some(c) if c >= opts.len() => v.push(format!(
                                "{at}: correct index {c} out of bounds (len {})",
                                opts.len()
                            )),
                            Some(_) => {}
                        }
                    }
                }
                match &ex.explanation {
                    None => v.push(format!("{at}: multiple_choice with no explanation")),
                    Some(e) if e.trim().is_empty() => {
                        v.push(format!("{at}: multiple_choice with blank explanation"))
                    }
                    Some(_) => {}
                }
            }
            ExerciseType::Translation => {
                match &ex.template {
                    None => v.push(format!("{at}: translation with no template")),
                    Some(t) if t.trim().is_empty() => {
                        v.push(format!("{at}: translation with blank template"))
                    }
                    Some(_) => {}
                }
            }
            ExerciseType::Ambiguity => {
                if ex.template.as_deref().map(str::trim).unwrap_or("").is_empty() {
                    v.push(format!("{at}: ambiguity with no template"));
                }
            }
        }
    });

    assert!(
        v.is_empty(),
        "\n{} structural violation(s) in learn exercises:\n  {}\n",
        v.len(),
        v.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Layer 3 — section content blocks (the lesson prose, examples, quizzes).
// ---------------------------------------------------------------------------

#[test]
fn every_section_block_is_sound() {
    let engine = ContentEngine::new();
    let mut v = Vec::new();

    for era in engine.eras() {
        for module in &era.modules {
            for sec in &module.sections {
                let base = format!("{}/{}/{}", era.meta.id, module.meta.id, sec.id);
                if sec.title.trim().is_empty() {
                    v.push(format!("{base}: section with empty title"));
                }
                for (i, block) in sec.content.iter().enumerate() {
                    let at = format!("{base} block#{i}");
                    match block {
                        ContentBlock::Paragraph { text } => {
                            if text.trim().is_empty() {
                                v.push(format!("{at}: empty paragraph"));
                            }
                        }
                        ContentBlock::Definition { term, definition } => {
                            if term.trim().is_empty() {
                                v.push(format!("{at}: definition with empty term"));
                            }
                            if definition.trim().is_empty() {
                                v.push(format!("{at}: definition `{term}` with empty body"));
                            }
                        }
                        ContentBlock::Example {
                            title,
                            premises,
                            conclusion,
                            ..
                        } => {
                            if title.trim().is_empty() {
                                v.push(format!("{at}: example with empty title"));
                            }
                            for (j, p) in premises.iter().enumerate() {
                                if p.trim().is_empty() {
                                    v.push(format!("{at}: example `{title}` premise {j} empty"));
                                }
                            }
                            if let Some(c) = conclusion {
                                if c.trim().is_empty() {
                                    v.push(format!("{at}: example `{title}` empty conclusion"));
                                }
                            }
                        }
                        ContentBlock::Symbols { title, symbols } => {
                            if title.trim().is_empty() {
                                v.push(format!("{at}: symbols block with empty title"));
                            }
                            for s in symbols {
                                if s.symbol.trim().is_empty() {
                                    v.push(format!("{at}: symbol with empty glyph"));
                                }
                                if s.name.trim().is_empty() {
                                    v.push(format!("{at}: symbol `{}` with empty name", s.symbol));
                                }
                                if s.meaning.trim().is_empty() {
                                    v.push(format!("{at}: symbol `{}` with empty meaning", s.symbol));
                                }
                            }
                        }
                        ContentBlock::Quiz {
                            question,
                            options,
                            correct,
                            explanation,
                        } => {
                            if question.trim().is_empty() {
                                v.push(format!("{at}: quiz with empty question"));
                            }
                            if options.len() < 2 {
                                v.push(format!("{at}: quiz with {} option(s)", options.len()));
                            }
                            let mut seen = std::collections::HashSet::new();
                            for (j, o) in options.iter().enumerate() {
                                if o.trim().is_empty() {
                                    v.push(format!("{at}: quiz option {j} empty"));
                                }
                                if !seen.insert(o.trim()) {
                                    v.push(format!("{at}: quiz duplicate option {:?}", o.trim()));
                                }
                            }
                            if *correct >= options.len() {
                                v.push(format!(
                                    "{at}: quiz correct index {correct} out of bounds (len {})",
                                    options.len()
                                ));
                            }
                            if let Some(e) = explanation {
                                if e.trim().is_empty() {
                                    v.push(format!("{at}: quiz with blank explanation"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    assert!(
        v.is_empty(),
        "\n{} section content violation(s):\n  {}\n",
        v.len(),
        v.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Layer 3.5 — no displayed string is over-escaped LaTeX.
//
// Options/prompts render through `MixedText` → `KatexSpan` → `katex.render`. In
// LaTeX a literal `\\` is the line-break macro; `\\cdot` therefore renders as a
// line break followed by the bare letters "cdot" (verified against katex 0.16.9).
// A correctly-escaped command is a SINGLE backslash (`\cdot`). So inside any
// displayed string, the byte sequence `\` `\` immediately followed by an ASCII
// letter is the unmistakable signature of a double-escaped command — a broken
// formula. (A genuine line break is `\\` + space/brace and is left alone.)
// ---------------------------------------------------------------------------

fn over_escape_offense(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b'\\' && b[i + 1] == b'\\' && b[i + 2].is_ascii_alphabetic() {
            let end = (i + 12).min(s.len());
            return Some(s[i..end].to_string());
        }
        i += 1;
    }
    None
}

/// Record an over-escape offense for `s` under `label` (free fn so callers
/// borrow `v` one at a time — no `&mut v`-capturing closures).
fn flag_over_escape(v: &mut Vec<String>, label: String, s: &str) {
    if let Some(snip) = over_escape_offense(s) {
        v.push(format!("{label}: over-escaped LaTeX `{snip}…`"));
    }
}

/// LaTeX commands the curriculum's formulas use. Inside `$…$`, one of these
/// appearing WITHOUT a leading backslash (e.g. `(exists x)`) renders as the bare
/// letters "exists" instead of `∃` — the mirror image of the over-escape bug.
const LATEX_COMMANDS: &[&str] = &[
    "cdot", "vee", "wedge", "lozenge", "square", "sim", "supset", "equiv", "exists", "forall",
    "underline", "neg", "land", "lor", "subset", "supseteq",
];

/// Find a bare (backslash-less) LaTeX command word inside a `$…$` math span.
fn bare_command_offense(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut in_math = false;
    let mut k = 0;
    while k < chars.len() {
        if chars[k] == '$' {
            in_math = !in_math;
            k += 1;
            continue;
        }
        if in_math && chars[k].is_ascii_alphabetic() {
            let prev = if k > 0 { Some(chars[k - 1]) } else { None };
            let start = k;
            while k < chars.len() && chars[k].is_ascii_alphabetic() {
                k += 1;
            }
            let word: String = chars[start..k].iter().collect();
            if LATEX_COMMANDS.contains(&word.as_str()) && prev != Some('\\') {
                return Some(word);
            }
            continue;
        }
        k += 1;
    }
    None
}

fn flag_bare_command(v: &mut Vec<String>, label: String, s: &str) {
    if let Some(w) = bare_command_offense(s) {
        v.push(format!("{label}: bare LaTeX command `{w}` (missing backslash → renders as literal text)"));
    }
}

#[test]
fn no_displayed_string_has_bare_latex_command() {
    let engine = ContentEngine::new();
    let mut v = Vec::new();
    for_each_exercise(&engine, |era, module, ex| {
        let at = loc(era, module, &ex.id);
        flag_bare_command(&mut v, format!("{at} [prompt]"), &ex.prompt);
        if let Some(opts) = &ex.options {
            for (i, o) in opts.iter().enumerate() {
                flag_bare_command(&mut v, format!("{at} [option {i}]"), o);
            }
        }
        if let Some(e) = &ex.explanation {
            flag_bare_command(&mut v, format!("{at} [explanation]"), e);
        }
    });
    assert!(
        v.is_empty(),
        "\n{} bare LaTeX command(s) (render as literal letters, e.g. \"exists\" for ∃):\n  {}\n",
        v.len(),
        v.join("\n  ")
    );
}

#[test]
fn no_displayed_string_is_over_escaped() {
    let engine = ContentEngine::new();
    let mut v = Vec::new();

    for_each_exercise(&engine, |era, module, ex| {
        let at = loc(era, module, &ex.id);
        flag_over_escape(&mut v, format!("{at} [prompt]"), &ex.prompt);
        if let Some(opts) = &ex.options {
            for (i, o) in opts.iter().enumerate() {
                flag_over_escape(&mut v, format!("{at} [option {i}]"), o);
            }
        }
        if let Some(e) = &ex.explanation {
            flag_over_escape(&mut v, format!("{at} [explanation]"), e);
        }
        if let Some(h) = &ex.hint {
            flag_over_escape(&mut v, format!("{at} [hint]"), h);
        }
    });

    for era in engine.eras() {
        for module in &era.modules {
            for sec in &module.sections {
                let base = format!("{}/{}/{}", era.meta.id, module.meta.id, sec.id);
                for (i, block) in sec.content.iter().enumerate() {
                    let at = format!("{base} block#{i}");
                    match block {
                        ContentBlock::Paragraph { text } => {
                            flag_over_escape(&mut v, at, text)
                        }
                        ContentBlock::Definition { term, definition } => {
                            flag_over_escape(&mut v, format!("{at} term"), term);
                            flag_over_escape(&mut v, at, definition);
                        }
                        ContentBlock::Example {
                            title,
                            premises,
                            conclusion,
                            ..
                        } => {
                            flag_over_escape(&mut v, format!("{at} title"), title);
                            for p in premises {
                                flag_over_escape(&mut v, format!("{at} premise"), p);
                            }
                            if let Some(c) = conclusion {
                                flag_over_escape(&mut v, format!("{at} conclusion"), c);
                            }
                        }
                        ContentBlock::Symbols { title, symbols } => {
                            flag_over_escape(&mut v, format!("{at} title"), title);
                            for s in symbols {
                                flag_over_escape(&mut v, format!("{at} meaning"), &s.meaning);
                                if let Some(ex) = &s.example {
                                    flag_over_escape(&mut v, format!("{at} example"), ex);
                                }
                            }
                        }
                        ContentBlock::Quiz {
                            question,
                            options,
                            explanation,
                            ..
                        } => {
                            flag_over_escape(&mut v, format!("{at} question"), question);
                            for o in options {
                                flag_over_escape(&mut v, format!("{at} option"), o);
                            }
                            if let Some(e) = explanation {
                                flag_over_escape(&mut v, format!("{at} explanation"), e);
                            }
                        }
                    }
                }
            }
        }
    }

    assert!(
        v.is_empty(),
        "\n{} over-escaped LaTeX string(s) (render as broken `\\\\cmd` line-break + literal letters):\n  {}\n",
        v.len(),
        v.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Layer 4 — every exercise actually generates a Challenge on the real engine
// path (lexicon fill + `compile()`), the same code the lesson UI runs. Catches
// lexicon/parser drift that would leave a "Run" button erroring in production.
// ---------------------------------------------------------------------------

#[test]
fn every_exercise_generates_a_challenge() {
    let engine = ContentEngine::new();
    let generator = Generator::new();
    let mut v = Vec::new();

    for_each_exercise(&engine, |era, module, ex| {
        let at = loc(era, module, &ex.id);

        // Try a handful of seeds: rejection sampling over the lexicon means a
        // single unlucky seed can fail to draw a compilable sentence even for a
        // perfectly good exercise. The UI reseeds on every visit, so "generable
        // under some seed" is the honest contract.
        let mut challenge = None;
        for seed in 0u64..6 {
            let mut rng = StdRng::seed_from_u64(seed.wrapping_mul(0x9E37) ^ ex.id.len() as u64);
            if let Some(c) = generator.generate(ex, &mut rng) {
                challenge = Some(c);
                break;
            }
        }

        let Some(c) = challenge else {
            v.push(format!("{at}: never generated a challenge in 6 seeds"));
            return;
        };

        if c.sentence.trim().is_empty() {
            v.push(format!("{at}: generated an empty sentence"));
        }
        if c.sentence.contains('{') || c.sentence.contains('}') {
            v.push(format!("{at}: unfilled template slot in `{}`", c.sentence));
        }

        match &c.answer {
            AnswerType::FreeForm { golden_logic } => {
                if golden_logic.trim().is_empty() {
                    v.push(format!("{at}: translation produced empty golden logic"));
                }
            }
            AnswerType::MultipleChoice {
                options,
                correct_index,
            } => {
                if *correct_index >= options.len() {
                    v.push(format!(
                        "{at}: MC challenge correct_index {correct_index} ≥ options {}",
                        options.len()
                    ));
                }
            }
            AnswerType::Ambiguity { readings } => {
                if readings.is_empty() {
                    v.push(format!("{at}: ambiguity produced zero readings"));
                }
            }
        }
    });

    assert!(
        v.is_empty(),
        "\n{} generation failure(s) on the live engine path:\n  {}\n",
        v.len(),
        v.join("\n  ")
    );
}
