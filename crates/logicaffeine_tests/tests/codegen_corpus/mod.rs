//! The programs the generated-code locks hold every emitter to: the benchmark corpus as checked in under
//! `benchmarks/programs/`, and a corpus of feature programs covering what the benchmarks do not — records, enums and
//! `Inspect`, Text-keyed maps, text building and interpolation, options, closures, lists of records, sets, CRDT fields,
//! pipes, tuples, recursion, word operations, closures bound to names, inexact complex values copied, powers of reals,
//! a number written in binary, text built from bytes, a fixed-size list filled in a loop, waits, remainders and
//! quotients by a divisor known only when the program runs, and a transcendental sum the emitter computes four lanes at
//! a time. Every feature program runs on the tree-walker.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// One program of a corpus: its name (a Rust identifier), its LOGOS source, and how to run it.
pub struct Program {
    pub name: String,
    pub source: String,
    pub run: Run,
}

/// How a program is run to check what it prints.
pub enum Run {
    /// No arguments; it prints what the tree-walker prints.
    AsTheTreeWalker,
    /// With one argument; it prints the checked-in expected output for that argument.
    WithArgument { arg: String, expected: String },
}

/// The workspace root, above this crate.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the tests crate sits two levels under the workspace root")
        .to_path_buf()
}

/// Every benchmark program, in name order, each run with the smallest argument it has an expected output for.
pub fn benchmark_programs() -> Vec<Program> {
    let root = workspace_root().join("benchmarks/programs");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("main.lg").is_file())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 30, "the benchmark corpus shrank to {} programs", dirs.len());
    dirs.iter()
        .map(|dir| {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let source = std::fs::read_to_string(dir.join("main.lg")).expect("read main.lg");
            let (arg, expected) = smallest_expected(dir)
                .unwrap_or_else(|| panic!("{name} has no expected_<n>.txt"));
            Program {
                name,
                source,
                run: Run::WithArgument { arg, expected },
            }
        })
        .collect()
}

/// The smallest `n` a benchmark directory holds `expected_<n>.txt` for, and that file's text.
fn smallest_expected(dir: &Path) -> Option<(String, String)> {
    let mut sizes: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            let n = file.strip_prefix("expected_")?.strip_suffix(".txt")?.parse::<u64>().ok()?;
            Some((n, e.path()))
        })
        .collect();
    sizes.sort();
    let (n, path) = sizes.into_iter().next()?;
    Some((n.to_string(), std::fs::read_to_string(path).ok()?))
}

/// The feature corpus.
pub fn feature_programs() -> Vec<Program> {
    FEATURES
        .iter()
        .map(|(name, source)| Program {
            name: (*name).to_string(),
            source: (*source).to_string(),
            run: Run::AsTheTreeWalker,
        })
        .collect()
}

/// Both corpora.
pub fn all_programs() -> Vec<Program> {
    let mut all = benchmark_programs();
    all.extend(feature_programs());
    all
}

const FEATURES: &[(&str, &str)] = &[
    (
        "records",
        "## A Point has:
    An x: Int.
    A y: Int.
    A label: Text.

## To shifted (p: Point, dx: Int) -> Point:
    Let q be a new Point with x p's x + dx and y p's y and label p's label.
    Return q.

## To manhattan (p: Point) -> Int:
    Return abs(p's x) + abs(p's y).

## Main
Let mutable p be a new Point with x 3 and y 4 and label \"origin-ish\".
Set p's x to p's x * 2.
Let q be shifted(p, 5).
Show q's x.
Show manhattan(q).
Show p's label.
",
    ),
    (
        "shapes",
        "## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.
    A Rect with width Int and height Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (r):
            Return 3 * r * r.
        When Square (d):
            Return d * d.
        When Rect (w, h):
            Return w * h.

## To kind (s: Shape) -> Text:
    Inspect s:
        When Circle (r):
            Return \"round\".
        Otherwise:
            Return \"angular\".

## Main
Let shapes be [a new Circle with radius 2, a new Square with side 5, a new Rect with width 3 and height 7].
Let mutable total be 0.
Repeat for s in shapes:
    Set total to total + area(s).
    Show kind(s).
Show total.
",
    ),
    (
        "wordcount",
        "## Main
Let words be split(\"the cat and the hat and the bat\", \" \").
Let mutable counts be a new Map of Text to Int.
Repeat for w in words:
    If counts contains w:
        Set counts at w to item w of counts + 1.
    Otherwise:
        Set counts at w to 1.
Repeat for (k, v) in counts:
    Show \"{k}: {v}\".
Show length of counts.
Show item \"the\" of counts.
",
    ),
    (
        "text",
        "## To vowels (s: Text) -> Int:
    Let mutable n be 0.
    Repeat for c in s:
        If c equals \"a\" or c equals \"e\" or c equals \"i\" or c equals \"o\" or c equals \"u\":
            Set n to n + 1.
    Return n.

## Main
Let name be \"logicaffeine\".
Let mutable built be \"\".
Let mutable i be 0.
While i is less than 5:
    Set built to built + \"ab\".
    Set i to i + 1.
Show built.
Show \"\" + name + \" has \" + vowels(name) + \" vowels\".
Show \"{name} is {length of name} long\".
Let pi be 3.14159.
Show \"{pi:.2}\".
",
    ),
    (
        "options",
        "## To find (xs: Seq of Int, target: Int) -> Option of Int:
    Repeat for i from 1 to length of xs:
        If item i of xs equals target:
            Return some i.
    Return none.

## Main
Inspect find([4, 7, 9], 7):
    When Some (at): Show at.
    When None: Show \"absent\".
Let missing be find([4, 7, 9], 5).
If missing is none:
    Show \"absent\".
",
    ),
    (
        "closures",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## To adder (n: Int) -> fn(Int) -> Int:
    Return (x: Int) -> x + n.

## Main
Let add5 be adder(5).
Show apply(add5, 10).
Show apply((y: Int) -> y * y, 7).
Let mutable xs be [1, 2, 3, 4].
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + apply(add5, x).
Show total.
",
    ),
    (
        "people",
        "## A Person has:
    A name: Text.
    An age: Int.

## To oldest (people: Seq of Person) -> Text:
    Let mutable best be item 1 of people.
    Repeat for p in people:
        If p's age is greater than best's age:
            Set best to p.
    Return best's name.

## Main
Let mutable people be a new Seq of Person.
Push a new Person with name \"Ada\" and age 36 to people.
Push a new Person with name \"Alan\" and age 41 to people.
Push a new Person with name \"Grace\" and age 85 to people.
Show oldest(people).
Let mutable years be 0.
Repeat for p in people:
    Set years to years + p's age.
Show years.
Set item 1 of people to a new Person with name \"Ada L.\" and age 37.
Let first be item 1 of people.
Show first's name.
",
    ),
    (
        "sets",
        "## Main
Let mutable seen be a new Set of Int.
Let mutable i be 1.
While i is at most 20:
    Add i * i % 7 to seen.
    Set i to i + 1.
Show length of seen.
Show seen contains 4.
Remove 4 from seen.
Show seen contains 4.
Let evens be {2, 4, 6}.
Let both be seen union evens.
Show length of both.
",
    ),
    (
        "crdt",
        "## Definition
A Board is Shared and has:
    a points, which is ConvergentCount.
    a games, which is Int.

## Main
Let mutable local be a new Board.
Let mutable remote be a new Board.
Increase local's points by 3.
Increase remote's points by 4.
Set local's games to 2.
Set remote's games to 5.
Merge remote into local.
Show local's points.
Show local's games.
",
    ),
    (
        "pipes",
        "## Main
Let ch be a Pipe of Int.
Let mutable i be 0.
While i is less than 10:
    Send i into ch.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than 10:
    Receive x from ch.
    Set total to total + x.
    Set i to i + 1.
Show total.
",
    ),
    (
        "tuples",
        "## To divmod (n: Int) -> (Int, Int):
    Return (n / 5, n mod 5).

## To parts (n: Int) -> (quotient: Int, remainder: Int):
    Return (n / 5, n mod 5).

## Main
Let (q, rem) be divmod(17).
Show q.
Show rem.
Let r be parts(17).
Show r's quotient.
Show item 2 of divmod(23).
",
    ),
    (
        "recursion",
        "## To ways (n: Int) -> Int:
    If n is less than 2:
        Return 1.
    Return ways(n - 1) + ways(n - 2).

## To sumTo (n: Int, acc: Int) -> Int:
    If n equals 0:
        Return acc.
    Return sumTo(n - 1, acc + n).

## Main
Show ways(25).
Show sumTo(1000, 0).
",
    ),
    (
        "bits",
        "## Main
Let counts be [1, 7, 31, 62, 64, 65].
Let mutable h be 0.
Repeat for c in counts:
    Let up be 1 shifted left by c.
    Let down be 4096 shifted right by c.
    Set h to (h * 31 + (up & 1023) + down) % 1000003.
    Show count_ones(up - 1).
Show h.
Show 255 ^ 170.
Show 12 | 3.
",
    ),
    (
        "bytes",
        "## To letters (n: Int) -> Text:
    Let mutable out be a new Seq of Int.
    Push 122 to out.
    Repeat for i from 1 to n:
        Push 96 + i to out.
        Push (i * 7 + 3) % 26 + 97 to out.
    Return text_from_bytes(out).

## Main
Show letters(5).
",
    ),
    (
        "digest",
        "## To digest4 (a: Int) and (b: Int) -> Seq of Int:
    Let mutable h be a new Seq of Int.
    Push a to h.
    Push b to h.
    Let mutable out be a new Seq of Int.
    Repeat for x in h:
        Push x % 256 to out.
        Push (x / 256) % 256 to out.
    Return out.

## Main
Let d be digest4(513, 1027).
Show (item 1 of d) + (item 2 of d) + (item 3 of d) + (item 4 of d).
",
    ),
    (
        "waits",
        "## Main
Show 1.
Sleep 0.
Let pause be 2.
Sleep pause.
Show 2.
",
    ),
    (
        "remainders",
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"10\").
Let mutable total be 0.
Let mutable i be -3.
While i is less than n:
    Let r be (i * i) % n.
    Let q be (i * 7) / n.
    Set total to total + r + q.
    Set i to i + 1.
Show total.
",
    ),
    (
        "closure_values",
        "## Main
Let bonus be 5.
Let add be (n: Int) -> n + bonus.
Let scored be (n: Int) ->:
    If n is greater than 10:
        Return n + bonus.
    Return n.
Show add(3).
Show scored(3).
Show scored(30).
",
    ),
    (
        "complex_copies",
        "## To lastOf (xs: Seq of ComplexF) -> ComplexF:
    Let mutable last be item 1 of xs.
    Repeat for x in xs:
        Set last to x.
    Return last.

## Main
Let xs be [complex(1.0, 2.0), complex(3.0, 4.0), complex(0.0, -1.0)].
Let a be item 2 of xs.
Let mutable b be complex(0.5, 0.0).
Show b.
Set b to a.
Show b.
Show a.
Show conjugate(lastOf(xs)).
",
    ),
    (
        "powers",
        "## To fade (x: Real) -> Real:
    Return 1.0 / (1.0 + 2.718281828459045 ** (0.0 - x)).

## Main
Show fade(0.5).
Let base be 2.0.
Show base to the power of 10.
Show 3 ** 4.
",
    ),
    (
        "binary",
        "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"37\").
Show toBinary(n).
Show toBinary(0).
",
    ),
    (
        "trig_sum",
        "## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 1000:
    Set sum to sum + trunc(acos(toReal(i mod 1000) / 1000.0) * 1000000.0).
    Set i to i + 1.
Show sum.
",
    ),
];
