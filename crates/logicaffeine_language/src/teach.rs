//! The lesson table for LOGOS constructs: every lesson a surface shows
//! (LSP hover, completion documentation, the REPL's `:explain`) comes from
//! this table, so the editor and the terminal always teach the same thing.
//!
//! A [`ConstructDoc`] is complete by construction: the type has no optional
//! teaching fields, so an entry cannot exist without a plain-English
//! sentence, a runnable example, and a socratic question or tip. The
//! `doc_for` match is wildcard-free (the `token_class` idiom): adding a
//! `TokenType` variant does not compile until someone decides whether it
//! deserves a lesson — the decision lives here, next to the enum, not in an
//! editor plugin. `doc_for_block` is total: every `##` block type teaches.

use crate::token::{BlockType, TokenType};

/// A complete lesson for one construct. Every field is required by the TYPE —
/// an entry cannot exist without a plain sentence, a runnable example, and a
/// socratic question or tip. Clear-and-easy is structural, not aspirational.
pub struct ConstructDoc {
    /// The construct's display name ("Give", "Main", "Seq").
    pub name: &'static str,
    /// ONE plain sentence: what it does. Kept under 90 characters by the
    /// teach lock — if it needs more, the rest belongs in the question/tip.
    pub what: &'static str,
    /// A runnable snippet (code-block body, no fences). Must lex.
    pub example: &'static str,
    /// The socratic seat: a guiding question (contains `?`) or an explicit
    /// `Tip:` — it leads the reader to the next insight, never just restates.
    pub question_or_tip: &'static str,
    /// A LOGOS_QUICKGUIDE.md heading slug for "read more", when one fits.
    pub guide_anchor: Option<&'static str>,
}

const fn lesson(
    name: &'static str,
    what: &'static str,
    example: &'static str,
    question_or_tip: &'static str,
    guide_anchor: Option<&'static str>,
) -> ConstructDoc {
    ConstructDoc {
        name,
        what,
        example,
        question_or_tip,
        guide_anchor,
    }
}

// ---------------------------------------------------------------------------
// Statement keywords
// ---------------------------------------------------------------------------

static LET: ConstructDoc = lesson(
    "Let",
    "Declares a new variable.",
    "Let x be 5.\nLet name: Text be \"Alice\".",
    "Will the value change later? Then declare it `Let mutable x be 5.` — plain `Let` is immutable.",
    Some("2-variables--mutation"),
);

static SET_KW: ConstructDoc = lesson(
    "Set",
    "Updates an existing mutable variable.",
    "Set x to 10.",
    "Was `x` declared with `Let mutable`? Only mutable bindings can be `Set`.",
    Some("2-variables--mutation"),
);

static SWAP_KW: ConstructDoc = lesson(
    "Swap",
    "Exchanges two variables' values — the zero-garbage REVERSIBLE primitive (its own inverse).",
    "Swap x and y.",
    "Are both `x` and `y` defined? Swap runs backward with no history — it reverses at the Landauer floor.",
    Some("2-variables--mutation"),
);

static RETURN: ConstructDoc = lesson(
    "Return",
    "Hands a value back from the current function.",
    "Return x.",
    "Does the value's type match the function's declared `-> Type`? `Return.` alone returns nothing.",
    Some("6-control-flow"),
);

static IF: ConstructDoc = lesson(
    "If",
    "Runs a block only when its condition holds.",
    "If x > 0:\n    Show x.\nOtherwise:\n    Show 0.",
    "What should happen when the condition is false — nothing, an `Otherwise:` branch, or an `elif`?",
    Some("6-control-flow"),
);

static WHILE: ConstructDoc = lesson(
    "While",
    "Repeats a block as long as its condition stays true.",
    "While x > 0:\n    Set x to x - 1.",
    "Does the body change the condition? A body that never does loops forever — `(decreasing e)` proves termination.",
    Some("6-control-flow"),
);

static REPEAT: ConstructDoc = lesson(
    "Repeat",
    "Walks a collection, binding each element in turn.",
    "Repeat for item in items:\n    Show item.",
    "Need positions instead of elements? Count with `for i from 1 to n:` — LOGOS is 1-based.",
    Some("5-collections"),
);

static SHOW: ConstructDoc = lesson(
    "Show",
    "Displays a value while only borrowing it — you keep ownership.",
    "Show x.",
    "Need the value afterward? `Show` lends; `Give` transfers — pick by who owns it next.",
    Some("13-output"),
);

static GIVE: ConstructDoc = lesson(
    "Give",
    "Transfers ownership of a value to a new owner.",
    "Give x to processor.",
    "Who should own this value afterward — you, or the receiver? Keep it by giving `a copy of x`.",
    Some("13-output"),
);

static PUSH: ConstructDoc = lesson(
    "Push",
    "Appends a value to the end of a sequence.",
    "Push 5 to items.",
    "Tip: `Push` grows a Seq by one; for Sets use `Add v to s.` — sets have no order to push onto.",
    Some("5-collections"),
);

static INSPECT: ConstructDoc = lesson(
    "Inspect",
    "Pattern-matches a value, running one branch per variant.",
    "Inspect shape:\n    When Circle (r):\n        Show r.\n    Otherwise:\n        Show \"other\".",
    "Is every variant handled? `Otherwise:` catches the rest — or list each `When` and let the checker verify.",
    Some("6-control-flow"),
);

static CALL: ConstructDoc = lesson(
    "Call",
    "Invokes a function as a statement.",
    "Call process with data.",
    "Tip: in expressions call directly — `add(3, 7)`; `Call f with a and b.` is the statement form.",
    Some("7-functions--closures"),
);

static NEW: ConstructDoc = lesson(
    "New",
    "Creates a struct or variant instance with named field values.",
    "Let p be a new Point with x 10 and y 20.",
    "Tip: fields are set by name — `with x 10 and y 20` — so argument order never bites.",
    Some("8-structs-enums--field-access"),
);

static ESCAPE: ConstructDoc = lesson(
    "Escape",
    "Embeds raw foreign code, skipping every LOGOS checker.",
    "Escape to Rust:\n    println!(\"hello\");",
    "Could plain LOGOS express this? Escaped code is invisible to the ownership and type checkers.",
    None,
);

static CHECK: ConstructDoc = lesson(
    "Check",
    "Enforces a security capability at runtime — a mandatory gate.",
    "Check that balance is at least amount.",
    "What may this code path do, and who says so? `## Policy` blocks define the capabilities.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static POP: ConstructDoc = lesson(
    "Pop",
    "Removes the last element of a sequence.",
    "Pop from xs.\nPop from xs into y.",
    "Need the popped value? `Pop from xs into y.` binds it; plain `Pop` discards it.",
    Some("5-collections"),
);

static ADD: ConstructDoc = lesson(
    "Add",
    "Inserts a value into a set.",
    "Add v to s.",
    "Tip: sets keep one of each value — adding a duplicate changes nothing, and order is not kept.",
    Some("5-collections"),
);

static REMOVE: ConstructDoc = lesson(
    "Remove",
    "Deletes a value from a set, or every item of a list that matches a condition.",
    "Remove v from s.\nRemove every t from todos where t's done.",
    "Tip: removing a value that isn't present is a quiet no-op — test `s contains v` when it matters.",
    Some("5-collections"),
);

static BREAK: ConstructDoc = lesson(
    "Break",
    "Exits the innermost loop immediately.",
    "While true:\n    Break.",
    "Which loop should stop? `Break.` only leaves the innermost one.",
    Some("6-control-flow"),
);

static ASSERT: ConstructDoc = lesson(
    "Assert",
    "Checks a condition at runtime, failing loudly when it is false.",
    "Assert that x is equal to 42.",
    "Is this a debugging aid or a security rule? Debug checks `Assert`; mandatory gates use `Check`.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static TRUST: ConstructDoc = lesson(
    "Trust",
    "States a justified assumption, carrying its reason.",
    "Trust that x is greater than 0 because \"set to 10\".",
    "Why is this safe? The `because` reason is required — future readers hold you to it.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static INCREASE: ConstructDoc = lesson(
    "Increase",
    "Grows a shared CRDT counter field — merges without conflicts.",
    "Increase c's points by 10.",
    "Tip: `Increase` is for `Shared` CRDT fields; plain integers update with `Set i to i + 1.`",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static DECREASE: ConstructDoc = lesson(
    "Decrease",
    "Shrinks a shared CRDT counter field — merges without conflicts.",
    "Decrease g's score by 30.",
    "Tip: `Decrease` is for `Shared` CRDT fields; plain integers update with `Set i to i - 1.`",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static SPAWN: ConstructDoc = lesson(
    "Spawn",
    "Starts an agent — an independent concurrent actor.",
    "Spawn an EchoAgent called \"echo\".",
    "How will you reach it later? The `called \"name\"` handle is how messages find the agent.",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static MOUNT: ConstructDoc = lesson(
    "Mount",
    "Brings a component into the document, or a CRDT onto a journal file.",
    "Mount App at \"#root\".\nMount counter at \"data/counter.journal\".",
    "What is being mounted — a component or a CRDT? A component paints and runs its load handlers once (`Mount App.` takes the whole document); a CRDT loads its journal and writes every later change back to it.",
    None,
);

static MERGE: ConstructDoc = lesson(
    "Merge",
    "Folds one CRDT replica into another without conflicts.",
    "Merge remote into local.",
    "Tip: merge order never matters — replicas converge to the same state either way.",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

static CONVERGE: ConstructDoc = lesson(
    "Converge",
    "Joins one CRDT replica into another by the idempotent lattice join.",
    "Converge remote into local.",
    "Tip: unlike Merge (which adds bare-Int fields), Converge takes their MAX — applying it twice equals once.",
    Some("12-distributed-crdt-concurrency-networking-zones"),
);

// ---------------------------------------------------------------------------
// Block headers
// ---------------------------------------------------------------------------

static BLOCK_MAIN: ConstructDoc = lesson(
    "Main",
    "The program's entry point — statements here run top to bottom.",
    "## Main\nShow \"Hello, World!\".",
    "Tip: one `## Main` per program; everything else is definitions it can call.",
    Some("1-program-structure"),
);

static BLOCK_FUNCTION: ConstructDoc = lesson(
    "To",
    "Defines a function; parameters and return type live in the header.",
    "## To add (a: Int, b: Int) -> Int:\n    Return a + b.",
    "What does it give back? `-> Type` declares it; omit the arrow for a procedure.",
    Some("7-functions--closures"),
);

static BLOCK_COMPONENT: ConstructDoc = lesson(
    "Component",
    "A piece of a page: its own State, the Actions that may change it, and the View it renders.",
    "## Component Counter\nState:\n    Let count be 0.\nActions:\n    To add:\n        Set count to count + 1.\nView:\n    <button on-click={add}>{count}</button>",
    "What state does it own, and what does its view read? Each instance keeps its own record, so placing it twice places two independent counters.",
    Some("7-functions--closures"),
);

static BLOCK_THEOREM: ConstructDoc = lesson(
    "Theorem",
    "Declares a proposition to be proved.",
    "## Theorem: Socrates\nGiven: All men are mortal. Socrates is a man.\nProve: Socrates is mortal.\nProof: Auto.",
    "What structure does the claim have — universal, implication, equality? The proof strategy follows it.",
    Some("1-program-structure"),
);

static BLOCK_PROOF: ConstructDoc = lesson(
    "Proof",
    "Holds the proof steps for the theorem above it.",
    "Proof: Auto.",
    "Stuck? Start with `Auto.` — the prover reports what it can and cannot discharge.",
    None,
);

static BLOCK_DEFINITION: ConstructDoc = lesson(
    "Definition",
    "Introduces new terminology for later sentences to use.",
    "## Definition: A bachelor is an unmarried man.",
    "Tip: `## Definition` explains terms; executable data shapes live in `## A ... has:` blocks.",
    None,
);

static BLOCK_DEFINE: ConstructDoc = lesson(
    "Define",
    "Mints a predicate the prover can unfold by definition.",
    "## Define: A number is tiny if it is less than 10.",
    "Tip: proofs may unfold this exactly — the prover substitutes the body for the name.",
    None,
);

static BLOCK_AXIOM: ConstructDoc = lesson(
    "Axiom",
    "Declares a named formal axiom as a shared premise.",
    "## Axiom cong_refl: for all a b, Cong(a,b,b,a).",
    "Tip: axioms are believed, not proved — keep them few and inspectable.",
    None,
);

static BLOCK_THEORY: ConstructDoc = lesson(
    "Theory",
    "Names a development grouping the axioms and theorems after it.",
    "## Theory Tarski",
    "Tip: a theory bundles `## Axiom`s with the `## Theorem`s proved from them.",
    None,
);

static BLOCK_TYPEDEF: ConstructDoc = lesson(
    "TypeDef",
    "Defines a struct or enum type in English.",
    "## A Point has:\n    An x: Int.\n    A y: Int.",
    "One thing with fields, or one of several shapes? `has:` makes a struct; `is one of:` an enum.",
    Some("8-structs-enums--field-access"),
);

static BLOCK_POLICY: ConstructDoc = lesson(
    "Policy",
    "Defines the security rules that `Check` statements enforce.",
    "## Policy\nUsers can read public files.",
    "Who may do what? Policies name capabilities; `Check that ...` gates on them.",
    Some("10-contracts-refinement-assert-trust-check"),
);

static BLOCK_LOGIC: ConstructDoc = lesson(
    "Logic",
    "Holds direct logical notation instead of English.",
    "## Logic\nforall x. Man(x) -> Mortal(x)",
    "Tip: use it when symbols say it better — English and notation share one prover.",
    None,
);

static BLOCK_EXAMPLE: ConstructDoc = lesson(
    "Example",
    "Shows an illustrative example the compiler treats as documentation.",
    "## Example\nShow 42.",
    "Tip: examples read as prose — the highlighter fades them so code stands out.",
    None,
);

static BLOCK_NOTE: ConstructDoc = lesson(
    "Note",
    "Documentation prose the compiler skips and the highlighter fades.",
    "## Note\nThis module parses dates.",
    "Tip: a `## Note` right above a definition becomes that definition's documentation.",
    None,
);

static BLOCK_REQUIRES: ConstructDoc = lesson(
    "Requires",
    "Declares external crate dependencies for escaped code.",
    "## Requires\nserde",
    "Tip: only `Escape` blocks need this — pure LOGOS programs never do.",
    None,
);

static BLOCK_HARDWARE: ConstructDoc = lesson(
    "Hardware",
    "Declares hardware signals for verification.",
    "## Hardware\nclk is a signal.",
    "What are the inputs and state bits? Properties below verify against exactly these signals.",
    None,
);

static BLOCK_PROPERTY: ConstructDoc = lesson(
    "Property",
    "States temporal assertions about hardware signals.",
    "## Property\nThe counter is eventually zero.",
    "Always or eventually? Temporal words carry the meaning — the prover checks every cycle.",
    None,
);

static BLOCK_TEST: ConstructDoc = lesson(
    "Test",
    "Declares an independent test named by its prose; the body checks with `Expect that`.",
    "## Test: Addition carries correctly\nLet x be 1 plus 1.\nExpect that x is 2.",
    "What should be true when this test ends? Say it with `Expect that` — every expectation is checked, and failures report together.",
    None,
);

static BLOCK_PAGE: ConstructDoc = lesson(
    "Page",
    "A page component served at a URL path; its State, Actions, View and Style follow.",
    "## Page /todos\nState:\n    Let count be 0.\nView:\n    <p>{count}</p>",
    "Which URL should reach this page, and what must it learn from that URL? `## Page /todos/:id (id: Int)` binds a path parameter.",
    None,
);

static BLOCK_STORE: ConstructDoc = lesson(
    "Store",
    "Shared state plus the only actions that may change it, usable from many components.",
    "## Store Session\nState:\n    Let name be \"guest\".\nActions:\n    To login (who: Text):\n        Set name to who.",
    "Which components need this state? Each writes `Use Session.` and reads `Session's name`; only the store's own actions change it.",
    None,
);

static BLOCK_SESSION: ConstructDoc = lesson(
    "Session",
    "The fields a visitor's session carries between requests, held in one signed cookie.",
    "## Session\n    a user, which is Text.\n\n## Route POST /login (name: Text) -> Response:\n    Set the session's user to name.\n    Return redirectTo(\"/\").",
    "What must this visitor still be true of on their NEXT request? Only that belongs in a session — it rides in a cookie, so keep it small and never secret.",
    None,
);

static BLOCK_DIRECTIVE: ConstructDoc = lesson(
    "Directive",
    "A custom attribute: code that runs on an element when the attribute appears or changes.",
    "## Directive tooltip (el: Element) and (text: Text):\n    Set the attribute \"title\" of el to text.",
    "What should the element do when this value changes? Write it once here and use `tooltip={…}` on any element.",
    None,
);

static BLOCK_ROUTE: ConstructDoc = lesson(
    "Route",
    "A typed HTTP route: method, path with `:params`, typed inputs, and a JSON return value.",
    "## Route GET /api/todos/:id (id: Int) -> Result of Todo and HttpError:\n    Return findTodo(id).",
    "What does the client receive on success, and which status when the id is unknown? A `Result` return names both.",
    None,
);

static BLOCK_MIDDLEWARE: ConstructDoc = lesson(
    "Before each request",
    "Runs before every route; return `some(response)` to answer early or `none` to pass.",
    "## Before each request (request: Request) -> Option of Response:\n    Return none.",
    "Which requests must never reach a handler? Answer them here with `some(unauthorized())`.",
    None,
);

static BLOCK_REDIRECT: ConstructDoc = lesson(
    "Redirect",
    "Sends an old path permanently to a new one.",
    "## Redirect /old-todos -> /todos.",
    "Tip: a redirect keeps old links alive when a page moves — the browser lands on the new path.",
    None,
);

static BLOCK_ERROR_PAGE: ConstructDoc = lesson(
    "Error page",
    "The view served for an HTTP error status.",
    "## Error page 404\nView:\n    <h1>Nothing here</h1>",
    "What helps a lost visitor most — a search box, the home link, or both? This view is theirs.",
    None,
);

static EXPECT_STMT: ConstructDoc = lesson(
    "Expect",
    "A test expectation: checks a condition, records any failure, and keeps going.",
    "Expect that the balance is at least 0.",
    "Is this a test observation or a runtime guard? `Expect that` lives in tests; in ordinary code use `Require that`.",
    None,
);

static ADVANCE_STMT: ConstructDoc = lesson(
    "Advance",
    "Moves a test's virtual clock forward instantly — no real waiting.",
    "Advance time by 3 seconds.",
    "Tip: tests never wait — the clock is sealed and virtual, so a timeout test finishes in microseconds.",
    None,
);

static SKIP_STMT: ConstructDoc = lesson(
    "Skip",
    "Skips a test loudly: the reason is mandatory and every skip counts as a debt.",
    "Skip this test because \"the flag has not shipped\".",
    "Why is this test off? The reason travels with the skip, and the budget for skips only shrinks.",
    None,
);

static NOTE_STMT: ConstructDoc = lesson(
    "Note",
    "Records a developer observation — structured, replayable, elidable in release.",
    "Note that the cache missed with the key.",
    "Who reads this? `Note` is for you replaying the run; `Warn` is for the operator; `Record` is the audit trail.",
    None,
);

static WARN_STMT: ConstructDoc = lesson(
    "Warn",
    "Records an operator warning — never dropped, never sampled.",
    "Warn that the retry limit was reached with the attempts.",
    "Is this for the operator or the developer? A `Warn` always ships and always survives.",
    None,
);

static RECORD_STMT: ConstructDoc = lesson(
    "Record",
    "Records a durable audit or business event.",
    "Record that the payment settled with the amount and the customer.",
    "Does this belong in the permanent trail? `Record` is the audit surface; `Note` is ephemeral.",
    None,
);

static DURING_STMT: ConstructDoc = lesson(
    "During",
    "Opens a trace span over a block — no start/stop, so a span can never leak.",
    "During parsing the file:\n    Let ast be the parse of text.",
    "What work does this block do? The span is named by the doing, and it closes with the block.",
    None,
);

static BLOCK_NO: ConstructDoc = lesson(
    "No",
    "Turns one optimization off for the code that follows.",
    "## No Memo",
    "Tip: use it to pin a benchmark or dodge a pathological case — semantics never change.",
    None,
);

static BLOCK_TIER: ConstructDoc = lesson(
    "Tier",
    "Pins the hotness tier at which an optimization runs.",
    "## Tier Memo eager",
    "Tip: `eager`, `t1`–`t3`, or `never` — this tunes WHEN the optimizer fires, not correctness.",
    None,
);

static BLOCK_REQUIRE: ConstructDoc = lesson(
    "Require",
    "Demands an optimization fires; if it can't, the compile fails (inverse of `## No`).",
    "## Require memo",
    "Tip: depend on an optimization on purpose (e.g. `## Require closedform`) so losing it becomes a loud error, not a silent slowdown.",
    None,
);

static BLOCK_SUSPECTED_TYPO: ConstructDoc = lesson(
    "Unknown header",
    "An unknown header that looks like a typo of a real one.",
    "## Mian",
    "Did you mean the suggested header? Unknown `##` headers otherwise read as prose.",
    None,
);

// ---------------------------------------------------------------------------
// Primitive and built-in generic types
// ---------------------------------------------------------------------------

static TY_INT: ConstructDoc = lesson(
    "Int",
    "A whole number.",
    "Let n: Int be 42.",
    "Tip: `/` divides and `%` takes the remainder — `x is divisible by n` reads best in conditions.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TY_NAT: ConstructDoc = lesson(
    "Nat",
    "A whole number that can never be negative.",
    "Let count: Nat be 0.",
    "Can this value ever go below zero? If subtraction might take it there, use Int instead.",
    None,
);

static TY_TEXT: ConstructDoc = lesson(
    "Text",
    "A string of characters.",
    "Let name: Text be \"Alice\".",
    "Tip: build strings by interpolation — `\"Hello, {name}!\"` — or `a combined with b`.",
    Some("4-strings"),
);

static TY_BOOL: ConstructDoc = lesson(
    "Bool",
    "Either true or false.",
    "Let ready: Bool be true.",
    "Tip: `and`/`or`/`not` short-circuit and return Bool; 0 and empty collections count as false.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TY_FLOAT: ConstructDoc = lesson(
    "Float",
    "A floating-point number for fractional values.",
    "Let pi: Float be 3.14159.",
    "Tip: floats round — format output with `\"{pi:.2}\"`, and avoid them for money.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TY_UNIT: ConstructDoc = lesson(
    "Unit",
    "The empty value — what procedures return.",
    "## To greet (name: Text):\n    Show name.",
    "Tip: you rarely write Unit; a function without `-> Type` returns it implicitly.",
    None,
);

static TY_CHAR: ConstructDoc = lesson(
    "Char",
    "A single character.",
    "Let c: Char be 'a'.",
    "Tip: Text is a sequence of Chars — one Char is the unit you get when you walk it.",
    None,
);

static TY_BYTE: ConstructDoc = lesson(
    "Byte",
    "A single byte — a whole number from 0 to 255.",
    "Let b: Byte be 255.",
    "Tip: bytes are the unit of binary data; whole numbers past 255 need Int.",
    None,
);

static TY_LIST: ConstructDoc = lesson(
    "List",
    "An ordered, growable collection (another name for Seq).",
    "Let xs be a new List of Int.",
    "Tip: `[1, 2, 3]` is the literal form; `Push` appends; indexing is 1-based.",
    Some("5-collections"),
);

static TY_SEQ: ConstructDoc = lesson(
    "Seq",
    "An ordered, growable collection — the canonical list type.",
    "Let xs: Seq of Int be [1, 2, 3].",
    "Tip: `item 1 of xs` is the FIRST element — LOGOS indexing is 1-based.",
    Some("5-collections"),
);

static TY_MAP: ConstructDoc = lesson(
    "Map",
    "A dictionary from keys to values.",
    "Let m be a new Map of Text to Int.\nSet m at \"a\" to 1.",
    "Tip: read with `item k of m` and write with `Set m at k to v.`",
    Some("5-collections"),
);

static TY_SET: ConstructDoc = lesson(
    "Set",
    "An unordered collection holding each value once.",
    "Let s be a new Set of Int.\nAdd 3 to s.",
    "Tip: on sets `&`/`|`/`^` are intersection/union/symmetric-difference; `a without b` subtracts.",
    Some("5-collections"),
);

static TY_OPTION: ConstructDoc = lesson(
    "Option",
    "A value that is either present or absent.",
    "Let maybe be some 30.\nInspect maybe:\n    When OptionSome (v):\n        Show v.\n    When OptionNone:\n        Show \"nothing\".",
    "What happens when the value is absent? `Inspect` makes you answer both ways.",
    Some("9-options--pattern-matching"),
);

static TY_RESULT: ConstructDoc = lesson(
    "Result",
    "A success value or an error value — one or the other.",
    "## To native read (path: Text) -> Result of Text and Text",
    "Which side is which? `Result of Ok and Err` — handle both with `Inspect`.",
    None,
);

/// Every lesson, for parity ratchets and the REPL's suggestion list.
// ---------------------------------------------------------------------------
// Comparisons: one lesson per meaning, whichever way it is written. A program
// says `<` or `is less than`, `==` or `is equal to`, and the hover, completion
// and `:explain` answer the same for both (the user, 2026-11-01).
// ---------------------------------------------------------------------------

static LESS_THAN: ConstructDoc = lesson(
    "Less than",
    "True when the left value is smaller than the right, written `<` or `is less than`.",
    "If n is less than 2:\n    Show n.",
    "Should an equal pair count too? Then write `is at most` (`<=`).",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static GREATER_THAN: ConstructDoc = lesson(
    "Greater than",
    "True when the left value is larger than the right, written `>` or `is greater than`.",
    "If balance is greater than 0:\n    Show balance.",
    "Should an equal pair count too? Then write `is at least` (`>=`).",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static AT_MOST: ConstructDoc = lesson(
    "At most",
    "True when the left is smaller than or equal to the right, written `<=` or `is at most`.",
    "Require that amount is at most balance.",
    "Tip: `is less than or equal to` reads the same way, and so does `<=`.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static AT_LEAST: ConstructDoc = lesson(
    "At least",
    "True when the left is larger than or equal to the right, written `>=` or `is at least`.",
    "Require that amount is at least 1.",
    "Tip: `is greater than or equal to` reads the same way, and so does `>=`.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static EQUAL: ConstructDoc = lesson(
    "Equal",
    "True when both sides hold the same value, written `==`, `is equal to` or `equals`.",
    "If answer is equal to 42:\n    Show \"right\".",
    "A test or an assignment? `Set x to 5.` assigns; `x == 5` only compares.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static NOT_EQUAL: ConstructDoc = lesson(
    "Not equal",
    "True when the two sides differ, written `!=` or `is not equal to`.",
    "If guess is not equal to secret:\n    Show \"try again\".",
    "Tip: `not (x == y)` means the same thing, and reads worse.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

// ---------------------------------------------------------------------------
// Arithmetic and connectives, by symbol or by word (DAT-106).
// ---------------------------------------------------------------------------

static PLUS_OP: ConstructDoc = lesson(
    "Plus",
    "Adds two numbers, written `+` or `plus`.",
    "Let total be price + tax.\nSet count to count plus 1.",
    "Adding two Texts? `+` joins them end to end.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static MINUS_OP: ConstructDoc = lesson(
    "Minus",
    "Takes the right number from the left, written `-` or `minus`.",
    "Let change be paid - price.",
    "Tip: `Decrease x by 1.` says `Set x to x - 1.` in one word.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static TIMES_OP: ConstructDoc = lesson(
    "Times",
    "Multiplies two numbers, written `*` or `times`.",
    "Let area be width * height.",
    "Need a power? `**` raises: `2 ** 10` is 1024.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static DIVIDE_OP: ConstructDoc = lesson(
    "Divide",
    "Divides the left number by the right, written `/`.",
    "Let half be total / 2.",
    "Tip: `//` keeps the whole part and `%` the remainder.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static REMAINDER_OP: ConstructDoc = lesson(
    "Remainder",
    "The remainder of the left divided by the right, written `%`.",
    "If n % 2 is equal to 0:\n    Show \"even\".",
    "Checking a factor? `n is divisible by 7` reads better than `n % 7 is equal to 0`.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);


static AND: ConstructDoc = lesson(
    "And",
    "True when both sides are true, written `and`.",
    "If x is greater than 0 and x is less than 10:\n    Show x.",
    "Tip: group a mixed test with parentheses, as in `(a or b) and c`.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static OR: ConstructDoc = lesson(
    "Or",
    "True when either side is true, written `or`.",
    "If day is equal to \"Sat\" or day is equal to \"Sun\":\n    Show \"weekend\".",
    "Tip: `or` is true when both sides are true as well.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

static NOT: ConstructDoc = lesson(
    "Not",
    "Flips a truth value, written `not`.",
    "If not done:\n    Show \"still going\".",
    "Tip: `is not equal to` reads better than `not (a is equal to b)`.",
    Some("3-arithmetic-comparison-logic-bitwise"),
);

pub static ALL_DOCS: &[&ConstructDoc] = &[
    // arithmetic and connectives
    &PLUS_OP,
    &MINUS_OP,
    &TIMES_OP,
    &DIVIDE_OP,
    &REMAINDER_OP,
    &AND,
    &OR,
    &NOT,
    // comparisons
    &LESS_THAN,
    &GREATER_THAN,
    &AT_MOST,
    &AT_LEAST,
    &EQUAL,
    &NOT_EQUAL,
    // keywords
    &LET,
    &SET_KW,
    &SWAP_KW,
    &RETURN,
    &IF,
    &WHILE,
    &REPEAT,
    &SHOW,
    &GIVE,
    &PUSH,
    &INSPECT,
    &CALL,
    &NEW,
    &ESCAPE,
    &CHECK,
    &POP,
    &ADD,
    &REMOVE,
    &BREAK,
    &ASSERT,
    &TRUST,
    &INCREASE,
    &DECREASE,
    &SPAWN,
    &MERGE,
    &CONVERGE,
    &MOUNT,
    &EXPECT_STMT,
    &ADVANCE_STMT,
    &SKIP_STMT,
    &NOTE_STMT,
    &WARN_STMT,
    &RECORD_STMT,
    &DURING_STMT,
    // block headers
    &BLOCK_MAIN,
    &BLOCK_FUNCTION,
    &BLOCK_COMPONENT,
    &BLOCK_THEOREM,
    &BLOCK_PROOF,
    &BLOCK_DEFINITION,
    &BLOCK_DEFINE,
    &BLOCK_AXIOM,
    &BLOCK_THEORY,
    &BLOCK_TYPEDEF,
    &BLOCK_POLICY,
    &BLOCK_LOGIC,
    &BLOCK_EXAMPLE,
    &BLOCK_NOTE,
    &BLOCK_REQUIRES,
    &BLOCK_HARDWARE,
    &BLOCK_PROPERTY,
    &BLOCK_NO,
    &BLOCK_TIER,
    &BLOCK_REQUIRE,
    &BLOCK_SUSPECTED_TYPO,
    &BLOCK_TEST,
    &BLOCK_PAGE,
    &BLOCK_STORE,
    &BLOCK_SESSION,
    &BLOCK_DIRECTIVE,
    &BLOCK_ROUTE,
    &BLOCK_MIDDLEWARE,
    &BLOCK_REDIRECT,
    &BLOCK_ERROR_PAGE,
    // types
    &TY_INT,
    &TY_NAT,
    &TY_TEXT,
    &TY_BOOL,
    &TY_FLOAT,
    &TY_UNIT,
    &TY_CHAR,
    &TY_BYTE,
    &TY_LIST,
    &TY_SEQ,
    &TY_MAP,
    &TY_SET,
    &TY_OPTION,
    &TY_RESULT,
];

/// The lesson for a statement keyword, when the token deserves one.
///
/// Wildcard-free: a new `TokenType` variant does not compile until someone
/// decides whether it teaches (a lesson arm) or not (the grouped `None` arm).
pub fn doc_for(kind: &TokenType) -> Option<&'static ConstructDoc> {
    match kind {
        // Comparisons, by symbol or by the English the lexer folds into one token.
        TokenType::Lt => Some(&LESS_THAN),
        TokenType::Gt => Some(&GREATER_THAN),
        TokenType::LtEq => Some(&AT_MOST),
        TokenType::GtEq => Some(&AT_LEAST),
        TokenType::EqEq | TokenType::Identity | TokenType::Equals => Some(&EQUAL),
        TokenType::NotEq => Some(&NOT_EQUAL),
        TokenType::Plus => Some(&PLUS_OP),
        TokenType::Minus => Some(&MINUS_OP),
        TokenType::Star => Some(&TIMES_OP),
        TokenType::Slash => Some(&DIVIDE_OP),
        TokenType::Percent => Some(&REMAINDER_OP),
        TokenType::And => Some(&AND),
        TokenType::Or => Some(&OR),
        TokenType::Not => Some(&NOT),
        TokenType::Let => Some(&LET),
        TokenType::Set => Some(&SET_KW),
        TokenType::Swap => Some(&SWAP_KW),
        TokenType::Return => Some(&RETURN),
        TokenType::If => Some(&IF),
        TokenType::While => Some(&WHILE),
        TokenType::Repeat => Some(&REPEAT),
        TokenType::Show => Some(&SHOW),
        TokenType::Give => Some(&GIVE),
        TokenType::Push => Some(&PUSH),
        TokenType::Inspect => Some(&INSPECT),
        TokenType::Call => Some(&CALL),
        TokenType::New => Some(&NEW),
        TokenType::Escape => Some(&ESCAPE),
        TokenType::Expect => Some(&EXPECT_STMT),
        TokenType::Advance => Some(&ADVANCE_STMT),
        TokenType::Skip => Some(&SKIP_STMT),
        TokenType::Note => Some(&NOTE_STMT),
        TokenType::Warn => Some(&WARN_STMT),
        TokenType::Record => Some(&RECORD_STMT),
        TokenType::During => Some(&DURING_STMT),
        TokenType::Check => Some(&CHECK),
        TokenType::Pop => Some(&POP),
        TokenType::Add => Some(&ADD),
        TokenType::Remove => Some(&REMOVE),
        TokenType::Break => Some(&BREAK),
        TokenType::Assert => Some(&ASSERT),
        TokenType::Trust => Some(&TRUST),
        TokenType::Increase => Some(&INCREASE),
        TokenType::Decrease => Some(&DECREASE),
        TokenType::Spawn => Some(&SPAWN),
        TokenType::Mount => Some(&MOUNT),
        TokenType::Merge => Some(&MERGE),
        TokenType::Converge => Some(&CONVERGE),

        // No lesson (yet): block headers teach through `doc_for_block`;
        // everything else is either structural, symbolic, or a word whose
        // meaning the sentence around it carries.
        TokenType::BlockHeader { .. }
        | TokenType::All
        | TokenType::No
        | TokenType::Some
        | TokenType::Any
        | TokenType::Both
        | TokenType::Most
        | TokenType::Few
        | TokenType::Many
        | TokenType::Cardinal(_)
        | TokenType::AtLeast(_)
        | TokenType::AtMost(_)
        | TokenType::InfinitelyMany
        | TokenType::FinitelyMany
        | TokenType::AllButFinitelyMany
        | TokenType::InfinitelyOften
        | TokenType::Anything
        | TokenType::Anyone
        | TokenType::Nothing
        | TokenType::Nobody
        | TokenType::NoOne
        | TokenType::Nowhere
        | TokenType::Ever
        | TokenType::Never
        | TokenType::Then
        | TokenType::Iff
        | TokenType::Because
        | TokenType::Although
        | TokenType::Until
        | TokenType::Release
        | TokenType::WeakUntil
        | TokenType::Implies
        | TokenType::Must
        | TokenType::Shall
        | TokenType::Should
        | TokenType::Can
        | TokenType::May
        | TokenType::Cannot
        | TokenType::Would
        | TokenType::Could
        | TokenType::Might
        | TokenType::Had
        | TokenType::Be
        | TokenType::For
        | TokenType::In
        | TokenType::From
        | TokenType::Require
        | TokenType::Requires
        | TokenType::Ensures
        | TokenType::Otherwise
        | TokenType::Else
        | TokenType::Elif
        | TokenType::Either
        | TokenType::Native
        | TokenType::EscapeBlock(_)
        | TokenType::RawBlock { .. }
        | TokenType::Given
        | TokenType::Prove
        | TokenType::Auto
        | TokenType::Read
        | TokenType::Write
        | TokenType::Console
        | TokenType::File
        | TokenType::Copy
        | TokenType::Through
        | TokenType::Length
        | TokenType::At
        | TokenType::Contains
        | TokenType::Union
        | TokenType::Intersection
        | TokenType::Inside
        | TokenType::Zone
        | TokenType::Called
        | TokenType::Size
        | TokenType::Mapped
        | TokenType::Attempt
        | TokenType::Following
        | TokenType::Simultaneously
        | TokenType::Send
        | TokenType::Await
        | TokenType::Portable
        | TokenType::Manifest
        | TokenType::Chunk
        | TokenType::Shared
        | TokenType::Tally
        | TokenType::SharedSet
        | TokenType::SharedSequence
        | TokenType::CollaborativeSequence
        | TokenType::SharedMap
        | TokenType::Divergent
        | TokenType::Append
        | TokenType::Resolve
        | TokenType::RemoveWins
        | TokenType::AddWins
        | TokenType::YATA
        | TokenType::Values
        | TokenType::Listen
        | TokenType::NetConnect
        | TokenType::Sleep
        | TokenType::Sync
        | TokenType::Persistent
        | TokenType::Combined
        | TokenType::Followed
        | TokenType::Launch
        | TokenType::Task
        | TokenType::Pipe
        | TokenType::Receive
        | TokenType::Stop
        | TokenType::Try
        | TokenType::Into
        | TokenType::First
        | TokenType::After
        | TokenType::Colon
        | TokenType::Indent
        | TokenType::Dedent
        | TokenType::Newline
        | TokenType::Noun(_)
        | TokenType::Adjective(_)
        | TokenType::NonIntersectiveAdjective(_)
        | TokenType::Adverb(_)
        | TokenType::ScopalAdverb(_)
        | TokenType::TemporalAdverb(_)
        | TokenType::Verb { .. }
        | TokenType::ProperName(_)
        | TokenType::Ambiguous { .. }
        | TokenType::Performative(_)
        | TokenType::Exclamation
        | TokenType::Article(_)
        | TokenType::Auxiliary(_)
        | TokenType::Is
        | TokenType::Are
        | TokenType::Was
        | TokenType::Were
        | TokenType::That
        | TokenType::Who
        | TokenType::What
        | TokenType::Where
        | TokenType::Whose
        | TokenType::When
        | TokenType::Why
        | TokenType::Does
        | TokenType::Do
        | TokenType::Reflexive
        | TokenType::Reciprocal
        | TokenType::Respectively
        | TokenType::Pronoun { .. }
        | TokenType::Preposition(_)
        | TokenType::Particle(_)
        | TokenType::Comparative(_)
        | TokenType::Superlative(_)
        | TokenType::Than
        | TokenType::To
        | TokenType::PresupTrigger(_)
        | TokenType::Focus(_)
        | TokenType::Measure(_)
        | TokenType::Number(_)
        | TokenType::MoneyLiteral { .. }
        | TokenType::DurationLiteral { .. }
        | TokenType::DateLiteral { .. }
        | TokenType::TimeLiteral { .. }
        | TokenType::CalendarUnit(_)
        | TokenType::Ago
        | TokenType::Hence
        | TokenType::Before
        | TokenType::StringLiteral(_)
        | TokenType::InterpolatedString(_)
        | TokenType::CharLiteral(_)
        | TokenType::Item
        | TokenType::Items
        | TokenType::Possessive
        | TokenType::LParen
        | TokenType::RParen
        | TokenType::LBracket
        | TokenType::RBracket
        | TokenType::LBrace
        | TokenType::Amp
        | TokenType::VBar
        | TokenType::Tilde
        | TokenType::Caret
        | TokenType::RBrace
        | TokenType::Comma
        | TokenType::Period
        | TokenType::Dot
        | TokenType::Xor
        | TokenType::Shifted
        | TokenType::PlusEq
        | TokenType::MinusEq
        | TokenType::StarEq
        | TokenType::SlashEq
        | TokenType::PercentEq
        | TokenType::StarStar
        | TokenType::SlashSlash
        | TokenType::Arrow
        | TokenType::Assign
        | TokenType::Mut
        | TokenType::Identifier
        | TokenType::EOF => None,
    }
}

/// The lesson for a `##` block header. TOTAL — every block type teaches, and
/// a new `BlockType` variant does not compile until it gets a lesson.
pub fn doc_for_block(block: &BlockType) -> &'static ConstructDoc {
    match block {
        BlockType::SuspectedTypo { .. } => &BLOCK_SUSPECTED_TYPO,
        BlockType::Theorem => &BLOCK_THEOREM,
        BlockType::Main => &BLOCK_MAIN,
        BlockType::Definition => &BLOCK_DEFINITION,
        BlockType::Define => &BLOCK_DEFINE,
        BlockType::Axiom => &BLOCK_AXIOM,
        BlockType::Theory => &BLOCK_THEORY,
        BlockType::Proof => &BLOCK_PROOF,
        BlockType::Example => &BLOCK_EXAMPLE,
        BlockType::Logic => &BLOCK_LOGIC,
        BlockType::Note => &BLOCK_NOTE,
        BlockType::Function => &BLOCK_FUNCTION,
        BlockType::Component => &BLOCK_COMPONENT,
        BlockType::TypeDef => &BLOCK_TYPEDEF,
        BlockType::Policy => &BLOCK_POLICY,
        BlockType::Requires => &BLOCK_REQUIRES,
        BlockType::Hardware => &BLOCK_HARDWARE,
        BlockType::Property => &BLOCK_PROPERTY,
        BlockType::No => &BLOCK_NO,
        BlockType::Tier => &BLOCK_TIER,
        BlockType::Require => &BLOCK_REQUIRE,
        BlockType::Test { .. } => &BLOCK_TEST,
        BlockType::Page { .. } => &BLOCK_PAGE,
        BlockType::Store => &BLOCK_STORE,
        BlockType::Session => &BLOCK_SESSION,
        BlockType::Directive => &BLOCK_DIRECTIVE,
        BlockType::Route { .. } => &BLOCK_ROUTE,
        BlockType::Middleware => &BLOCK_MIDDLEWARE,
        BlockType::Redirect { .. } => &BLOCK_REDIRECT,
        BlockType::ErrorPage { .. } => &BLOCK_ERROR_PAGE,
    }
}

/// The lesson for a primitive or built-in generic type name.
pub fn doc_for_primitive(name: &str) -> Option<&'static ConstructDoc> {
    match name {
        "Int" => Some(&TY_INT),
        "Nat" => Some(&TY_NAT),
        "Text" => Some(&TY_TEXT),
        "Bool" => Some(&TY_BOOL),
        "Float" => Some(&TY_FLOAT),
        "Unit" => Some(&TY_UNIT),
        "Char" => Some(&TY_CHAR),
        "Byte" => Some(&TY_BYTE),
        "List" => Some(&TY_LIST),
        "Seq" => Some(&TY_SEQ),
        "Map" => Some(&TY_MAP),
        "Set" => Some(&TY_SET),
        "Option" => Some(&TY_OPTION),
        "Result" => Some(&TY_RESULT),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Literate-doc extraction — the prose → documentation pipeline
// ---------------------------------------------------------------------------

/// One documented definition pulled from a literate LOGOS module.
pub struct LiterateDoc {
    /// The defined name (`read`, `Point`).
    pub name: String,
    /// The full `##` header line, verbatim.
    pub signature: String,
    /// The body of the `## Note` block directly above the header, when one
    /// exists. Notes are the ONLY per-definition doc carrier — bare prose
    /// between sections is not literate LOGOS.
    pub doc: Option<String>,
}

/// The module-level documentation: the prose between the `# Title` line and
/// the first `##` section (the region the lexer skips).
pub fn module_doc(source: &str) -> Option<String> {
    let first_section = if source.starts_with("## ") {
        0
    } else {
        source.find("\n## ").map(|i| i + 1).unwrap_or(source.len())
    };
    let prose: Vec<&str> = source[..first_section]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    (!prose.is_empty()).then(|| prose.join("\n"))
}

/// The documentation for the `##` header starting at `header_start`: the body
/// of a `## Note` block IMMEDIATELY above it (the nearest preceding header
/// must be the Note — any other header in between means no doc).
pub fn doc_for_header_at(source: &str, header_start: usize) -> Option<String> {
    let head = source.get(..header_start)?;
    let mut nearest_header: Option<usize> = None;
    let mut offset = 0;
    for line in head.split_inclusive('\n') {
        if line.trim_start().starts_with("## ") {
            nearest_header = Some(offset);
        }
        offset += line.len();
    }
    let note_start = nearest_header?;
    let note_line_end = note_start + head[note_start..].find('\n')?;
    if head[note_start..note_line_end].trim() != "## Note" {
        return None;
    }
    let body = head[note_line_end + 1..].trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// Every `## To …` / `## A … ` definition in a literate module, with its
/// `## Note` documentation attached where one sits directly above. Types
/// declared inside a `## Definition` body inherit the block's Note.
pub fn extract_literate_docs(source: &str) -> Vec<LiterateDoc> {
    let mut docs = Vec::new();
    let mut offset = 0;
    let mut definition_doc: Option<Option<String>> = None;
    for line in source.split_inclusive('\n') {
        let header = line.trim_end().trim_start();
        if header.starts_with("## ") && header != "## Note" {
            // `## Note` is an attachment to the NEXT definition, not a section
            // change — it must not end the `## Definition` block it sits in.
            definition_doc = (header == "## Definition").then(|| doc_for_header_at(source, offset));
        }
        if let Some(name) = literate_definition_name(header) {
            docs.push(LiterateDoc {
                name,
                signature: header.to_string(),
                doc: doc_for_header_at(source, offset),
            });
        } else if let Some(block_doc) = &definition_doc {
            if let Some(name) = definition_body_type_name(header) {
                docs.push(LiterateDoc {
                    name,
                    signature: header.to_string(),
                    // A note directly above the declaration wins; the block's
                    // own doc is the fallback.
                    doc: doc_for_header_at(source, offset).or_else(|| block_doc.clone()),
                });
            }
        }
        offset += line.len();
    }
    docs
}

/// The type a `## Definition` BODY line declares: `A <Name> has …` /
/// `A <Name> is …` (mirrors the loader's type-header rule).
fn definition_body_type_name(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("A ")
        .or_else(|| line.strip_prefix("An "))?;
    let word = rest
        .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
        .next()?;
    if !word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let tail = rest[word.len()..].trim_start();
    (tail.starts_with("has") || tail.starts_with("is") || tail.starts_with("of"))
        .then(|| word.to_string())
}

/// The name a `##` definition header defines, mirroring the loader's
/// trigger-name rules: `## To [native|private]… <name>` and `## A/An [private] <TypeName> …`.
/// Modifiers come in any order; a parameterless header's trailing `:` is not part of the name.
fn literate_definition_name(header: &str) -> Option<String> {
    if let Some(mut rest) = header.strip_prefix("## To ") {
        while let Some(r) = rest
            .strip_prefix("native ")
            .or_else(|| rest.strip_prefix("private "))
        {
            rest = r;
        }
        let name = rest
            .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
            .next()?;
        return (!name.is_empty()).then(|| name.to_string());
    }
    let rest = header
        .strip_prefix("## A ")
        .or_else(|| header.strip_prefix("## An "))?;
    let rest = rest.strip_prefix("private ").unwrap_or(rest);
    let word = rest
        .split(|c: char| c == '(' || c == '.' || c == ':' || c.is_whitespace())
        .next()?;
    word.chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
        .then(|| word.to_string())
}

/// Every lesson whose name matches the word, case-insensitively — a word can
/// name more than one construct (`Set` the statement, `Set` the type), and an
/// honest teacher shows both.
pub fn docs_for_word(word: &str) -> Vec<&'static ConstructDoc> {
    ALL_DOCS
        .iter()
        .filter(|doc| doc.name.eq_ignore_ascii_case(word))
        .copied()
        .collect()
}

/// The first lesson whose name matches the word, case-insensitively.
pub fn doc_for_word(word: &str) -> Option<&'static ConstructDoc> {
    docs_for_word(word).into_iter().next()
}

/// The comparison a run of English tokens spells, read around the token at `at`: `is less than`,
/// `greater than or equal to`, `at least`, `is equal to`, `not equal to`, `equals`. The lexer
/// hands most of these to the parser one word at a time (`less` is a comparative, `than` is
/// `than`), so no single token carries the meaning; this reads the words the parser reads. Gives
/// the lesson and the span of the whole phrase, for a hover to cover and an editor to colour.
pub fn doc_for_phrase(source: &str, tokens: &[crate::token::Token], at: usize) -> Option<(&'static ConstructDoc, logicaffeine_base::Span)> {
    use logicaffeine_base::Span;
    let token = tokens.get(at)?;
    // The words of the tokens around `at`, each with the token it came from (a token can hold
    // several words: `is equal to` is one token, `at least three` another).
    let lo = at.saturating_sub(5);
    let hi = (at + 6).min(tokens.len());
    let mut words: Vec<(String, usize)> = Vec::new();
    for (i, t) in tokens.iter().enumerate().take(hi).skip(lo) {
        let text = source.get(t.span.start..t.span.end).unwrap_or("");
        for w in text.split_whitespace() {
            words.push((w.to_ascii_lowercase(), i));
        }
    }
    const PHRASES: &[(&[&str], &ConstructDoc)] = &[
        (&["less", "than", "or", "equal", "to"], &AT_MOST),
        (&["greater", "than", "or", "equal", "to"], &AT_LEAST),
        (&["not", "equal", "to"], &NOT_EQUAL),
        (&["is", "equal", "to"], &EQUAL),
        (&["is", "identical", "to"], &EQUAL),
        (&["less", "than"], &LESS_THAN),
        (&["fewer", "than"], &LESS_THAN),
        (&["greater", "than"], &GREATER_THAN),
        (&["at", "least"], &AT_LEAST),
        (&["at", "most"], &AT_MOST),
        (&["equal", "to"], &EQUAL),
        (&["equals"], &EQUAL),
        (&["divisible", "by"], &REMAINDER_OP),
    ];
    // `plus`, `minus` and `times` are operators only between two operands (`Repeat 3 times:` counts
    // repetitions); the parser reads them the same way, as `check_op_word` does.
    const ARITHMETIC_WORDS: &[(&str, &ConstructDoc)] = &[("plus", &PLUS_OP), ("minus", &MINUS_OP), ("times", &TIMES_OP)];
    let operand = |t: &crate::token::Token| {
        matches!(
            t.kind,
            TokenType::Number(_)
                | TokenType::Identifier
                | TokenType::ProperName(_)
                | TokenType::Noun(_)
                | TokenType::RParen
                | TokenType::LParen
                | TokenType::StringLiteral(_)
        )
    };
    let text = source.get(token.span.start..token.span.end).unwrap_or("").to_ascii_lowercase();
    if let Some((_, doc)) = ARITHMETIC_WORDS.iter().find(|(w, _)| *w == text) {
        let before = at.checked_sub(1).and_then(|i| tokens.get(i)).is_some_and(operand);
        let after = tokens.get(at + 1).is_some_and(operand);
        if before && after {
            return Some((doc, Span::new(token.span.start, token.span.end)));
        }
    }
    // Longest phrase first, so `less than or equal to` is not read as `less than`.
    for (phrase, doc) in PHRASES {
        for start in 0..words.len() {
            let end = start + phrase.len();
            if end > words.len() {
                break;
            }
            if words[start..end].iter().zip(phrase.iter()).all(|((w, _), p)| w == p) {
                let covers = words[start..end].iter().any(|(_, i)| *i == at);
                // `equal to` inside `not equal to` is the longer phrase, found first; a bare
                // `equal to` preceded by `not` never gets here.
                if covers {
                    let first = words[start].1;
                    let last = words[end - 1].1;
                    return Some((doc, Span::new(tokens[first].span.start, tokens[last].span.end)));
                }
            }
        }
    }
    None
}
