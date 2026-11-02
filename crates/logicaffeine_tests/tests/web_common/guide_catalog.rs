//! The shared guide-content catalog: one construct table, one set of section
//! class lists, consumed by BOTH lock batteries (`guide_coverage.rs` for
//! structure, `guide_pedagogy.rs` for teaching craft). One catalog means the
//! two batteries can never disagree about what is taught where.
//!
//! Included via `#[path = "common/guide_catalog.rs"]` — files under
//! `tests/common/` are not test binaries themselves. Each battery reads the
//! part of the catalog it checks, so the rest is unused in that binary.

#![allow(dead_code)]

/// One teachable language construct: the section that teaches it and probe
/// strings, at least one of which must literally appear in that section's
/// prose or example code. The catalog is the standing RED for content waves —
/// add entries first, write the section until they pass.
pub struct Construct {
    pub feature: &'static str,
    pub section: &'static str,
    pub probes: &'static [&'static str],
    /// Usage DETECTORS for the dependency-order lock (P10): distinctive
    /// surface strings scanned over example/exercise CODE (string literals
    /// stripped, boundary-checked). Opt-in — only constructs with a reliable
    /// keyword surface carry detectors (min length 4; never a bare English
    /// word); a construct with `&[]` is honestly outside the ordering lock.
    /// When two catalog entries share a surface (e.g. `copy of`), the
    /// detector lives on the EARLIEST teaching home only.
    pub detectors: &'static [&'static str],
}

/// Sections that legitimately carry no construct entries (narrative or
/// mirror surfaces, not primary teaching homes).
pub const META_SECTIONS: &[&str] = &["introduction", "quick-reference"];

/// Sections that are navigational/reference/montage surfaces rather than
/// concept-teaching surfaces: exempt from the hook requirement (a reference
/// table needs no headache) — NOT exempt from objectives.
pub const NON_TEACHING: &[&str] = &[
    "introduction",
    "getting-started",
    "cli-largo",
    "quick-reference",
    "appendix-keywords",
    "appendix-errors",
    "appendix-runnability",
];

/// End-of-part cumulative Checkpoint sections (Parts I–VI). Append-only once
/// created; each entry is locked by `checkpoints_interleave_earlier_concepts`
/// in guide_pedagogy.rs. Empty until wave W3 lands the first one.
pub const CHECKPOINT_SECTIONS: &[&str] = &[
    "part-1-checkpoint",
    "part-2-checkpoint",
    "part-3-checkpoint",
    "part-4-checkpoint",
    "part-5-checkpoint",
    "part-6-checkpoint",
];

pub const LANGUAGE_SURFACE: &[Construct] = &[
    // ── How LOGOS reads English (the mental-model section) ─────────────
    Construct {
        feature: "English/symbolic twins",
        section: "reading-english",
        probes: &["2 plus 3"],
        detectors: &[],
    },
    Construct {
        feature: "1-based inclusive indexing exception",
        section: "reading-english",
        probes: &["items 1 through 2"],
        detectors: &[],
    },
    Construct {
        feature: "equals-not-is rule",
        section: "reading-english",
        probes: &["equals"],
        detectors: &[],
    },
    // ── Getting started ─────────────────────────────────────────────────
    Construct {
        feature: "one-line installer",
        section: "getting-started",
        probes: &["install.sh"],
        detectors: &[],
    },
    Construct {
        feature: "hello world",
        section: "getting-started",
        probes: &["Hello, World!"],
        detectors: &[],
    },
    // ── Tutorial ─────────────────────────────────────────────────────────
    Construct {
        feature: "staged build-one-program tutorial",
        section: "tutorial-project",
        probes: &["(low + high) // 2"],
        detectors: &[],
    },
    // ── Practice ─────────────────────────────────────────────────────────
    Construct {
        feature: "narrated complete programs",
        section: "complete-examples",
        probes: &["counts[w] + 1"],
        detectors: &[],
    },
    Construct {
        feature: "goal-indexed recipes",
        section: "recipes",
        probes: &["clamp(15, 1, 10)"],
        detectors: &[],
    },
    // ── Variables and types ─────────────────────────────────────────────
    Construct {
        feature: "Let binding",
        section: "variables-and-types",
        probes: &["Let x be 5"],
        detectors: &[],
    },
    Construct {
        feature: "Set mutation",
        section: "variables-and-types",
        probes: &["Set x to 10"],
        detectors: &[],
    },
    Construct {
        feature: "type annotation",
        section: "variables-and-types",
        probes: &[": Byte be"],
        detectors: &[],
    },
    Construct {
        feature: "Real decimals + the Float alias",
        section: "variables-and-types",
        probes: &["`Float` is another name for exactly the same type"],
        detectors: &[],
    },
    // ── Text ─────────────────────────────────────────────────────────────
    Construct {
        feature: "string interpolation",
        section: "text",
        probes: &["Hello, {name}!"],
        detectors: &[],
    },
    Construct {
        feature: "format specs",
        section: "text",
        probes: &["{pi:.2}"],
        detectors: &[],
    },
    Construct {
        feature: "debug interpolation",
        section: "text",
        probes: &["{v=}"],
        detectors: &[],
    },
    Construct {
        feature: "followed by (seq concat)",
        section: "text",
        probes: &["followed by"],
        detectors: &["followed by"],
    },
    Construct {
        feature: "Char literals",
        section: "text",
        probes: &["`\\n`"],
        detectors: &[],
    },
    // ── Closures ─────────────────────────────────────────────────────────
    Construct {
        feature: "expression closures",
        section: "closures",
        probes: &["(x: Int) -> x + 1"],
        detectors: &[],
    },
    Construct {
        feature: "block closures",
        section: "closures",
        probes: &["->:"],
        detectors: &["->:"],
    },
    Construct {
        feature: "function-typed parameters",
        section: "closures",
        probes: &["fn(Int) -> Int"],
        detectors: &["fn("],
    },
    // ── Time, Money, and Measures ────────────────────────────────────────
    Construct {
        feature: "Duration literals",
        section: "variables-and-types",
        probes: &["500ms"],
        detectors: &[],
    },
    Construct {
        feature: "Date + calendar extractors",
        section: "variables-and-types",
        probes: &["year_of("],
        detectors: &[],
    },
    Construct {
        feature: "Time and Span literals",
        section: "variables-and-types",
        probes: &["2 weeks"],
        detectors: &[],
    },
    Construct {
        feature: "Money exactness",
        section: "variables-and-types",
        probes: &["$19.99"],
        detectors: &[],
    },
    Construct {
        feature: "Quantity dimensions",
        section: "variables-and-types",
        probes: &["quantity(3, \"meter\")"],
        detectors: &[],
    },
    Construct {
        feature: "UUID",
        section: "variables-and-types",
        probes: &["uuid_version("],
        detectors: &[],
    },
    Construct {
        feature: "Sleep",
        section: "variables-and-types",
        probes: &["Sleep 2ms"],
        detectors: &["Sleep "],
    },
    // ── Numbers (the numeric tower) ─────────────────────────────────────
    Construct {
        feature: "Nat via annotation",
        section: "variables-and-types",
        probes: &["Let count: Nat be"],
        detectors: &[],
    },
    Construct {
        feature: "Rational exact arithmetic",
        section: "variables-and-types",
        probes: &["35/6"],
        detectors: &[],
    },
    Construct {
        feature: "BigInt auto-promotion",
        section: "variables-and-types",
        probes: &["9223372036854775807 + 1"],
        detectors: &[],
    },
    Construct {
        feature: "Decimal exactness",
        section: "variables-and-types",
        probes: &["decimal(\"0.1\")"],
        detectors: &[],
    },
    Construct {
        feature: "Complex numbers",
        section: "variables-and-types",
        probes: &["complex(0, 1)"],
        detectors: &[],
    },
    Construct {
        feature: "Float/Real alias",
        section: "variables-and-types",
        probes: &["another name for exactly the same type"],
        detectors: &[],
    },
    Construct {
        feature: "math builtins",
        section: "variables-and-types",
        probes: &["sqrt("],
        detectors: &[],
    },
    // ── Operators ───────────────────────────────────────────────────────
    Construct {
        feature: "English arithmetic",
        section: "operators",
        probes: &["divided by"],
        detectors: &[],
    },
    Construct {
        feature: "English comparisons",
        section: "operators",
        probes: &["is at most"],
        detectors: &[],
    },
    Construct {
        feature: "logical and/or/not",
        section: "operators",
        probes: &["Show not a"],
        detectors: &[],
    },
    Construct {
        feature: "chained comparisons",
        section: "operators",
        probes: &["1 <= x <= 10"],
        detectors: &[],
    },
    Construct {
        feature: "number predicates",
        section: "operators",
        probes: &["is divisible by"],
        detectors: &[],
    },
    Construct {
        feature: "floor division + power",
        section: "operators",
        probes: &["**"],
        detectors: &[],
    },
    Construct {
        feature: "approximate equality",
        section: "operators",
        probes: &["is approximately"],
        detectors: &[],
    },
    // ── Machine words ───────────────────────────────────────────────────
    Construct {
        feature: "wrapping word arithmetic",
        section: "machine-words",
        probes: &["word32(4294967295)"],
        detectors: &["word32(", "word64("],
    },
    Construct {
        feature: "word bitwise + rotate",
        section: "machine-words",
        probes: &["rotl("],
        detectors: &["rotl("],
    },
    Construct {
        feature: "integer shifts + popcount",
        section: "machine-words",
        probes: &["count_ones("],
        detectors: &[
            "shifted left by",
            "shifted right by",
            "count_ones(",
            "intOfWord64(",
        ],
    },
    Construct {
        feature: "xorshift PRNG",
        section: "machine-words",
        probes: &["shifted left by 13"],
        detectors: &[],
    },
    // ── Control flow ────────────────────────────────────────────────────
    Construct {
        feature: "If/Otherwise",
        section: "control-flow",
        probes: &["Otherwise:"],
        detectors: &["If ", "Otherwise"],
    },
    Construct {
        feature: "counted Repeat",
        section: "control-flow",
        probes: &["Repeat for i from 1 to 5"],
        detectors: &[],
    },
    Construct {
        feature: "Break",
        section: "control-flow",
        probes: &["Break."],
        detectors: &["Break"],
    },
    Construct {
        feature: "While loop",
        section: "control-flow",
        probes: &["While count is at most 5"],
        detectors: &["While "],
    },
    Construct {
        feature: "Repeat for-each",
        section: "control-flow",
        probes: &["Repeat for n in numbers"],
        detectors: &["Repeat for"],
    },
    // ── Functions ───────────────────────────────────────────────────────
    Construct {
        feature: "function definition",
        section: "functions",
        probes: &["## To greet"],
        detectors: &["## To "],
    },
    Construct {
        feature: "Return",
        section: "functions",
        probes: &["Return a + b"],
        detectors: &["Return "],
    },
    Construct {
        feature: "recursion",
        section: "functions",
        probes: &["factorial(n - 1)"],
        detectors: &[],
    },
    // ── Collections ─────────────────────────────────────────────────────
    Construct {
        feature: "list literal",
        section: "collections",
        probes: &["[1, 2, 3, 4, 5]"],
        detectors: &[],
    },
    Construct {
        feature: "1-based item access",
        section: "collections",
        probes: &["item 1 of"],
        detectors: &["item "],
    },
    Construct {
        feature: "Push and Pop",
        section: "collections",
        probes: &["Pop from numbers into last"],
        detectors: &["Push ", "Pop "],
    },
    Construct {
        feature: "inclusive slicing",
        section: "collections",
        probes: &["items 2 through 3 of"],
        detectors: &[" through "],
    },
    Construct {
        feature: "maps",
        section: "collections",
        probes: &["a new Map of Text to Int"],
        detectors: &["Map of"],
    },
    Construct {
        feature: "sets",
        section: "collections",
        probes: &["a new Set of Int"],
        detectors: &["Set of", "Add ", "Remove "],
    },
    Construct {
        feature: "set union/intersection",
        section: "collections",
        probes: &["a intersection b"],
        detectors: &["union", "intersection"],
    },
    Construct {
        feature: "tuples",
        section: "collections",
        probes: &["(10, 20)"],
        detectors: &[],
    },
    Construct {
        feature: "bracket indexing",
        section: "collections",
        probes: &["items[1]"],
        detectors: &[],
    },
    Construct {
        feature: "typed empty lists + capacity",
        section: "collections",
        probes: &["[] of Int"],
        detectors: &["] of ", "with capacity"],
    },
    Construct {
        feature: "index assignment",
        section: "collections",
        probes: &["Set item 2 of"],
        detectors: &[],
    },
    Construct {
        feature: "map destructuring iteration",
        section: "collections",
        probes: &["Repeat for (name, age) in"],
        detectors: &[],
    },
    Construct {
        feature: "aliasing vs copy of",
        section: "collections",
        probes: &["copy of a"],
        detectors: &["copy of"],
    },
    // ── User-defined types ──────────────────────────────────────────────
    Construct {
        feature: "struct definition",
        section: "user-types",
        probes: &["A Point has:"],
        detectors: &[" has:"],
    },
    Construct {
        feature: "struct construction",
        section: "user-types",
        probes: &["a new Point with x 10 and y 20"],
        detectors: &[],
    },
    Construct {
        feature: "possessive field access",
        section: "user-types",
        probes: &["p's x"],
        detectors: &["'s "],
    },
    Construct {
        feature: "enum definition",
        section: "user-types",
        probes: &["A Direction is either:"],
        detectors: &["is either:"],
    },
    Construct {
        feature: "Inspect/When pattern matching",
        section: "user-types",
        probes: &["Inspect heading:"],
        detectors: &["Inspect ", "When "],
    },
    // ── Generics ────────────────────────────────────────────────────────
    Construct {
        feature: "generic types",
        section: "generics",
        probes: &["A Box of [T] has:"],
        detectors: &["of ["],
    },
    // ── Ownership ───────────────────────────────────────────────────────
    Construct {
        feature: "Give (move)",
        section: "memory-ownership",
        probes: &["Give message to consume"],
        detectors: &["Give "],
    },
    Construct {
        feature: "Show-to (borrow)",
        section: "memory-ownership",
        probes: &["Show profile to display"],
        detectors: &[],
    },
    Construct {
        feature: "copy of",
        section: "memory-ownership",
        probes: &["copy of original"],
        detectors: &[],
    },
    // ── Zones ───────────────────────────────────────────────────────────
    Construct {
        feature: "zone blocks",
        section: "zones",
        probes: &["Inside a zone called"],
        detectors: &["Inside a zone called"],
    },
    Construct {
        feature: "sized zones",
        section: "zones",
        probes: &["of size 2 MB"],
        detectors: &[],
    },
    Construct {
        feature: "memory-mapped zones",
        section: "zones",
        probes: &["mapped from"],
        detectors: &[],
    },
    // ── Concurrency ─────────────────────────────────────────────────────
    Construct {
        feature: "Attempt all (async join)",
        section: "concurrency",
        probes: &["Attempt all of the following:"],
        detectors: &["Attempt all"],
    },
    Construct {
        feature: "Simultaneously (parallel)",
        section: "concurrency",
        probes: &["Simultaneously:"],
        detectors: &["Simultaneously:"],
    },
    Construct {
        feature: "Launch a task",
        section: "concurrency",
        probes: &["Launch a task to worker(1)"],
        detectors: &["Launch a task"],
    },
    Construct {
        feature: "Stop a task handle",
        section: "concurrency",
        probes: &["Stop job."],
        detectors: &["Stop "],
    },
    Construct {
        feature: "pipes (channels)",
        section: "concurrency",
        probes: &["a new Pipe of Int"],
        detectors: &["Pipe of"],
    },
    Construct {
        feature: "Send/Receive",
        section: "concurrency",
        probes: &["Receive x from messages"],
        detectors: &["Receive "],
    },
    Construct {
        feature: "non-blocking try send/receive",
        section: "concurrency",
        probes: &["Try to send"],
        detectors: &["Try to send", "Try to receive"],
    },
    Construct {
        feature: "select (Await the first of)",
        section: "concurrency",
        probes: &["Await the first of:"],
        detectors: &["Await the first of"],
    },
    // ── CRDTs ───────────────────────────────────────────────────────────
    Construct {
        feature: "Shared structs",
        section: "crdt",
        probes: &["is Shared and has:"],
        detectors: &["is Shared"],
    },
    Construct {
        feature: "ConvergentCount (G-Counter)",
        section: "crdt",
        probes: &["ConvergentCount"],
        detectors: &["ConvergentCount", "Increase "],
    },
    Construct {
        feature: "Tally (PN-Counter)",
        section: "crdt",
        probes: &["a Tally"],
        detectors: &["Tally", "Decrease "],
    },
    Construct {
        feature: "LastWriteWins register",
        section: "crdt",
        probes: &["LastWriteWins of Text"],
        detectors: &["LastWriteWins"],
    },
    Construct {
        feature: "Divergent + Resolve",
        section: "crdt",
        probes: &["Resolve page's title"],
        detectors: &["Divergent ", "Resolve "],
    },
    Construct {
        feature: "SharedSet (OR-Set)",
        section: "crdt",
        probes: &["SharedSet of Text"],
        detectors: &["SharedSet"],
    },
    Construct {
        feature: "SharedSequence + Append",
        section: "crdt",
        probes: &["Append \"Line 1\""],
        detectors: &["SharedSequence", "Append "],
    },
    Construct {
        feature: "SharedMap (OR-Map)",
        section: "crdt",
        probes: &["SharedMap from Text to Int"],
        detectors: &["SharedMap"],
    },
    Construct {
        feature: "Merge",
        section: "crdt",
        probes: &["Merge remote into local"],
        detectors: &["Merge "],
    },
    Construct {
        feature: "Sync on a topic",
        section: "crdt",
        probes: &["Sync score on"],
        detectors: &["Sync "],
    },
    Construct {
        feature: "Mount / Persistent",
        section: "crdt",
        probes: &["be mounted at"],
        detectors: &[],
    },
    // ── Security ────────────────────────────────────────────────────────
    Construct {
        feature: "policy predicates",
        section: "security",
        probes: &["is admin if the user's role"],
        detectors: &["## Policy"],
    },
    Construct {
        feature: "capabilities",
        section: "security",
        probes: &["can edit the Document if"],
        detectors: &[],
    },
    Construct {
        feature: "Check enforcement",
        section: "security",
        probes: &["Check that"],
        detectors: &["Check that"],
    },
    // ── Networking ──────────────────────────────────────────────────────
    Construct {
        feature: "Listen",
        section: "networking",
        probes: &["Listen on \"/ip4/"],
        detectors: &["Listen on"],
    },
    Construct {
        feature: "Connect",
        section: "networking",
        probes: &["Connect to server_addr"],
        detectors: &["Connect to"],
    },
    Construct {
        feature: "PeerAgent",
        section: "networking",
        probes: &["a PeerAgent at"],
        detectors: &["PeerAgent"],
    },
    Construct {
        feature: "Portable types",
        section: "networking",
        probes: &["is Portable and has:"],
        detectors: &["is Portable"],
    },
    Construct {
        feature: "mDNS discovery",
        section: "networking",
        probes: &["mDNS"],
        detectors: &[],
    },
    // ── Error handling ──────────────────────────────────────────────────
    Construct {
        feature: "failure-as-enum pattern",
        section: "error-handling",
        probes: &["a new Failure with message"],
        detectors: &[],
    },
    Construct {
        feature: "stdlib Result",
        section: "error-handling",
        probes: &["a new Ok with value"],
        detectors: &["a new Ok", "a new Err", "Result of", "When Ok", "When Err"],
    },
    Construct {
        feature: "Option some/none + Inspect",
        section: "error-handling",
        probes: &["When Some (v):"],
        detectors: &["When Some", "When None"],
    },
    Construct {
        feature: "Require that (hard assertion)",
        section: "error-handling",
        probes: &["Require that"],
        detectors: &["Require that"],
    },
    // ── Refinement types (moved here when advanced-features dissolved, W2.4) ──
    Construct {
        feature: "refinement types",
        section: "z3-verification",
        probes: &["where it > 0"],
        detectors: &[" where it"],
    },
    // ── Modules / CLI / stdlib ──────────────────────────────────────────
    Construct {
        feature: "module imports",
        section: "modules",
        probes: &["Use `Use`"],
        detectors: &[],
    },
    Construct {
        feature: "largo build/run",
        section: "cli-largo",
        probes: &["largo build"],
        detectors: &[],
    },
    Construct {
        feature: "largo wider verbs",
        section: "cli-largo",
        probes: &["largo prove"],
        detectors: &[],
    },
    Construct {
        feature: "core builtins",
        section: "stdlib",
        probes: &["abs(n)"],
        detectors: &[],
    },
    Construct {
        feature: "stdlib modules",
        section: "stdlib",
        probes: &["randomInt"],
        detectors: &[],
    },
    // ── Building web apps ───────────────────────────────────────────────
    Construct {
        feature: "a page mounts itself",
        section: "building-web-apps",
        probes: &["## Page /"],
        detectors: &["## Page"],
    },
    Construct {
        feature: "State, Actions and View sections",
        section: "building-web-apps",
        probes: &["State:", "Actions:", "View:"],
        detectors: &[],
    },
    Construct {
        feature: "an action as an event handler",
        section: "building-web-apps",
        probes: &["on-click={bump}"],
        detectors: &["on-click="],
    },
    Construct {
        feature: "derived state",
        section: "building-web-apps",
        probes: &["always be"],
        detectors: &["always be"],
    },
    Construct {
        feature: "keyed list rendering",
        section: "building-web-apps",
        probes: &["keyed by t's id"],
        detectors: &["keyed by"],
    },
    Construct {
        feature: "two-way binding",
        section: "building-web-apps",
        probes: &["bind={draft}"],
        detectors: &["bind={"],
    },
    Construct {
        feature: "components with props and children",
        section: "building-web-apps",
        probes: &["{children}"],
        detectors: &["## Component"],
    },
    Construct {
        feature: "a test that drives a page",
        section: "building-web-apps",
        probes: &["## Test:"],
        detectors: &["## Test"],
    },
    // ── Reference appendices (W7) ────────────────────────────────────────
    Construct {
        feature: "operator vocabulary table",
        section: "appendix-keywords",
        probes: &["symbolic twin"],
        detectors: &[],
    },
    Construct {
        feature: "error message gallery",
        section: "appendix-errors",
        probes: &["fail **on purpose**"],
        detectors: &[],
    },
    Construct {
        feature: "playground-vs-compiled honesty table",
        section: "appendix-runnability",
        probes: &["truth badge"],
        detectors: &[],
    },
    // ── Pillars (W6) ─────────────────────────────────────────────────────
    Construct {
        feature: "execution tiers + opt decorators",
        section: "execution-tiers",
        probes: &["## No Memo"],
        detectors: &["## Tier", "## No "],
    },
    Construct {
        feature: "Escape to Rust",
        section: "interop",
        probes: &["Escape to Rust:"],
        detectors: &["Escape to Rust"],
    },
    Construct {
        feature: "wasm export decorator",
        section: "interop",
        probes: &["is exported for wasm"],
        detectors: &["is exported for wasm"],
    },
    Construct {
        feature: "native function declarations",
        section: "interop",
        probes: &["## To native"],
        detectors: &[],
    },
    Construct {
        feature: "theorem blocks + largo prove",
        section: "proofs",
        probes: &["Proof: By automation."],
        detectors: &["## Theorem", "Given:", "Prove:"],
    },
    Construct {
        feature: "crypto stdlib in LOGOS",
        section: "stdlib",
        probes: &["sha3_256("],
        detectors: &[],
    },
    Construct {
        feature: "policy denial diagnostics",
        section: "security",
        probes: &["Check that u is admin"],
        detectors: &[],
    },
    Construct {
        feature: "deterministic replay",
        section: "concurrency",
        probes: &["LOGOS_SEED"],
        detectors: &[],
    },
    // ── Logic mode / verification ───────────────────────────────────────
    Construct {
        feature: "FOL quantifiers",
        section: "logic-mode",
        probes: &["∀"],
        detectors: &[],
    },
    Construct {
        feature: "modal operators",
        section: "logic-mode",
        probes: &["◇"],
        detectors: &[],
    },
    Construct {
        feature: "connectives",
        section: "logic-mode",
        probes: &["↔"],
        detectors: &[],
    },
    Construct {
        feature: "Assert",
        section: "assertions-trust",
        probes: &["Assert that"],
        detectors: &["Assert that"],
    },
    Construct {
        feature: "Trust with because",
        section: "assertions-trust",
        probes: &["Trust that"],
        detectors: &["Trust that"],
    },
    Construct {
        feature: "Z3 refinement verification",
        section: "z3-verification",
        probes: &["where it >= 0 and it <= 100"],
        detectors: &[],
    },
];
