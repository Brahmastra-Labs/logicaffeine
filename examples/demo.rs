use logos::{compile, compile_all_scopes, compile_with_options, CompileOptions, OutputFormat};

fn main() {
    println!("═══════════════════════════════════════════════════════════════════");
    println!("                    LOGICAFFEINE 1.0 DEMO");
    println!("              Montague Semantics + Lambda Calculus");
    println!("═══════════════════════════════════════════════════════════════════\n");

    section("BASIC PREDICATION");
    demo(&[
        "John runs.",
        "Mary sleeps.",
        "Socrates thinks.",
        "The dog barks.",
    ]);

    section("TEMPORAL LOGIC (Past/Future)");
    demo(&[
        "John ran.",
        "John runs.",
        "John will run.",
        "Mary jumped.",
        "The dog barked.",
        "The cat will sleep.",
        "Socrates taught Plato.",
    ]);

    section("ASPECTUAL OPERATORS (Progressive)");
    demo(&[
        "John is running.",
        "Mary is sleeping.",
        "John was running.",
        "The dog was barking.",
        "Mary is reading.",
    ]);

    section("DEFINITENESS (Russell's Descriptions)");
    demo(&[
        "A dog barks.",
        "The dog barks.",
        "A cat sleeps.",
        "The cat ran.",
        "A man loves Mary.",
        "The king is bald.",
        "The president speaks.",
    ]);

    section("UNIVERSAL & EXISTENTIAL QUANTIFIERS");
    demo(&[
        "All men are mortal.",
        "Some cats are black.",
        "No dogs are cats.",
        "All birds fly.",
        "Some philosophers are wise.",
        "No fish can walk.",
        "Every student studies.",
    ]);

    section("GENERALIZED QUANTIFIERS");
    demo(&[
        "Most dogs bark.",
        "Few cats swim.",
        "Three dogs ran.",
        "At least two birds fly.",
        "At most five cats sleep.",
    ]);

    section("BINARY RELATIONS");
    demo(&[
        "John loves Mary.",
        "Mary loves John.",
        "Socrates taught Plato.",
        "The cat chased the mouse.",
        "Bill sees John.",
    ]);

    section("TERNARY RELATIONS (Ditransitives)");
    demo(&[
        "John gave the book to Mary.",
        "Mary sent a letter to John.",
    ]);

    section("REFLEXIVE BINDING");
    demo(&[
        "John loves himself.",
        "Mary sees herself.",
        "John gave the book to himself.",
        "The cat cleaned itself.",
    ]);

    section("RELATIVE CLAUSES (Subject Gap)");
    demo(&[
        "All dogs that bark are loud.",
        "All cats that sleep are lazy.",
        "All men who think are wise.",
        "All birds that fly are free.",
    ]);

    section("RELATIVE CLAUSES (Object Gap)");
    demo(&[
        "The cat that the dog chased ran.",
        "The man who Mary loves left.",
        "The book that John read is good.",
    ]);

    section("ADJECTIVES AS PREDICATES");
    demo(&[
        "All happy dogs are friendly.",
        "All old men are wise.",
        "Some tall women are athletes.",
        "All big cats are dangerous.",
    ]);

    section("MODAL OPERATORS (Alethic)");
    demo(&[
        "All cats must sleep.",
        "Some birds can fly.",
        "John can swim.",
        "All code cannot run.",
        "Mary can dance.",
    ]);

    section("MODAL OPERATORS (Deontic)");
    demo(&[
        "All students should study.",
        "John may leave.",
        "Mary should work.",
    ]);

    section("IDENTITY STATEMENTS");
    demo(&[
        "Clark is equal to Superman.",
        "Socrates is identical to Socrates.",
        "Hesperus is equal to Phosphorus.",
        "Bruce is equal to Batman.",
    ]);

    section("LOGICAL CONNECTIVES");
    demo(&[
        "John runs and Mary sleeps.",
        "John runs or Mary sleeps.",
        "If John runs, then Mary sleeps.",
        "A if and only if B.",
        "All men are mortal and some cats are black.",
    ]);

    section("WH-QUESTIONS");
    demo(&[
        "Who loves Mary?",
        "What does John love?",
    ]);

    section("YES/NO QUESTIONS");
    demo(&[
        "Does John love Mary?",
        "Does the dog bark?",
    ]);

    section("PASSIVE VOICE");
    demo(&[
        "Mary was loved by John.",
        "The book was read.",
    ]);

    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("                    SCOPE AMBIGUITY ANALYSIS");
    println!("═══════════════════════════════════════════════════════════════════\n");

    scope_demo("All dogs bark.");
    scope_demo("John loves Mary.");
    scope_demo("All men are mortal.");
    scope_demo("Some cats are black.");

    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("                       LaTeX OUTPUT");
    println!("═══════════════════════════════════════════════════════════════════\n");

    let latex_options = CompileOptions {
        format: OutputFormat::LaTeX,
    };

    let latex_examples = vec![
        "All men are mortal.",
        "John ran.",
        "John was running.",
        "The dog barks.",
        "A if and only if B.",
        "Who loves Mary?",
    ];

    for input in &latex_examples {
        println!("Input:  \"{}\"", input);
        match compile_with_options(input, latex_options) {
            Ok(output) => println!("LaTeX:  {}\n", output),
            Err(e) => println!("Error:  {:?}\n", e),
        }
    }

    section("COMPARATIVES (Degree Semantics)");
    demo(&[
        "John is taller than Mary.",
        "The dog is faster than the cat.",
        "Mary is smarter than John.",
        "Bill is older than Bob.",
    ]);

    section("SUPERLATIVES");
    demo(&[
        "John is the tallest man.",
        "Rex is the fastest dog.",
        "Mary is the smartest student.",
    ]);

    section("PLURALS & AGGREGATION (Mereology)");
    demo(&[
        "John and Mary met.",
        "John and Mary ran.",
        "The students gathered.",
        "Bill and Bob collaborated.",
    ]);

    section("EXISTENTIAL CLAIMS");
    demo(&["God is."]);

    section("SCOPAL ADVERBS");
    demo(&[
        "John almost died.",
        "Mary nearly won.",
        "John allegedly stole.",
        "Bill probably left.",
    ]);

    section("CONTROL THEORY (Subject Control)");
    demo(&[
        "John wants to run.",
        "Mary tried to leave.",
        "Bill hopes to win.",
        "John decided to stay.",
    ]);

    section("CONTROL THEORY (Object Control)");
    demo(&[
        "John persuaded Mary to leave.",
        "Bill forced John to run.",
        "Mary convinced Bill to stay.",
    ]);

    section("CONTROL THEORY (Promise - Special Case)");
    demo(&["John promised Mary to leave."]);

    section("PRESUPPOSITION TRIGGERS");
    demo(&[
        "John stopped smoking.",
        "Mary started running.",
        "John regrets leaving.",
        "Bill stopped working.",
        "Mary started singing.",
    ]);

    section("FOCUS OPERATORS");
    demo(&[
        "Only John loves Mary.",
        "Even John ran.",
        "Only Mary left.",
        "Even Bill won.",
    ]);

    section("MANNER ADVERBS (Neo-Davidsonian)");
    demo(&[
        "John ran quickly.",
        "Mary spoke loudly.",
        "Bill worked carefully.",
    ]);

    section("COUNTERFACTUALS");
    demo(&["If John had run, Mary would sleep."]);

    section("POSSESSION (Genitive Case)");
    demo(&[
        "John's dog barks.",
        "Mary's cat sleeps.",
        "The king's horse ran.",
        "The dog of John barks.",
        "John loves Mary's cat.",
        "John's dog chased the cat.",
    ]);

    section("DITRANSITIVE PASSIVES");
    demo(&[
        "The book was given to Mary by John.",
        "The letter was sent to Bill by Mary.",
        "The story was told to the children by the teacher.",
    ]);

    section("RAISING VERBS (vs Control)");
    demo(&[
        "John seems to sleep.",
        "Mary appears to run.",
        "John happens to win.",
        "John wants to run.",
    ]);

    section("TEMPORAL ADVERBS (Time Coordinates)");
    demo(&[
        "John ran yesterday.",
        "Mary runs today.",
        "Bill will leave tomorrow.",
        "John runs now.",
    ]);

    section("NON-INTERSECTIVE ADJECTIVES");
    demo(&[
        "A fake gun is dangerous.",
        "A former senator spoke.",
        "The alleged thief escaped.",
    ]);

    section("INTERSECTIVE VS NON-INTERSECTIVE");
    demo(&[
        "A red ball bounced.",
        "A fake ball bounced.",
    ]);

    println!("═══════════════════════════════════════════════════════════════════");
    println!("                         SUMMARY");
    println!("═══════════════════════════════════════════════════════════════════");
    println!("Logicaffeine 1.0 supports:");
    println!("  • First-Order Logic with N-ary predicates");
    println!("  • Temporal operators (P, F) for tense");
    println!("  • Aspectual operators (Prog, Perf)");
    println!("  • Russell's definite descriptions");
    println!("  • Generalized quantifiers (Most, Few, Cardinal, AtLeast, AtMost)");
    println!("  • Modal logic (Alethic □/◇, Deontic O/P)");
    println!("  • Lambda calculus (β-reduction)");
    println!("  • Scope ambiguity enumeration");
    println!("  • Wh-questions as lambda abstractions");
    println!("  • Comparatives & Superlatives (Degree Semantics)");
    println!("  • Plurals & Aggregation (Mereology)");
    println!("  • Scopal Adverbs (Almost, Nearly, Allegedly, Probably)");
    println!("  • Manner Adverbs (Neo-Davidsonian event semantics)");
    println!("  • Existential claims (bare copula)");
    println!("  • Control Theory (PRO binding - Subject/Object control)");
    println!("  • Presupposition triggers (Stop, Start, Regret)");
    println!("  • Focus operators (Only, Even)");
    println!("  • Counterfactual conditionals");
    println!("  • Reflexive binding (himself, herself, itself)");
    println!("  • Relative clauses (subject-gap & object-gap)");
    println!("  • Passive voice");
    println!("  • Identity statements");
    println!("  • Possession / Genitive case ('s and 'of' constructions)");
    println!("  • Ditransitive passives ('given to X by Y')");
    println!("  • Raising verbs (seem, appear, happen) vs Control verbs");
    println!("  • Temporal adverbs (yesterday, today, tomorrow, now)");
    println!("  • Non-intersective adjectives (fake, former, alleged)");
    println!("═══════════════════════════════════════════════════════════════════\n");
}

fn section(title: &str) {
    println!("--- {} ---\n", title);
}

fn demo(sentences: &[&str]) {
    for input in sentences {
        println!("  \"{}\"", input);
        match compile(input) {
            Ok(output) => println!("  → {}\n", output),
            Err(e) => println!("  → Error: {:?}\n", e),
        }
    }
}

fn scope_demo(input: &str) {
    println!("Input: \"{}\"", input);
    match compile_all_scopes(input) {
        Ok(readings) => {
            println!("Readings: {}", readings.len());
            for (i, reading) in readings.iter().enumerate() {
                println!("  [{}] {}", i + 1, reading);
            }
        }
        Err(e) => println!("Error: {:?}", e),
    }
    println!();
}
