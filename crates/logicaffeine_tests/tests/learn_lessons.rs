//! The Learn roadmap's lessons: concise, correct, checked against the compiler.
//!
//! Every section of the curriculum is one lesson on the /learn roadmap: a short teaching, two
//! examples, and one question that moves the learner on. Exercises (`ex_*.json`) are the optional
//! "more practice" of the lesson they name. This suite holds that shape and the content's truth:
//!
//!   1. Shape. Every lesson has at least two examples and exactly one question; the teaching is
//!      prose, definitions and symbol tables, short enough to read in a minute; lessons are numbered
//!      by their module's place on the roadmap.
//!   2. Truth. A formula shown beside a sentence is exactly what LOGOS prints for that sentence. A
//!      typed question's key is exactly what LOGOS prints, and the grader accepts it written the way
//!      a learner writes it. A choice question has one right option and options the grader tells
//!      apart. No hint gives its answer away, and no example is the question it prepares for.
//!   3. Copy. No text the Learn page can show carries an em dash, a semicolon, a dash between words,
//!      or TeX.
//!   4. Practice. Every exercise names a lesson of its own module, and no module asks the same
//!      exercise twice.
//!
//! Every test gathers all its violations and reports them together.

#![cfg(feature = "web-tests")]

use logicaffeine_web::content::{ContentBlock, ContentEngine, Era, ExerciseConfig, ExerciseType, LessonExample, LessonQuestion, Module, Section};
use logicaffeine_web::grader::check_answer;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// The lessons whose question is typed: the ones that teach writing a formula LOGOS prints.
const TYPED: &[&str] = &["1.4", "1.5", "6.1", "8.1", "8.2", "8.3", "8.4", "9.1", "9.2", "10.1", "11.2", "12.1"];

/// The most words a lesson's teaching runs to, and the most any one paragraph or definition does.
const TEACHING_WORDS: usize = 110;
const BLOCK_WORDS: usize = 50;
/// The most words in an example's note, a question's hint, and a question's explanation.
const NOTE_WORDS: usize = 45;
const HINT_WORDS: usize = 35;
const EXPLANATION_WORDS: usize = 60;

fn engine() -> &'static ContentEngine {
    ContentEngine::shared()
}

/// Every lesson with its era and module, in roadmap order.
fn lessons() -> Vec<(&'static Era, &'static Module, &'static Section)> {
    engine()
        .eras()
        .iter()
        .flat_map(|era| era.modules.iter().map(move |module| (era, module)))
        .flat_map(|(era, module)| module.sections.iter().map(move |section| (era, module, section)))
        .collect()
}

fn modules() -> Vec<&'static Module> {
    engine().eras().iter().flat_map(|era| era.modules.iter()).collect()
}

fn at(module: &Module, section: &Section) -> String {
    format!("{} {} ({})", section.id, section.title, module.meta.id)
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

fn report(what: &str, v: &[String]) {
    assert!(v.is_empty(), "\n{} {what}:\n  {}\n", v.len(), v.join("\n  "));
}

/// The formula as a learner types it: the strength LOGOS prints after a modal (`◇_{0.5}`) left out,
/// as the grader ignores it.
fn learner_form(key: &str) -> String {
    let mut out = String::new();
    let mut rest = key;
    while let Some(i) = rest.find("_{") {
        out.push_str(&rest[..i]);
        rest = match rest[i..].find('}') {
            Some(close) => &rest[i + close + 1..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The variable the key binds first, if it binds one.
fn first_bound_variable(key: &str) -> Option<String> {
    let at = key.find(['∀', '∃'])?;
    let var: String = key[at..].chars().skip(1).skip_while(|c| c.is_whitespace()).take_while(|c| c.is_alphanumeric()).collect();
    (!var.is_empty()).then_some(var)
}

/// `s` with every whole-word `from` replaced by `to`.
fn rename_word(s: &str, from: &str, to: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let from: Vec<char> = from.chars().collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&from) && (i == 0 || !word(chars[i - 1])) && chars.get(i + from.len()).map_or(true, |c| !word(*c)) {
            out.push_str(to);
            i += from.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The key with its first predicate renamed to one no lesson uses.
fn first_predicate_renamed(key: &str) -> Option<String> {
    let chars: Vec<char> = key.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_uppercase() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            if chars.get(i) == Some(&'(') {
                let name: String = chars[start..i].iter().collect();
                if !["HAB", "Past", "Future"].contains(&name.as_str()) {
                    return Some(format!("{}Qwerty{}", chars[..start].iter().collect::<String>(), chars[i..].iter().collect::<String>()));
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Every piece of punctuation the Learn page must not show, with a short excerpt around it.
fn punctuation_faults(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let space = |i: Option<usize>| i.map_or(true, |i| i >= chars.len() || chars[i].is_whitespace());
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let what = match chars[i] {
            '\u{2014}' | '\u{2015}' | '\u{2E3A}' | '\u{2E3B}' | '\u{FE58}' | '\u{FE31}' => Some("an em dash"),
            ';' | '\u{037E}' | '\u{FF1B}' | '\u{FE54}' => Some("a semicolon"),
            '\u{2013}' if space(i.checked_sub(1)) || space(Some(i + 1)) => Some("an en dash between words"),
            '-' => {
                let start = i;
                while i + 1 < chars.len() && chars[i + 1] == '-' {
                    i += 1;
                }
                (space(start.checked_sub(1)) && space(Some(i + 1))).then_some("a hyphen standing in for a dash")
            }
            _ => None,
        };
        if let Some(what) = what {
            let from = i.saturating_sub(25);
            let to = (i + 25).min(chars.len());
            found.push(format!("{what}: …{}…", chars[from..to].iter().collect::<String>()));
        }
        i += 1;
    }
    found
}

/// Every string a section shows, labelled by where it sits.
fn section_texts(section: &Section) -> Vec<(String, String)> {
    let mut out = vec![("title".to_string(), section.title.clone())];
    for (i, block) in section.content.iter().enumerate() {
        let at = |what: &str| format!("teaching #{i} {what}");
        match block {
            ContentBlock::Paragraph { text } => out.push((at("paragraph"), text.clone())),
            ContentBlock::Definition { term, definition } => {
                out.push((at("term"), term.clone()));
                out.push((at("definition"), definition.clone()));
            }
            ContentBlock::Symbols { title, symbols } => {
                out.push((at("symbols title"), title.clone()));
                for s in symbols {
                    out.extend([("symbol", &s.symbol), ("symbol name", &s.name), ("symbol meaning", &s.meaning)].map(|(w, t)| (at(w), t.clone())));
                    out.extend(s.example.iter().map(|t| (at("symbol example"), t.clone())));
                }
            }
        }
    }
    for (i, example) in section.examples.iter().enumerate() {
        let at = |what: &str| format!("example {} {what}", i + 1);
        match example {
            LessonExample::Formula { sentence, formula, note } => {
                out.push((at("sentence"), sentence.clone()));
                out.push((at("formula"), formula.clone()));
                out.extend(note.iter().map(|n| (at("note"), n.clone())));
            }
            LessonExample::Worked { title, lines, conclusion, note } => {
                out.push((at("title"), title.clone()));
                out.extend(lines.iter().map(|l| (at("line"), l.clone())));
                out.extend(conclusion.iter().map(|c| (at("conclusion"), c.clone())));
                out.extend(note.iter().map(|n| (at("note"), n.clone())));
            }
        }
    }
    match &section.question {
        Some(LessonQuestion::Translate { sentence, answer, hint, explanation }) => {
            out.extend([("question sentence", sentence), ("question answer", answer), ("question hint", hint), ("question explanation", explanation)].map(|(w, t)| (w.to_string(), t.clone())));
        }
        Some(LessonQuestion::Choice { prompt, options, hint, explanation, .. }) => {
            out.push(("question prompt".to_string(), prompt.clone()));
            out.extend(options.iter().map(|o| ("question option".to_string(), o.clone())));
            out.push(("question hint".to_string(), hint.clone()));
            out.push(("question explanation".to_string(), explanation.clone()));
        }
        None => {}
    }
    out
}

/// Every string an exercise shows, labelled by where it sits.
fn exercise_texts(ex: &ExerciseConfig) -> Vec<(String, String)> {
    let mut out = vec![("prompt".to_string(), ex.prompt.clone())];
    out.extend(ex.template.iter().map(|t| ("template".to_string(), t.clone())));
    out.extend(ex.options.iter().flatten().map(|o| ("option".to_string(), o.clone())));
    out.extend(ex.hint.iter().map(|h| ("hint".to_string(), h.clone())));
    out.extend(ex.explanation.iter().map(|e| ("explanation".to_string(), e.clone())));
    out
}

/// Whether two answers are the same answer to the grader.
fn same_to_the_grader(a: &str, b: &str) -> bool {
    check_answer(a, b).correct || check_answer(b, a).correct
}

/// Whether a hint names the answer it is meant to lead to.
fn gives_away(hint: &str, answer: &str) -> bool {
    let answer = answer.trim();
    (answer.chars().count() >= 6 || answer.split_whitespace().count() >= 2) && hint.to_lowercase().contains(&answer.to_lowercase())
}

// ---------------------------------------------------------------------------------------------
// 1. Shape
// ---------------------------------------------------------------------------------------------

#[test]
fn every_lesson_has_two_examples_and_one_question() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        if section.examples.len() < 2 {
            v.push(format!("{}: {} example(s), the main path shows two", at(module, section), section.examples.len()));
        }
        if section.question.is_none() {
            v.push(format!("{}: no question", at(module, section)));
        }
    }
    report("lessons without their two examples and one question", &v);
}

#[test]
fn a_lesson_teaches_in_prose_definitions_and_symbol_tables() {
    // Examples and the question have their own place in a lesson; the teaching around them is text.
    // Read from the files themselves, so a block of another kind is named here rather than only
    // dropping its lesson from the load.
    const TEACHING: &[&str] = &["paragraph", "definition", "symbols"];
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        if section.content.is_empty() {
            v.push(format!("{}: nothing to read before the examples", at(module, section)));
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/assets/curriculum");
    for era in std::fs::read_dir(&root).unwrap().flatten() {
        for module in std::fs::read_dir(era.path()).into_iter().flatten().flatten() {
            for file in std::fs::read_dir(module.path()).into_iter().flatten().flatten() {
                let name = file.file_name().to_string_lossy().to_string();
                if !(name.starts_with("sec_") && name.ends_with(".json")) {
                    continue;
                }
                let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file.path()).unwrap()).unwrap();
                for block in json["content"].as_array().into_iter().flatten() {
                    let kind = block["type"].as_str().unwrap_or("(none)");
                    if !TEACHING.contains(&kind) {
                        v.push(format!("{}: a {kind:?} block in the teaching", file.path().display()));
                    }
                }
            }
        }
    }
    report("lessons whose teaching is not prose", &v);
}

#[test]
fn every_lesson_teaches_briefly() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        let mut total = 0;
        for block in &section.content {
            let (kind, n) = match block {
                ContentBlock::Paragraph { text } => ("paragraph", words(text)),
                ContentBlock::Definition { term, definition } => ("definition", words(term) + words(definition)),
                ContentBlock::Symbols { symbols, .. } => ("symbols", symbols.iter().map(|s| words(&s.name) + words(&s.meaning)).sum()),
            };
            total += n;
            if kind != "symbols" && n > BLOCK_WORDS {
                v.push(format!("{}: a {kind} of {n} words (at most {BLOCK_WORDS})", at(module, section)));
            }
        }
        if total > TEACHING_WORDS {
            v.push(format!("{}: {total} words of teaching (at most {TEACHING_WORDS})", at(module, section)));
        }
        for example in &section.examples {
            let note = match example {
                LessonExample::Formula { note, .. } | LessonExample::Worked { note, .. } => note,
            };
            if let Some(n) = note.as_deref().filter(|n| words(n) > NOTE_WORDS) {
                v.push(format!("{}: an example note of {} words (at most {NOTE_WORDS})", at(module, section), words(n)));
            }
        }
        if let Some(LessonQuestion::Translate { hint, explanation, .. } | LessonQuestion::Choice { hint, explanation, .. }) = &section.question {
            if words(hint) > HINT_WORDS {
                v.push(format!("{}: a hint of {} words (at most {HINT_WORDS})", at(module, section), words(hint)));
            }
            if words(explanation) > EXPLANATION_WORDS {
                v.push(format!("{}: an explanation of {} words (at most {EXPLANATION_WORDS})", at(module, section), words(explanation)));
            }
        }
    }
    report("lessons that run long", &v);
}

#[test]
fn lessons_are_numbered_by_their_module_on_the_roadmap() {
    // Lesson ids are progress keys and the numbers a learner reads: module m's lessons are m.1, m.2…
    let mut v = Vec::new();
    for (m, module) in modules().into_iter().enumerate() {
        for (s, section) in module.sections.iter().enumerate() {
            let expected = format!("{}.{}", m + 1, s + 1);
            if section.id != expected {
                v.push(format!("{}: lesson {} of module {} should be {expected}", at(module, section), s + 1, m + 1));
            }
        }
    }
    report("lessons numbered out of place", &v);
}

#[test]
fn lesson_ids_are_unique_across_the_roadmap() {
    let mut seen = HashMap::new();
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        if let Some(other) = seen.insert(section.id.clone(), module.meta.id.clone()) {
            v.push(format!("{} is both {other}'s and {}'s", section.id, module.meta.id));
        }
    }
    report("repeated lesson ids", &v);
}

#[test]
fn the_typed_lessons_are_the_ones_that_teach_writing_formulas() {
    let typed: BTreeSet<&str> = lessons()
        .into_iter()
        .filter(|(_, _, s)| matches!(s.question, Some(LessonQuestion::Translate { .. })))
        .map(|(_, _, s)| s.id.as_str())
        .collect();
    assert_eq!(typed, TYPED.iter().copied().collect::<BTreeSet<_>>(), "the lessons with a typed question");
}

#[test]
fn every_module_and_era_reads_in_a_line() {
    let mut v = Vec::new();
    for era in engine().eras() {
        if words(&era.meta.description) > 25 {
            v.push(format!("era {}: description of {} words (at most 25)", era.meta.id, words(&era.meta.description)));
        }
        for module in &era.modules {
            if words(&module.meta.summary) > 20 {
                v.push(format!("module {}: summary of {} words (at most 20)", module.meta.id, words(&module.meta.summary)));
            }
        }
    }
    report("eras or modules that need more than a line", &v);
}

// ---------------------------------------------------------------------------------------------
// 2. Truth
// ---------------------------------------------------------------------------------------------

#[test]
fn every_formula_shown_is_what_logos_prints() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        for example in &section.examples {
            if let LessonExample::Formula { sentence, formula, .. } = example {
                match logicaffeine_language::compile(sentence) {
                    Ok(printed) if printed == *formula => {}
                    Ok(printed) => v.push(format!("{}: {sentence:?} shows {formula:?}, LOGOS prints {printed:?}", at(module, section))),
                    Err(e) => v.push(format!("{}: {sentence:?} does not compile: {e:?}", at(module, section))),
                }
            }
        }
    }
    report("formulas that differ from what LOGOS prints", &v);
}

#[test]
fn every_typed_key_is_what_logos_prints() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        if let Some(LessonQuestion::Translate { sentence, answer, .. }) = &section.question {
            match logicaffeine_language::compile(sentence) {
                Ok(printed) if printed == *answer => {}
                Ok(printed) => v.push(format!("{}: {sentence:?} is keyed {answer:?}, LOGOS prints {printed:?}", at(module, section))),
                Err(e) => v.push(format!("{}: {sentence:?} does not compile: {e:?}", at(module, section))),
            }
        }
    }
    report("typed keys that differ from what LOGOS prints", &v);
}

#[test]
fn the_grader_accepts_every_typed_key_as_a_learner_writes_it() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        let Some(LessonQuestion::Translate { answer, .. }) = &section.question else { continue };
        let typed = learner_form(answer);
        if !check_answer(&typed, answer).correct {
            v.push(format!("{}: {typed:?} is marked wrong against {answer:?}", at(module, section)));
        }
        if let Some(var) = first_bound_variable(answer) {
            let renamed = rename_word(&typed, &var, "zz");
            if !check_answer(&renamed, answer).correct {
                v.push(format!("{}: renaming the bound {var} is marked wrong: {renamed:?}", at(module, section)));
            }
        }
        match first_predicate_renamed(answer) {
            Some(wrong) if check_answer(&wrong, answer).correct => v.push(format!("{}: a wrong predicate is marked right: {wrong:?}", at(module, section))),
            _ => {}
        }
    }
    report("typed keys the grader misreads", &v);
}

#[test]
fn every_choice_question_has_one_right_option_the_grader_can_tell_apart() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        let Some(LessonQuestion::Choice { prompt, options, correct, .. }) = &section.question else { continue };
        let where_ = at(module, section);
        if prompt.trim().is_empty() {
            v.push(format!("{where_}: empty prompt"));
        }
        if !(3..=4).contains(&options.len()) {
            v.push(format!("{where_}: {} options (three or four)", options.len()));
        }
        if *correct >= options.len() {
            v.push(format!("{where_}: the right option {correct} is not among {} options", options.len()));
        }
        for (i, a) in options.iter().enumerate() {
            if a.trim().is_empty() {
                v.push(format!("{where_}: option {i} is empty"));
            }
            for b in &options[i + 1..] {
                if a.trim() == b.trim() || same_to_the_grader(a, b) {
                    v.push(format!("{where_}: {a:?} and {b:?} are the same answer"));
                }
            }
        }
    }
    report("choice questions with a muddled key", &v);
}

#[test]
fn no_hint_gives_its_answer_away() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        match &section.question {
            Some(LessonQuestion::Translate { answer, hint, .. }) => {
                if hint.contains(answer.as_str()) || hint.contains(learner_form(answer).as_str()) {
                    v.push(format!("{}: the hint holds the key", at(module, section)));
                }
            }
            Some(LessonQuestion::Choice { options, correct, hint, .. }) => {
                if let Some(right) = options.get(*correct).filter(|right| gives_away(hint, right)) {
                    v.push(format!("{}: the hint names the right option {right:?}", at(module, section)));
                }
            }
            None => {}
        }
        if let Some(LessonQuestion::Translate { hint, explanation, .. } | LessonQuestion::Choice { hint, explanation, .. }) = &section.question {
            if hint.trim().is_empty() || explanation.trim().is_empty() {
                v.push(format!("{}: a question needs a hint for a wrong answer and an explanation", at(module, section)));
            }
        }
    }
    for module in modules() {
        for ex in &module.exercises {
            if let (Some(options), Some(correct), Some(hint)) = (&ex.options, ex.correct, &ex.hint) {
                if let Some(right) = options.get(correct).filter(|right| gives_away(hint, right)) {
                    v.push(format!("{} {}: the hint names the right option {right:?}", module.meta.id, ex.id));
                }
            }
        }
    }
    report("hints that give the answer away", &v);
}

#[test]
fn no_example_is_the_question_it_prepares_for() {
    let mut v = Vec::new();
    for (_, module, section) in lessons() {
        let Some(LessonQuestion::Translate { sentence, answer, .. }) = &section.question else { continue };
        for example in &section.examples {
            if let LessonExample::Formula { sentence: shown, formula, .. } = example {
                if shown.trim() == sentence.trim() || same_to_the_grader(formula, answer) {
                    v.push(format!("{}: the example {shown:?} answers the question", at(module, section)));
                }
            }
        }
    }
    report("examples that answer their own question", &v);
}

// ---------------------------------------------------------------------------------------------
// 3. Copy
// ---------------------------------------------------------------------------------------------

fn every_text() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for era in engine().eras() {
        out.push((format!("era {}", era.meta.id), era.meta.title.clone()));
        out.push((format!("era {}", era.meta.id), era.meta.description.clone()));
        for module in &era.modules {
            out.push((format!("module {}", module.meta.id), module.meta.title.clone()));
            out.push((format!("module {}", module.meta.id), module.meta.summary.clone()));
            for section in &module.sections {
                out.extend(section_texts(section).into_iter().map(|(w, t)| (format!("{} {w}", at(module, section)), t)));
            }
            for ex in &module.exercises {
                out.extend(exercise_texts(ex).into_iter().map(|(w, t)| (format!("{} {} {w}", module.meta.id, ex.id), t)));
            }
        }
    }
    out
}

#[test]
fn no_learn_text_carries_an_em_dash_a_semicolon_or_a_dash_between_words() {
    let mut v = Vec::new();
    for (where_, text) in every_text() {
        for fault in punctuation_faults(&text) {
            v.push(format!("{where_}: {fault}"));
        }
    }
    report("pieces of punctuation the Learn page must not show", &v);
}

#[test]
fn no_learn_text_carries_tex() {
    // The page prints notation as the symbols themselves: a `$…$` span or a `\command` reaches the
    // learner as raw TeX.
    let mut v = Vec::new();
    for (where_, text) in every_text() {
        if text.contains('$') || text.contains('\\') {
            v.push(format!("{where_}: {text:?}"));
        }
    }
    report("strings carrying TeX", &v);
}

#[test]
fn the_fallacies_module_stays_clear_of_religion_in_every_field() {
    // Fallacies and Argumentation illustrates each fallacy with an everyday example; its lessons and
    // practice stay clear of religion so the lesson is about the reasoning, not the topic.
    const TERMS: &[&str] = &[
        "god", "gods", "bible", "religion", "religious", "church", "prayer", "pray", "deity", "divine", "heaven", "hell",
        "scripture", "faith", "atheist", "atheism", "theist", "theism",
    ];
    let module = engine().get_module("first-steps", "fallacies").expect("fallacies module");
    let mut texts: Vec<(String, String)> = Vec::new();
    for section in &module.sections {
        texts.extend(section_texts(section).into_iter().map(|(w, t)| (format!("{} {w}", section.id), t)));
    }
    for ex in &module.exercises {
        texts.extend(exercise_texts(ex).into_iter().map(|(w, t)| (format!("{} {w}", ex.id), t)));
    }
    let mut v = Vec::new();
    for (where_, text) in &texts {
        let ws: Vec<String> = text.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase).collect();
        if let Some(term) = TERMS.iter().find(|t| ws.iter().any(|w| w == *t)) {
            v.push(format!("{where_}: {term:?} in {text:?}"));
        }
    }
    report("religious examples in the fallacies module", &v);
}

// ---------------------------------------------------------------------------------------------
// 4. Practice
// ---------------------------------------------------------------------------------------------

#[test]
fn every_exercise_practises_a_lesson_of_its_own_module() {
    let mut v = Vec::new();
    for module in modules() {
        let ids: HashSet<&str> = module.sections.iter().map(|s| s.id.as_str()).collect();
        for ex in &module.exercises {
            match ex.section.as_deref() {
                Some(id) if ids.contains(id) => {}
                Some(id) => v.push(format!("{} {}: names lesson {id}, which {} does not have", module.meta.id, ex.id, module.meta.id)),
                None => v.push(format!("{} {}: names no lesson", module.meta.id, ex.id)),
            }
        }
    }
    report("exercises that practise no lesson of their module", &v);
}

#[test]
fn no_module_asks_the_same_exercise_twice() {
    let mut v = Vec::new();
    for module in modules() {
        let mut seen: HashMap<String, &str> = HashMap::new();
        for ex in &module.exercises {
            let mut options: Vec<String> = ex.options.iter().flatten().map(|o| o.trim().to_string()).collect();
            options.sort();
            let key = format!("{}|{}|{}", ex.prompt.trim(), ex.template.as_deref().unwrap_or(""), options.join("|"));
            if let Some(other) = seen.insert(key, ex.id.as_str()) {
                v.push(format!("{}: {} asks what {other} asks", module.meta.id, ex.id));
            }
        }
    }
    report("repeated exercises", &v);
}

#[test]
fn every_multiple_choice_exercise_has_options_the_grader_tells_apart() {
    let mut v = Vec::new();
    for module in modules() {
        for ex in module.exercises.iter().filter(|e| e.exercise_type == ExerciseType::MultipleChoice) {
            let options = ex.options.as_deref().unwrap_or_default();
            for (i, a) in options.iter().enumerate() {
                for b in &options[i + 1..] {
                    if same_to_the_grader(a, b) {
                        v.push(format!("{} {}: {a:?} and {b:?} are the same formula", module.meta.id, ex.id));
                    }
                }
            }
        }
    }
    report("exercises offering one formula twice", &v);
}
