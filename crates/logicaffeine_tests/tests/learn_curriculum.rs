//! How the Learn Logic curriculum loads — the layer under every card, count and exercise.
//!
//! `learn_examples.rs` audits what the loaded curriculum SAYS. This file pins how it LOADS:
//!
//!   1. Nothing is dropped. A curriculum file that fails to parse, a module meta that is
//!      missing a field, or a stray file whose name the loader ignores would otherwise vanish
//!      without a trace — and take its exercises out of every audit with it.
//!   2. Exercises play in the order their ids count (`A_1.2` before `A_1.10`), not in the
//!      order a string compare happens to put them.
//!   3. The curriculum is parsed once per process. Render paths borrow the shared engine;
//!      they never re-parse hundreds of JSON files on every render.
//!   4. Every module carries the card metadata the Learn page shows, so the page has no
//!      second, hand-kept copy of the curriculum to drift from.

#![cfg(feature = "web-tests")]

use logicaffeine_web::content::ContentEngine;
use std::path::{Path, PathBuf};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web")
}

#[test]
fn curriculum_loads_every_file_without_dropping_one() {
    let issues = ContentEngine::load_issues();
    assert!(
        issues.is_empty(),
        "\n{} curriculum file(s) did not load — the page would silently lose them:\n{}\n",
        issues.len(),
        issues.join("\n")
    );
}

#[test]
fn every_curriculum_file_on_disk_is_loaded() {
    let engine = ContentEngine::shared();
    let root = crate_dir().join("assets/curriculum");
    let mut on_disk = 0usize;
    for era in std::fs::read_dir(&root).unwrap() {
        let era = era.unwrap().path();
        if !era.is_dir() {
            continue;
        }
        for module in std::fs::read_dir(&era).unwrap() {
            let module = module.unwrap().path();
            if !module.is_dir() {
                continue;
            }
            on_disk += std::fs::read_dir(&module)
                .unwrap()
                .filter_map(|f| f.ok())
                .filter(|f| {
                    let name = f.file_name().to_string_lossy().into_owned();
                    name.starts_with("ex_") && name.ends_with(".json")
                })
                .count();
        }
    }
    let loaded: usize = engine
        .eras()
        .iter()
        .flat_map(|e| e.modules.iter())
        .map(|m| m.exercises.len())
        .sum();
    assert_eq!(
        loaded, on_disk,
        "{on_disk} ex_*.json files on disk but {loaded} exercises loaded"
    );
}

#[test]
fn exercises_play_in_natural_id_order() {
    let engine = ContentEngine::shared();
    let mut violations = Vec::new();
    for era in engine.eras() {
        for module in &era.modules {
            let ids: Vec<&str> = module.exercises.iter().map(|e| e.id.as_str()).collect();
            let mut expected = ids.clone();
            expected.sort_by(|a, b| natural_key(a).cmp(&natural_key(b)));
            if ids != expected {
                let first_wrong = ids
                    .iter()
                    .zip(&expected)
                    .position(|(a, b)| a != b)
                    .unwrap();
                violations.push(format!(
                    "{}/{}: position {} is {} but should be {}",
                    era.meta.id, module.meta.id, first_wrong, ids[first_wrong], expected[first_wrong]
                ));
            }
        }
    }
    assert!(violations.is_empty(), "\n{}\n", violations.join("\n"));
}

#[test]
fn syllogistic_opens_with_its_first_three_exercises_in_order() {
    let module = ContentEngine::shared()
        .get_module("first-steps", "syllogistic")
        .expect("syllogistic module");
    let first: Vec<&str> = module.exercises.iter().take(3).map(|e| e.id.as_str()).collect();
    assert_eq!(first, ["A_1.1", "A_1.2", "A_1.3"]);
}

#[test]
fn propositional_counts_past_one_hundred_in_order() {
    let module = ContentEngine::shared()
        .get_module("building-blocks", "propositional")
        .expect("propositional module");
    let ids: Vec<&str> = module.exercises.iter().map(|e| e.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).unwrap_or_else(|| panic!("{id} missing"));
    assert!(pos("C_6.9") < pos("C_6.10"), "C_6.9 must precede C_6.10");
    assert!(pos("C_6.99") < pos("C_6.100"), "C_6.99 must precede C_6.100");
}

#[test]
fn the_shared_engine_is_one_instance() {
    let a = ContentEngine::shared() as *const ContentEngine;
    let b = ContentEngine::shared() as *const ContentEngine;
    assert_eq!(a, b, "ContentEngine::shared() must hand out one parsed curriculum");
}

#[test]
fn render_paths_never_reparse_the_curriculum() {
    let mut offenders = Vec::new();
    scan(&crate_dir().join("src/ui"), &mut offenders);
    assert!(
        offenders.is_empty(),
        "\nUI code must borrow ContentEngine::shared(); these re-parse every curriculum file on each render:\n{}\n",
        offenders.join("\n")
    );
}

fn scan(dir: &Path, offenders: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, offenders);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (i, line) in text.lines().enumerate() {
                if line.contains("ContentEngine::new(") || line.contains("ContentEngine::default(") {
                    offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
    }
}

#[test]
fn every_era_and_module_says_what_it_teaches() {
    // The roadmap heads each era with its description and each module with its summary.
    let engine = ContentEngine::shared();
    let mut v = Vec::new();
    for era in engine.eras() {
        if era.meta.description.trim().is_empty() {
            v.push(format!("{}: empty era description", era.meta.id));
        }
        for module in &era.modules {
            if module.meta.summary.trim().is_empty() {
                v.push(format!("{}/{}: empty summary", era.meta.id, module.meta.id));
            }
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

#[test]
fn module_ids_are_unique_across_the_whole_curriculum() {
    // Module ids double as DOM ids and URL fragments on /learn.
    let engine = ContentEngine::shared();
    let mut seen = std::collections::HashMap::new();
    for era in engine.eras() {
        for module in &era.modules {
            if let Some(prev) = seen.insert(module.meta.id.clone(), era.meta.id.clone()) {
                panic!("module id {:?} appears in both {prev} and {}", module.meta.id, era.meta.id);
            }
        }
    }
}

#[test]
fn eras_and_modules_follow_their_declared_order() {
    let engine = ContentEngine::shared();
    let era_orders: Vec<u32> = engine.eras().iter().map(|e| e.meta.order).collect();
    let mut sorted = era_orders.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(era_orders, sorted, "era order values must be strictly increasing");
    for era in engine.eras() {
        let orders: Vec<u32> = era.modules.iter().map(|m| m.meta.order).collect();
        let mut sorted = orders.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(orders, sorted, "{}: module order values must be strictly increasing", era.meta.id);
        for module in &era.modules {
            let orders: Vec<u32> = module.sections.iter().map(|s| s.order).collect();
            let mut sorted = orders.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(orders, sorted, "{}/{}: section order values must be strictly increasing", era.meta.id, module.meta.id);
        }
    }
}

/// Reference ordering for the tests: digit runs compare as numbers, everything else as text.
fn natural_key(id: &str) -> Vec<(u8, String, u64)> {
    let mut key = Vec::new();
    let mut chars = id.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut digits = String::new();
            while let Some(&d) = chars.peek() {
                if !d.is_ascii_digit() {
                    break;
                }
                digits.push(d);
                chars.next();
            }
            key.push((1, String::new(), digits.parse().unwrap()));
        } else {
            let mut text = String::new();
            while let Some(&t) = chars.peek() {
                if t.is_ascii_digit() {
                    break;
                }
                text.push(t);
                chars.next();
            }
            key.push((0, text, 0));
        }
    }
    key
}

#[test]
fn templates_conjugate_a_verb_after_a_singular_subject() {
    // `{Verb}` fills the bare lexicon form, right after "does not" or "can"; directly after a
    // singular subject it has to agree ("Every dog runs"), or the learner is asked to
    // translate "Every dog run".
    let singular_subject = ["{ProperName}", "{Noun}", "{Noun:Singular}", "{Noun2:Singular}"];
    let mut v = Vec::new();
    for era in ContentEngine::shared().eras() {
        for module in &era.modules {
            for ex in &module.exercises {
                let Some(template) = &ex.template else { continue };
                for subject in singular_subject {
                    if template.contains(&format!("{subject} {{Verb}}")) {
                        v.push(format!("{}/{} {}: {template:?} puts a bare {{Verb}} after {subject}", era.meta.id, module.meta.id, ex.id));
                    }
                }
            }
        }
    }
    assert!(v.is_empty(), "\n{}\n", v.join("\n"));
}

#[test]
fn fallacies_module_teaches_without_religious_examples() {
    // Fallacies and Argumentation illustrates each fallacy with an everyday example; its
    // examples stay clear of religion so the lesson is about the reasoning, not the topic.
    use logicaffeine_web::content::{ContentBlock, LessonExample, LessonQuestion};
    const TERMS: &[&str] = &[
        "god", "gods", "bible", "religion", "religious", "church", "prayer", "pray", "deity",
        "divine", "heaven", "hell", "scripture", "faith", "atheist", "atheism", "theist", "theism",
    ];
    let module = ContentEngine::shared().get_module("first-steps", "fallacies").expect("fallacies module");
    let mut texts: Vec<(String, String)> = Vec::new();
    for section in &module.sections {
        let at = format!("section {}", section.id);
        for block in &section.content {
            match block {
                ContentBlock::Paragraph { text } => texts.push((at.clone(), text.clone())),
                ContentBlock::Definition { term, definition } => {
                    texts.push((at.clone(), term.clone()));
                    texts.push((at.clone(), definition.clone()));
                }
                ContentBlock::Symbols { title, symbols } => {
                    texts.push((at.clone(), title.clone()));
                    for s in symbols {
                        texts.extend([&s.symbol, &s.name, &s.meaning].into_iter().map(|t| (at.clone(), t.clone())));
                        texts.extend(s.example.iter().map(|t| (at.clone(), t.clone())));
                    }
                }
            }
        }
        for example in &section.examples {
            match example {
                LessonExample::Formula { sentence, formula, note } => {
                    texts.push((at.clone(), sentence.clone()));
                    texts.push((at.clone(), formula.clone()));
                    texts.extend(note.iter().map(|t| (at.clone(), t.clone())));
                }
                LessonExample::Worked { title, lines, conclusion, note } => {
                    texts.push((at.clone(), title.clone()));
                    texts.extend(lines.iter().map(|l| (at.clone(), l.clone())));
                    texts.extend(conclusion.iter().chain(note.iter()).map(|t| (at.clone(), t.clone())));
                }
            }
        }
        match &section.question {
            Some(LessonQuestion::Choice { prompt, options, hint, explanation, .. }) => {
                texts.push((at.clone(), prompt.clone()));
                texts.extend(options.iter().map(|o| (at.clone(), o.clone())));
                texts.push((at.clone(), hint.clone()));
                texts.push((at.clone(), explanation.clone()));
            }
            Some(LessonQuestion::Translate { sentence, answer, hint, explanation }) => {
                texts.extend([sentence, answer, hint, explanation].into_iter().map(|t| (at.clone(), t.clone())));
            }
            None => {}
        }
    }
    for ex in &module.exercises {
        let at = format!("exercise {}", ex.id);
        texts.push((at.clone(), ex.prompt.clone()));
        texts.extend(ex.hint.iter().chain(ex.explanation.iter()).map(|t| (at.clone(), t.clone())));
        texts.extend(ex.options.iter().flatten().map(|o| (at.clone(), o.clone())));
    }
    let mut hits = Vec::new();
    for (at, text) in &texts {
        let words: Vec<String> = text.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase).collect();
        if let Some(term) = TERMS.iter().find(|t| words.iter().any(|w| w == *t)) {
            hits.push(format!("{at}: {term:?} in {text:?}"));
        }
    }
    assert!(hits.is_empty(), "\n{}\n", hits.join("\n"));
}
