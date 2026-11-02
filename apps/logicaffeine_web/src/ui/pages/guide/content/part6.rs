//! Part VI: Logic and Verification — sections: logic-mode, assertions-trust, z3-verification.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, Misconception, RecapQuestion, Runnability,
    Section,
};

pub(super) const PART_6: &[Section] = &[
    Section {
        id: "logic-mode",
        number: 33,
        title: "Logic Mode",
        part: "Part VI: Logic and Verification",
        content: r#"
LOGOS can translate English sentences into First-Order Logic (FOL). This is useful for formal verification, knowledge representation, and understanding the logical structure of natural language.

### Quantifiers

| English | Symbol | Output |
|---------|--------|--------|
| All X are Y | `∀` | `∀x(X(x) → Y(x))` |
| Some X is Y | `∃` | `∃x(X(x) ∧ Y(x))` |
| No X is Y | `¬∃` | `¬∃x(X(x) ∧ Y(x))` |

### Connectives

| English | Symbol |
|---------|--------|
| and | `∧` |
| or | `∨` |
| not | `¬` |
| if...then | `→` |
| if and only if | `↔` |

### Modals

| English | Symbol |
|---------|--------|
| can, may, might | `◇` (possibility) |
| must | `□` (necessity) |

### Tense and Aspect

- `PAST(P)` — past tense
- `FUT(P)` — future tense
- `PROG(P)` — progressive aspect
- `PERF(P)` — perfect aspect

> **Mental model:** Every verb is an EVENT with name tags on its participants:
> "John runs" becomes ∃e(Run(e) ∧ Agent(e, John)) — there exists a running, and John
> is its agent. Adverbs decorate the event, tense stamps it, roles name who did what
> to whom. The limit: pure states (being tall) ride the same machinery — linguists
> argue about that; the parser commits.
"#,
        hook: "Translate 'All birds fly' to logic. If you wrote ∀x(Bird(x) → Fly(x)), you are in good company — and you missed where the flying HAPPENS. Give every verb an event, and adverbs, tense, and who-did-what suddenly have a home.",
        objectives: &[
            "translate quantified English sentences to FOL",
            "read event variables and thematic roles (Agent, Theme)",
            "predict the shape negative quantifiers take",
        ],
        misconceptions: &[
            Misconception {
                id: "m-event-sem",
                expectation: "'All birds fly' should be ∀x(Bird(x) → Fly(x)) — that is Logic 101.",
                reality: "LOGOS emits neo-Davidsonian event semantics: ∀x((Bird(x) ∧ Animal(x)) → ∃e(Fly(e) ∧ Agent(e, x))) — the flying is an event x participates in. That extra e is what lets adverbs, tense, and roles compose instead of exploding the predicate. (Animal(x) is the dictionary noting that every bird is an animal.)",
                evidence: "logic-universal",
            },
        ],
        examples: &[
            CodeExample {
                id: "logic-universal",
                label: "Universal Quantifier",
                mode: ExampleMode::Logic,
                runnability: Runnability::Playground,
                expected_output: Some("∀x(((Bird(x) ∧ Animal(x)) → ∃e(Fly(e) ∧ Agent(e, x))))"),
                predict_first: true,
                code: "All birds fly.",
            },
            CodeExample {
                id: "logic-existential",
                label: "Existential Quantifier",
                mode: ExampleMode::Logic,
                runnability: Runnability::Playground,
                expected_output: Some("∃x((((Cat(x) ∧ Animal(x)) ∧ Mammal(x)) ∧ ∃e(Sleep(e) ∧ Agent(e, x))))"),
                predict_first: false,
                code: "Some cats sleep.",
            },
            CodeExample {
                id: "logic-negative",
                label: "Negative Quantifier",
                mode: ExampleMode::Logic,
                runnability: Runnability::Playground,
                expected_output: Some("∀x(((Fish(x) ∧ Animal(x)) → ¬∃e(Fly(e) ∧ Agent(e, x))))"),
                predict_first: false,
                code: "No fish fly.",
            },
            CodeExample {
                id: "logic-conditional",
                label: "Conditional",
                mode: ExampleMode::Logic,
                runnability: Runnability::Playground,
                expected_output: Some("∀e((HAB(Run(e) ∧ Agent(e, John)) → HAB(∃e(Walk(e) ∧ Agent(e, Mary)))))"),
                predict_first: false,
                code: "If John runs, then Mary walks.",
            },
            CodeExample {
                id: "logic-modal",
                label: "Modal Operators",
                mode: ExampleMode::Logic,
                runnability: Runnability::Playground,
                expected_output: Some("◇_{0.5} ∃e(Swim(e) ∧ Agent(e, John))"),
                predict_first: false,
                code: "John can swim.",
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-fol-birds",
                kind: ExerciseKind::Practice,
                concepts: &["FOL quantifiers"],
                prompt: "Write the First-Order Logic for: All birds fly. (Run the Universal Quantifier example above to see the shape.)",
                mode: ExampleMode::Logic,
                expected: "∀x((Bird(x) → ∃e(Fly(e) ∧ Agent(e, x))))",
                starter: "∀x(",
                hints: &[
                    "Start with the universal quantifier, then give every bird an implication.",
                    "Flying is an event: introduce ∃e and tie it to the bird with Agent(e, x).",
                ],
                reference: "∀x((Bird(x) → ∃e(Fly(e) ∧ Agent(e, x))))",
            },
            Exercise {
                id: "ex-fol-sing",
                kind: ExerciseKind::Practice,
                concepts: &["FOL quantifiers"],
                prompt: "Write the FOL for: Some birds sing.",
                mode: ExampleMode::Logic,
                expected: "∃x((Bird(x) ∧ ∃e(Sing(e) ∧ Agent(e, x))))",
                starter: "∃x(",
                hints: &[
                    "Existentials pair with ∧, not → — some bird both is a bird and sings.",
                    "Singing is an event: ∃e ties it to the bird with Agent(e, x).",
                ],
                reference: "∃x((Bird(x) ∧ ∃e(Sing(e) ∧ Agent(e, x))))",
            },
            Exercise {
                id: "ex-fol-nocats",
                kind: ExerciseKind::Practice,
                concepts: &["FOL quantifiers"],
                prompt: "Write the FOL for: No cats bark.",
                mode: ExampleMode::Logic,
                expected: "∀x((Cat(x) → ¬∃e(Bark(e) ∧ Agent(e, x))))",
                starter: "∀x(",
                hints: &[
                    "'No X does Y' reads: for all X, there is NO Y-event.",
                    "The negation sits on the event existential: ¬∃e.",
                ],
                reference: "∀x((Cat(x) → ¬∃e(Bark(e) ∧ Agent(e, x))))",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-logic-event",
                question: "What does the `e` in ∃e(Fly(e) ∧ Agent(e, x)) stand for?",
                answer: "The flying EVENT — verbs become events with named participant roles.",
            },
            RecapQuestion {
                id: "rq-logic-no",
                question: "How does 'No fish fly' negate — on the fish or on the flying?",
                answer: "On the flying: ∀x(Fish(x) → ¬∃e(...)) — every fish lacks a flying event.",
            },
            RecapQuestion {
                id: "rq-logic-some",
                question: "Universal pairs with →; what does the existential pair with, and why?",
                answer: "∧ — 'some cat sleeps' asserts both cat-hood and sleeping of one witness.",
            },
        ],
    },
    Section {
        id: "assertions-trust",
        number: 34,
        title: "Assertions and Trust",
        part: "Part VI: Logic and Verification",
        content: r#"
LOGOS bridges imperative programming with formal verification through assertions and proof statements.

### Assert

Use `Assert` to verify conditions at runtime. If an assertion fails, the program stops with a clear error message.

### Trust with Justification

Use `Trust` for conditions the compiler can't verify automatically. The `because` clause is **mandatory**—it documents your reasoning.

### Trust Generates Debug Assertions

In development builds, `Trust` becomes a `debug_assert!`. In release builds, it generates no code—the trust is assumed.

### Auditing Trust Statements

Every `Trust` stays a plain `Trust that ... because ...` line in your source—search for `Trust that` to review every assumption, each carrying the `because` justification that documents why it holds.

### Proof Blocks (Advanced)

For formal verification, use theorem blocks with proofs documented in comments.
"#,
        hook: "withdraw(50, 100) must never leave a negative balance — but 'must never' in a comment guards nothing. Assert makes the sentence executable; Trust records the reason when you can't check.",
        objectives: &[
            "write Assert and Trust-with-because guards",
            "distinguish what debug and release builds keep",
            "predict what a failing Assert does to the run",
        ],
        misconceptions: &[
            Misconception {
                id: "m-assert-release",
                expectation: "Assertions are plain if-statements — they always run.",
                reality: "Trust compiles to a debug-only assertion and vanishes in release; Assert runs and can be tuned by build settings; only security Check can NEVER be dropped. Choose the guard by what must survive shipping.",
                evidence: "assert-denied",
            },
        ],
        examples: &[
            CodeExample {
                id: "assert-example",
                label: "Assert",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("50"),
                predict_first: false,
                code: r#"## To withdraw (amount: Int) from (balance: Int) -> Int:
    Assert that amount is greater than 0.
    Assert that amount is at most balance.
    Return balance - amount.

## Main
Let result be withdraw(50, 100).
Show result."#,
            },
            CodeExample {
                id: "assert-denied",
                label: "A Failing Assert (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Assertion failed",
                },
                expected_output: None,
                predict_first: true,
                code: r#"## Main
Assert that 1 is greater than 2.
Show "unreachable"."#,
            },
            CodeExample {
                id: "trust-example",
                label: "Trust with Justification",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("10"),
                predict_first: false,
                code: r#"## To process_positive (n: Int) -> Int:
    Trust that n is greater than 0 because "caller guarantees positive input".
    Return n * 2.

## Main
Let result be process_positive(5).
Show result."#,
            },
            CodeExample {
                id: "assertions",
                label: "Assertions",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5"),
                predict_first: false,
                code: r#"## To divide_safe (a: Int) and (b: Int) -> Int:
    Assert that b is not 0.
    Return a / b.

## Main
Let result be divide_safe(10, 2).
Show result."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-assert-hole",
                kind: ExerciseKind::Completion,
                concepts: &["Assert"],
                prompt: "Fill the hole so the guard reads as an inequality and the division runs.",
                mode: ExampleMode::Imperative,
                expected: "3",
                starter: "## To safe_ratio (a: Int) and (b: Int) -> Int:\n    Assert that b is ??? 0.\n    Return a / b.\n\n## Main\nShow safe_ratio(9, 3).",
                hints: &[
                    "The inequality's English spelling is two words.",
                    "It is the twin of `!=`.",
                ],
                reference: "## To safe_ratio (a: Int) and (b: Int) -> Int:\n    Assert that b is not 0.\n    Return a / b.\n\n## Main\nShow safe_ratio(9, 3).",
            },
            Exercise {
                id: "ex-assert-guard",
                kind: ExerciseKind::Practice,
                concepts: &["Assert"],
                prompt: "Guard safe_ratio with an Assert that b is not 0, then show safe_ratio(9, 3).",
                mode: ExampleMode::Imperative,
                expected: "3",
                starter: "## To safe_ratio (a: Int) and (b: Int) -> Int:\n    Return a / b.\n\n## Main\nShow safe_ratio(9, 3).",
                hints: &[
                    "The guard is the function's first line.",
                    "`is not` states the inequality — assert b against zero.",
                ],
                reference: "## To safe_ratio (a: Int) and (b: Int) -> Int:\n    Assert that b is not 0.\n    Return a / b.\n\n## Main\nShow safe_ratio(9, 3).",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-assert-fail",
                question: "What happens when an Assert's condition is false?",
                answer: "The program stops with `Assertion failed` — the next line never runs.",
            },
            RecapQuestion {
                id: "rq-assert-trust",
                question: "When is Trust the right tool instead of Assert?",
                answer: "When the compiler can't check it but you can justify it — the mandatory `because` documents the reasoning.",
            },
            RecapQuestion {
                id: "rq-assert-audit",
                question: "How do you audit every assumption in a codebase?",
                answer: "Search for `Trust that` — each hit carries its because-justification.",
            },
        ],
    },
    Section {
        id: "z3-verification",
        number: 35,
        title: "Z3 Static Verification",
        part: "Part VI: Logic and Verification",
        content: r#"
LOGOS can use the Z3 SMT solver to verify refinement types at compile time.

### What is Z3?

Z3 is a theorem prover. Instead of checking constraints at runtime, Z3 proves (or disproves) them at compile time.

| Approach | When Checked | If Violated |
|----------|--------------|-------------|
| Runtime assertion | When code runs | Program crashes |
| Z3 verification | At compile time | Compilation fails |

### Variable Tracking

Z3 tracks constraints through variable assignments.

### Compound Predicates

Multiple constraints can be combined.

### Function Preconditions

Z3 verifies function contracts.

### Enabling Z3 Verification

Enable with `largo build --verify` or in `Largo.toml`.

### What Z3 Can Prove

| Constraint Type | Example | Z3 Support |
|-----------------|---------|------------|
| Integer bounds | `it > 0`, `it < 100` | Full |
| Equality | `it == 5` | Full |
| Arithmetic | `it * 2 < 100` | Full |
| Boolean logic | `it > 0 and it < 10` | Full |

In the playground, refinements are CARRIED, not checked — the proof happens at
`largo build --verify`. The second example below violates its own refinement on
purpose: predict what the playground does with it before you run it.
"#,
        hook: "A percentage that cannot leave 0–100, a balance positive by construction — refinement types state the invariant where the type lives, and Z3 proves it before the program ever runs.",
        objectives: &[
            "write refinement annotations with where clauses",
            "predict what the playground does with a violated refinement",
            "explain when Z3 actually checks (largo build --verify)",
        ],
        misconceptions: &[
            Misconception {
                id: "m-refine-runtime",
                expectation: "A violated refinement crashes at runtime — the type is a guard.",
                reality: "Refinements are compile-time contracts for Z3 (`largo build --verify`); the playground carries the annotation and prints -5 without complaint. The proof happens before the program exists — that is the point of static verification.",
                evidence: "z3-refinement",
            },
        ],
        examples: &[
            CodeExample {
                id: "refinement-types",
                label: "Refinement Types",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5\n85"),
                predict_first: false,
                code: r#"## Main
Let positive: Int where it > 0 be 5.
Let percentage: Int where it >= 0 and it <= 100 be 85.
Show positive.
Show percentage."#,
            },
            CodeExample {
                id: "z3-refinement",
                label: "A Violated Refinement in the Playground (Predict First)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("-5"),
                predict_first: true,
                code: r#"## Main
Let positive: Int where it > 0 be -5.
Show positive."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-z3-when",
                question: "When are refinement types actually proven?",
                answer: "At `largo build --verify` — Z3 checks them at compile time, before any run.",
            },
            RecapQuestion {
                id: "rq-z3-playground",
                question: "What does the playground do with `Let positive: Int where it > 0 be -5.`?",
                answer: "Prints -5 — the annotation is carried, not enforced; verification is a build step.",
            },
            RecapQuestion {
                id: "rq-z3-vs-assert",
                question: "Refinement vs Assert — when does each catch a violation?",
                answer: "The refinement at compile time (the build fails); the Assert at run time (the program stops).",
            },
        ],
    },

    Section {
        id: "proofs",
        number: 36,
        title: "The Proof Engine: Theorems from English",
        part: "Part VI: Logic and Verification",
        content: r#"
A `## Theorem` block states premises and a goal in plain English; the proof engine finds
the derivation and the CoC kernel certifies it — not merely claims it. Run the example
below and the proof happens live, right here. The classic:

```logos
## Theorem: Socrates_Is_Mortal
Given: All men are mortal.
Given: Socrates is a man.
Prove: Socrates is mortal.
Proof: By automation.
```

```bash
$ largo prove syllogism.lg
Proved: Socrates_Is_Mortal — kernel-certified.
```

`## Axiom` and `## Theory` blocks build larger developments — whole formal theories stated
in source text — and `largo sat` exposes the certified SAT solver directly (DIMACS in,
DRAT proofs out).

> **Mental model:** The kernel is a referee, not a player: the proof engine searches
> for the derivation, the kernel only CHECKS it, step by minimal step. A bug in the
> search wastes time; only a bug in the tiny referee could certify a falsehood — which
> is exactly why the referee stays tiny. The limit: "Not proved" means the search
> failed, never that the statement is false.
"#,
        hook: "'All men are mortal; Socrates is a man.' You already know the conclusion — now make the machine PROVE it, live, through a kernel that checks every step rather than taking the prover's word.",
        objectives: &[
            "write Theorem blocks with Given, Prove, and Proof lines",
            "predict the verdict line a successful proof produces",
            "distinguish the prover (searches) from the kernel (certifies)",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "theorem-socrates",
                label: "The Syllogism, Proved Live",
                mode: ExampleMode::Theorem,
                runnability: Runnability::Playground,
                expected_output: Some("Proved: Socrates_Is_Mortal — kernel-certified.\nGoal: Mortal(Socrates)"),
                predict_first: true,
                code: r#"## Theorem: Socrates_Is_Mortal
Given: All men are mortal.
Given: Socrates is a man.
Prove: Socrates is mortal.
Proof: By automation."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-proofs-parts",
                question: "What three line-kinds make up a Theorem block?",
                answer: "Given (premises), Prove (the goal), Proof (the strategy — By automation searches).",
            },
            RecapQuestion {
                id: "rq-proofs-kernel",
                question: "Why does a tiny kernel make the verdict trustworthy?",
                answer: "The prover only proposes; the kernel re-checks every step — small enough to audit, so 'certified' means checked, not claimed.",
            },
            RecapQuestion {
                id: "rq-proofs-notproved",
                question: "Does 'Not proved' mean the statement is false?",
                answer: "No — only that the search failed to find a derivation.",
            },
        ],
    },
    Section {
        id: "part-6-checkpoint",
        number: 37,
        title: "Checkpoint: Logic and Verification",
        part: "Part VI: Logic and Verification",
        content: r#"
Logic mode, assertions, refinements, proofs — the trust ladder from runtime guards to
kernel-certified theorems. Three challenges blend the rungs with earlier parts.
"#,
        hook: "",
        objectives: &[
            "combine guards, refinements, and functions across the trust ladder",
            "diagnose which Part VI section to reread when a challenge stumps you",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[
            Exercise {
                id: "cp6-assert-ratio",
                kind: ExerciseKind::Challenge,
                concepts: &["Assert", "function definition"],
                prompt: "Write safe_ratio(a, b) with an Assert guard against b = 0, and show safe_ratio(9, 3).",
                mode: ExampleMode::Imperative,
                expected: "3",
                starter: "## Main\nShow safe_ratio(9, 3).",
                hints: &[
                    "The guard comes before the division.",
                    "`is not` states the inequality.",
                ],
                reference: "## To safe_ratio (a: Int) and (b: Int) -> Int:\n    Assert that b is not 0.\n    Return a / b.\n\n## Main\nShow safe_ratio(9, 3).",
            },
            Exercise {
                id: "cp6-refine-carry",
                kind: ExerciseKind::Challenge,
                concepts: &["Z3 refinement verification", "string interpolation"],
                prompt: "Bind pct as an Int refined to 0..=100 with value 85, then print `85%` by interpolation.",
                mode: ExampleMode::Imperative,
                expected: "85%",
                starter: "## Main\n",
                hints: &[
                    "The refinement reads `: Int where it >= 0 and it <= 100`.",
                    "The percent sign sits outside the braces.",
                ],
                reference: "## Main\nLet pct: Int where it >= 0 and it <= 100 be 85.\nShow \"{pct}%\".",
            },
            Exercise {
                id: "cp6-trust-doc",
                kind: ExerciseKind::Challenge,
                concepts: &["Trust with because", "function definition"],
                prompt: "Write half(n) that Trusts n is greater than 0 (with a because), returns n / 2, and show half(10).",
                mode: ExampleMode::Imperative,
                expected: "5",
                starter: "## Main\nShow half(10).",
                hints: &[
                    "Trust lines read: Trust that <condition> because \"<reason>\".",
                    "The because string records the caller's guarantee.",
                ],
                reference: "## To half (n: Int) -> Int:\n    Trust that n is greater than 0 because \"callers validated the total\".\n    Return n / 2.\n\n## Main\nShow half(10).",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "cq6-ladder",
                question: "Order the trust ladder: where do Assert, Check, refinements, and Theorems each catch problems?",
                answer: "Assert and Check at runtime (Check never optimized away); refinements at build via Z3; Theorems as kernel-certified derivations.",
            },
            RecapQuestion {
                id: "cq6-event",
                question: "In LOGOS FOL, what does an adverb like 'quickly' attach to?",
                answer: "The EVENT — the e in ∃e(Run(e) ∧ ...) is what adverbs decorate.",
            },
            RecapQuestion {
                id: "cq6-because",
                question: "Which construct requires a written justification, and what is it for?",
                answer: "Trust — the mandatory `because` documents assumptions so they can be audited later.",
            },
            RecapQuestion {
                id: "cq6-refine",
                question: "Your refinement is violated but the playground printed the value anyway. Bug?",
                answer: "No — refinements are checked by Z3 at largo build --verify, not at playground runtime.",
            },
            RecapQuestion {
                id: "cq6-verdict",
                question: "What does the proof engine print when the syllogism succeeds?",
                answer: "`Proved: <Name> — kernel-certified.` plus the goal line.",
            },
            RecapQuestion {
                id: "cq6-search",
                question: "Prover vs kernel — which one do you have to trust?",
                answer: "Only the kernel — the prover's output is re-checked step by step.",
            },
        ],
    },
];
