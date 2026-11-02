//! Part II: The Language — sections: variables-and-types, text, operators, control-flow, functions, closures, collections, user-types, generics, error-handling.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, Misconception, RecapQuestion, Runnability,
    Section,
};

pub(super) const PART_2: &[Section] = &[
    Section {
        id: "variables-and-types",
        number: 6,
        title: "Variables and Types",
        part: "Part II: The Language",
        content: r#"
Variables are containers that hold values. In LOGOS, you create and modify variables using natural English sentences.

### Creating Variables

Use `Let` to create a new variable. The word `be` assigns a value to the variable.

### Changing Values

Use `Set` to change an existing variable. The difference between `Let` and `Set`:
- `Let` creates a *new* variable
- `Set` modifies an *existing* variable

`Let mutable x be 0.` flags a binding you intend to reassign — loop counters, running
totals. The checker does not currently reject `Set` on a plain `Let`, so the `mutable`
is for your reader: it marks the moving parts.



### Primitive Types

| Type | Description | Examples |
|------|-------------|----------|
| `Int` | Whole numbers | `5`, `-10`, `0`, `1000000` |
| `Bool` | True or false | `true`, `false` |
| `Text` | Strings of characters | `"Hello"`, `"LOGOS"`, `""` |
| `Real` / `Float` | Decimal numbers | `3.14`, `-0.5`, `98.6` |
| `Char` | Single character | see examples below |
| `Byte` | 8-bit unsigned (0-255) | `42: Byte`, `255: Byte` |

Decimal literals have type `Real`, and `Float` is another name for exactly the same type —
either annotation accepts them.

### Characters (Char)

Characters represent single Unicode characters, wrapped in backticks. See the example below.

### Bytes (Byte)

Bytes are 8-bit unsigned integers (0-255), useful for binary data and low-level operations.

### Type Annotations

Usually, LOGOS infers the type from the value you assign. But you can be explicit with `: Type`.

### The exact numeric tower

---

### Nat — counts by intent

Annotate `: Nat` for values that are counts: `Let count: Nat be 5.` documents that the
number is never meaningfully negative.

### Rational — exact fractions

A `Rational` does arithmetic exactly — no rounding, ever. Give a value the annotation and
every division stays a true fraction: half of 7 plus a third of 7 is exactly `35/6`, not
`5.8333…`.

### BigInt — integers that never overflow

When `Int` arithmetic exceeds 64 bits, LOGOS promotes the result automatically instead of
wrapping or crashing. The example below adds 1 to the largest 64-bit integer — the sum no
64-bit integer can hold — and prints it exactly.

### Decimal — money-grade exactness

Binary floats cannot represent one tenth: `0.1 + 0.2` famously prints
`0.30000000000000004`. `decimal("0.1")` is exact base-10 arithmetic — the same sum is
exactly `0.3`. When the number is money, use `decimal` (or the `Money` type).

### Complex

`complex(re, im)` builds an exact complex number: `complex(0, 1)` is the imaginary unit,
and squaring it gives `-1`.

### The math toolbox

`sqrt(x)`, `floor(x)`, `ceil(x)`, `round(x)`, `pow(a, b)`, `abs(x)`, `min(a, b)`,
`max(a, b)` — all built in, no import needed.

*Try it: in the Decimal example, add a third `decimal("0.1")` to the sum — then try the
same with plain floats and compare.*

### Time, money, and measures

---

### Time

- `500ms`, `2s`, `5min` — a `Duration` (SI time).
- `2026-05-20` — a `Date`.
- `4pm`, `9:30am`, `noon` — a `Time` of day.
- `2 weeks`, `3 months` — a calendar `Span`.

Calendar extractors read components: `year_of(d)`, `month_of(d)`, `day_of(d)`,
`weekday_of(d)`.

### Money

`$19.99`, `€5`, `£10` — exact amounts in a real currency, quantized to the currency's
minor unit. No float cents, ever: `$19.99 + $0.01` is exactly `20.00 USD`, and splitting
a bill rounds to real cents.

### Quantities

`quantity(3, "meter")` is a dimensioned physical value. Adding a length to a time is not
a rounding error — it is a caught, impossible operation (run the mismatch example).

### Identifiers

`uuid("…")` parses an RFC 9562 UUID; `uuid_version(id)` reads its version nibble.

### Pausing

`Sleep 2ms.` (or `Sleep 500.` — milliseconds) pauses the program.

*Try it: change the bill split to four ways — where does the remainder cent go?*

### The complete type map

Every type in the language, in one place. The ones marked with a section name get their
deep dive there; everything else is taught right here.

| Family | Types |
|--------|-------|
| Primitives | `Int`, `Nat`, `Real` / `Float`, `Bool`, `Text`, `Char`, `Byte`, `Nothing` (spelled `Unit` in stdlib signatures) |
| Exact numbers | `Rational`, `BigInt` (automatic on overflow), `Decimal`, `Complex`, modular integers via `modular(v, m)` |
| Time | `Duration`, `Date`, `Time`, `Span`, `Moment` |
| Measures & identity | `Money`, `Quantity`, `Uuid` |
| Machine words | `Word8`, `Word16`, `Word32`, `Word64` — wrapping ring arithmetic (see Bits, Bytes, and Machine Words) |
| Collections | `Seq of T`, `Map of K to V`, `Set of T`, tuples (see Collections) |
| Maybe & failure | `Option of T` (`some x` / `none`), `Result of T and E` (see Error Handling and Optionals) |
| Your own | structs and enums (see User-Defined Types), generics (see Generics) |
| Concurrency | `Pipe of T` (see Concurrency) |
| Distributed | `ConvergentCount`, `Tally`, `LastWriteWins of T`, `Divergent T`, `SharedSet of T`, `SharedSequence of T`, `CollaborativeSequence of T`, `SharedMap from K to V` (see Distributed Types) |

"#,
        hook: "A program is mostly nouns: the player's score, the bill's total, today's date. Before anything can happen those nouns need names and values — and three of the types below catch bugs that float straight through every other language.",
        objectives: &[
            "predict which numeric type keeps 0.1 + 0.2 exact",
            "write `Let` and `Set` bindings, with type annotations where they add meaning",
            "choose Money, Decimal, or Rational when the arithmetic must not drift",
            "name what `Let mutable` signals to the reader",
        ],
        misconceptions: &[
            Misconception {
                id: "m-float-dust",
                expectation: "0.1 + 0.2 is 0.3 — floats hold decimals, that's their whole point.",
                reality: "Binary floats cannot represent one tenth, so 0.1 + 0.2 prints 0.30000000000000004. When the number is value-critical, `decimal(\"0.1\")` does exact base-10 arithmetic — the same sum is exactly 0.3.",
                evidence: "decimal-vs-float",
            },
            Misconception {
                id: "m-money-float",
                expectation: "Splitting $20.00 three ways gives $6.666666… — someone rounds it later and hopes.",
                reality: "Money is quantized to the currency's minor unit: the split rounds to real cents at the operation itself, so `bill / 3` is exactly 6.67 USD. No float cents exist anywhere in the pipeline.",
                evidence: "money-split",
            },
            Misconception {
                id: "m-overflow",
                expectation: "Add 1 to the largest 64-bit integer and you get a crash, a wrap to negative, or undefined behavior — pick your language.",
                reality: "LOGOS promotes the result to BigInt automatically: the arithmetic stays mathematically correct and the program keeps running. Fixed-width wrapping still exists — as an explicit choice, in machine words.",
                evidence: "bigint-promotion",
            },
            Misconception {
                id: "m-units",
                expectation: "Adding meters to seconds compiles fine and produces garbage at runtime — units live in comments.",
                reality: "Dimensions are part of the value: a length plus a time is refused with a named error, not a number. The mistake is impossible, not merely discouraged.",
                evidence: "quantity-mismatch",
            },
        ],
        examples: &[
            CodeExample {
                id: "creating-variables",
                label: "Creating Variables",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5\nBob"),
                predict_first: false,
                code: r#"## Main
Let x be 5.
Let name be "Bob".
Let is_active be true.
Let temperature be 98.6.
Show x.
Show name."#,
            },
            CodeExample {
                id: "changing-variables",
                label: "Changing Variables",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5\n10\n11"),
                predict_first: false,
                code: r#"## Main
Let x be 5.
Show x.

Set x to 10.
Show x.

Set x to x + 1.
Show x."#,
            },
            CodeExample {
                id: "byte-type",
                label: "Byte Type",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("255\n0\n128"),
                predict_first: false,
                code: r#"## Main
Let max: Byte be 255.
Let min: Byte be 0.
Let mid: Byte be 128.
Show max.
Show min.
Show mid."#,
            },
            CodeExample {
                id: "nat-annotation",
                label: "Nat: Counts by Intent",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5"),
                predict_first: false,
                code: r#"## Main
Let count: Nat be 5.
Show count."#,
            },
            CodeExample {
                id: "rational-exact",
                label: "Rational: Exact Fractions",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("35/6"),
                predict_first: false,
                code: r#"## Main
Let r: Rational be 7.
Let half be r / 2.
Let third be r / 3.
Show half + third."#,
            },
            CodeExample {
                id: "bigint-promotion",
                label: "BigInt: Past 64 Bits Without Overflow",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("9223372036854775808"),
                predict_first: true,
                code: r#"## Main
Show 9223372036854775807 + 1."#,
            },
            CodeExample {
                id: "decimal-vs-float",
                label: "Decimal: 0.1 + 0.2, Answered",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("0.30000000000000004\n0.3"),
                predict_first: true,
                code: r#"## Main
Show 0.1 + 0.2.
Show decimal("0.1") + decimal("0.2")."#,
            },
            CodeExample {
                id: "complex-i",
                label: "Complex: i Squared",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("-1"),
                predict_first: false,
                code: r#"## Main
Let i be complex(0, 1).
Show i * i."#,
            },
            CodeExample {
                id: "float-real-alias",
                label: "Float and Real Are One Type",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("true"),
                predict_first: false,
                code: r#"## Main
Let x: Float be 3.14.
Let y: Real be 3.14.
Show x equals y."#,
            },
            CodeExample {
                id: "math-toolbox",
                label: "The Math Toolbox",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("4\n3\n4\n4\n1024"),
                predict_first: false,
                code: r#"## Main
Show sqrt(16).
Show floor(3.7).
Show ceil(3.2).
Show round(3.5).
Show pow(2, 10)."#,
            },
            CodeExample {
                id: "duration-literal",
                label: "Durations",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("500ms"),
                predict_first: false,
                code: r#"## Main
Let d be 500ms.
Show d."#,
            },
            CodeExample {
                id: "date-calendar",
                label: "Dates and Calendar Extractors",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("2026-05-20\n2026"),
                predict_first: false,
                code: r#"## Main
Let d be 2026-05-20.
Show d.
Show year_of(d)."#,
            },
            CodeExample {
                id: "time-span",
                label: "Times of Day and Calendar Spans",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("16:00:00\n14 days"),
                predict_first: false,
                code: r#"## Main
Let t be 4pm.
Show t.
Let s be 2 weeks.
Show s."#,
            },
            CodeExample {
                id: "money-exact",
                label: "Money: No Float Cents",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("20.00 USD"),
                predict_first: false,
                code: r#"## Main
Show $19.99 + $0.01."#,
            },
            CodeExample {
                id: "money-split",
                label: "Splitting a Bill",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("6.67 USD"),
                predict_first: true,
                code: r#"## Main
Let bill be $20.00.
Show bill / 3."#,
            },
            CodeExample {
                id: "quantity-dimensions",
                label: "Dimensioned Quantities",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3 m"),
                predict_first: false,
                code: r#"## Main
Let len be quantity(3, "meter").
Show len."#,
            },
            CodeExample {
                id: "quantity-mismatch",
                label: "Adding Meters to Seconds (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "cannot add quantities of different dimensions (L vs T)",
                },
                expected_output: None,
                predict_first: true,
                code: r#"## Main
Let len be quantity(3, "meter").
Let t be quantity(2, "second").
Show len + t."#,
            },
            CodeExample {
                id: "uuid-version",
                label: "UUIDs",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("4"),
                predict_first: false,
                code: r#"## Main
Let id be uuid("550e8400-e29b-41d4-a716-446655440000").
Show uuid_version(id)."#,
            },
            CodeExample {
                id: "sleep-tick",
                label: "Sleep",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("tick\ntock"),
                predict_first: false,
                code: r#"## Main
Show "tick".
Sleep 2ms.
Show "tock"."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-vars-hole",
                kind: ExerciseKind::Completion,
                concepts: &["Set mutation"],
                prompt: "Fill the hole so the score changes to 25.",
                mode: ExampleMode::Imperative,
                expected: "25",
                starter: "## Main\nLet score be 10.\n??? score to 25.\nShow score.",
                hints: &[
                    "Creating uses `Let ... be`; changing uses a different verb.",
                    "The changing verb pairs with `to`, not `be`.",
                ],
                reference: "## Main\nLet score be 10.\nSet score to 25.\nShow score.",
            },
            Exercise {
                id: "ex-vars-decimal",
                kind: ExerciseKind::Practice,
                concepts: &["Decimal exactness"],
                prompt: "Add `decimal(\"19.99\")` and `decimal(\"0.01\")` and print the sum — watch it come out exact.",
                mode: ExampleMode::Imperative,
                expected: "20.00",
                starter: "## Main\n",
                hints: &[
                    "Both operands come from `decimal(\"...\")`.",
                    "One Show of the sum is the whole program.",
                ],
                reference: "## Main\nShow decimal(\"19.99\") + decimal(\"0.01\").",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-vars-let-set",
                question: "What is the difference between `Let` and `Set`?",
                answer: "`Let` creates a new binding; `Set` changes an existing one. `Let mutable` flags the intent to change.",
            },
            RecapQuestion {
                id: "rq-vars-money",
                question: "Why does `$19.99 + $0.01` never produce float dust?",
                answer: "Money is exact and quantized to the currency's minor unit — the arithmetic happens in cents, not binary fractions.",
            },
            RecapQuestion {
                id: "rq-vars-bigint",
                question: "What happens when Int arithmetic exceeds 64 bits?",
                answer: "The value promotes to BigInt automatically — no wrap, no crash.",
            },
        ],
    },
    Section {
        id: "text",
        number: 7,
        title: "Text",
        part: "Part II: The Language",
        content: r#"
Strings in LOGOS are `Text`. Build them by concatenation or — usually better — by
interpolation: write the variable inside braces and it renders in place.

### Interpolation

`"Hello, {name}!"` inserts `name`'s value. A format spec after a colon controls the
rendering: `"{pi:.2}"` rounds to two decimal places. And `"{v=}"` is the print-debugging
form — it renders as `v=42`, the name and the value together.

Braces evaluate an expression, not only a name: `"{a + b}"` renders the sum, and English
operator phrases work too — `"{length of tasks}"` prints the count. Whatever `Show x.`
prints, `Show "{x}".` prints in place.

What a hole cannot hold is *more than one* value, and that is refused rather than rendered
in part — half a hole rendered silently is the one outcome that leaves the line looking
like it worked. Braces you meant literally, like a JSON object or a CSS rule, are doubled:
`{{` and `}}` render as `{` and `}`.

### Joining sequences

`+` concatenates Text. Sequences join with `followed by`: `a followed by b` is one new
sequence with `b`'s elements after `a`'s.

### Characters

A `Char` is a single Unicode character in backticks: `` `a` ``, `` `\n` ``, `` `\t` ``.

*Try it: in the format-spec example, change `.2` to `.4` — then remove the spec entirely.*
"#,
        hook: "Build the line `Ada — level 3` out of a name and a number. Gluing strings with `+` works, and buries the sentence's shape under punctuation. Interpolation keeps the sentence readable — and it formats π to two decimals for free.",
        objectives: &[
            "write interpolated Text with format specs",
            "distinguish `+` concatenation from `followed by`",
            "read a hole as one whole expression, and double braces to print one literally",
        ],
        misconceptions: &[
            Misconception {
                id: "m-interp-phrase",
                expectation: "Braces are name substitution — anything with English words in it, like `{length of tasks}`, has to be computed into a variable first.",
                reality: "A hole runs the ordinary expression parser, so `{length of tasks}` and `{a + b}` both evaluate. The real boundary is arity, not vocabulary: a hole holds exactly ONE value, so a string containing a JSON object is refused rather than rendering the part that parsed and dropping the rest.",
                evidence: "interp-of-phrase",
            },
        ],
        examples: &[
            CodeExample {
                id: "interp-basic",
                label: "Interpolation",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Hello, Ada!"),
                predict_first: false,
                code: r#"## Main
Let name be "Ada".
Show "Hello, {name}!"."#,
            },
            CodeExample {
                id: "interp-format",
                label: "Format Specs",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3.14"),
                predict_first: false,
                code: r#"## Main
Let pi be 3.14159.
Show "{pi:.2}"."#,
            },
            CodeExample {
                id: "interp-debug",
                label: "Debug Interpolation",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("v=42"),
                predict_first: false,
                code: r#"## Main
Let v be 42.
Show "{v=}"."#,
            },
            CodeExample {
                id: "interp-of-phrase",
                label: "`of`-Phrases Inside Braces",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("You have 2 tasks left."),
                predict_first: true,
                code: r#"## Main
Let tasks be ["write", "test"].
Show "You have {length of tasks} tasks left."."#,
            },
            CodeExample {
                id: "interp-one-value",
                label: "A Hole Holds One Value (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "I read `\"a\"` in this interpolation hole and then found `: 1`, which is not part of it. A hole holds ONE value, so everything between the braces has to be that one value. If you meant the braces themselves — a JSON object, a CSS rule — double them: `{{` and `}}` print `{` and `}`.",
                },
                expected_output: None,
                predict_first: true,
                code: r#"## Main
Show "{\"a\": 1}"."#,
            },
            CodeExample {
                id: "interp-literal-brace",
                label: "Printing a Literal Brace",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("p { color: red; }"),
                predict_first: false,
                code: r#"## Main
Show "p {{ color: red; }}"."#,
            },
            CodeExample {
                id: "followed-by",
                label: "followed by: Joining Sequences",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1, 2, 3, 4]"),
                predict_first: false,
                code: r#"## Main
Let a be [1, 2].
Let b be [3, 4].
Show a followed by b."#,
            },
            CodeExample {
                id: "text-concat",
                label: "Text Concatenation",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Hello, World!"),
                predict_first: false,
                code: r#"## Main
Let first be "Hello".
Let second be "World".
Let message be first + ", " + second + "!".
Show message."#,
            },
            CodeExample {
                id: "char-literals",
                label: "Character Literals",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("a\nChar type uses backticks"),
                predict_first: false,
                code: r#"## Main
Let letter be `a`.
Let newline be `\n`.
Let tab be `\t`.
Let escaped be `\\`.
Show letter.
Show "Char type uses backticks"."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-text-format",
                kind: ExerciseKind::Completion,
                concepts: &["format specs"],
                prompt: "Fill the hole so π prints with three decimal places.",
                mode: ExampleMode::Imperative,
                expected: "3.142",
                starter: "## Main\nLet pi be 3.14159.\nShow \"{pi:???}\".",
                hints: &[
                    "A format spec follows the colon: a dot, then how many decimal places.",
                    "Three places wanted.",
                ],
                reference: "## Main\nLet pi be 3.14159.\nShow \"{pi:.3}\".",
            },
            Exercise {
                id: "ex-text-badge",
                kind: ExerciseKind::Practice,
                concepts: &["string interpolation"],
                prompt: "Let name be \"Ada\" and level be 3; print exactly `Ada — level 3` with one interpolated Show.",
                mode: ExampleMode::Imperative,
                expected: "Ada — level 3",
                starter: "## Main\n",
                hints: &[
                    "Two Lets, then one Show with two brace holes.",
                    "The dash is an em dash — copy it from the prompt.",
                ],
                reference: "## Main\nLet name be \"Ada\".\nLet level be 3.\nShow \"{name} — level {level}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-text-interp",
                question: "Which of these render inside braces: a variable name, `a + b`, `length of xs`?",
                answer: "The name and the arithmetic. `length of xs` must be computed first and interpolated by name.",
            },
            RecapQuestion {
                id: "rq-text-concat",
                question: "`+` joins Text. What joins two sequences?",
                answer: "`followed by` — one new sequence, the second's elements after the first's.",
            },
            RecapQuestion {
                id: "rq-text-char",
                question: "How do you write a Char literal, and what wraps it?",
                answer: "Backticks — a single Unicode character like `a` or the escape for newline.",
            },
        ],
    },
    Section {
        id: "operators",
        number: 8,
        title: "Operators and Expressions",
        part: "Part II: The Language",
        content: r#"
Operators let you combine values into expressions. LOGOS supports both symbolic operators (like `+`) and English words (like `plus`).

### Arithmetic

| Operation | Symbol | English |
|-----------|--------|---------|
| Addition | `+` | `plus` |
| Subtraction | `-` | `minus` |
| Multiplication | `*` | `times` |
| Division | `/` | `divided by` |
| Modulo | `%` | `modulo` |

### Comparisons

| Operation | Symbol | English |
|-----------|--------|---------|
| Less than | `<` | `is less than` |
| Greater than | `>` | `is greater than` |
| Less or equal | `<=` | `is at most` |
| Greater or equal | `>=` | `is at least` |
| Equal | `==` | `equals` |
| Not equal | `!=` | `is not` |

### Logical Operators

| Operation | Keyword | Meaning |
|-----------|---------|---------|
| AND | `and` | Both must be true |
| OR | `or` | At least one must be true |
| NOT | `not` | Inverts true/false |
"#,
        hook: "You know `+` and `<`. Now try: is 7 between 1 and 10? Is 15 divisible by 5? Does 0.1 + 0.2 equal 0.3 — and should it? LOGOS spells these questions the way you would ask them out loud.",
        objectives: &[
            "translate between symbolic and English operator spellings",
            "predict the result of Int division before running it",
            "use chained comparisons and the number predicates",
        ],
        misconceptions: &[
            Misconception {
                id: "m-int-div",
                expectation: "10 / 3 is 3.333… — division produces fractions (Python 3, JavaScript).",
                reality: "Int division stays in the integers: 10 / 3 is 3, and `%` hands you the remainder 1. Ask for fractions explicitly — Rational when it must be exact, floats when it must be fast.",
                evidence: "operators-int-division",
            },
            Misconception {
                id: "m-chained",
                expectation: "`1 <= x <= 10` is a bug: it compares a Bool to 10 (C), or needs `&&` (JavaScript).",
                reality: "Chained comparisons mean the mathematics: both legs must hold, exactly as written on a blackboard. `1 <= x <= 10` is the range test you meant.",
                evidence: "chained-comparisons",
            },
        ],
        examples: &[
            CodeExample {
                id: "arithmetic",
                label: "Arithmetic Operations",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Sum: 13\nDifference: 7\nProduct: 30\nQuotient: 3\nRemainder: 1"),
                predict_first: false,
                code: r#"## Main
Let a be 10.
Let b be 3.

Let sum be a + b.
Let diff be a - b.
Let prod be a * b.
Let quot be a / b.
Let rem be a % b.

Show "Sum: " + sum.
Show "Difference: " + diff.
Show "Product: " + prod.
Show "Quotient: " + quot.
Show "Remainder: " + rem."#,
            },
            CodeExample {
                id: "operators-int-division",
                label: "Int Division Stays Whole",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3\n1"),
                predict_first: true,
                code: r#"## Main
Show 10 / 3.
Show 10 % 3."#,
            },
            CodeExample {
                id: "comparisons",
                label: "Comparisons",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("true\nfalse\ntrue\ntrue\ntrue"),
                predict_first: false,
                code: r#"## Main
Let x be 5.
Let y be 10.

Show x is less than y.
Show x is greater than y.
Show x equals 5.
Show x is at most 5.
Show x is at least 5."#,
            },
            CodeExample {
                id: "logical",
                label: "Logical Operators",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("false\ntrue\nfalse"),
                predict_first: false,
                code: r#"## Main
Let a be true.
Let b be false.

Show a and b.
Show a or b.
Show not a."#,
            },
            CodeExample {
                id: "chained-comparisons",
                label: "Chained Comparisons",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("true"),
                predict_first: true,
                code: r#"## Main
Let x be 5.
Show 1 <= x <= 10."#,
            },
            CodeExample {
                id: "number-predicates",
                label: "is even / is odd / is divisible by / is between",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("true\ntrue\ntrue\ntrue"),
                predict_first: false,
                code: r#"## Main
Show 6 is even.
Show 7 is odd.
Show 15 is divisible by 5.
Show 7 is between 1 and 10."#,
            },
            CodeExample {
                id: "floor-div-power",
                label: "Floor Division and Exponentiation",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3\n-4\n1024"),
                predict_first: false,
                code: r#"## Main
Show 7 // 2.
Let a be 0 - 7.
Show a // 2.
Show 2 ** 10."#,
            },
            CodeExample {
                id: "approximately",
                label: "is approximately (float-tolerant equality)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("true"),
                predict_first: false,
                code: r#"## Main
Show 0.1 + 0.2 is approximately 0.3."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-ops-between",
                kind: ExerciseKind::Completion,
                concepts: &["number predicates"],
                prompt: "Fill the hole so the range test reads like English and prints `true`.",
                mode: ExampleMode::Imperative,
                expected: "true",
                starter: "## Main\nShow 7 is ??? 1 and 10.",
                hints: &[
                    "One word names the range relationship.",
                    "It pairs with `and` to name both ends.",
                ],
                reference: "## Main\nShow 7 is between 1 and 10.",
            },
            Exercise {
                id: "ex-predicates",
                kind: ExerciseKind::Practice,
                concepts: &["number predicates"],
                prompt: "Print two lines: whether 12 is even, then whether 12 is divisible by 5.",
                mode: ExampleMode::Imperative,
                expected: "true\nfalse",
                starter: "## Main\nShow 12 is even.",
                hints: &[
                    "The second line asks a different question about the same number.",
                    "`is divisible by` is the predicate you want — what does it say about 12 and 5?",
                ],
                reference: "## Main\nShow 12 is even.\nShow 12 is divisible by 5.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-ops-div",
                question: "What do `10 / 3` and `10 % 3` print?",
                answer: "3 and 1 — quotient and remainder, both integers.",
            },
            RecapQuestion {
                id: "rq-ops-chain",
                question: "How does `1 <= x <= 10` differ from the same line in C?",
                answer: "LOGOS reads it as mathematics — both comparisons must hold. C compares a Bool to 10.",
            },
            RecapQuestion {
                id: "rq-ops-approx",
                question: "When does `is approximately` beat `equals`?",
                answer: "Comparing floats — it tolerates the tiny binary error that exact equality trips over.",
            },
        ],
    },
    Section {
        id: "control-flow",
        number: 9,
        title: "Control Flow",
        part: "Part II: The Language",
        content: r#"
Control flow determines which code runs and in what order. LOGOS provides conditionals and loops using natural English syntax.

### Conditionals

Use `If` to execute code only when a condition is true. The colon (`:`) after the condition opens an indented block.

### If/Otherwise

Use `Otherwise` to handle the false case.

### While Loops

Use `While` to repeat code as long as a condition is true.

### For-Each Loops

Use `Repeat for` to iterate over collections.
"#,
        hook: "Thirty exam scores need letter grades, and score seventeen needs a different answer than score three. One `Show` per score can't do it — the program has to decide, and it has to repeat.",
        objectives: &[
            "write If/Otherwise branches and While loops",
            "choose between counted Repeat and for-each iteration",
            "predict how many times `from 1 to 5` runs",
        ],
        misconceptions: &[
            Misconception {
                id: "m-inclusive-range",
                expectation: "A counted loop `from 1 to 5` runs four times — ranges exclude their end (Python's range, Rust's ..).",
                reality: "`Repeat for i from 1 to 5` runs five times: 1 through 5, both ends included — the same convention as LOGOS slices. One inclusive rule everywhere beats two exclusive ones somewhere.",
                evidence: "repeat-from-to",
            },
        ],
        examples: &[
            CodeExample {
                id: "if-otherwise",
                label: "If/Otherwise",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("It's comfortable."),
                predict_first: false,
                code: r#"## Main
Let temperature be 72.

If temperature is greater than 80:
    Show "It's hot!".
Otherwise:
    Show "It's comfortable."."#,
            },
            CodeExample {
                id: "while-loop",
                label: "While Loop",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("1\n2\n3\n4\n5"),
                predict_first: false,
                code: r#"## Main
Let count be 1.

While count is at most 5:
    Show count.
    Set count to count + 1."#,
            },
            CodeExample {
                id: "for-each",
                label: "For-Each Loop",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("1\n2\n3\n4\n5"),
                predict_first: false,
                code: r#"## Main
Let numbers be [1, 2, 3, 4, 5].

Repeat for n in numbers:
    Show n."#,
            },
            CodeExample {
                id: "grading",
                label: "Grading Example",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Grade: B"),
                predict_first: false,
                code: r#"## Main
Let score be 85.

If score is at least 90:
    Show "Grade: A".
If score is at least 80 and score is less than 90:
    Show "Grade: B".
If score is at least 70 and score is less than 80:
    Show "Grade: C".
If score is less than 70:
    Show "Grade: F"."#,
            },
            CodeExample {
                id: "repeat-from-to",
                label: "Counted Repeat (1-based, inclusive)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("1\n2\n3\n4\n5"),
                predict_first: true,
                code: r#"## Main
Repeat for i from 1 to 5:
    Show i."#,
            },
            CodeExample {
                id: "break-stmt",
                label: "Break",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3"),
                predict_first: false,
                code: r#"## Main
Let mutable i be 0.
While true:
    Set i to i + 1.
    If i equals 3:
        Break.
Show i."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-flow-grade",
                kind: ExerciseKind::Completion,
                concepts: &["If/Otherwise"],
                prompt: "Fill the hole so the false branch fires and prints the lower grade.",
                mode: ExampleMode::Imperative,
                expected: "Grade: C or below",
                starter: "## Main\nLet score be 72.\nIf score is at least 80:\n    Show \"Grade: B or better\".\n???:\n    Show \"Grade: C or below\".",
                hints: &[
                    "The false branch has its own keyword.",
                    "It ends with a colon, like If.",
                ],
                reference: "## Main\nLet score be 72.\nIf score is at least 80:\n    Show \"Grade: B or better\".\nOtherwise:\n    Show \"Grade: C or below\".",
            },
            Exercise {
                id: "ex-flow-countdown",
                kind: ExerciseKind::Practice,
                concepts: &["While loop"],
                prompt: "Count down from 5 to 1 with a While loop, then print `Liftoff!`.",
                mode: ExampleMode::Imperative,
                expected: "5\n4\n3\n2\n1\nLiftoff!",
                starter: "## Main\nLet mutable n be 5.",
                hints: &[
                    "Loop while n is at least 1.",
                    "Show n before decrementing, and Liftoff! after the loop ends.",
                ],
                reference: "## Main\nLet mutable n be 5.\nWhile n is at least 1:\n    Show n.\n    Set n to n - 1.\nShow \"Liftoff!\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-flow-pick",
                question: "While or Repeat for — which for 'until a condition changes', which for 'once per element'?",
                answer: "While for the condition; Repeat for the elements.",
            },
            RecapQuestion {
                id: "rq-flow-range",
                question: "How many times does `Repeat for i from 1 to 5` run?",
                answer: "Five — both ends are included.",
            },
            RecapQuestion {
                id: "rq-flow-break",
                question: "What does `Break.` do inside `While true:`?",
                answer: "Exits the loop immediately — the only way out of an always-true loop.",
            },
        ],
    },
    Section {
        id: "functions",
        number: 10,
        title: "Functions",
        part: "Part II: The Language",
        content: r#"
Functions are reusable blocks of code. In LOGOS, you define functions using natural English headers that describe what the function does.

### Defining Functions

A function definition starts with `## To` followed by the function name.

### Parameters

Functions can accept parameters—values passed in when the function is called. Separate
them with `and` in the definition (commas also parse; the guide writes the English form).
Calls use commas: `add(3, 5)`.

### Return Values

Use `-> Type` to specify what the function returns.

### Recursion

Functions can call themselves. This is called recursion. Every recursive function needs:
1. A **base case** — when to stop recursing
2. A **recursive case** — calling itself with a "smaller" problem
"#,
        hook: "The midpoint formula appears three times in the finished guessing game. Change it in one place and the other two copies are already wrong. Code you can name once and call anywhere is code that cannot drift apart.",
        objectives: &[
            "write functions with typed parameters and return values",
            "trace a recursive call down to its base case",
            "name the two cases every recursive function needs",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "simple-function",
                label: "Simple Function",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Hello, Alice!\nHello, Bob!"),
                predict_first: false,
                code: r#"## To greet (name: Text):
    Show "Hello, " + name + "!".

## Main
greet("Alice").
greet("Bob")."#,
            },
            CodeExample {
                id: "function-return",
                label: "Function with Return",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("8"),
                predict_first: false,
                code: r#"## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
Let sum be add(3, 5).
Show sum."#,
            },
            CodeExample {
                id: "factorial",
                label: "Recursive Factorial",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("120"),
                predict_first: false,
                code: r#"## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(5)."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-fn-signature",
                kind: ExerciseKind::Completion,
                concepts: &["function definition"],
                prompt: "Fill the hole in the signature so `double` type-checks and prints 42.",
                mode: ExampleMode::Imperative,
                expected: "42",
                starter: "## To double (n: ???) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).",
                hints: &[
                    "The parameter is a whole number.",
                    "Type names start with a capital letter.",
                ],
                reference: "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).",
            },
            Exercise {
                id: "ex-fn-max3",
                kind: ExerciseKind::Practice,
                concepts: &["function definition", "Return"],
                prompt: "Write `max3`, the largest of three Ints (the built-in `max` takes two), and show max3(3, 9, 5).",
                mode: ExampleMode::Imperative,
                expected: "9",
                starter: "## Main\nShow max3(3, 9, 5).",
                hints: &[
                    "Two `max` calls nest: the winner of the first pair meets the third.",
                    "The whole body is one Return line.",
                ],
                reference: "## To max3 (a: Int) and (b: Int) and (c: Int) -> Int:\n    Return max(max(a, b), c).\n\n## Main\nShow max3(3, 9, 5).",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-fn-parts",
                question: "What two cases does every recursive function need?",
                answer: "A base case that stops, and a recursive case that calls itself on a smaller problem.",
            },
            RecapQuestion {
                id: "rq-fn-sep",
                question: "How are parameters separated in a definition — and in a call?",
                answer: "`and` in the definition (commas also parse); commas at the call.",
            },
            RecapQuestion {
                id: "rq-fn-return",
                question: "What does `-> Int` declare in a function header?",
                answer: "The return type.",
            },
        ],
    },
    Section {
        id: "closures",
        number: 11,
        title: "Closures and Function Values",
        part: "Part II: The Language",
        content: r#"
Functions are values. A closure is a function written inline — bind it to a name, pass it
to another function, return it.

### Expression closures

`(x: Int) -> x + 1` is a complete function: one parameter, one expression, the result is
the return value.

### Block closures

When the body needs statements, write `->:` and indent — the block form.

### Functions that take functions

A parameter typed `fn(Int) -> Int` accepts any matching closure. This is how map/filter
pipelines and callbacks are built.

> **Mental model:** A closure is a recipe card that also photographs the pantry:
> `(x: Int) -> x + n` copies the value `n` holds at the moment the card is written.
> Reassign `n` later and the card still cooks from the photograph, not the shelf.
> The photograph is not the whole kitchen, though — parameters still arrive fresh
> on every call.

*Try it: in the last example, swap the doubling closure for `(n: Int) -> n * n`.*
"#,
        hook: "A sort needs the rule 'compare by age'; a filter needs 'keep the positives'. Those rules are tiny functions — and they work best passed around like any other value, written right where they're needed.",
        objectives: &[
            "write expression and block closures",
            "use function-typed parameters to accept behavior as an argument",
            "predict what a closure captures when its variable later changes",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "closure-basic",
                label: "An Expression Closure",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("42"),
                predict_first: false,
                code: r#"## Main
Let inc be (x: Int) -> x + 1.
Show inc(41)."#,
            },
            CodeExample {
                id: "closure-block",
                label: "A Block Closure",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("positive"),
                predict_first: false,
                code: r#"## Main
Let classify be (n: Int) ->:
    If n is greater than 0:
        Return "positive".
    Return "not positive".
Show classify(5)."#,
            },
            CodeExample {
                id: "closure-arg",
                label: "Passing a Closure to a Function",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("42"),
                predict_first: false,
                code: r#"## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Show apply((n: Int) -> n * 2, 21)."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-closure-hole",
                kind: ExerciseKind::Completion,
                concepts: &["expression closures"],
                prompt: "Fill the hole to finish the doubling closure.",
                mode: ExampleMode::Imperative,
                expected: "42",
                starter: "## Main\nLet double be (x: Int) ??? x * 2.\nShow double(21).",
                hints: &[
                    "An arrow separates the parameter from the body.",
                    "The same arrow that writes a function's return type.",
                ],
                reference: "## Main\nLet double be (x: Int) -> x * 2.\nShow double(21).",
            },
            Exercise {
                id: "ex-closure-twice",
                kind: ExerciseKind::Practice,
                concepts: &["function-typed parameters"],
                prompt: "Write `twice` that applies a function two times — twice(f, x) = f(f(x)) — and show twice((n: Int) -> n + 3, 10).",
                mode: ExampleMode::Imperative,
                expected: "16",
                starter: "## Main\nShow twice((n: Int) -> n + 3, 10).",
                hints: &[
                    "The first parameter's type is `fn(Int) -> Int`.",
                    "Call f on the result of calling f.",
                ],
                reference: "## To twice (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(f(x)).\n\n## Main\nShow twice((n: Int) -> n + 3, 10).",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-closure-capture",
                question: "A closure mentions `n`, and `n` is Set to a new value afterwards. Which value does the closure use?",
                answer: "The value `n` held when the closure was created — capture copies; it does not watch.",
            },
            RecapQuestion {
                id: "rq-closure-forms",
                question: "When do you need `->:` instead of `->`?",
                answer: "When the body needs statements — the block form.",
            },
            RecapQuestion {
                id: "rq-closure-type",
                question: "What is the type of a parameter that accepts Int-to-Int functions?",
                answer: "`fn(Int) -> Int`.",
            },
        ],
    },
    Section {
        id: "collections",
        number: 12,
        title: "Collections",
        part: "Part II: The Language",
        content: r#"
Collections hold multiple values. LOGOS provides four main collection types:

| Collection | Description | Index Type |
|------------|-------------|------------|
| `Seq of T` | Ordered list | Int (1-based) |
| `Map of K to V` | Key-value pairs | Any key type |
| `Set of T` | Unique elements | N/A (membership) |
| `Tuple` | Fixed-size, mixed types | Int (1-based) |

### Creating Lists

Create a list with square brackets, or create an empty list with a type.

### Accessing Elements

LOGOS uses **1-based indexing**. The first element is at position 1, not 0. Why? Because that's how humans count.

### Modifying Collections

- `Push` to add an element to the end
- `Pop` to remove and get the last element (a list with no items stops the program, naming the list)
- `copy of` to create a deep copy

### Slicing

Extract a portion of a list with `through`. Slicing is **inclusive** on both ends.

### Maps (Dictionaries)

Maps store key-value pairs. Unlike lists which use integer indexing, maps use keys of any type.

**Create a map:**

`Let prices be a new Map of Text to Int.`

**Access a value by key:**

`Let cost be prices["iron"].`

**Set a value by key:**

`Set prices["iron"] to 100.`

Maps are useful for lookups, caches, and associating data without needing a struct.

### Bracket Syntax

Both lists and maps support bracket indexing as an alternative to `item X of`:

| English Style | Bracket Style |
|---------------|---------------|
| `item 1 of items` | `items[1]` |
| `item "iron" of prices` | `prices["iron"]` |
| `Set item "key" of map to val.` | `Set map["key"] to val.` |

Both compile to the same code—use whichever reads better in context.

### Sets

Sets store unique elements with no duplicates. Unlike lists, sets have no order and no index.

**Create a set:**

`Let numbers be a new Set of Int.`

**Add elements:**

`Add 5 to numbers.`

**Remove elements:**

`Remove 5 from numbers.`

**Check membership:**

`If numbers contains 5:` or `If 5 in numbers:`

**Set operations:**
- `a union b` — elements in either set
- `a intersection b` — elements in both sets

### Tuples

Tuples are fixed-size collections that can hold values of different types. Unlike lists, tuples can mix integers, text, and other types in a single collection.

**Create a tuple:**

`Let point be (10, 20).`

`Let record be ("Alice", 25, true).`

**Access elements (1-indexed):**

`Let x be point[1].` or `Let x be item 1 of point.`

**Get length:**

`Let size be length of record.`

Tuples are useful for returning multiple values from functions or grouping related but differently-typed data without defining a struct.
"#,
        hook: "One player has one score — `Let` handles that. A leaderboard has a hundred scores, a shop keeps prices by item name, and a lottery draw allows no duplicates. Different shapes of 'many' want different containers.",
        objectives: &[
            "choose the right container among Seq, Map, Set, and Tuple",
            "predict what aliasing does to a list before you run it",
            "write map updates and set membership checks",
        ],
        misconceptions: &[
            Misconception {
                id: "m-alias",
                expectation: "`Let b be a` gives me my own copy of the list — changing b leaves a alone.",
                reality: "A plain binding aliases: `a` and `b` are two names for one list until you ask for `copy of`. Predict what `a` looks like after pushing to `b` — then run the evidence.",
                evidence: "copy-aliasing",
            },
            Misconception {
                id: "m-map-miss",
                expectation: "Reading a missing key returns null, None, or a default — every map I know is forgiving.",
                reality: "A missing key is an error that names the key. If absence is meaningful in your domain, say so with Option — a map that invents values hides bugs instead of reporting them.",
                evidence: "map-miss",
            },
        ],
        examples: &[
            CodeExample {
                id: "creating-lists",
                label: "Creating Lists",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1, 2, 3, 4, 5]\n[Alice, Bob, Charlie]"),
                predict_first: false,
                code: r#"## Main
Let numbers be [1, 2, 3, 4, 5].
Let names be ["Alice", "Bob", "Charlie"].
Show numbers.
Show names."#,
            },
            CodeExample {
                id: "accessing-elements",
                label: "Accessing Elements (1-indexed)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("apple\nbanana\ncherry"),
                predict_first: false,
                code: r#"## Main
Let fruits be ["apple", "banana", "cherry"].

Let first be item 1 of fruits.
Let second be item 2 of fruits.
Let third be item 3 of fruits.

Show first.
Show second.
Show third."#,
            },
            CodeExample {
                id: "push-pop",
                label: "Push and Pop",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1, 2, 3, 4, 5]\n5\n[1, 2, 3, 4]"),
                predict_first: false,
                code: r#"## Main
Let numbers be [1, 2, 3].
Push 4 to numbers.
Push 5 to numbers.
Show numbers.

Pop from numbers into last.
Show last.
Show numbers."#,
            },
            CodeExample {
                id: "slicing",
                label: "Slicing (inclusive, 1-based)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[banana, cherry]"),
                predict_first: false,
                code: r#"## Main
Let fruits be ["apple", "banana", "cherry", "date"].
Let middle be items 2 through 3 of fruits.
Show middle."#,
            },
            CodeExample {
                id: "list-iteration",
                label: "Iterating and Accumulating",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Total: 150"),
                predict_first: false,
                code: r#"## Main
Let numbers be [10, 20, 30, 40, 50].
Let total be 0.

Repeat for n in numbers:
    Set total to total + n.

Show "Total: " + total."#,
            },
            CodeExample {
                id: "map-create",
                label: "Creating Maps",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Iron: 10\nCopper: 25\nGold: 100"),
                predict_first: false,
                code: r#"## Main
Let prices be a new Map of Text to Int.
Set prices["iron"] to 10.
Set prices["copper"] to 25.
Set prices["gold"] to 100.
Show "Iron: " + prices["iron"].
Show "Copper: " + prices["copper"].
Show "Gold: " + prices["gold"]."#,
            },
            CodeExample {
                id: "map-access",
                label: "Map Access",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Wood: 50"),
                predict_first: false,
                code: r#"## Main
Let inventory be a new Map of Text to Int.
Set inventory["wood"] to 50.
Set inventory["stone"] to 30.

Let wood_count be inventory["wood"].
Show "Wood: " + wood_count."#,
            },
            CodeExample {
                id: "map-miss",
                label: "A Missing Key Is an Error (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Key 'missing' not found in map",
                },
                expected_output: None,
                predict_first: true,
                code: r#"## Main
Let prices be a new Map of Text to Int.
Set prices["iron"] to 10.
Show prices["missing"]."#,
            },
            CodeExample {
                id: "map-update",
                label: "Map Update",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Initial: 100\nUpdated: 150"),
                predict_first: false,
                code: r#"## Main
Let scores be a new Map of Text to Int.
Set scores["Alice"] to 100.
Show "Initial: " + scores["Alice"].

Set scores["Alice"] to 150.
Show "Updated: " + scores["Alice"]."#,
            },
            CodeExample {
                id: "bracket-syntax",
                label: "Bracket Syntax",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("10\n5\n99"),
                predict_first: false,
                code: r#"## Main
Let items be [10, 20, 30].
Let prices be a new Map of Text to Int.

Set prices["iron"] to 5.
Set prices["gold"] to 100.

Show items[1].
Show prices["iron"].
Set items[2] to 99.
Show items[2]."#,
            },
            CodeExample {
                id: "set-create",
                label: "Creating Sets",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("{1, 2, 3}"),
                predict_first: false,
                code: r#"## Main
Let numbers be a new Set of Int.
Add 1 to numbers.
Add 2 to numbers.
Add 3 to numbers.
Add 1 to numbers.
Show numbers."#,
            },
            CodeExample {
                id: "set-contains",
                label: "Set Membership",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("3 is prime\n4 is not prime"),
                predict_first: false,
                code: r#"## Main
Let primes be a new Set of Int.
Add 2 to primes.
Add 3 to primes.
Add 5 to primes.
Add 7 to primes.

If primes contains 3:
    Show "3 is prime".
If primes contains 4:
    Show "4 is prime".
Otherwise:
    Show "4 is not prime"."#,
            },
            CodeExample {
                id: "set-remove",
                label: "Adding and Removing",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("{red, green, blue}\n{red, blue}"),
                predict_first: false,
                code: r#"## Main
Let colors be a new Set of Text.
Add "red" to colors.
Add "green" to colors.
Add "blue" to colors.
Show colors.

Remove "green" from colors.
Show colors."#,
            },
            CodeExample {
                id: "set-operations",
                label: "Set Operations",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Intersection: {2, 3}\nUnion: {1, 2, 3, 4}"),
                predict_first: false,
                code: r#"## Main
Let a be a new Set of Int.
Let b be a new Set of Int.

Add 1 to a. Add 2 to a. Add 3 to a.
Add 2 to b. Add 3 to b. Add 4 to b.

Let both be a intersection b.
Let either be a union b.
Show "Intersection: " + both.
Show "Union: " + either."#,
            },
            CodeExample {
                id: "tuple-create",
                label: "Creating Tuples",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("(10, 20)\n(Alice, 25, true)"),
                predict_first: false,
                code: r#"## Main
Let point be (10, 20).
Let record be ("Alice", 25, true).
Show point.
Show record."#,
            },
            CodeExample {
                id: "tuple-access",
                label: "Accessing Tuple Elements",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("answer\n42"),
                predict_first: false,
                code: r#"## Main
Let data be ("answer", 42).
Let label be data[1].
Let value be data[2].
Show label.
Show value."#,
            },
            CodeExample {
                id: "tuple-mixed",
                label: "Mixed-Type Tuples",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Bob\n30\n5.9\n3"),
                predict_first: false,
                code: r#"## Main
Let person be ("Bob", 30, 5.9).
Show item 1 of person.
Show item 2 of person.
Show item 3 of person.
Show length of person."#,
            },
            CodeExample {
                id: "empty-typed-list",
                label: "Typed Empty Lists",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[7]"),
                predict_first: false,
                code: r#"## Main
Let xs be [] of Int.
Push 7 to xs.
Show xs."#,
            },
            CodeExample {
                id: "with-capacity",
                label: "Capacity Hints",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1]"),
                predict_first: false,
                code: r#"## Main
Let xs be a new Seq of Int with capacity 10.
Push 1 to xs.
Show xs."#,
            },
            CodeExample {
                id: "set-item",
                label: "Index Assignment",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[10, 99, 30]"),
                predict_first: false,
                code: r#"## Main
Let xs be [10, 20, 30].
Set item 2 of xs to 99.
Show xs."#,
            },
            CodeExample {
                id: "map-destructure",
                label: "Destructuring Map Iteration",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Ada is 36"),
                predict_first: false,
                code: r#"## Main
Let ages be a new Map of Text to Int.
Set ages["Ada"] to 36.
Repeat for (name, age) in ages:
    Show "{name} is {age}"."#,
            },
            CodeExample {
                id: "copy-aliasing",
                label: "copy of vs Aliasing",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("[1, 2]\n[1, 2, 3]"),
                predict_first: true,
                code: r#"## Main
Let a be [1, 2].
Let b be copy of a.
Push 3 to b.
Show a.
Show b."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-coll-count",
                kind: ExerciseKind::Completion,
                concepts: &["maps"],
                prompt: "Fill the hole so the counter increments to 2.",
                mode: ExampleMode::Imperative,
                expected: "2",
                starter: "## Main\nLet counts be a new Map of Text to Int.\nSet counts[\"ada\"] to 1.\nSet counts[\"ada\"] to counts[\"ada\"] ??? 1.\nShow counts[\"ada\"].",
                hints: &[
                    "Increment means: the old value, plus one.",
                    "The map read and the map write share one Set line.",
                ],
                reference: "## Main\nLet counts be a new Map of Text to Int.\nSet counts[\"ada\"] to 1.\nSet counts[\"ada\"] to counts[\"ada\"] + 1.\nShow counts[\"ada\"].",
            },
            Exercise {
                id: "ex-coll-dedupe",
                kind: ExerciseKind::Practice,
                concepts: &["sets"],
                prompt: "Add 3, 1, 3, 2, 1 to a Set of Int and print how many distinct numbers it holds.",
                mode: ExampleMode::Imperative,
                expected: "3",
                starter: "## Main\nLet seen be a new Set of Int.",
                hints: &[
                    "Sets ignore duplicates on their own — Add all five.",
                    "`length of` counts the survivors.",
                ],
                reference: "## Main\nLet seen be a new Set of Int.\nAdd 3 to seen.\nAdd 1 to seen.\nAdd 3 to seen.\nAdd 2 to seen.\nAdd 1 to seen.\nShow length of seen.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-coll-pick",
                question: "Scores by player name, unique winners, ordered history — which containers?",
                answer: "Map of Text to Int; Set; Seq.",
            },
            RecapQuestion {
                id: "rq-coll-alias",
                question: "When do two names point at one list, and how do you break the link?",
                answer: "After `Let b be a` they alias; `Let b be copy of a` clones.",
            },
            RecapQuestion {
                id: "rq-coll-miss",
                question: "What does reading a missing map key do?",
                answer: "Errors, naming the key — absence is never silently invented.",
            },
        ],
    },
    Section {
        id: "user-types",
        number: 13,
        title: "User-Defined Types",
        part: "Part II: The Language",
        content: r#"
Beyond primitive types and collections, LOGOS lets you define your own types to model your problem domain.

### Structs

A struct (structure) groups related values together. Define one in a `## Definition` block using `A [TypeName] has:` syntax.

### Creating Instances

Use `a new [Type] with [fields]` to create instances.

### Accessing Fields

Use `'s` (possessive) to access fields.

### Enums

An enum (enumeration) defines a type that can be one of several variants using `A [TypeName] is either:` syntax.

### Pattern Matching

Use `Inspect` to handle different enum variants with `When` clauses.
"#,
        hook: "A point is an x AND a y; a direction is north OR south OR east OR west. Every domain has its own ANDs and ORs — declare them, and LOGOS refuses every value that doesn't fit.",
        objectives: &[
            "write struct definitions with `has:` and enum definitions with `is either:`",
            "read and write possessive field access fluently",
            "write exhaustive Inspect/When matches",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "struct-basic",
                label: "Basic Struct",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("10\n20"),
                predict_first: false,
                code: r#"## Definition
A Point has:
    an x: Int.
    a y: Int.

## Main
Let p be a new Point with x 10 and y 20.
Show p's x.
Show p's y."#,
            },
            CodeExample {
                id: "struct-person",
                label: "Person Struct",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Alice\n25"),
                predict_first: false,
                code: r#"## Definition
A Person has:
    a name: Text.
    an age: Int.

## Main
Let alice be a new Person with name "Alice" and age 25.
Show alice's name.
Show alice's age."#,
            },
            CodeExample {
                id: "enum-direction",
                label: "Simple Enum",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("North"),
                predict_first: false,
                code: r#"## Definition
A Direction is either:
    North.
    South.
    East.
    West.

## Main
Let heading be North.
Show heading."#,
            },
            CodeExample {
                id: "inspect-when",
                label: "Pattern Matching (Inspect/When)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Heading east"),
                predict_first: false,
                code: r#"## Definition
A Direction is either:
    North.
    South.
    East.
    West.

## Main
Let heading be East.
Inspect heading:
    When North:
        Show "Heading north".
    When South:
        Show "Heading south".
    When East:
        Show "Heading east".
    When West:
        Show "Heading west"."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-types-arm",
                kind: ExerciseKind::Completion,
                concepts: &["Inspect/When pattern matching"],
                prompt: "Fill the hole so the Green arm fires.",
                mode: ExampleMode::Imperative,
                expected: "go",
                starter: "## Definition\nA Light is either:\n    Red.\n    Green.\n\n## Main\nLet l be Green.\nInspect l:\n    When Red:\n        Show \"stop\".\n    ??? Green:\n        Show \"go\".",
                hints: &[
                    "Every arm starts with the same keyword.",
                    "Arms read `When <Variant>:`.",
                ],
                reference: "## Definition\nA Light is either:\n    Red.\n    Green.\n\n## Main\nLet l be Green.\nInspect l:\n    When Red:\n        Show \"stop\".\n    When Green:\n        Show \"go\".",
            },
            Exercise {
                id: "ex-types-pet",
                kind: ExerciseKind::Practice,
                concepts: &["struct definition", "possessive field access"],
                prompt: "Define a Pet struct with a name (Text) and an age (Int); create one named \"Rex\" aged 4 and print `Rex is 4`.",
                mode: ExampleMode::Imperative,
                expected: "Rex is 4",
                starter: "## Definition\n",
                hints: &[
                    "Declare two fields, each on its own line ending with a period.",
                    "Interpolate the fields possessively: braces around `p's name`.",
                ],
                reference: "## Definition\nA Pet has:\n    a name: Text.\n    an age: Int.\n\n## Main\nLet p be a new Pet with name \"Rex\" and age 4.\nShow \"{p's name} is {p's age}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-types-and-or",
                question: "Struct or enum: a point with x and y? a direction that is north or south?",
                answer: "Struct for the AND (`has:`); enum for the OR (`is either:`).",
            },
            RecapQuestion {
                id: "rq-types-access",
                question: "How do you read the age field of a struct bound to `alice`?",
                answer: "`alice's age` — possessive access; the apostrophe is grammar.",
            },
            RecapQuestion {
                id: "rq-types-exhaust",
                question: "What happens when an Inspect misses a variant and has no Otherwise?",
                answer: "An error — matches must be exhaustive.",
            },
        ],
    },
    Section {
        id: "generics",
        number: 14,
        title: "Generics",
        part: "Part II: The Language",
        content: r#"
Generics let you write types and functions that work with any type, not only specific ones.

### Generic Types

Define a generic type with `[T]` in the type name. The `[T]` is a placeholder that gets replaced with a real type when you use it.

### Multiple Type Parameters

You can have multiple type parameters like `[A]` and `[B]`.

### Generic Collections

Collections are generic types. `Seq of Int` is a sequence of integers.

### Nested Generics

You can nest generic types like `Seq of (Seq of Int)` for a matrix.

> **Mental model:** `Box of [T]` is a labeled shipping container: `[T]` is the blank
> on the label. Stamp it — `Box of Int` — and the checker refuses mismatched cargo at
> the dock. The label exists for the checker alone: by the time the program runs,
> there is no label left, only the value that passed inspection.
"#,
        hook: "A Box that holds an Int and a Box that holds Text are the same idea — writing the type twice would be writing the same bug twice. Write it once, with a blank where the type goes.",
        objectives: &[
            "write a generic type with one and with two parameters",
            "explain what the [T] label buys at check time",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "generic-box",
                label: "Generic Box",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("42\nHello"),
                predict_first: false,
                code: r#"## Definition
A Box of [T] has:
    a contents: T.

## Main
Let int_box be a new Box of Int with contents 42.
Let text_box be a new Box of Text with contents "Hello".

Show int_box's contents.
Show text_box's contents."#,
            },
            CodeExample {
                id: "generic-pair",
                label: "Generic Pair",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("1\none"),
                predict_first: false,
                code: r#"## Definition
A Pair of [A] and [B] has:
    a first: A.
    a second: B.

## Main
Let p be a new Pair of Int and Text with first 1 and second "one".
Show p's first.
Show p's second."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-gen-box",
                kind: ExerciseKind::Completion,
                concepts: &["generic types"],
                prompt: "Fill the hole with the type that matches the contents.",
                mode: ExampleMode::Imperative,
                expected: "sealed",
                starter: "## Definition\nA Box of [T] has:\n    a contents: T.\n\n## Main\nLet b be a new Box of ??? with contents \"sealed\".\nShow b's contents.",
                hints: &[
                    "What type is \"sealed\"?",
                    "The stamp in `Box of ...` names the contents' type.",
                ],
                reference: "## Definition\nA Box of [T] has:\n    a contents: T.\n\n## Main\nLet b be a new Box of Text with contents \"sealed\".\nShow b's contents.",
            },
            Exercise {
                id: "ex-gen-pair",
                kind: ExerciseKind::Practice,
                concepts: &["generic types"],
                prompt: "Make a Pair of Text and Int with first \"answer\" and second 42, and print both fields.",
                mode: ExampleMode::Imperative,
                expected: "answer\n42",
                starter: "## Definition\nA Pair of [A] and [B] has:\n    a first: A.\n    a second: B.\n\n## Main\n",
                hints: &[
                    "Two stamps this time: `Pair of Text and Int`.",
                    "Fields read possessively: `p's first`, `p's second`.",
                ],
                reference: "## Definition\nA Pair of [A] and [B] has:\n    a first: A.\n    a second: B.\n\n## Main\nLet p be a new Pair of Text and Int with first \"answer\" and second 42.\nShow p's first.\nShow p's second.",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-gen-blank",
                question: "What does `[T]` declare in a type definition?",
                answer: "A type parameter — a blank the user of the type fills in per use.",
            },
            RecapQuestion {
                id: "rq-gen-check",
                question: "When is `Box of Int` vs `Box of Text` enforced?",
                answer: "At check time, before the program runs — at run time only the value remains.",
            },
            RecapQuestion {
                id: "rq-gen-nest",
                question: "Can generic types nest?",
                answer: "Yes — `Seq of (Seq of Int)` is a matrix.",
            },
        ],
    },
    Section {
        id: "error-handling",
        number: 15,
        title: "Error Handling",
        part: "Part II: The Language",
        content: r#"
LOGOS uses **Socratic error messages**—friendly, educational feedback that teaches while it corrects.

### The Philosophy

Instead of cryptic compiler errors, LOGOS explains:
1. **What** went wrong
2. **Where** it happened
3. **Why** it's a problem
4. **How** to fix it

### Modeling Failure

Model success and failure as an enum and pattern-match on it — the `Outcome` example below shows
the whole pattern. The standard library's generic `Result` works exactly the same way: construct
with `a new Ok with value x` / `a new Err with error msg`, match with `When Ok (v):` /
`When Err (e):`. And for "maybe a value": `some x` and `none` build an `Option` you can
`Inspect` with `When Some (v):` / `When None:`.

### Error Propagation

Errors propagate naturally through return values. Handle them where appropriate.

### Defensive Programming

Use assertions and guards to prevent errors before they happen.
"#,
        hook: "Divide by zero, an age of -5, a missing file: real inputs fail. The question is whether a failure becomes a crash, a silent wrong answer, or a value your program can Inspect and handle like any other.",
        objectives: &[
            "distinguish guard-and-return from failure-as-value",
            "write Result and Option handling with Inspect",
            "choose Require that for conditions that must stop the program",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "defensive-divide",
                label: "Safe Division with Guard",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("10 / 2 = 5\nError: Cannot divide by zero\nResult after error: 0"),
                predict_first: false,
                code: r#"## To safe_divide (a: Int) and (b: Int) -> Int:
    If b equals 0:
        Show "Error: Cannot divide by zero".
        Return 0.
    Return a / b.

## Main
Let result be safe_divide(10, 2).
Show "10 / 2 = " + result.
Let bad be safe_divide(5, 0).
Show "Result after error: " + bad."#,
            },
            CodeExample {
                id: "validation-example",
                label: "Input Validation",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Age 25 valid: true\nError: Age cannot be negative\nAge -5 valid: false"),
                predict_first: false,
                code: r#"## To validate_age (age: Int) -> Bool:
    If age is less than 0:
        Show "Error: Age cannot be negative".
        Return false.
    If age is greater than 150:
        Show "Error: Age seems unrealistic".
        Return false.
    Return true.

## Main
Let valid be validate_age(25).
Show "Age 25 valid: " + valid.
Let invalid be validate_age(-5).
Show "Age -5 valid: " + invalid."#,
            },
            CodeExample {
                id: "result-pattern",
                label: "Success/Failure with Pattern Matching",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Halved: 5\nError: cannot halve a negative"),
                predict_first: false,
                code: r#"## Definition
An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## To checked_halve (n: Int) -> Outcome:
    If n is less than 0:
        Return a new Failure with message "cannot halve a negative".
    Return a new Success with value (n / 2).

## Main
Let good be checked_halve(10).
Inspect good:
    When Success (v):
        Show "Halved: " + v.
    When Failure (m):
        Show "Error: " + m.

Let bad be checked_halve(-4).
Inspect bad:
    When Success (v):
        Show "Halved: " + v.
    When Failure (m):
        Show "Error: " + m."#,
            },
            CodeExample {
                id: "stdlib-result",
                label: "The Standard Library Result",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("got 5\nerror: divide by zero"),
                predict_first: false,
                code: r#"## To checked_div (a: Int) and (b: Int) -> Result of Int and Text:
    If b equals 0:
        Return a new Err with error "divide by zero".
    Return a new Ok with value (a / b).

## Main
Let r be checked_div(10, 2).
Inspect r:
    When Ok (v):
        Show "got " + v.
    When Err (e):
        Show "error: " + e.
Let bad be checked_div(1, 0).
Inspect bad:
    When Ok (v):
        Show "got " + v.
    When Err (e):
        Show "error: " + e."#,
            },
            CodeExample {
                id: "option-some-none",
                label: "Option: some and none",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("found 4\nnone"),
                predict_first: false,
                code: r#"## Main
Let hit be some 4.
Inspect hit:
    When Some (v):
        Show "found " + v.
    When None:
        Show "none".
Let miss be none.
Inspect miss:
    When Some (v):
        Show "found " + v.
    When None:
        Show "none"."#,
            },
            CodeExample {
                id: "require-that",
                label: "Require that (Fails on Purpose)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::ExpectedError {
                    message: "Assertion failed",
                },
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Let n be 0 - 5.
Require that n is greater than 0.
Show "unreachable"."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-err-hole",
                kind: ExerciseKind::Completion,
                concepts: &["stdlib Result"],
                prompt: "Fill the hole with the failure constructor so the error path prints.",
                mode: ExampleMode::Imperative,
                expected: "error: divide by zero",
                starter: "## To checked_div (a: Int) and (b: Int) -> Result of Int and Text:\n    If b equals 0:\n        Return a new ??? with error \"divide by zero\".\n    Return a new Ok with value (a / b).\n\n## Main\nLet r be checked_div(1, 0).\nInspect r:\n    When Ok (v):\n        Show \"got \" + v.\n    When Err (e):\n        Show \"error: \" + e.",
                hints: &[
                    "Result has two constructors: one for success, one for failure.",
                    "The failure constructor pairs with `with error`.",
                ],
                reference: "## To checked_div (a: Int) and (b: Int) -> Result of Int and Text:\n    If b equals 0:\n        Return a new Err with error \"divide by zero\".\n    Return a new Ok with value (a / b).\n\n## Main\nLet r be checked_div(1, 0).\nInspect r:\n    When Ok (v):\n        Show \"got \" + v.\n    When Err (e):\n        Show \"error: \" + e.",
            },
            Exercise {
                id: "ex-err-option",
                kind: ExerciseKind::Practice,
                concepts: &["Option some/none + Inspect"],
                prompt: "Inspect `some 42` and print `found 42`; then Inspect `none` and print `none`.",
                mode: ExampleMode::Imperative,
                expected: "found 42\nnone",
                starter: "## Main\nLet hit be some 42.\nLet miss be none.",
                hints: &[
                    "Each Inspect needs a Some arm (binding v in parentheses) and a None arm.",
                    "The Some arm can concatenate: \"found \" + v.",
                ],
                reference: "## Main\nLet hit be some 42.\nLet miss be none.\nInspect hit:\n    When Some (v):\n        Show \"found \" + v.\n    When None:\n        Show \"none\".\nInspect miss:\n    When Some (v):\n        Show \"found \" + v.\n    When None:\n        Show \"none\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-err-value",
                question: "How does a LOGOS function report failure without crashing?",
                answer: "It returns failure as a value — an Outcome/Result variant the caller Inspects.",
            },
            RecapQuestion {
                id: "rq-err-option",
                question: "What type do `some 4` and `none` build, and what is it for?",
                answer: "Option — presence or absence made explicit, instead of invented defaults.",
            },
            RecapQuestion {
                id: "rq-err-require",
                question: "What does `Require that` do when its condition is false?",
                answer: "Stops the program with an assertion failure — a hard gate, on purpose.",
            },
        ],
    },
    Section {
        id: "part-2-checkpoint",
        number: 16,
        title: "Checkpoint: The Language",
        part: "Part II: The Language",
        content: r#"
Ten sections of core language deserve proof of mastery. The four challenges below
each blend two or more sections — none can be solved by pattern-matching a single
example, and one reaches back to Part I on purpose.

If a challenge stumps you, its hints name the moves; the section to reread follows
from them.
"#,
        hook: "",
        objectives: &[
            "combine collections, control flow, functions, and interpolation in one program",
            "diagnose which Part II section to reread when a challenge stumps you",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[
            Exercise {
                id: "cp2-invoice",
                kind: ExerciseKind::Challenge,
                concepts: &["Repeat for-each", "string interpolation", "list literal"],
                prompt: "Sum the costs [450, 1200, 300] with a loop and print exactly `total: 1950 cents`.",
                mode: ExampleMode::Imperative,
                expected: "total: 1950 cents",
                starter: "## Main\nLet costs be [450, 1200, 300].",
                hints: &[
                    "A mutable running total starts at 0.",
                    "One interpolated Show after the loop reports it.",
                ],
                reference: "## Main\nLet costs be [450, 1200, 300].\nLet mutable total be 0.\nRepeat for c in costs:\n    Set total to total + c.\nShow \"total: {total} cents\".",
            },
            Exercise {
                id: "cp2-guess-fn",
                kind: ExerciseKind::Challenge,
                concepts: &["function definition", "staged build-one-program tutorial"],
                prompt: "Wrap the tutorial's midpoint in a function `mid(lo, hi)` and show mid(1, 100) and mid(50, 75).",
                mode: ExampleMode::Imperative,
                expected: "50\n62",
                starter: "## Main\nShow mid(1, 100).\nShow mid(50, 75).",
                hints: &[
                    "The body is one Return with the tutorial's formula.",
                    "Floor division keeps the guess whole.",
                ],
                reference: "## To mid (lo: Int) and (hi: Int) -> Int:\n    Return (lo + hi) // 2.\n\n## Main\nShow mid(1, 100).\nShow mid(50, 75).",
            },
            Exercise {
                id: "cp2-classify",
                kind: ExerciseKind::Challenge,
                concepts: &["block closures", "number predicates"],
                prompt: "Bind a block closure `classify` that returns \"even\" or \"odd\", then print the classification of 1, 2, 3.",
                mode: ExampleMode::Imperative,
                expected: "odd\neven\nodd",
                starter: "## Main\n",
                hints: &[
                    "`->:` opens a closure body that can hold an If.",
                    "`is even` is the predicate; Repeat over [1, 2, 3] to print.",
                ],
                reference: "## Main\nLet classify be (n: Int) ->:\n    If n is even:\n        Return \"even\".\n    Return \"odd\".\nRepeat for n in [1, 2, 3]:\n    Show classify(n).",
            },
            Exercise {
                id: "cp2-outcome",
                kind: ExerciseKind::Challenge,
                concepts: &["failure-as-enum pattern", "string interpolation"],
                prompt: "Define an Outcome enum (Success with an Int, Failure with a Text), halve 10 through a checked function, and print `half is 5`.",
                mode: ExampleMode::Imperative,
                expected: "half is 5",
                starter: "## Definition\n",
                hints: &[
                    "Payload variants declare their field in parentheses right after the variant name.",
                    "Inspect the call's result; the Success arm interpolates `{v}`.",
                ],
                reference: "## Definition\nAn Outcome is either:\n    Success (value: Int).\n    Failure (message: Text).\n\n## To checked_halve (n: Int) -> Outcome:\n    If n is less than 0:\n        Return a new Failure with message \"negative\".\n    Return a new Success with value (n / 2).\n\n## Main\nLet r be checked_halve(10).\nInspect r:\n    When Success (v):\n        Show \"half is {v}\".\n    When Failure (m):\n        Show \"failed: {m}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "cq2-container",
                question: "Which container keeps insertion order and allows duplicates?",
                answer: "Seq — the list.",
            },
            RecapQuestion {
                id: "cq2-div",
                question: "What does `10 / 3` print, and how do you get the remainder?",
                answer: "3; `10 % 3` gives 1.",
            },
            RecapQuestion {
                id: "cq2-chain",
                question: "How do you write 'x is at least 1 and at most 10' as one chained comparison?",
                answer: "`1 <= x <= 10`.",
            },
            RecapQuestion {
                id: "cq2-capture",
                question: "A closure captured a variable that later changed. Old value or new?",
                answer: "Old — capture copies at creation.",
            },
            RecapQuestion {
                id: "cq2-payment",
                question: "Struct or enum for 'a payment is cash or card'?",
                answer: "Enum — it is an OR.",
            },
            RecapQuestion {
                id: "cq2-result",
                question: "Why prefer Result over printing an error and returning 0?",
                answer: "The caller can Inspect and decide; a sentinel 0 is indistinguishable from a real answer.",
            },
            RecapQuestion {
                id: "cq2-money",
                question: "What is exact for money: Float, Decimal, or both?",
                answer: "Decimal (and the Money type) — never binary floats.",
            },
            RecapQuestion {
                id: "cq2-slice",
                question: "`items 2 through 3` of a five-item list — how many elements, and why?",
                answer: "Two — LOGOS slices include both ends.",
            },
        ],
    },
];
