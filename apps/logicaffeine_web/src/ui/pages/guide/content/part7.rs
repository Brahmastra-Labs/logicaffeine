//! Part VII: Practice — sections: complete-examples, recipes.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, RecapQuestion, Runnability, Section,
};

pub(super) const PART_7: &[Section] = &[
    Section {
        id: "complete-examples",
        number: 38,
        title: "Complete Examples",
        part: "Part VII: Practice",
        content: r#"
Complete programs, each read the way a problem is actually solved — Polya's four moves:
UNDERSTAND the problem (what exactly should print?), PLAN (which sections supply the
tools?), CARRY OUT (the build), and LOOK BACK (what generalizes, what breaks, what to
try next).

### Warm-ups: Factorial, Fibonacci, Filtering

Three classics to loosen up on — recursion, recursion + a loop, and the
iterate–guard–accumulate shape you will use weekly.

### Word Count

*Understand:* given words, print each distinct word with its count, first-seen order.
*Plan:* a Map for counts (Collections), a membership guard (Control Flow), destructured
iteration to report (Collections).

> **Look back:** the count-by-key shape generalizes to any tally — errors by type,
> sales by region. What breaks if the input is empty? Nothing: zero loop turns, zero
> report lines. Try counting characters instead of words.

### A Tiny Ledger

*Understand:* entries with labels and amounts; print each, then the total, in cents.
*Plan:* a struct for the entry (User-Defined Types), a typed empty list plus Push
(Collections), a running total (Control Flow), interpolation to report (Text).

> **Look back:** amounts stay integer cents to dodge float dust — the same discipline
> Money automates. What breaks with a negative amount? Nothing mechanical — but should
> a ledger allow it? Try a `credit`/`debit` enum per entry.

### A Todo List

*Understand:* seed tasks, add one, list all, then finish (pop) the newest.
*Plan:* list literals and Push/Pop (Collections), counted report via interpolation (Text).

> **Look back:** Pop takes the NEWEST — a stack. For first-in-first-out, a Pipe
> (Concurrency) is the queue shape. Try printing the remaining count after the pop.
"#,
        hook: "Watch three real programs get built the way problems are actually solved: understand, plan, build, look back. None of the code is new — the craft is in the moves.",
        objectives: &[
            "trace a complete program from problem statement to output",
            "name which section supplies each tool a build uses",
            "extend a finished program without breaking its shape",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "example-factorial",
                label: "Factorial",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5! = 120"),
                predict_first: false,
                code: r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Let result be factorial(5).
Show "5! = " + result."#,
            },
            CodeExample {
                id: "example-fibonacci",
                label: "Fibonacci",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Fibonacci sequence:\n0\n1\n1\n2\n3\n5\n8\n13\n21\n34"),
                predict_first: false,
                code: r#"## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show "Fibonacci sequence:".
Let i be 0.
While i is less than 10:
    Show fib(i).
    Set i to i + 1."#,
            },
            CodeExample {
                id: "example-filter",
                label: "Filter Positive Numbers",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Positives: [5, 8, 3, 7]"),
                predict_first: false,
                code: r#"## Main
Let data be [-2, 5, -1, 8, 3, -4, 7].
Let positives be a new Seq of Int.

Repeat for n in data:
    If n is greater than 0:
        Push n to positives.

Show "Positives: " + positives."#,
            },
            CodeExample {
                id: "program-wordcount",
                label: "Word Count",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("the: 2\ncat: 1\nsat: 1\non: 1\nmat: 1"),
                predict_first: false,
                code: r#"## Main
Let words be ["the", "cat", "sat", "on", "the", "mat"].
Let counts be a new Map of Text to Int.
Repeat for w in words:
    If counts contains w:
        Set counts[w] to counts[w] + 1.
    Otherwise:
        Set counts[w] to 1.
Repeat for (word, n) in counts:
    Show "{word}: {n}"."#,
            },
            CodeExample {
                id: "program-ledger",
                label: "A Tiny Ledger",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("coffee: 450 cents\nbook: 1200 cents\ntotal: 1650 cents"),
                predict_first: false,
                code: r#"## Definition
An Entry has:
    a label: Text.
    an amount: Int.

## Main
Let entries be [] of Entry.
Push a new Entry with label "coffee" and amount 450 to entries.
Push a new Entry with label "book" and amount 1200 to entries.
Let mutable total be 0.
Repeat for e in entries:
    Set total to total + e's amount.
    Show "{e's label}: {e's amount} cents".
Show "total: {total} cents"."#,
            },
            CodeExample {
                id: "program-todo",
                label: "A Todo List",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("todo (3):\n- write guide\n- run tests\n- ship it\nfinished: ship it"),
                predict_first: false,
                code: r#"## Main
Let tasks be ["write guide", "run tests"].
Push "ship it" to tasks.
Let n be length of tasks.
Show "todo ({n}):".
Repeat for t in tasks:
    Show "- {t}".
Pop from tasks into done.
Show "finished: {done}"."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-wc-extend",
                kind: ExerciseKind::Challenge,
                concepts: &["maps", "Repeat for-each"],
                prompt: "Extend Word Count: add a third \"the\" to the words list and print the report — predict how the first line changes.",
                mode: ExampleMode::Imperative,
                expected: "the: 3\ncat: 1\nsat: 1\non: 1\nmat: 1",
                starter: "## Main\nLet words be [\"the\", \"cat\", \"sat\", \"on\", \"the\", \"mat\"].\nLet counts be a new Map of Text to Int.\nRepeat for w in words:\n    If counts contains w:\n        Set counts[w] to counts[w] + 1.\n    Otherwise:\n        Set counts[w] to 1.\nRepeat for (word, n) in counts:\n    Show \"{word}: {n}\".",
                hints: &[
                    "Only the words list changes.",
                    "First-seen order is preserved — `the` stays first, its count grows.",
                ],
                reference: "## Main\nLet words be [\"the\", \"cat\", \"sat\", \"on\", \"the\", \"mat\", \"the\"].\nLet counts be a new Map of Text to Int.\nRepeat for w in words:\n    If counts contains w:\n        Set counts[w] to counts[w] + 1.\n    Otherwise:\n        Set counts[w] to 1.\nRepeat for (word, n) in counts:\n    Show \"{word}: {n}\".",
            },
            Exercise {
                id: "ex-ledger-big",
                kind: ExerciseKind::Challenge,
                concepts: &["struct definition", "If/Otherwise"],
                prompt: "Extend the Ledger: with coffee 450, book 1200, lamp 800, total ONLY the entries above 500 cents and print `big-ticket total: 2000 cents`.",
                mode: ExampleMode::Imperative,
                expected: "big-ticket total: 2000 cents",
                starter: "## Definition\nAn Entry has:\n    a label: Text.\n    an amount: Int.\n\n## Main\n",
                hints: &[
                    "Same accumulate shape, with a guard inside the loop.",
                    "Only book and lamp clear the bar.",
                ],
                reference: "## Definition\nAn Entry has:\n    a label: Text.\n    an amount: Int.\n\n## Main\nLet entries be [] of Entry.\nPush a new Entry with label \"coffee\" and amount 450 to entries.\nPush a new Entry with label \"book\" and amount 1200 to entries.\nPush a new Entry with label \"lamp\" and amount 800 to entries.\nLet mutable big be 0.\nRepeat for e in entries:\n    If e's amount is greater than 500:\n        Set big to big + e's amount.\nShow \"big-ticket total: {big} cents\".",
            },
            Exercise {
                id: "ex-todo-remaining",
                kind: ExerciseKind::Challenge,
                concepts: &["Push and Pop", "string interpolation"],
                prompt: "Extend the Todo list: after finishing the newest task, also print `remaining: 2`. Careful — `length of` doesn't parse inside braces.",
                mode: ExampleMode::Imperative,
                expected: "finished: ship it\nremaining: 2",
                starter: "## Main\nLet tasks be [\"write guide\", \"run tests\", \"ship it\"].\nPop from tasks into done.\nShow \"finished: {done}\".",
                hints: &[
                    "Compute the length into a name first.",
                    "Then interpolate the name.",
                ],
                reference: "## Main\nLet tasks be [\"write guide\", \"run tests\", \"ship it\"].\nPop from tasks into done.\nShow \"finished: {done}\".\nLet n be length of tasks.\nShow \"remaining: {n}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-complete-polya",
                question: "What are Polya's four moves, as this section uses them?",
                answer: "Understand the problem, plan the tools, carry out the build, look back at what generalizes.",
            },
            RecapQuestion {
                id: "rq-complete-cents",
                question: "Why does the ledger count integer cents instead of 4.50?",
                answer: "Integer arithmetic can't grow float dust — the same discipline the Money type automates.",
            },
            RecapQuestion {
                id: "rq-complete-stack",
                question: "Pop takes which task — and what shape does that make the list?",
                answer: "The newest — a stack (LIFO). A Pipe gives the queue shape instead.",
            },
        ],
    },

    Section {
        id: "recipes",
        number: 39,
        title: "Recipes: How Do I…?",
        part: "Part VII: Practice",
        content: r#"
Goal-indexed answers — each recipe is a complete program you can run and adapt. More
recipes land with every release; if a "how do I…?" you need is missing, that is a bug in
this guide.

- **Format a number to two decimals** — a format spec in the interpolation braces.
  Combines: Text (format specs).
- **Keep a value in range (clamp)** — `min` and `max` compose.
  Combines: the Standard Library (min/max) + Functions.
- **Count matching items** — iterate, guard with `If`, accumulate.
  Combines: Control Flow + Collections + Text (interpolation).

> **Look back:** every recipe is a shape, not a snippet — clamp works on any ordered
> values, count-matching on any predicate. When you adapt one, change the domain first
> and the shape last.
"#,
        hook: "How do I round to two decimals? Clamp to a range? Count the passing scores? Goal-first answers — each a complete program to run, adapt, and steal the shape of.",
        objectives: &[
            "use a recipe by adapting its domain, keeping its shape",
            "name which sections each recipe combines",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "recipe-two-decimals",
                label: "Format to Two Decimals",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("19.99"),
                predict_first: false,
                code: r#"## Main
Let price be 19.994999.
Show "{price:.2}"."#,
            },
            CodeExample {
                id: "recipe-clamp",
                label: "Clamp a Value to a Range",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("10\n1\n7"),
                predict_first: false,
                code: r#"## To clamp (x: Int) and (lo: Int) and (hi: Int) -> Int:
    Return min(max(x, lo), hi).

## Main
Show clamp(15, 1, 10).
Show clamp(-3, 1, 10).
Show clamp(7, 1, 10)."#,
            },
            CodeExample {
                id: "recipe-count-matching",
                label: "Count Matching Items",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3 of 5 passed"),
                predict_first: false,
                code: r#"## Main
Let scores be [90, 55, 72, 88, 41].
Let total be length of scores.
Let mutable passing be 0.
Repeat for s in scores:
    If s is at least 60:
        Set passing to passing + 1.
Show "{passing} of {total} passed"."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-recipes-clamp",
                question: "Which two builtins compose into clamp, and in what order?",
                answer: "min and max: min(max(x, lo), hi) — raise to the floor, then cap at the ceiling.",
            },
            RecapQuestion {
                id: "rq-recipes-shape",
                question: "What is the count-matching shape?",
                answer: "Iterate, guard with If, accumulate a counter — then report once after the loop.",
            },
        ],
    },
];
