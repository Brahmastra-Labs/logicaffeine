//! Example files for the Studio playground.
//!
//! These are seeded into the VFS on first launch to give users
//! something to work with immediately.

use logos_core::fs::{Vfs, VfsResult};

/// Seed example files into the VFS if they don't exist.
pub async fn seed_examples<V: Vfs>(vfs: &V) -> VfsResult<()> {
    let is_fresh_install = !vfs.exists("/examples").await?;

    // Create directory structure (always - create_dir_all is idempotent)
    vfs.create_dir_all("/examples/logic").await?;
    vfs.create_dir_all("/examples/code").await?;
    vfs.create_dir_all("/examples/math").await?;
    vfs.create_dir_all("/workspace").await?;

    // New code example subdirectories
    vfs.create_dir_all("/examples/code/basics").await?;
    vfs.create_dir_all("/examples/code/types").await?;
    vfs.create_dir_all("/examples/code/collections").await?;
    vfs.create_dir_all("/examples/code/functions").await?;
    vfs.create_dir_all("/examples/code/distributed").await?;
    vfs.create_dir_all("/examples/code/security").await?;
    vfs.create_dir_all("/examples/code/memory").await?;
    vfs.create_dir_all("/examples/code/concurrency").await?;
    vfs.create_dir_all("/examples/code/networking").await?;
    vfs.create_dir_all("/examples/code/advanced").await?;
    vfs.create_dir_all("/examples/code/native").await?;

    // For existing installs, only seed new advanced examples (skip base examples)
    if !is_fresh_install {
        seed_advanced_code_examples(vfs).await?;
        return Ok(());
    }

    // Seed Logic mode examples
    vfs.write("/examples/logic/simple-sentences.logic", LOGIC_SIMPLE.as_bytes()).await?;
    vfs.write("/examples/logic/quantifiers.logic", LOGIC_QUANTIFIERS.as_bytes()).await?;
    vfs.write("/examples/logic/tense-aspect.logic", LOGIC_TENSE.as_bytes()).await?;

    // Seed prover examples (theorem proving with derivation trees)
    vfs.write("/examples/logic/prover-demo.logic", LOGIC_PROVER.as_bytes()).await?;
    vfs.write("/examples/logic/syllogism.logic", LOGIC_SYLLOGISM.as_bytes()).await?;
    vfs.write("/examples/logic/trivial-proof.logic", LOGIC_TRIVIAL.as_bytes()).await?;
    vfs.write("/examples/logic/disjunctive-syllogism.logic", LOGIC_DISJUNCTIVE.as_bytes()).await?;
    vfs.write("/examples/logic/modus-tollens.logic", LOGIC_MODUS_TOLLENS.as_bytes()).await?;
    vfs.write("/examples/logic/leibniz-identity.logic", LOGIC_LEIBNIZ.as_bytes()).await?;
    vfs.write("/examples/logic/barber-paradox.logic", LOGIC_BARBER.as_bytes()).await?;

    // Seed Code mode examples (imperative)
    vfs.write("/examples/code/hello-world.logos", CODE_HELLO.as_bytes()).await?;
    vfs.write("/examples/code/fibonacci.logos", CODE_FIBONACCI.as_bytes()).await?;
    vfs.write("/examples/code/fizzbuzz.logos", CODE_FIZZBUZZ.as_bytes()).await?;
    vfs.write("/examples/code/collections.logos", CODE_COLLECTIONS.as_bytes()).await?;
    vfs.write("/examples/code/factorial.logos", CODE_FACTORIAL.as_bytes()).await?;
    vfs.write("/examples/code/prime-check.logos", CODE_PRIME.as_bytes()).await?;
    vfs.write("/examples/code/sum-list.logos", CODE_SUM_LIST.as_bytes()).await?;
    vfs.write("/examples/code/bubble-sort.logos", CODE_BUBBLE_SORT.as_bytes()).await?;
    vfs.write("/examples/code/struct-demo.logos", CODE_STRUCT.as_bytes()).await?;

    // Seed advanced Code mode examples (organized by category)
    // Type system
    vfs.write("/examples/code/types/enums.logos", CODE_ENUMS.as_bytes()).await?;
    vfs.write("/examples/code/types/generics.logos", CODE_GENERICS.as_bytes()).await?;
    // Collections
    vfs.write("/examples/code/collections/sets.logos", CODE_SETS.as_bytes()).await?;
    vfs.write("/examples/code/collections/maps.logos", CODE_MAPS.as_bytes()).await?;
    // Functions
    vfs.write("/examples/code/functions/higher-order.logos", CODE_HIGHER_ORDER.as_bytes()).await?;
    // Distributed
    vfs.write("/examples/code/distributed/counters.logos", CODE_CRDT_COUNTERS.as_bytes()).await?;
    // Security
    vfs.write("/examples/code/security/policies.logos", CODE_POLICIES.as_bytes()).await?;
    // Memory
    vfs.write("/examples/code/memory/zones.logos", CODE_ZONES.as_bytes()).await?;
    // Native-only (concurrency)
    vfs.write("/examples/code/native/tasks.logos", CODE_TASKS.as_bytes()).await?;
    vfs.write("/examples/code/native/channels.logos", CODE_CHANNELS.as_bytes()).await?;

    // NEW: Basics examples (guide sections 3-5)
    vfs.write("/examples/code/basics/variables.logos", CODE_BASICS_VARIABLES.as_bytes()).await?;
    vfs.write("/examples/code/basics/operators.logos", CODE_BASICS_OPERATORS.as_bytes()).await?;
    vfs.write("/examples/code/basics/control-flow.logos", CODE_BASICS_CONTROL_FLOW.as_bytes()).await?;

    // NEW: Enum patterns example (guide section 8)
    vfs.write("/examples/code/types/enums-patterns.logos", CODE_ENUMS_PATTERNS.as_bytes()).await?;

    // NEW: Ownership example (guide section 10)
    vfs.write("/examples/code/memory/ownership.logos", CODE_OWNERSHIP.as_bytes()).await?;

    // NEW: Concurrency example (guide section 12) - browser compatible
    vfs.write("/examples/code/concurrency/parallel.logos", CODE_CONCURRENCY_PARALLEL.as_bytes()).await?;

    // NEW: Additional distributed examples (guide section 13)
    vfs.write("/examples/code/distributed/tally.logos", CODE_CRDT_TALLY.as_bytes()).await?;
    vfs.write("/examples/code/distributed/merge.logos", CODE_CRDT_MERGE.as_bytes()).await?;

    // NEW: Networking examples (guide section 15) - native only
    vfs.write("/examples/code/networking/server.logos", CODE_NETWORK_SERVER.as_bytes()).await?;
    vfs.write("/examples/code/networking/client.logos", CODE_NETWORK_CLIENT.as_bytes()).await?;

    // NEW: Error handling example (guide section 16)
    vfs.write("/examples/code/error-handling.logos", CODE_ERROR_HANDLING.as_bytes()).await?;

    // NEW: Advanced examples (guide sections 17, 22-23)
    vfs.write("/examples/code/advanced/refinement.logos", CODE_ADVANCED_REFINEMENT.as_bytes()).await?;
    vfs.write("/examples/code/advanced/assertions.logos", CODE_ADVANCED_ASSERTIONS.as_bytes()).await?;

    // Seed Math mode examples (vernacular/theorem proving)
    vfs.write("/examples/math/natural-numbers.logos", MATH_NAT.as_bytes()).await?;
    vfs.write("/examples/math/boolean-logic.logos", MATH_BOOL.as_bytes()).await?;
    vfs.write("/examples/math/godel-sentence.logos", MATH_GODEL.as_bytes()).await?;
    vfs.write("/examples/math/incompleteness.logos", MATH_INCOMPLETENESS.as_bytes()).await?;
    vfs.write("/examples/math/prop-logic.logos", MATH_PROP_LOGIC.as_bytes()).await?;
    vfs.write("/examples/math/functions.logos", MATH_FUNCTIONS.as_bytes()).await?;
    vfs.write("/examples/math/list-ops.logos", MATH_LIST_OPS.as_bytes()).await?;
    vfs.write("/examples/math/pairs.logos", MATH_PAIRS.as_bytes()).await?;

    Ok(())
}

/// Seed only the advanced code examples (for existing installations).
/// Always overwrites to ensure latest syntax is used.
async fn seed_advanced_code_examples<V: Vfs>(vfs: &V) -> VfsResult<()> {
    // Create new directories for existing installs
    vfs.create_dir_all("/examples/code/basics").await?;
    vfs.create_dir_all("/examples/code/concurrency").await?;
    vfs.create_dir_all("/examples/code/networking").await?;
    vfs.create_dir_all("/examples/code/advanced").await?;

    // Type system
    vfs.write("/examples/code/types/enums.logos", CODE_ENUMS.as_bytes()).await?;
    vfs.write("/examples/code/types/generics.logos", CODE_GENERICS.as_bytes()).await?;
    vfs.write("/examples/code/types/enums-patterns.logos", CODE_ENUMS_PATTERNS.as_bytes()).await?;
    // Collections
    vfs.write("/examples/code/collections/sets.logos", CODE_SETS.as_bytes()).await?;
    vfs.write("/examples/code/collections/maps.logos", CODE_MAPS.as_bytes()).await?;
    // Functions
    vfs.write("/examples/code/functions/higher-order.logos", CODE_HIGHER_ORDER.as_bytes()).await?;
    // Distributed
    vfs.write("/examples/code/distributed/counters.logos", CODE_CRDT_COUNTERS.as_bytes()).await?;
    vfs.write("/examples/code/distributed/tally.logos", CODE_CRDT_TALLY.as_bytes()).await?;
    vfs.write("/examples/code/distributed/merge.logos", CODE_CRDT_MERGE.as_bytes()).await?;
    // Security
    vfs.write("/examples/code/security/policies.logos", CODE_POLICIES.as_bytes()).await?;
    // Memory
    vfs.write("/examples/code/memory/zones.logos", CODE_ZONES.as_bytes()).await?;
    vfs.write("/examples/code/memory/ownership.logos", CODE_OWNERSHIP.as_bytes()).await?;
    // Concurrency (browser compatible)
    vfs.write("/examples/code/concurrency/parallel.logos", CODE_CONCURRENCY_PARALLEL.as_bytes()).await?;
    // Networking (native only)
    vfs.write("/examples/code/networking/server.logos", CODE_NETWORK_SERVER.as_bytes()).await?;
    vfs.write("/examples/code/networking/client.logos", CODE_NETWORK_CLIENT.as_bytes()).await?;
    // Advanced
    vfs.write("/examples/code/advanced/refinement.logos", CODE_ADVANCED_REFINEMENT.as_bytes()).await?;
    vfs.write("/examples/code/advanced/assertions.logos", CODE_ADVANCED_ASSERTIONS.as_bytes()).await?;
    // Basics
    vfs.write("/examples/code/basics/variables.logos", CODE_BASICS_VARIABLES.as_bytes()).await?;
    vfs.write("/examples/code/basics/operators.logos", CODE_BASICS_OPERATORS.as_bytes()).await?;
    vfs.write("/examples/code/basics/control-flow.logos", CODE_BASICS_CONTROL_FLOW.as_bytes()).await?;
    // Error handling
    vfs.write("/examples/code/error-handling.logos", CODE_ERROR_HANDLING.as_bytes()).await?;
    // Native-only (concurrency)
    vfs.write("/examples/code/native/tasks.logos", CODE_TASKS.as_bytes()).await?;
    vfs.write("/examples/code/native/channels.logos", CODE_CHANNELS.as_bytes()).await?;

    // Logic examples (force update for existing installs)
    vfs.write("/examples/logic/barber-paradox.logic", LOGIC_BARBER.as_bytes()).await?;
    vfs.write("/examples/logic/modus-tollens.logic", LOGIC_MODUS_TOLLENS.as_bytes()).await?;
    vfs.write("/examples/logic/simple-sentences.logic", LOGIC_SIMPLE.as_bytes()).await?;
    vfs.write("/examples/logic/quantifiers.logic", LOGIC_QUANTIFIERS.as_bytes()).await?;
    vfs.write("/examples/logic/tense-aspect.logic", LOGIC_TENSE.as_bytes()).await?;
    vfs.write("/examples/logic/prover-demo.logic", LOGIC_PROVER.as_bytes()).await?;
    vfs.write("/examples/logic/syllogism.logic", LOGIC_SYLLOGISM.as_bytes()).await?;
    vfs.write("/examples/logic/trivial-proof.logic", LOGIC_TRIVIAL.as_bytes()).await?;
    vfs.write("/examples/logic/disjunctive-syllogism.logic", LOGIC_DISJUNCTIVE.as_bytes()).await?;
    vfs.write("/examples/logic/leibniz-identity.logic", LOGIC_LEIBNIZ.as_bytes()).await?;

    // Math examples (ensure they exist for all installs)
    vfs.create_dir_all("/examples/math").await?;
    vfs.write("/examples/math/natural-numbers.logos", MATH_NAT.as_bytes()).await?;
    vfs.write("/examples/math/boolean-logic.logos", MATH_BOOL.as_bytes()).await?;
    vfs.write("/examples/math/godel-sentence.logos", MATH_GODEL.as_bytes()).await?;
    vfs.write("/examples/math/incompleteness.logos", MATH_INCOMPLETENESS.as_bytes()).await?;
    vfs.write("/examples/math/prop-logic.logos", MATH_PROP_LOGIC.as_bytes()).await?;
    vfs.write("/examples/math/functions.logos", MATH_FUNCTIONS.as_bytes()).await?;
    vfs.write("/examples/math/list-ops.logos", MATH_LIST_OPS.as_bytes()).await?;
    vfs.write("/examples/math/pairs.logos", MATH_PAIRS.as_bytes()).await?;

    // Base code examples (ensure they exist for all installs)
    vfs.write("/examples/code/hello-world.logos", CODE_HELLO.as_bytes()).await?;
    vfs.write("/examples/code/fibonacci.logos", CODE_FIBONACCI.as_bytes()).await?;
    vfs.write("/examples/code/fizzbuzz.logos", CODE_FIZZBUZZ.as_bytes()).await?;
    vfs.write("/examples/code/collections.logos", CODE_COLLECTIONS.as_bytes()).await?;
    vfs.write("/examples/code/factorial.logos", CODE_FACTORIAL.as_bytes()).await?;
    vfs.write("/examples/code/prime-check.logos", CODE_PRIME.as_bytes()).await?;
    vfs.write("/examples/code/sum-list.logos", CODE_SUM_LIST.as_bytes()).await?;
    vfs.write("/examples/code/bubble-sort.logos", CODE_BUBBLE_SORT.as_bytes()).await?;
    vfs.write("/examples/code/struct-demo.logos", CODE_STRUCT.as_bytes()).await?;

    Ok(())
}

// ============================================================
// Logic Mode Examples (English -> FOL)
// ============================================================

const LOGIC_SIMPLE: &str = r#"# Simple Sentences

Every cat sleeps.
Some dogs bark loudly.
John loves Mary.
The quick brown fox jumps.
No student failed.
"#;

const LOGIC_QUANTIFIERS: &str = r#"# Quantifier Scope

Every student read a book.
A professor supervises every student.
No student failed every exam.
Some teacher praised every student.
Every dog chased some cat.
"#;

const LOGIC_TENSE: &str = r#"# Tense and Aspect

John was running.
Mary has eaten.
The train will arrive.
She had been sleeping.
They have been working.
"#;

// ============================================================
// Logic Mode Examples (Prover/Theorem Proving)
// ============================================================

const LOGIC_PROVER: &str = r#"## Theorem: Socrates_Mortality
Given: All men are mortal.
Given: Socrates is a man.
Prove: Socrates is mortal.
Proof: Auto.
"#;

const LOGIC_SYLLOGISM: &str = r#"## Theorem: Chain_Reasoning
Given: All men are mortal.
Given: All mortals are doomed.
Given: Plato is a man.
Prove: Plato is doomed.
Proof: Auto.
"#;

const LOGIC_TRIVIAL: &str = r#"## Theorem: Direct_Match
Given: Socrates is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

const LOGIC_DISJUNCTIVE: &str = r#"## Theorem: Disjunctive_Syllogism
Given: Either Alice or Bob is guilty.
Given: Alice is not guilty.
Prove: Bob is guilty.
Proof: Auto.
"#;

const LOGIC_MODUS_TOLLENS: &str = r#"## Theorem: Modus_Tollens_Chain
Given: If the butler did it, he was seen.
Given: If he was seen, he was caught.
Given: He was not caught.
Prove: The butler did not do it.
Proof: Auto.
"#;

const LOGIC_LEIBNIZ: &str = r#"## Theorem: Leibniz_Identity
Given: Clark is Superman.
Given: Clark is mortal.
Prove: Superman is mortal.
Proof: Auto.
"#;

const LOGIC_BARBER: &str = r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#;

// ============================================================
// Code Mode Examples (Imperative LOGOS)
// ============================================================

const CODE_HELLO: &str = r#"## Main

Let greeting be "Hello, LOGOS!".
Show greeting.

Let x be 10.
Let y be 20.
Let sum be x + y.

Show "The sum is:".
Show sum.
"#;

const CODE_FIBONACCI: &str = r#"## Main

Let n be 10.
Let a be 0.
Let b be 1.

Show "Fibonacci sequence:".
Show a.

Repeat for i from 1 to n:
    Show b.
    Let temp be a + b.
    Set a to b.
    Set b to temp.
"#;

const CODE_FIZZBUZZ: &str = r#"## Main

Repeat for i from 1 to 20:
    If i / 15 * 15 equals i:
        Show "FizzBuzz".
    Otherwise:
        If i / 3 * 3 equals i:
            Show "Fizz".
        Otherwise:
            If i / 5 * 5 equals i:
                Show "Buzz".
            Otherwise:
                Show i.
"#;

const CODE_COLLECTIONS: &str = r#"## Main

Let numbers be [1, 2, 3, 4, 5].
Show "Numbers:".
Show numbers.

Push 6 to numbers.
Show "After push:".
Show numbers.

Show "Length:".
Show length of numbers.

Show "First item:".
Show item 1 of numbers.

Show "Last item:".
Show item 6 of numbers.
"#;

const CODE_FACTORIAL: &str = r#"## To factorial (n: Int):
    If n <= 1:
        Return 1.
    Return n * factorial(n - 1).

## Main

Show "Factorial of 5:".
Let result be factorial(5).
Show result.

Show "Factorial of 10:".
Let big be factorial(10).
Show big.
"#;

const CODE_PRIME: &str = r#"## To is_prime (n: Int) -> Bool:
    If n <= 1:
        Return false.
    Let i be 2.
    While i * i <= n:
        If n / i * i equals n:
            Return false.
        Set i to i + 1.
    Return true.

## Main

Show "Prime numbers from 2 to 30:".
Repeat for num from 2 to 30:
    If is_prime(num):
        Show num.
"#;

const CODE_SUM_LIST: &str = r#"## Main

Let numbers be [10, 20, 30, 40, 50].
Let total be 0.

Repeat for n in numbers:
    Set total to total + n.

Show "Sum of [10, 20, 30, 40, 50]:".
Show total.
"#;

const CODE_BUBBLE_SORT: &str = r#"## Main

Let numbers be [64, 34, 25, 12, 22, 11, 90].
Let n be length of numbers.

Show "Before sorting:".
Show numbers.

Repeat for i from 1 to n:
    Repeat for j from 1 to (n - i):
        Let a be item j of numbers.
        Let b be item (j + 1) of numbers.
        If a > b:
            Set item j of numbers to b.
            Set item (j + 1) of numbers to a.

Show "After sorting:".
Show numbers.
"#;

const CODE_STRUCT: &str = r#"## Definition

A Person has:
    a public name, which is Text.
    a public age, which is Int.

## Main

Let alice be a new Person.
Set alice's name to "Alice".
Set alice's age to 30.

Let bob be a new Person.
Set bob's name to "Bob".
Set bob's age to 25.

Show "Person 1:".
Show alice's name.
Show alice's age.

Show "Person 2:".
Show bob's name.
Show bob's age.
"#;

// ============================================================
// Advanced Code Mode Examples (organized by category)
// ============================================================

// --- Type System ---

const CODE_ENUMS: &str = r#"# Enums & Pattern Matching

## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main

Let c be a new Red.
Inspect c:
    When Red: Show "It's red!".
    When Green: Show "It's green!".
    When Blue: Show "It's blue!".

Let c2 be a new Blue.
Inspect c2:
    When Red: Show "red".
    Otherwise: Show "not red".
"#;

const CODE_GENERICS: &str = r#"## Main

Let mut scores be a new Map of Text to Int.
Set scores["Alice"] to 100.
Set scores["Bob"] to 85.
Set scores["Charlie"] to 92.

Let alice_score be scores["Alice"].
Show "Alice's score:".
Show alice_score.

Set scores["Bob"] to 90.
Show "Bob's new score:".
Show scores["Bob"].

Let total be scores["Alice"] + scores["Bob"] + scores["Charlie"].
Show "Total:".
Show total.
"#;

// --- Collections ---

const CODE_SETS: &str = r#"## Main

Let names be a new Set of Text.
Add "Alice" to names.
Add "Bob" to names.
Add "Charlie" to names.
Add "Alice" to names.

Show "Set size (duplicates ignored):".
Show length of names.

If names contains "Bob":
    Show "Bob is in the set!".

Remove "Bob" from names.
Show "After removing Bob:".
Show length of names.

Let sum be 0.
Let numbers be a new Set of Int.
Add 10 to numbers.
Add 20 to numbers.
Add 30 to numbers.
Repeat for n in numbers:
    Set sum to sum + n.
Show "Sum of numbers:".
Show sum.
"#;

const CODE_MAPS: &str = r#"## Main

Let mut inventory be a new Map of Text to Int.
Set item "iron" of inventory to 50.
Set inventory["copper"] to 30.
Set inventory["gold"] to 10.

Show "Iron count:".
Show item "iron" of inventory.

Show "Copper count:".
Show inventory["copper"].

Set inventory["iron"] to 100.
Show "Updated iron:".
Show inventory["iron"].

Let total be item "iron" of inventory + inventory["copper"] + inventory["gold"].
Show "Total resources:".
Show total.
"#;

// --- Functions ---

const CODE_HIGHER_ORDER: &str = r#"## To double (x: Int):
    Return x * 2.

## To add (a: Int) and (b: Int):
    Return a + b.

## To isEven (n: Int) -> Bool:
    Return n / 2 * 2 equals n.

## Main

Show "Double of 21:".
Show double(21).

Show "Sum of 15 and 27:".
Show add(15, 27).

Show "Is 42 even?".
Show isEven(42).

Show "Is 17 even?".
Show isEven(17).
"#;

// --- Distributed ---

const CODE_CRDT_COUNTERS: &str = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Increase c's points by 5.
Increase c's points by 3.
Show "Total points:".
Show c's points.
"#;

// --- Security ---

const CODE_POLICIES: &str = r#"# Security Policies

## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main

Let u be a new User with role "admin".
Check that the u is admin.
Show "Admin check passed!".

Let guest be a new User with role "guest".
Show "Guest created (would fail admin check)".
"#;

// --- Memory ---

const CODE_ZONES: &str = r#"# Memory Zones

## Main

Show "Working with memory zones...".

Inside a zone called "Work":
    Let x be 42.
    Let y be 58.
    Let sum be x + y.
    Show "Sum in zone:".
    Show sum.

Inside a zone called "Buffer" of size 1 MB:
    Let value be 100.
    Show "Value in sized zone:".
    Show value.

Show "Zones cleaned up!".
"#;

// --- Native-only (Concurrency) ---

const CODE_TASKS: &str = r#"## To worker:
    Show "worker done".

## To greet (name: Text):
    Show name.

## Main

Launch a task to worker.
Show "main continues".

Launch a task to greet with "Hello from task".
Show "task launched".
"#;

const CODE_CHANNELS: &str = r#"## Main

Let ch be a Pipe of Int.
Show "pipe created".

Send 42 into ch.
Show "sent 42".

Receive x from ch.
Show "received:".
Show x.
"#;

// ============================================================
// Math Mode Examples (Vernacular/Theorem Proving)
// ============================================================

const MATH_NAT: &str = r#"-- Natural Numbers
-- The foundation of arithmetic in type theory

-- Define the natural number type
Inductive Nat := Zero : Nat | Succ : Nat -> Nat.

-- Define some numbers
Definition one : Nat := Succ Zero.
Definition two : Nat := Succ one.
Definition three : Nat := Succ two.

-- Check the types
Check Zero.
Check Succ.
Check one.
Check two.

-- Evaluate expressions
Eval three.
"#;

const MATH_BOOL: &str = r#"Inductive MyBool := Yes : MyBool | No : MyBool.

Check Yes.
Check No.
Eval Yes.
Eval No.

Definition id_bool : MyBool -> MyBool := fun b : MyBool => b.

Check id_bool.
Eval id_bool Yes.
Eval id_bool No.
"#;

const MATH_GODEL: &str = r#"-- Godel Sentence Construction
-- Building the self-referential sentence G

-- The Provable predicate: "there exists a derivation concluding s"
Definition Provable : Syntax -> Prop :=
  fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).

-- The Godel template T = "Not(Provable(x))"
-- When we apply the diagonal lemma, x becomes the code of T itself
Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).

-- The Godel sentence G = T[code(T)/x]
-- G says "I am not provable"
Definition G : Syntax := syn_diag T.

-- Check our constructions
Check Provable.
Check T.
Check G.

-- G has type Syntax (it's a syntactic object)
-- But Provable G has type Prop (it's a proposition)
Check (Provable G).
"#;

const MATH_INCOMPLETENESS: &str = r#"-- Godel's First Incompleteness Theorem
-- If LOGOS is consistent, G is not provable

-- Setup: Provable predicate
Definition Provable : Syntax -> Prop :=
  fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).

-- Consistency: the system cannot prove False
Definition Consistent : Prop := Not (Provable (SName "False")).

-- The Godel template and sentence
Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).
Definition G : Syntax := syn_diag T.

-- THE THEOREM STATEMENT
-- "If LOGOS is consistent, then G is not provable"
Definition Godel_I : Prop := Consistent -> Not (Provable G).

-- Check that our theorem statement is well-typed
Check Godel_I.
Check Consistent.
Check (Provable G).
Check (Not (Provable G)).

-- This is a proposition (a type in Prop)
-- A proof would be a term of this type
"#;

const MATH_PROP_LOGIC: &str = r#"-- Propositional Logic Types
-- Encoding logical connectives as types

Inductive MyProp :=
    PTrue : MyProp
  | PFalse : MyProp
  | PAnd : MyProp -> MyProp -> MyProp
  | POr : MyProp -> MyProp -> MyProp
  | PNot : MyProp -> MyProp.

-- Some example propositions
Definition p1 : MyProp := PTrue.
Definition p2 : MyProp := PFalse.
Definition p3 : MyProp := PAnd PTrue PTrue.
Definition p4 : MyProp := POr PTrue PFalse.
Definition p5 : MyProp := PNot PFalse.

-- Check and evaluate
Check p3.
Check p4.
Check p5.
Eval p3.
Eval p4.
Eval p5.
"#;

const MATH_FUNCTIONS: &str = r#"-- Simple Functions
-- Lambda calculus basics

-- Identity function
Definition id : Nat -> Nat := fun x : Nat => x.

-- Constant function
Definition const_zero : Nat -> Nat := fun x : Nat => Zero.

-- Apply successor twice
Definition double_succ : Nat -> Nat := fun x : Nat => Succ (Succ x).

-- Check types
Check id.
Check const_zero.
Check double_succ.

-- Evaluate some applications
Definition one : Nat := Succ Zero.
Definition two : Nat := Succ one.

Eval id one.
Eval const_zero two.
Eval double_succ one.
"#;

const MATH_LIST_OPS: &str = r#"-- List Operations
-- Polymorphic lists in type theory

-- Define a list type (built-in, but showing the structure)
Inductive MyList (A : Type) :=
    MyNil : MyList A
  | MyCons : A -> MyList A -> MyList A.

-- Example: a list of natural numbers
Definition nat_list : MyList Nat := MyCons Nat Zero (MyCons Nat (Succ Zero) (MyNil Nat)).

-- Check the types
Check MyNil.
Check MyCons.
Check nat_list.

-- Evaluate
Eval nat_list.
"#;

const MATH_PAIRS: &str = r#"-- Pairs and Products
-- Cartesian product types

Inductive MyPair (A : Type) (B : Type) :=
    MkPair : A -> B -> MyPair A B.

-- Example pairs
Definition nat_bool_pair : MyPair Nat MyBool := MkPair Nat MyBool Zero Yes.
Definition nat_nat_pair : MyPair Nat Nat := MkPair Nat Nat Zero (Succ Zero).

-- Check types
Check MkPair.
Check nat_bool_pair.
Check nat_nat_pair.

-- Evaluate
Eval nat_bool_pair.
Eval nat_nat_pair.
"#;

// ============================================================
// NEW: Basics Examples (Guide Sections 3-5)
// ============================================================

const CODE_BASICS_VARIABLES: &str = r#"# Variables and Types
-- Guide Section 3: All primitive types

## Main

Let name be "Alice".
Let age be 25.
Let is_active be true.
Let price be 19.99.

Show "Name: " + name.
Show "Age: " + age.
Show "Active: " + is_active.
Show "Price: " + price.

Let count be 100.
Let doubled be count * 2.
Show "Doubled: " + doubled.
"#;

const CODE_BASICS_OPERATORS: &str = r#"# Operators and Expressions
-- Guide Section 4: Arithmetic, comparisons, logical

## Main

Let a be 10.
Let b be 3.

Show "Arithmetic:".
Show "a + b = " + (a + b).
Show "a - b = " + (a - b).
Show "a * b = " + (a * b).
Show "a / b = " + (a / b).
Show "a % b = " + (a % b).

Show "Comparisons:".
Show "a > b?".
Show a is greater than b.
Show "a equals 10?".
Show a equals 10.
Show "a >= 5?".
Show a is at least 5.

Show "Logical:".
Let x be true.
Let y be false.
Show "x and y:".
Show x and y.
Show "x or y:".
Show x or y.
Show "not x:".
Show not x.
"#;

const CODE_BASICS_CONTROL_FLOW: &str = r#"# Control Flow
-- Guide Section 5: If/Otherwise, While, For-each

## Main

Let score be 85.

Show "Grading:".
If score is at least 90:
    Show "Grade: A".
If score is at least 80 and score is less than 90:
    Show "Grade: B".
If score is less than 80:
    Show "Grade: C or below".

Show "While loop:".
Let count be 1.
While count is at most 3:
    Show count.
    Set count to count + 1.

Show "For-each loop:".
Let items be [10, 20, 30].
Repeat for n in items:
    Show n.
"#;

// ============================================================
// NEW: Enum Patterns Example (Guide Section 8)
// ============================================================

const CODE_ENUMS_PATTERNS: &str = r#"# Enums and Pattern Matching
-- Guide Section 8: Full pattern matching demonstration

## A Status is one of:
    A Pending.
    A Active.
    A Completed.
    A Failed.

## Main

Let s be a new Active.
Show "Current status:".
Inspect s:
    When Pending: Show "Waiting to start".
    When Active: Show "In progress".
    When Completed: Show "Done!".
    When Failed: Show "Error occurred".

Let s2 be a new Completed.
Inspect s2:
    When Active: Show "still working".
    Otherwise: Show "not active".
"#;

// ============================================================
// NEW: Ownership Example (Guide Section 10)
// ============================================================

const CODE_OWNERSHIP: &str = r#"# Memory and Ownership
-- Guide Section 10: Give, Show, copy of

## To display (data: Text):
    Show "Viewing: " + data.

## To consume (data: Text):
    Show "Consumed: " + data.

## Main

Let profile be "User Profile Data".

Show profile to display.
Show "Still have profile: " + profile.

Let duplicate be copy of profile.
Give duplicate to consume.

Show "Original intact: " + profile.
"#;

// ============================================================
// NEW: Concurrency Example (Guide Section 12)
// ============================================================

const CODE_CONCURRENCY_PARALLEL: &str = r#"# Concurrency
-- Guide Section 12: Simultaneously and Attempt all
-- These work in the browser!

## Main

Show "Parallel computation:".
Simultaneously:
    Let a be 100.
    Let b be 200.

Show "a = " + a.
Show "b = " + b.
Show "Product: " + (a * b).

Show "Async concurrent:".
Attempt all of the following:
    Let x be 10.
    Let y be 20.

Show "Sum: " + (x + y).
"#;

// ============================================================
// NEW: Additional CRDT Examples (Guide Section 13)
// ============================================================

const CODE_CRDT_TALLY: &str = r#"# Tally (Bidirectional Counter)
-- Guide Section 13: PN-Counter that can increase and decrease

## Definition
A Score is Shared and has:
    points: Tally.

## Main
Let mutable s be a new Score.
Increase s's points by 100.
Show "After +100: " + s's points.

Decrease s's points by 30.
Show "After -30: " + s's points.

Increase s's points by 10.
Show "Final: " + s's points.
"#;

const CODE_CRDT_MERGE: &str = r#"# CRDT Merge
-- Guide Section 13: Merging replicas

## Definition
A Stats is Shared and has:
    views: ConvergentCount.

## Main
Let local be a new Stats.
Increase local's views by 100.
Show "Local views: " + local's views.

Let remote be a new Stats.
Increase remote's views by 50.
Show "Remote views: " + remote's views.

Merge remote into local.
Show "After merge: " + local's views.
"#;

// ============================================================
// NEW: Networking Examples (Guide Section 15) - Native Only
// ============================================================

const CODE_NETWORK_SERVER: &str = r#"# P2P Server
-- Guide Section 15: Listen and mDNS discovery
-- NOTE: Compiled programs only (not browser)

## Definition
A Message is Portable and has:
    content: Text.

## Main

Listen on "/ip4/0.0.0.0/tcp/8000".
Show "Server listening on port 8000".
Show "mDNS will auto-discover local peers".
"#;

const CODE_NETWORK_CLIENT: &str = r#"# P2P Client
-- Guide Section 15: Connect, PeerAgent, Send
-- NOTE: Compiled programs only (not browser)

## Definition
A Greeting is Portable and has:
    message: Text.

## Main

Let server be "/ip4/127.0.0.1/tcp/8000".
Connect to server.
Show "Connected!".

Let remote be a PeerAgent at server.
Let msg be a new Greeting with message "Hello, peer!".
Send msg to remote.
Show "Message sent".
"#;

// ============================================================
// NEW: Error Handling Example (Guide Section 16)
// ============================================================

const CODE_ERROR_HANDLING: &str = r#"# Error Handling
-- Guide Section 16: Defensive programming patterns

## To safe_divide (a: Int) and (b: Int) -> Int:
    If b equals 0:
        Show "Error: Cannot divide by zero".
        Return 0.
    Return a / b.

## To validate_age (age: Int) -> Bool:
    If age is less than 0:
        Show "Error: Age cannot be negative".
        Return false.
    If age is greater than 150:
        Show "Error: Age seems unrealistic".
        Return false.
    Return true.

## Main

Show "Safe division:".
Show "10 / 2 = " + safe_divide(10, 2).
Show "5 / 0 = " + safe_divide(5, 0).

Show "Age validation:".
Show "Age 25 valid: " + validate_age(25).
Show "Age -5 valid: " + validate_age(-5).
Show "Age 200 valid: " + validate_age(200).
"#;

// ============================================================
// NEW: Advanced Examples (Guide Sections 17, 22-23)
// ============================================================

const CODE_ADVANCED_REFINEMENT: &str = r#"# Refinement Types
-- Guide Section 17: Types with constraints

## Main

Let positive: Int where it > 0 be 5.
Let percentage: Int where it >= 0 and it <= 100 be 85.

Show "Positive value: " + positive.
Show "Percentage: " + percentage.

Let bounded: Int where it >= 1 and it <= 10 be 7.
Show "Bounded (1-10): " + bounded.
"#;

const CODE_ADVANCED_ASSERTIONS: &str = r#"# Assertions and Trust
-- Guide Sections 17, 22: Assert and Trust statements

## To withdraw (amount: Int) from (balance: Int) -> Int:
    Assert that amount is greater than 0.
    Assert that amount is at most balance.
    Return balance - amount.

## To process (n: Int) -> Int:
    Trust that n is greater than 0 because "caller guarantees positive input".
    Return n * 2.

## Main

Show "Withdrawal:".
Let result be withdraw(50, 100).
Show "Withdrew 50 from 100: " + result.

Show "Process with trust:".
Let doubled be process(5).
Show "5 doubled: " + doubled.
"#;
