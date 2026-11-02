use super::*;
use include_dir::{DirEntry, File};

const ERA_META: &str = r#"{"id":"e","title":"E","description":"An era.","order":1}"#;
const MODULE_META: &str =
    r#"{"id":"m","title":"M","pedagogy":"p","order":1,"summary":"S","difficulty":2}"#;
const EXERCISE: &str = r#"{"id":"X_1.1","type":"multiple_choice","difficulty":1,"prompt":"Q?","options":["a","b"],"correct":0}"#;
const SECTION: &str =
    r#"{"id":"1.1","title":"T","order":1,"content":[{"type":"paragraph","text":"Hi."}]}"#;

fn tree(module_files: &'static [DirEntry<'static>]) -> Dir<'static> {
    let module: &'static [DirEntry<'static>] = Box::leak(Box::new([DirEntry::Dir(Dir::new(
        "01_e/01_m",
        module_files,
    ))]));
    let era_entries: &'static [DirEntry<'static>] = Box::leak(
        vec![DirEntry::File(File::new(
            "01_e/meta.json",
            ERA_META.as_bytes(),
        ))]
        .into_iter()
        .chain(module.iter().cloned())
        .collect::<Vec<_>>()
        .into_boxed_slice(),
    );
    let root: &'static [DirEntry<'static>] =
        Box::leak(Box::new([DirEntry::Dir(Dir::new("01_e", era_entries))]));
    Dir::new("", root)
}

fn file(path: &'static str, contents: &'static str) -> DirEntry<'static> {
    DirEntry::File(File::new(path, contents.as_bytes()))
}

#[test]
fn a_clean_tree_loads_with_no_issues() {
    let files: &'static [DirEntry<'static>] = Box::leak(Box::new([
        file("01_e/01_m/meta.json", MODULE_META),
        file("01_e/01_m/ex_01.json", EXERCISE),
        file("01_e/01_m/sec_1.1.json", SECTION),
    ]));
    let (curriculum, issues) = ContentEngine::load_from(&tree(files));
    assert!(issues.is_empty(), "{issues:?}");
    let module = &curriculum.eras[0].modules[0];
    assert_eq!((module.exercises.len(), module.sections.len()), (1, 1));
}

#[test]
fn a_malformed_exercise_is_reported_by_path() {
    let files: &'static [DirEntry<'static>] = Box::leak(Box::new([
        file("01_e/01_m/meta.json", MODULE_META),
        file("01_e/01_m/ex_01.json", EXERCISE),
        file(
            "01_e/01_m/ex_02.json",
            r#"{"id":"X_1.2","type":"multiple_choice""#,
        ),
    ]));
    let (curriculum, issues) = ContentEngine::load_from(&tree(files));
    assert_eq!(curriculum.eras[0].modules[0].exercises.len(), 1);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].contains("ex_02.json"), "{issues:?}");
}

#[test]
fn an_exercise_of_an_unknown_type_is_reported() {
    let files: &'static [DirEntry<'static>] = Box::leak(Box::new([
        file("01_e/01_m/meta.json", MODULE_META),
        file(
            "01_e/01_m/ex_01.json",
            r#"{"id":"X_1.1","type":"essay","difficulty":1,"prompt":"Q?"}"#,
        ),
    ]));
    let (_, issues) = ContentEngine::load_from(&tree(files));
    assert!(
        issues
            .iter()
            .any(|i| i.contains("ex_01.json") && i.contains("essay")),
        "{issues:?}"
    );
}

#[test]
fn a_module_meta_missing_its_card_fields_is_reported_not_dropped_silently() {
    let files: &'static [DirEntry<'static>] = Box::leak(Box::new([
        file(
            "01_e/01_m/meta.json",
            r#"{"id":"m","title":"M","pedagogy":"p","order":1}"#,
        ),
        file("01_e/01_m/ex_01.json", EXERCISE),
    ]));
    let (curriculum, issues) = ContentEngine::load_from(&tree(files));
    assert!(curriculum.eras[0].modules.is_empty());
    assert!(
        issues
            .iter()
            .any(|i| i.contains("01_m/meta.json") && i.contains("summary")),
        "{issues:?}"
    );
}

#[test]
fn a_stray_or_misnamed_file_is_reported() {
    let files: &'static [DirEntry<'static>] = Box::leak(Box::new([
        file("01_e/01_m/meta.json", MODULE_META),
        file("01_e/01_m/exercise-1.json", EXERCISE),
        file("01_e/01_m/notes.txt", "todo"),
    ]));
    let (_, issues) = ContentEngine::load_from(&tree(files));
    assert!(
        issues.iter().any(|i| i.contains("exercise-1.json")),
        "{issues:?}"
    );
    assert!(issues.iter().any(|i| i.contains("notes.txt")), "{issues:?}");
}

#[test]
fn a_module_without_meta_is_reported() {
    let files: &'static [DirEntry<'static>] =
        Box::leak(Box::new([file("01_e/01_m/ex_01.json", EXERCISE)]));
    let (curriculum, issues) = ContentEngine::load_from(&tree(files));
    assert!(curriculum.eras[0].modules.is_empty());
    assert!(
        issues
            .iter()
            .any(|i| i.contains("01_m/meta.json") && i.contains("missing")),
        "{issues:?}"
    );
}

#[test]
fn loaded_exercises_come_out_in_natural_order() {
    const A: &str = r#"{"id":"X_1.10","type":"multiple_choice","difficulty":1,"prompt":"a","options":["a","b"],"correct":0}"#;
    const B: &str = r#"{"id":"X_1.2","type":"multiple_choice","difficulty":1,"prompt":"b","options":["a","b"],"correct":0}"#;
    const C: &str = r#"{"id":"X_1.1","type":"multiple_choice","difficulty":1,"prompt":"c","options":["a","b"],"correct":0}"#;
    let files: &'static [DirEntry<'static>] = Box::leak(Box::new([
        file("01_e/01_m/meta.json", MODULE_META),
        file("01_e/01_m/ex_a.json", A),
        file("01_e/01_m/ex_b.json", B),
        file("01_e/01_m/ex_c.json", C),
    ]));
    let (curriculum, _) = ContentEngine::load_from(&tree(files));
    let ids: Vec<&str> = curriculum.eras[0].modules[0]
        .exercises
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, ["X_1.1", "X_1.2", "X_1.10"]);
}

#[test]
fn natural_cmp_counts_digit_runs_as_numbers() {
    assert_eq!(natural_cmp("A_1.2", "A_1.10"), Ordering::Less);
    assert_eq!(natural_cmp("A_1.10", "A_1.2"), Ordering::Greater);
    assert_eq!(natural_cmp("C_6.9", "C_6.10"), Ordering::Less);
    assert_eq!(natural_cmp("C_6.99", "C_6.100"), Ordering::Less);
    assert_eq!(natural_cmp("L_12.2", "L_12.10"), Ordering::Less);
    assert_eq!(natural_cmp("L_2.30", "L_12.1"), Ordering::Less);
    assert_eq!(natural_cmp("A_1.1", "A_1.1"), Ordering::Equal);
}

#[test]
fn natural_cmp_compares_text_runs_as_text() {
    assert_eq!(natural_cmp("F_1.1", "FM_1.1"), Ordering::Less);
    assert_eq!(natural_cmp("P_1.1", "PH_1.1"), Ordering::Less);
    assert_eq!(natural_cmp("A_1.1", "B_1.1"), Ordering::Less);
    assert_eq!(natural_cmp("A_1", "A_1.1"), Ordering::Less);
    assert_eq!(natural_cmp("", "A"), Ordering::Less);
}

#[test]
fn natural_cmp_is_total_over_zero_padded_spellings() {
    assert_eq!(natural_cmp("A_01", "A_1"), "A_01".cmp("A_1"));
    assert_ne!(natural_cmp("A_01", "A_1"), Ordering::Equal);
    assert_eq!(natural_cmp("A_007", "A_8"), Ordering::Less);
    assert_eq!(natural_cmp("A_0", "A_00"), "A_0".cmp("A_00"));
}

#[test]
fn natural_cmp_handles_digit_runs_longer_than_any_integer() {
    let big = "X_123456789012345678901234567890";
    let bigger = "X_123456789012345678901234567891";
    assert_eq!(natural_cmp(big, bigger), Ordering::Less);
    assert_eq!(natural_cmp("X_9", big), Ordering::Less);
}

#[test]
fn natural_cmp_is_antisymmetric_and_transitive_over_the_real_ids() {
    let engine = ContentEngine::shared();
    let ids: Vec<&str> = engine
        .eras()
        .iter()
        .flat_map(|e| e.modules.iter())
        .flat_map(|m| m.exercises.iter())
        .map(|e| e.id.as_str())
        .take(120)
        .collect();
    for a in &ids {
        for b in &ids {
            assert_eq!(natural_cmp(a, b), natural_cmp(b, a).reverse(), "{a} vs {b}");
            for c in ids.iter().step_by(7) {
                if natural_cmp(a, b) != Ordering::Greater && natural_cmp(b, c) != Ordering::Greater
                {
                    assert_ne!(natural_cmp(a, c), Ordering::Greater, "{a} <= {b} <= {c}");
                }
            }
        }
    }
}

#[test]
fn test_dir_contents() {
    // Debug: print what's in the embedded directory
    println!("Files in CURRICULUM_DIR:");
    for f in CURRICULUM_DIR.files() {
        println!("  file: {:?}", f.path());
    }
    println!("Dirs in CURRICULUM_DIR:");
    for d in CURRICULUM_DIR.dirs() {
        println!("  era dir: {:?}", d.path());
        for f in d.files() {
            println!("    era file: {:?}", f.path());
        }
        for module_d in d.dirs() {
            println!("    module dir: {:?}", module_d.path());
            for f in module_d.files() {
                println!("      module file: {:?}", f.path());
            }
        }
    }
    assert!(
        !CURRICULUM_DIR.dirs().collect::<Vec<_>>().is_empty(),
        "Should have embedded directories"
    );
}

#[test]
fn test_curriculum_loads() {
    let engine = ContentEngine::new();
    assert!(
        engine.era_count() >= 4,
        "Should have at least 4 eras (got {})",
        engine.era_count()
    );
}

#[test]
fn test_first_steps_era_exists() {
    let engine = ContentEngine::new();
    let era = engine.get_era("first-steps");
    assert!(era.is_some(), "First Steps era should exist");
    assert_eq!(era.unwrap().meta.title, "First Steps");
}

#[test]
fn test_building_blocks_era_exists() {
    let engine = ContentEngine::new();
    let era = engine.get_era("building-blocks");
    assert!(era.is_some(), "Building Blocks era should exist");
    assert_eq!(era.unwrap().meta.title, "Building Blocks");
}

#[test]
fn test_expanding_horizons_era_exists() {
    let engine = ContentEngine::new();
    let era = engine.get_era("expanding-horizons");
    assert!(era.is_some(), "Expanding Horizons era should exist");
    assert_eq!(era.unwrap().meta.title, "Expanding Horizons");
}

#[test]
fn test_mastery_era_exists() {
    let engine = ContentEngine::new();
    let era = engine.get_era("mastery");
    assert!(era.is_some(), "Mastery era should exist");
    assert_eq!(era.unwrap().meta.title, "Mastery");
}

#[test]
fn test_introduction_module_exists() {
    let engine = ContentEngine::new();
    let module = engine.get_module("first-steps", "introduction");
    assert!(module.is_some(), "Introduction module should exist");
    assert_eq!(module.unwrap().meta.title, "Introduction");
}

#[test]
fn test_syllogistic_module_exists() {
    let engine = ContentEngine::new();
    let module = engine.get_module("first-steps", "syllogistic");
    assert!(module.is_some(), "Syllogistic module should exist");
    let m = module.unwrap();
    assert_eq!(m.meta.title, "Syllogistic Logic");
    assert!(
        m.exercises.len() >= 90,
        "Should have at least 90 exercises (got {})",
        m.exercises.len()
    );
}

#[test]
fn test_propositional_module_exists() {
    let engine = ContentEngine::new();
    let module = engine.get_module("building-blocks", "propositional");
    assert!(module.is_some(), "Propositional module should exist");
    let m = module.unwrap();
    assert_eq!(m.meta.title, "Basic Propositional Logic");
    assert!(
        m.exercises.len() >= 100,
        "Should have at least 100 exercises (got {})",
        m.exercises.len()
    );
}

#[test]
fn test_exercises_load() {
    let engine = ContentEngine::new();
    let count = engine.exercise_count("first-steps", "syllogistic");
    assert!(
        count >= 90,
        "Syllogistic module should have at least 90 exercises"
    );
}

#[test]
fn test_exercise_has_explanation() {
    let engine = ContentEngine::new();
    let ex = engine.get_exercise("first-steps", "syllogistic", "A_1.1");
    assert!(ex.is_some(), "Exercise A_1.1 should exist");
    let exercise = ex.unwrap();
    assert!(
        exercise.explanation.is_some(),
        "Exercise should have explanation"
    );
    assert!(exercise.options.is_some(), "Exercise should have options");
    assert_eq!(exercise.exercise_type, ExerciseType::MultipleChoice);
}

#[test]
fn test_all_eras_have_modules() {
    let engine = ContentEngine::new();

    // First Steps: 5 modules
    let first_steps_modules = [
        "introduction",
        "syllogistic",
        "definitions",
        "fallacies",
        "inductive",
    ];
    for module in first_steps_modules {
        assert!(
            engine.get_module("first-steps", module).is_some(),
            "first-steps/{} should exist",
            module
        );
    }

    // Building Blocks: 2 modules
    let building_blocks_modules = ["propositional", "proofs"];
    for module in building_blocks_modules {
        assert!(
            engine.get_module("building-blocks", module).is_some(),
            "building-blocks/{} should exist",
            module
        );
    }

    // Expanding Horizons: 6 modules
    let expanding_modules = [
        "quantificational",
        "relations",
        "modal",
        "further_modal",
        "deontic",
        "belief",
    ];
    for module in expanding_modules {
        assert!(
            engine.get_module("expanding-horizons", module).is_some(),
            "expanding-horizons/{} should exist",
            module
        );
    }

    // Mastery: 5 modules
    let mastery_modules = ["ethics", "metalogic", "history", "deviant", "philosophy"];
    for module in mastery_modules {
        assert!(
            engine.get_module("mastery", module).is_some(),
            "mastery/{} should exist",
            module
        );
    }
}
