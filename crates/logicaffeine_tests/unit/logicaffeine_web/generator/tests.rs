use super::*;
use crate::content::ContentEngine;
use rand::rngs::StdRng;
use rand::SeedableRng;

#[test]
fn test_generate_translation_challenge() {
    let engine = ContentEngine::new();
    let generator = Generator::new();
    let mut rng = StdRng::seed_from_u64(42);

    // Use introduction module which has Translation exercises
    let exercise = engine.get_exercise("first-steps", "introduction", "I_1.1");
    assert!(
        exercise.is_some(),
        "Exercise first-steps/introduction/I_1.1 should exist"
    );
    let exercise = exercise.unwrap();
    let challenge = generator.generate(exercise, &mut rng);

    assert!(challenge.is_some(), "Should generate a challenge");
    let challenge = challenge.unwrap();
    assert!(
        !challenge.sentence.is_empty(),
        "Sentence should not be empty"
    );

    if let AnswerType::FreeForm { golden_logic } = &challenge.answer {
        assert!(!golden_logic.is_empty(), "Golden logic should not be empty");
    } else {
        panic!("Expected FreeForm answer type");
    }
}

#[test]
fn test_generate_multiple_choice() {
    let engine = ContentEngine::new();
    let generator = Generator::new();
    let mut rng = StdRng::seed_from_u64(42);

    // Use syllogistic module which has MultipleChoice exercises
    let exercise = engine.get_exercise("first-steps", "syllogistic", "A_1.1");
    assert!(
        exercise.is_some(),
        "Exercise first-steps/syllogistic/A_1.1 should exist"
    );
    let exercise = exercise.unwrap();
    let challenge = generator.generate(exercise, &mut rng);

    assert!(challenge.is_some(), "Should generate a challenge");
    let challenge = challenge.unwrap();

    if let AnswerType::MultipleChoice {
        options,
        correct_index,
    } = &challenge.answer
    {
        assert_eq!(options.len(), 4, "Should have 4 options");
        assert!(
            *correct_index < options.len(),
            "Correct index should be within options range"
        );
    } else {
        panic!("Expected MultipleChoice answer type");
    }
}

#[test]
fn test_fill_template_proper_names() {
    let generator = Generator::new();
    let mut rng = StdRng::seed_from_u64(42);

    let constraints = HashMap::new();
    let result = generator.fill_template("{ProperName} runs.", &constraints, &mut rng);

    assert!(result.is_some());
    let sentence = result.unwrap();
    assert!(
        sentence.ends_with(" runs."),
        "Template should be filled: {}",
        sentence
    );
    assert!(!sentence.starts_with("{"), "Slot should be replaced");
}

#[test]
fn test_fill_template_with_modifier() {
    let generator = Generator::new();
    let mut rng = StdRng::seed_from_u64(42);

    let constraints = HashMap::new();
    let result = generator.fill_template("All {Noun:Plural} run.", &constraints, &mut rng);

    assert!(result.is_some());
    let sentence = result.unwrap();
    assert!(
        !sentence.contains("{"),
        "All slots should be filled: {}",
        sentence
    );
}

#[test]
fn test_deterministic_with_seed() {
    let generator = Generator::new();
    let mut rng1 = StdRng::seed_from_u64(12345);
    let mut rng2 = StdRng::seed_from_u64(12345);

    let constraints = HashMap::new();
    let result1 = generator.fill_template("{ProperName} is {Adjective}.", &constraints, &mut rng1);
    let result2 = generator.fill_template("{ProperName} is {Adjective}.", &constraints, &mut rng2);

    assert_eq!(result1, result2, "Same seed should produce same output");
}

#[test]
fn test_all_introduction_exercises() {
    let engine = ContentEngine::new();
    let generator = Generator::new();

    let module = engine.get_module("first-steps", "introduction");
    assert!(module.is_some(), "Introduction module should exist");
    let module = module.unwrap();

    println!(
        "Introduction module has {} exercises",
        module.exercises.len()
    );

    for (i, exercise) in module.exercises.iter().enumerate() {
        let mut rng = StdRng::seed_from_u64(42 + i as u64);
        println!(
            "Exercise {}: id={}, type={:?}",
            i, exercise.id, exercise.exercise_type
        );

        let challenge = generator.generate(exercise, &mut rng);
        if challenge.is_none() {
            println!("  FAILED to generate challenge!");
            if let Some(template) = &exercise.template {
                println!("  Template: {}", template);
                let filled = generator.fill_template(
                    template,
                    &exercise.constraints,
                    &mut StdRng::seed_from_u64(42),
                );
                println!("  Filled template: {:?}", filled);
                if let Some(sentence) = filled {
                    let compiled = compile(&sentence);
                    println!("  Compile result: {:?}", compiled);
                }
            }
        } else {
            println!("  OK: {:?}", challenge.as_ref().map(|c| &c.sentence));
        }
        assert!(
            challenge.is_some(),
            "Exercise {} ({}) should generate a challenge",
            i,
            exercise.id
        );
    }
}

#[test]
fn test_all_exercises_across_all_modules() {
    let engine = ContentEngine::new();
    let generator = Generator::new();

    let mut total_exercises = 0;
    let mut successful = 0;
    let mut failed_exercises = Vec::new();

    for era in engine.eras() {
        for module in &era.modules {
            if let Some(m) = engine.get_module(&era.meta.id, &module.meta.id) {
                for (i, exercise) in m.exercises.iter().enumerate() {
                    total_exercises += 1;
                    let mut rng = StdRng::seed_from_u64(42 + i as u64);

                    let challenge = generator.generate(exercise, &mut rng);
                    if challenge.is_some() {
                        successful += 1;
                    } else {
                        failed_exercises.push(format!(
                            "{}/{}/{}",
                            era.meta.id, module.meta.id, exercise.id
                        ));
                    }
                }
            }
        }
    }

    println!("Total exercises: {}", total_exercises);
    println!("Successful: {}", successful);
    println!("Failed: {}", failed_exercises.len());

    if !failed_exercises.is_empty() {
        println!("\nFailed exercises:");
        for ex in &failed_exercises {
            println!("  - {}", ex);
        }
    }

    assert!(
        failed_exercises.is_empty(),
        "every exercise must generate a challenge; failed: {:?}",
        failed_exercises
    );
}
