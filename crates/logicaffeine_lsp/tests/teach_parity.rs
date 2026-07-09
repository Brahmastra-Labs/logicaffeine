//! The quickguide ↔ teach-table parity ratchet: every construct row in
//! LOGOS_QUICKGUIDE.md maps to a lesson (or records why prose is enough),
//! and every lesson is reachable from the guide (or records why it exists
//! beyond it). Adding a guide row without deciding its lesson fails; keeping
//! a stale mapping fails; orphaning a lesson fails. The guide and the
//! teaching brain can never drift apart silently.

#[path = "harness/quickguide.rs"]
mod quickguide;

use logicaffeine_language::teach::ALL_DOCS;
use quickguide::construct_rows;

enum Lesson {
    /// The row's construct is taught by this named lesson.
    Teach(&'static str),
    /// The row needs no lesson; the reason is part of the decision.
    Prose(&'static str),
}
use Lesson::{Prose, Teach};

const OPERATOR_TABLE: &str =
    "the operator table is its own lesson; hover teaches the operands' types";
const FORMAT_SPECS: &str = "format specs are enumerated by the table itself";
const PROPOSED: &str = "a (proposed) form — designed, not yet in the language";
const LITERALS: &str = "literal formats; the table enumerates them";
const CLOSURES: &str = "closures await a lesson of their own; the row is the example";

/// (section, construct) → decision, for EVERY quickguide table row.
const GUIDE_LESSONS: &[(&str, &str, Lesson)] = &[
    // 1. Program structure
    ("1. Program structure", "Entry point", Teach("Main")),
    ("1. Program structure", "Function", Teach("To")),
    ("1. Program structure", "Procedure", Teach("To")),
    ("1. Program structure", "Native import", Teach("To")),
    ("1. Program structure", "Struct", Teach("TypeDef")),
    ("1. Program structure", "Enum", Teach("TypeDef")),
    ("1. Program structure", "Theorem", Teach("Theorem")),
    // 2. Variables & mutation
    ("2. Variables & mutation", "Bind", Teach("Let")),
    ("2. Variables & mutation", "Mutable bind", Teach("Let")),
    ("2. Variables & mutation", "Reassign", Teach("Set")),
    ("2. Variables & mutation", "Increment", Teach("Set")),
    ("2. Variables & mutation", "Decrement", Teach("Set")),
    // 3. Arithmetic, comparison, logic, bitwise
    ("3. Arithmetic, comparison, logic, bitwise", "Add / sub", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Mul / div / mod", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Concatenate (Text)", Teach("Text")),
    ("3. Arithmetic, comparison, logic, bitwise", "Equal", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Not equal", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Greater / less", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "≥ / ≤", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Chained", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Range test", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Parity / divisibility", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Logical", Teach("Bool")),
    ("3. Arithmetic, comparison, logic, bitwise", "Bitwise and/or", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Bitwise xor", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Bitwise not", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Shift", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Negate", Prose(OPERATOR_TABLE)),
    ("3. Arithmetic, comparison, logic, bitwise", "Popcount", Prose(OPERATOR_TABLE)),
    // 4. Strings
    ("4. Strings", "Concatenate", Teach("Text")),
    ("4. Strings", "Interpolate", Teach("Text")),
    ("4. Strings", "Format precision", Prose(FORMAT_SPECS)),
    ("4. Strings", "Align", Prose(FORMAT_SPECS)),
    ("4. Strings", "Debug", Prose(FORMAT_SPECS)),
    ("4. Strings", "Currency", Prose(FORMAT_SPECS)),
    ("4. Strings", "Multiline", Teach("Text")),
    ("4. Strings", "split/join/trim/case/replace", Prose(PROPOSED)),
    // 5.1 Create
    ("5.1 Create", "List, empty", Teach("List")),
    ("5.1 Create", "List, literal", Teach("Seq")),
    ("5.1 Create", "List, pre-sized", Teach("Seq")),
    ("5.1 Create", "Map, empty", Teach("Map")),
    ("5.1 Create", "Map, with capacity", Teach("Map")),
    ("5.1 Create", "Map, literal", Prose(PROPOSED)),
    ("5.1 Create", "Set, empty", Teach("Set")),
    ("5.1 Create", "Set, literal", Prose(PROPOSED)),
    // 5.2 Read, write, slice
    ("5.2 Read, write, slice", "Index read (1-based)", Teach("Seq")),
    ("5.2 Read, write, slice", "Map lookup", Teach("Map")),
    ("5.2 Read, write, slice", "Index write", Teach("Set")),
    ("5.2 Read, write, slice", "Map insert", Teach("Map")),
    ("5.2 Read, write, slice", "Slice (inclusive)", Teach("Seq")),
    ("5.2 Read, write, slice", "Length", Teach("Seq")),
    ("5.2 Read, write, slice", "Membership", Teach("Seq")),
    ("5.2 Read, write, slice", "Copy", Teach("Give")),
    // 5.3 Mutate (lists & sets)
    ("5.3 Mutate (lists & sets)", "Append", Teach("Push")),
    ("5.3 Mutate (lists & sets)", "Pop", Teach("Pop")),
    ("5.3 Mutate (lists & sets)", "Set add", Teach("Add")),
    ("5.3 Mutate (lists & sets)", "Set remove", Teach("Remove")),
    ("5.3 Mutate (lists & sets)", "Union / intersection", Teach("Set")),
    // 5.4 Iterate & transform
    ("5.4 Iterate & transform", "For-each", Teach("Repeat")),
    ("5.4 Iterate & transform", "Counted", Teach("Repeat")),
    ("5.4 Iterate & transform", "Pairs (map)", Teach("Repeat")),
    ("5.4 Iterate & transform", "Map / filter", Prose(PROPOSED)),
    ("5.4 Iterate & transform", "Reduce / sum", Prose(PROPOSED)),
    ("5.4 Iterate & transform", "Sort", Prose(PROPOSED)),
    ("5.4 Iterate & transform", "any / all / count", Prose(PROPOSED)),
    // 6. Control flow
    ("6. Control flow", "If / else", Teach("If")),
    ("6. Control flow", "Else-if", Teach("If")),
    ("6. Control flow", "While", Teach("While")),
    ("6. Control flow", "While + variant", Teach("While")),
    ("6. Control flow", "Break", Teach("Break")),
    ("6. Control flow", "Return", Teach("Return")),
    ("6. Control flow", "Conditional value", Prose(PROPOSED)),
    ("6. Control flow", "Match", Teach("Inspect")),
    // 7. Functions & closures
    ("7. Functions & closures", "Define", Teach("To")),
    ("7. Functions & closures", "Define (prose)", Teach("To")),
    ("7. Functions & closures", "Define (prepositional)", Teach("To")),
    ("7. Functions & closures", "Call", Teach("Call")),
    ("7. Functions & closures", "Call (statement)", Teach("Call")),
    ("7. Functions & closures", "Closure (expr)", Prose(CLOSURES)),
    ("7. Functions & closures", "Closure (block)", Prose(CLOSURES)),
    ("7. Functions & closures", "HOF parameter", Prose(CLOSURES)),
    ("7. Functions & closures", "Return a closure", Prose(CLOSURES)),
    ("7. Functions & closures", "Call a closure value", Teach("Call")),
    // 8. Structs, enums & field access
    ("8. Structs, enums & field access", "Struct def", Teach("TypeDef")),
    ("8. Structs, enums & field access", "Construct", Teach("New")),
    ("8. Structs, enums & field access", "Field read", Teach("New")),
    ("8. Structs, enums & field access", "Nested field", Teach("New")),
    ("8. Structs, enums & field access", "Field write", Teach("Set")),
    ("8. Structs, enums & field access", "Method (UFCS)", Teach("Call")),
    ("8. Structs, enums & field access", "Enum def", Teach("TypeDef")),
    ("8. Structs, enums & field access", "Variant construct", Teach("New")),
    ("8. Structs, enums & field access", "Match variant", Teach("Inspect")),
    // 9. Options & pattern matching
    ("9. Options & pattern matching", "Some", Teach("Option")),
    ("9. Options & pattern matching", "None", Teach("Option")),
    ("9. Options & pattern matching", "Match", Teach("Inspect")),
    ("9. Options & pattern matching", "Optional chaining", Prose(PROPOSED)),
    // 10. Contracts
    ("10. Contracts: refinement, assert, trust, check", "Refinement type", Teach("Let")),
    ("10. Contracts: refinement, assert, trust, check", "Compound refinement", Teach("Let")),
    ("10. Contracts: refinement, assert, trust, check", "Assert (debug)", Teach("Assert")),
    ("10. Contracts: refinement, assert, trust, check", "Trust (justified)", Teach("Trust")),
    ("10. Contracts: refinement, assert, trust, check", "Check (mandatory)", Teach("Check")),
    // 11. Temporal literals
    ("11. Temporal literals", "Duration", Prose(LITERALS)),
    ("11. Temporal literals", "Date", Prose(LITERALS)),
    ("11. Temporal literals", "Time of day", Prose(LITERALS)),
    ("11. Temporal literals", "Calendar span", Prose(LITERALS)),
    ("11. Temporal literals", "Combined", Prose(LITERALS)),
    // 12. Distributed
    ("12. Distributed: CRDT, concurrency, networking, zones", "Shared struct", Teach("TypeDef")),
    ("12. Distributed: CRDT, concurrency, networking, zones", "CRDT increment", Teach("Increase")),
    ("12. Distributed: CRDT, concurrency, networking, zones", "CRDT decrement", Teach("Decrease")),
    ("12. Distributed: CRDT, concurrency, networking, zones", "CRDT merge", Teach("Merge")),
    ("12. Distributed: CRDT, concurrency, networking, zones", "Shared set", Teach("Add")),
    (
        "12. Distributed: CRDT, concurrency, networking, zones",
        "Shared sequence",
        Prose("RGA Append awaits a lesson of its own"),
    ),
    ("12. Distributed: CRDT, concurrency, networking, zones", "Spawn agent", Teach("Spawn")),
    (
        "12. Distributed: CRDT, concurrency, networking, zones",
        "Zone (arena)",
        Prose("zones await a lesson of their own"),
    ),
    (
        "12. Distributed: CRDT, concurrency, networking, zones",
        "Listen",
        Prose("the raw multiaddr surface is leaky (smells §I-5); teach it after it settles"),
    ),
    // 13. Output
    ("13. Output", "Print", Teach("Show")),
    ("13. Output", "Print formatted", Teach("Show")),
    ("13. Output", "Move into a sink", Teach("Give")),
];

/// Lessons that exist beyond the guide's tables, each with its reason.
const TEACH_EXTRA: &[(&str, &str)] = &[
    ("Escape", "the escape hatch is deliberately not in the guide"),
    ("Proof", "logic-mode surface beyond the quickguide's imperative scope"),
    ("Definition", "logic-mode surface beyond the quickguide's imperative scope"),
    ("Define", "logic-mode surface beyond the quickguide's imperative scope"),
    ("Axiom", "logic-mode surface beyond the quickguide's imperative scope"),
    ("Theory", "logic-mode surface beyond the quickguide's imperative scope"),
    ("Policy", "spec surface; the guide mentions Check but not Policy blocks"),
    ("Logic", "logic-mode surface beyond the quickguide's imperative scope"),
    ("Example", "documentation block; not a guide construct"),
    ("Note", "documentation block; not a guide construct"),
    ("Requires", "escape-hatch dependency surface; not a guide construct"),
    ("Hardware", "hardware-verification surface beyond the quickguide"),
    ("Property", "hardware-verification surface beyond the quickguide"),
    ("No", "optimizer annotation; not a guide construct"),
    ("Tier", "optimizer annotation; not a guide construct"),
    ("Unknown header", "an error artifact, not a construct"),
    ("Int", "primitive types have no construct row — the guide teaches them by use"),
    ("Nat", "primitive types have no construct row — the guide teaches them by use"),
    ("Float", "primitive types have no construct row — the guide teaches them by use"),
    ("Unit", "primitive types have no construct row — the guide teaches them by use"),
    ("Char", "primitive types have no construct row — the guide teaches them by use"),
    ("Byte", "primitive types have no construct row — the guide teaches them by use"),
    ("Result", "primitive types have no construct row — the guide teaches them by use"),
];

#[test]
fn every_guide_row_decides_its_lesson_and_nothing_is_stale() {
    let guide = include_str!("../../../LOGOS_QUICKGUIDE.md");
    let rows = construct_rows(guide);
    assert!(rows.len() > 80, "the guide should parse to a rich row set, got {}", rows.len());

    for row in &rows {
        let hits = GUIDE_LESSONS
            .iter()
            .filter(|(section, construct, _)| *section == row.section && *construct == row.construct)
            .count();
        assert!(
            hits > 0,
            "guide row [{} :: {}] has no lesson decision — add it to GUIDE_LESSONS \
             (Teach a lesson or record why Prose is enough)",
            row.section,
            row.construct
        );
    }
    for (section, construct, _) in GUIDE_LESSONS {
        assert!(
            rows.iter().any(|r| r.section == *section && r.construct == *construct),
            "stale GUIDE_LESSONS entry [{section} :: {construct}] — no such guide row"
        );
    }
}

#[test]
fn every_taught_mapping_names_a_real_lesson_and_every_lesson_is_reachable() {
    let mut reachable: Vec<&str> = Vec::new();
    for (section, construct, lesson) in GUIDE_LESSONS {
        match lesson {
            Teach(name) => {
                assert!(
                    ALL_DOCS.iter().any(|d| d.name == *name),
                    "[{section} :: {construct}] maps to unknown lesson {name:?}"
                );
                reachable.push(name);
            }
            Prose(reason) => {
                assert!(!reason.is_empty(), "[{section} :: {construct}] needs a reason");
            }
        }
    }

    for doc in ALL_DOCS {
        let in_guide = reachable.contains(&doc.name);
        let extra = TEACH_EXTRA.iter().any(|(name, _)| *name == doc.name);
        assert!(
            in_guide || extra,
            "{}: lesson unreachable from the guide — map a row to it or record it in \
             TEACH_EXTRA with a reason",
            doc.name
        );
        assert!(
            !(in_guide && extra),
            "{}: reachable from the guide but still in TEACH_EXTRA — remove the excuse",
            doc.name
        );
    }
    for (name, reason) in TEACH_EXTRA {
        assert!(!reason.is_empty(), "{name}: excuses record their reason");
        assert!(
            ALL_DOCS.iter().any(|d| d.name == *name),
            "{name}: stale TEACH_EXTRA entry — no such lesson"
        );
    }
}
