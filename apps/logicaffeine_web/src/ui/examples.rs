//! Example files for the Studio playground.
//!
//! These are seeded into the VFS on first launch to give users
//! something to work with immediately.

use logicaffeine_system::fs::{Vfs, VfsResult};

/// Seed example files into the VFS if they don't exist.
pub async fn seed_examples<V: Vfs>(vfs: &V) -> VfsResult<()> {
    // Create directory structure (idempotent).
    vfs.create_dir_all("/examples/logic").await?;
    vfs.create_dir_all("/examples/code").await?;
    vfs.create_dir_all("/examples/math").await?;
    vfs.create_dir_all("/examples/hardware").await?;
    vfs.create_dir_all("/workspace").await?;
    for sub in [
        "basics", "types", "collections", "functions", "distributed", "security", "memory",
        "concurrency", "networking", "advanced", "native", "temporal",
    ] {
        vfs.create_dir_all(&format!("/examples/code/{sub}")).await?;
    }

    // Retire earlier toy RTL examples replaced by the real-hardware set.
    for obsolete in ["toggle.v", "mutex.v", "counter.v"] {
        let _ = vfs.remove(&format!("/examples/hardware/{obsolete}")).await;
    }

    // The four registries are the single source of truth for every shipped Studio
    // example: seed exactly them — the same specs the `example_health` tests drive,
    // so seeding and testing can never drift.
    for spec in ALL_LOGIC_EXAMPLES
        .iter()
        .chain(ALL_CODE_EXAMPLES)
        .chain(ALL_MATH_EXAMPLES)
        .chain(ALL_HARDWARE_SPECS)
    {
        vfs.write(spec.vfs_path, spec.source.as_bytes()).await?;
    }

    Ok(())
}

/// Seed only the advanced code examples (for existing installations).
/// Always overwrites to ensure latest syntax is used.
/// Hardware-mode examples: `(filename, English spec)`. Each is a single English hardware
/// specification that the Studio synthesizes to SystemVerilog Assertions and then certifies
/// (in-browser, no Z3) against the spec. Pure spec sentences — the loader feeds the whole
/// file to the synthesizer. The `hardware_examples` integration test verifies every one of
/// these synthesizes, round-trips as certified-equivalent, and has a reachable trigger.
pub const HARDWARE_EXAMPLES: &[(&str, &str)] = &[
    ("handshake.hw", "Always, if request is high, then acknowledge is high."),
    ("enable-ready.hw", "Always, if enable is high, then ready is high."),
    ("start-done.hw", "Always, if start is high, then done is high."),
    ("write-full.hw", "Always, if write is high, then full is high."),
    // Signal-DESIGN spec ("conflicts with"): the Studio synthesizes a conflict-free phase plan
    // with our own SAT solver and certifies it uses the fewest possible phases (no Z3).
    (
        "intersection-design.hw",
        "Movements: ns-through, ew-through, ns-left, ew-left, pedestrian.\n\
         ns-through conflicts with ew-through and ew-left.\n\
         ew-through conflicts with ns-left.\n\
         ns-left conflicts with ew-left.\n\
         pedestrian conflicts with ns-through, ew-through, ns-left and ew-left.",
    ),
];

/// RTL examples: `(filename, Verilog)`. Synthesizable Verilog the Studio parses into a
/// transition system and bounded-model-checks / k-induction-proves in the browser (no Z3).
/// Verified by the `hardware_examples` integration test. Opened in Hardware mode (they live
/// under `/examples/hardware`); `load_hardware_spec` routes `module … endmodule` content to
/// the RTL BMC path.
pub const RTL_EXAMPLES: &[(&str, &str)] = &[
    (
        "arbiter.v",
        "// 2-master round-robin arbiter. Grants are issued in mutually-exclusive branches.\n// PROVEN by k-induction (for every request/turn sequence): the two masters are\n// NEVER granted the bus at the same time.\nmodule arbiter(input clk, input r0, input r1);\n  reg g0;\n  reg g1;\n  reg turn;\n  initial begin g0 = 0; g1 = 0; turn = 0; end\n  always @(posedge clk) begin\n    if (r0 && (!r1 || turn == 0)) begin\n      g0 <= 1;\n      g1 <= 0;\n    end else if (r1) begin\n      g0 <= 0;\n      g1 <= 1;\n    end else begin\n      g0 <= 0;\n      g1 <= 0;\n    end\n    turn <= ~turn;\n  end\n  assert property (~(g0 & g1));\nendmodule\n",
    ),
    (
        "bad-arbiter.v",
        "// The classic arbiter BUG: each request is granted independently. If both masters\n// request in the same cycle, BOTH are granted. BMC finds the violation at step 1 and\n// the waveform shows r0=r1=1 -> g0=g1=1 (a real mutual-exclusion failure).\nmodule bad_arbiter(input clk, input r0, input r1);\n  reg g0;\n  reg g1;\n  initial begin g0 = 0; g1 = 0; end\n  always @(posedge clk) begin\n    if (r0) g0 <= 1; else g0 <= 0;\n    if (r1) g1 <= 1; else g1 <= 0;\n  end\n  assert property (~(g0 & g1));\nendmodule\n",
    ),
    (
        "fifo.v",
        "// FIFO occupancy counter, depth 8, over FREE push/pop inputs. Increment only when not\n// full, decrement only when not empty. PROVEN: the occupancy never overflows past 8 for\n// ANY push/pop sequence — even though `count` is a 4-bit register that could reach 15.\nmodule fifo(input clk, input push, input pop);\n  reg [3:0] count;\n  initial count = 0;\n  always @(posedge clk)\n    if (push && (count < 4'd8) && !(pop && (count > 4'd0)))\n      count <= count + 1;\n    else if (pop && (count > 4'd0) && !(push && (count < 4'd8)))\n      count <= count - 1;\n    else\n      count <= count;\n  assert property (count <= 4'd8);\nendmodule\n",
    ),
    (
        "onehot.v",
        "// 3-state one-hot ring FSM. The rotation a<=c, b<=a, c<=b permutes the state bits.\n// PROVEN invariant: EXACTLY one of a/b/c is ever high (at-least-one AND at-most-one).\nmodule onehot(input clk);\n  reg a;\n  reg b;\n  reg c;\n  initial begin a = 1; b = 0; c = 0; end\n  always @(posedge clk) begin\n    a <= c;\n    b <= a;\n    c <= b;\n  end\n  assert property ((a | b | c) & ~(a & b) & ~(a & c) & ~(b & c));\nendmodule\n",
    ),
    (
        "reset-mirror.v",
        "// Two registers with identical reset+toggle logic over a FREE reset input.\n// PROVEN by k-induction for EVERY reset sequence: a and b always stay equal.\nmodule mirror(input clk, input rst);\n  reg a;\n  reg b;\n  initial begin a = 0; b = 0; end\n  always @(posedge clk) begin\n    if (rst) a <= 0; else a <= ~a;\n    if (rst) b <= 0; else b <= ~b;\n  end\n  assert property (a == b);\nendmodule\n",
    ),
    (
        "traffic-safe.v",
        "// 🚦 A COMPLEX signalized intersection — the kind you verify for a formal-methods\n// final. NS/EW through movements, PROTECTED left turns (nsl/ewl), and a PEDESTRIAN phase,\n// sequenced by a 9-state controller with a per-phase timer. Lights: 0=red 1=green 2=yellow.\n// PROVEN by k-induction: no two conflicting movements are ever active together, and\n// pedestrians get WALK only when EVERY vehicle movement is red.\nmodule traffic(input clk);\n  reg [1:0] ns;\n  reg [1:0] ew;\n  reg [1:0] nsl;\n  reg [1:0] ewl;\n  reg ped;\n  reg [3:0] phase;\n  reg [2:0] timer;\n  initial begin ns=2'd0; ew=2'd0; nsl=2'd1; ewl=2'd0; ped=1'd0; phase=4'd0; timer=3'd1; end\n  always @(posedge clk)\n    if (timer == 3'd0) begin\n      timer <= 3'd1;\n      if (phase == 4'd0) begin phase<=4'd1; nsl<=2'd2; ns<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd1) begin phase<=4'd2; nsl<=2'd0; ns<=2'd1; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd2) begin phase<=4'd3; ns<=2'd2; nsl<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd3) begin phase<=4'd4; ewl<=2'd1; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd4) begin phase<=4'd5; ewl<=2'd2; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd5) begin phase<=4'd6; ewl<=2'd0; ew<=2'd1; ns<=2'd0; nsl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd6) begin phase<=4'd7; ew<=2'd2; ns<=2'd0; nsl<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd7) begin phase<=4'd8; ped<=1'd1; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ewl<=2'd0; end\n      else begin phase<=4'd0; nsl<=2'd1; ns<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n    end else\n      timer <= timer - 1;\n  assert property (~((ns != 2'd0) & (ew != 2'd0)) & ~((ped == 1'd1) & ((ns != 2'd0) | (ew != 2'd0) | (nsl != 2'd0) | (ewl != 2'd0))));\nendmodule\n",
    ),
    (
        "traffic-crash.v",
        "// 🚦💥 The SAME intersection with ONE dangerous bug: the pedestrian phase raises WALK\n// but forgets to clear NS, so it sends pedestrians into live cross traffic. BMC finds the\n// exact cycle — watch the lights step through the REAL trace, then flash CONFLICT.\nmodule traffic(input clk);\n  reg [1:0] ns;\n  reg [1:0] ew;\n  reg [1:0] nsl;\n  reg [1:0] ewl;\n  reg ped;\n  reg [3:0] phase;\n  reg [2:0] timer;\n  initial begin ns=2'd0; ew=2'd0; nsl=2'd1; ewl=2'd0; ped=1'd0; phase=4'd0; timer=3'd1; end\n  always @(posedge clk)\n    if (timer == 3'd0) begin\n      timer <= 3'd1;\n      if (phase == 4'd0) begin phase<=4'd1; nsl<=2'd2; ns<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd1) begin phase<=4'd2; nsl<=2'd0; ns<=2'd1; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd2) begin phase<=4'd3; ns<=2'd2; nsl<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd3) begin phase<=4'd4; ewl<=2'd1; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd4) begin phase<=4'd5; ewl<=2'd2; ns<=2'd0; ew<=2'd0; nsl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd5) begin phase<=4'd6; ewl<=2'd0; ew<=2'd1; ns<=2'd0; nsl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd6) begin phase<=4'd7; ew<=2'd2; ns<=2'd0; nsl<=2'd0; ewl<=2'd0; ped<=1'd0; end\n      else if (phase == 4'd7) begin phase<=4'd8; ped<=1'd1; ns<=2'd1; ew<=2'd0; nsl<=2'd0; ewl<=2'd0; end\n      else begin phase<=4'd0; nsl<=2'd1; ns<=2'd0; ew<=2'd0; ewl<=2'd0; ped<=1'd0; end\n    end else\n      timer <= timer - 1;\n  assert property (~((ns != 2'd0) & (ew != 2'd0)) & ~((ped == 1'd1) & ((ns != 2'd0) | (ew != 2'd0) | (nsl != 2'd0) | (ewl != 2'd0))));\nendmodule\n",
    ),
    (
        "queue-jam.v",
        "// \u{1F697}\u{1F6A6} TRAFFIC FLOW: an approach's queue is a counter. Served only every other cycle\n// while a car arrives each cycle (demand > capacity), so the queue climbs until it JAMS. BMC\n// finds the exact cycle it overflows \u{2014} watch q ramp up in the waveform.\nmodule flow(input clk);\n  reg [2:0] q;\n  reg phase;\n  initial begin q = 3'd0; phase = 1'd0; end\n  always @(posedge clk) begin\n    phase <= ~phase;\n    if (phase == 1'd1) q <= q + 3'd1;\n  end\n  assert property (q < 3'd7);\nendmodule\n",
    ),
    (
        "queue-stable.v",
        "// \u{1F697}\u{2705} TRAFFIC FLOW: the same queue, but service keeps up with demand, so a starting\n// backlog only ever DRAINS. PROVEN by k-induction to never jam \u{2014} for all time, not just a\n// bounded window.\nmodule flow(input clk);\n  reg [2:0] q;\n  initial begin q = 3'd5; end\n  always @(posedge clk)\n    if (q != 3'd0) q <= q - 3'd1;\n  assert property (q < 3'd7);\nendmodule\n",
    ),
];

/// Register-allocation examples: a basic block's variable live ranges + a register budget. Opened
/// in Hardware mode; `load_hardware_spec` routes a `registers:` spec to the certified linear-scan
/// allocator and renders the live-range timeline (coloured by register, spill clique flagged red).
pub const REGALLOC_EXAMPLES: &[(&str, &str)] = &[
    (
        "register-alloc-fits.hw",
        "# Register allocation — a basic block whose live ranges fit in 3 registers.\n\
         # Each line is `variable: firstInstr-lastInstr`; `registers:` is the physical budget.\n\
         registers: 3\n\
         a: 0-4\n\
         b: 1-3\n\
         c: 2-6\n\
         d: 5-9\n\
         e: 7-10\n",
    ),
    (
        "register-alloc-spill.hw",
        "# Register allocation — OVER PRESSURE: four variables are live at once but only 3\n\
         # registers exist, so the allocator certifies (via the mutually-interfering clique)\n\
         # that at least one must spill.\n\
         registers: 3\n\
         a: 0-6\n\
         b: 1-7\n\
         c: 2-8\n\
         d: 3-9\n",
    ),
];

/// Pigeonhole examples: `pigeons: N` → PHP(N), `N` pigeons into `N-1` holes. Opened in Hardware
/// mode; `load_hardware_spec` routes a `pigeons:` spec to the live solver, which animates the
/// doomed pigeon and emits a certified symmetry-breaking refutation (Hall witness + Heule PR proof,
/// no Z3) — the family every resolution-based solver (Kissat, CaDiCaL, Z3) needs `2^Ω(n)` steps on.
pub const PIGEONHOLE_EXAMPLES: &[(&str, &str)] = &[
    (
        "pigeonhole.hw",
        "# Pigeonhole — N pigeons into N-1 holes is impossible. Watch the last pigeon find no home.\n\
         # Our prover certifies UNSAT in polynomial time (maximum matching + symmetry breaking);\n\
         # every resolution solver (Kissat, CaDiCaL, Z3) needs exponentially many steps here.\n\
         pigeons: 6\n",
    ),
    (
        "pigeonhole-12.hw",
        "# PHP(12): far past where Z3 times out, our certified PR proof stays polynomial.\n\
         pigeons: 12\n",
    ),
];

// ============================================================
// Logic Mode Examples (English -> FOL)
// ============================================================

pub const LOGIC_SIMPLE: &str = r#"# Simple Sentences

Every cat sleeps.
Some dogs bark loudly.
John loves Mary.
The quick brown fox jumps.
No student failed.
"#;

pub const LOGIC_QUANTIFIERS: &str = r#"# Quantifier Scope

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

// The full PuzzleBaron "Simon" logic grid — four trips × four categories (year, state,
// friend, activity) with all six clues (both of-pair clues included) — solved by the
// SAME kernel-certified prover as Socrates: grounded to its finite domain and closed by
// unit propagation (no Z3, runs in WASM). Every cell is forced by the exactly-one-each
// bijections plus the clues; `Beta is in Florida` falls out via the Florida=hunting and
// hunting/2004 of-pair clues. Renders the full certified derivation.
const LOGIC_SIMON: &str = r#"## Theorem: Simon
Given: Alpha, Beta, Gamma, and Delta are four different trips.
Given: 2001, 2002, 2003, and 2004 are four different years.
Given: Connecticut, Florida, Kentucky, and Maine are four different states.
Given: Bill, Lillie, Neal, and Yvonne are four different friends.
Given: Cycling, hunting, kayaking, and skydiving are four different activities.
Given: Every trip is in 2001 or in 2002 or in 2003 or in 2004.
Given: Exactly one trip is in 2001.
Given: Exactly one trip is in 2002.
Given: Exactly one trip is in 2003.
Given: Exactly one trip is in 2004.
Given: Every trip is in Connecticut or in Florida or in Kentucky or in Maine.
Given: Exactly one trip is in Connecticut.
Given: Exactly one trip is in Florida.
Given: Exactly one trip is in Kentucky.
Given: Exactly one trip is in Maine.
Given: Every trip is with Bill or with Lillie or with Neal or with Yvonne.
Given: Exactly one trip is with Bill.
Given: Exactly one trip is with Lillie.
Given: Exactly one trip is with Neal.
Given: Exactly one trip is with Yvonne.
Given: Every trip is cycling or hunting or kayaking or skydiving.
Given: Exactly one trip is cycling.
Given: Exactly one trip is hunting.
Given: Exactly one trip is kayaking.
Given: Exactly one trip is skydiving.
Given: Alpha is in 2001.
Given: Beta is in 2002.
Given: Gamma is in 2003.
Given: Delta is in 2004.
Given: Of the hunting trip and the 2004 trip, one was with Neal and the other was in Connecticut.
Given: The Florida trip was the hunting trip.
Given: Neither the trip with Bill nor the Florida trip is the 2001 trip.
Given: The trip with Yvonne is not in Kentucky.
Given: Of the skydiving trip and the Maine trip, one was in 2003 and the other was with Bill.
Given: The 2003 trip is not the cycling trip.
Prove: Beta is in Florida.
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

pub const LOGIC_LEIBNIZ: &str = r#"## Theorem: Leibniz_Identity
Given: Clark is Superman.
Given: Clark is mortal.
Prove: Superman is mortal.
Proof: Auto.
"#;

pub const LOGIC_BARBER: &str = r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#;

// ============================================================
// Code Mode Examples (Imperative LOGOS)
// ============================================================

pub const CODE_HELLO: &str = r#"## Main

Let greeting be "Hello, LOGOS!".
Show greeting.

Let x be 10.
Let y be 20.
Let sum be x + y.

Show "The sum is:".
Show sum.
"#;

/// Hello World using = assignment syntax (mutability auto-inferred)
const CODE_HELLO2: &str = r#"## Main

greeting = "Hello, World!".
Show greeting.

counter = 0.
Set counter to counter + 1.
Show counter.
"#;

pub const CODE_FIBONACCI: &str = r#"## Main

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

/// FizzBuzz using optional Repeat and Otherwise If / Else If chains
const CODE_FIZZBUZZ2: &str = r#"## Main

for i from 1 to 20:
    If i / 15 * 15 equals i:
        Show "FizzBuzz".
    Otherwise If i / 3 * 3 equals i:
        Show "Fizz".
    Else If i / 5 * 5 equals i:
        Show "Buzz".
    Otherwise:
        Show i.
"#;

/// FizzBuzz using optional Repeat and Python-style elif
const CODE_FIZZBUZZ3: &str = r#"## Main

for i from 1 to 20:
    If i / 15 * 15 equals i:
        Show "FizzBuzz".
    elif i / 3 * 3 equals i:
        Show "Fizz".
    elif i / 5 * 5 equals i:
        Show "Buzz".
    Else:
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

pub const CODE_CRDT_COUNTERS: &str = r#"## Definition
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

pub const MATH_NAT: &str = r#"-- Natural Numbers
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

pub const MATH_BOOL: &str = r#"Inductive MyBool := Yes : MyBool | No : MyBool.

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

pub const MATH_PROP_LOGIC: &str = r#"-- Propositional Logic Types
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

-- A small boolean type to pair with numbers.
Inductive MyBool := Yes : MyBool | No : MyBool.

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

const MATH_CIRCUIT: &str = r#"-- Logic Gates as a Circuit
-- Encode the gates formally, then hit the crab Compile button to extract
-- runnable Rust. The same code could run in WASM or drive a hardware circuit —
-- this is "encode it, extract it, run it".

Inductive MyBit := Lo : MyBit | Hi : MyBit.

Definition not1 : MyBit -> MyBit := fun a : MyBit =>
  match a return (fun _ : MyBit => MyBit) with | Lo => Hi | Hi => Lo.

Definition and2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit =>
  match a return (fun _ : MyBit => MyBit) with | Lo => Lo | Hi => b.

Definition or2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit =>
  match a return (fun _ : MyBit => MyBit) with | Lo => b | Hi => Hi.

-- XOR built from the primitive gates
Definition xor2 : MyBit -> MyBit -> MyBit := fun a : MyBit => fun b : MyBit =>
  or2 (and2 a (not1 b)) (and2 (not1 a) b).

-- Try it: XOR truth table
Eval xor2 Lo Hi.
Eval xor2 Hi Hi.
"#;

const MATH_PROPERTY: &str = r#"-- Proven theorems become RUNNABLE Rust property checks.
-- Define a function, prove a theorem about it, then hit Compile: the proof turns
-- into a `check_*` function over the extracted code that the demo main runs.

Inductive Num := Z : Num | S : Num -> Num.

Definition add : Num -> Num -> Num := fix rec => fun n : Num => fun m : Num =>
  match n return (fun _ : Num => Num) with | Z => m | S k => S (rec k m).

Definition one : Num := S Z.

-- A proven theorem: adding Z on the left is the identity (true by computation,
-- so `refl` proves it). Compile → `fn check_add_zero_l(n) -> bool { add(Z, n) == n }`.
Definition add_zero_l : (forall n : Num, Eq Num (add Z n) n) := fun n : Num => refl Num n.

Eval add one one.
"#;

const MATH_COLLATZ: &str = r###"-- ============================================
-- THE COLLATZ CONJECTURE (Literate Mode)
-- ============================================
-- The Collatz sequence: if n is even, n/2; if odd, 3n+1
-- Conjecture: All positive integers eventually reach 1
-- This is one of mathematics' most famous unsolved problems!
--
-- This example demonstrates the Literate Specification syntax:
-- - "A X is either" for sum types (instead of Inductive)
-- - "## To" for functions (instead of Definition)
-- - "Consider/When/Yield" for pattern matching (instead of match)
-- - Implicit recursion (no explicit "fix")

-- ============================================
-- TYPE DEFINITIONS
-- ============================================

-- A Decision represents a binary choice (Yes/No)
-- Replaces: Inductive MyBool := Yes : MyBool | No : MyBool.
A Decision is either Yes or No.

-- ============================================
-- BOOLEAN OPERATIONS
-- ============================================

-- Negate a decision: Yes becomes No, No becomes Yes
## To negate (d: Decision) -> Decision:
    Consider d:
        When Yes: Yield No.
        When No: Yield Yes.

-- ============================================
-- NATURAL NUMBERS (using kernel's Nat)
-- ============================================

-- Addition: add two natural numbers
-- Note: Recursion is implicit - we just call "add" in the body
## To add (n: Nat) and (m: Nat) -> Nat:
    Consider n:
        When Zero: Yield m.
        When Succ k: Yield Succ (add(k, m)).

-- ============================================
-- PARITY CHECKS
-- ============================================

-- Check if a number is even (Yes) or odd (No)
-- isEven(0) = Yes, isEven(n+1) = negate(isEven(n))
## To check_parity (n: Nat) -> Decision:
    Consider n:
        When Zero: Yield Yes.
        When Succ k: Yield negate(check_parity(k)).

-- Check if a number is odd
## To is_odd (n: Nat) -> Decision:
    Yield negate(check_parity(n)).

-- ============================================
-- ARITHMETIC HELPERS
-- ============================================

-- Half: floor division by 2
-- half(0) = 0
-- half(n+1) = if odd(n) then 1+half(n) else half(n)
## To halve (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k:
            Consider is_odd(k):
                When Yes: Yield Succ (halve(k)).
                When No: Yield halve(k).

-- Double: 2n = n + n
## To double (n: Nat) -> Nat:
    Yield add(n, n).

-- Triple: 3n = n + 2n
## To triple (n: Nat) -> Nat:
    Yield add(n, double(n)).

-- Multiplication: n * m
-- Used for defining 6k + 4
## To mul (n: Nat) and (m: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k: Yield add(m, mul(k, m)).

-- Useful constants
Definition four : Nat := Succ (Succ (Succ (Succ Zero))).
Definition five : Nat := Succ (four).
Definition six : Nat := Succ (five).
Definition sixteen : Nat := double (double four).

-- ============================================
-- THE COLLATZ STEP
-- ============================================

-- The Collatz step function:
-- if even: n/2
-- if odd: 3n+1
## To take_collatz_step (n: Nat) -> Nat:
    Consider check_parity(n):
        When Yes: Yield halve(n).
        When No: Yield Succ (triple(n)).

-- ============================================
-- VERIFICATION
-- ============================================

Check negate.
Check check_parity.
Check is_odd.
Check halve.
Check take_collatz_step.

-- Test check_parity: 0=even, 1=odd, 2=even, 3=odd
Eval (check_parity Zero).
Eval (check_parity (Succ Zero)).
Eval (check_parity (Succ (Succ Zero))).
Eval (check_parity (Succ (Succ (Succ Zero)))).

-- Test halve: halve(4) = 2
Eval (halve (Succ (Succ (Succ (Succ Zero))))).

-- Test take_collatz_step
-- 2 -> 1 (even, so divide by 2)
Eval (take_collatz_step (Succ (Succ Zero))).

-- 4 -> 2 (even, so divide by 2)
Eval (take_collatz_step (Succ (Succ (Succ (Succ Zero))))).

-- ============================================
-- PART 2: THE LOGICAL ENGINE
-- ============================================
-- The step function above is the "computational engine" - it runs.
-- But to REASON about the Collatz conjecture, we need a "logical engine".
--
-- Why can't we just write a function that counts steps to 1?
-- Because the termination checker cannot verify it always halts!
-- The compiler would reject it - that's the "termination wall".
--
-- Instead, we define a PREDICATE: "n eventually reaches 1"
-- This describes the PROPERTY without requiring computation to terminate.

-- ============================================
-- REACHABILITY PREDICATE
-- ============================================
-- A proof that 'n' reaches 1 is a tree:
--   - Done: n IS 1 (with equality proof)
--   - Step: if take_collatz_step(n) reaches 1, so does n

Inductive ReachesOne (n : Nat) :=
    | Done : Eq Nat n (Succ Zero) -> ReachesOne n
    | Step : ReachesOne (take_collatz_step n) -> ReachesOne n.

-- ============================================
-- CONCRETE PROOFS
-- ============================================
-- Let's prove specific numbers reach 1 by constructing proof trees.
-- This is "running" the conjecture in the type system!

-- Proof that 1 reaches 1 (trivial base case)
-- Note: Constructors for parameterized inductives take the parameter first
Definition one_reaches : ReachesOne (Succ Zero) :=
    Done (Succ Zero) (refl Nat (Succ Zero)).

-- Proof that 2 reaches 1
-- Chain: 2 -> 1 (since 2 is even, take_collatz_step(2) = halve(2) = 1)
Definition two_reaches : ReachesOne (Succ (Succ Zero)) :=
    Step (Succ (Succ Zero)) (Done (Succ Zero) (refl Nat (Succ Zero))).

-- Proof that 4 reaches 1
-- Chain: 4 -> 2 -> 1
Definition four_reaches : ReachesOne (Succ (Succ (Succ (Succ Zero)))) :=
    Step (Succ (Succ (Succ (Succ Zero))))
        (Step (Succ (Succ Zero))
            (Done (Succ Zero) (refl Nat (Succ Zero)))).

-- Proof that 8 reaches 1
-- Chain: 8 -> 4 -> 2 -> 1
Definition eight_reaches : ReachesOne (Succ (Succ (Succ (Succ (Succ (Succ (Succ (Succ Zero)))))))) :=
    Step (Succ (Succ (Succ (Succ (Succ (Succ (Succ (Succ Zero))))))))
        (Step (Succ (Succ (Succ (Succ Zero))))
            (Step (Succ (Succ Zero))
                (Done (Succ Zero) (refl Nat (Succ Zero))))).

-- Verify the proofs type-check
Check one_reaches.
Check two_reaches.
Check four_reaches.
Check eight_reaches.

-- ============================================
-- THE INVERSE COLLATZ TREE
-- ============================================
-- A different perspective: instead of proving numbers GO to 1,
-- prove numbers COME FROM 1 by reversing the rules.
--
-- From any n in the tree:
--   - 2n is always in the tree (reverse the "even" rule)
--   - (n-1)/3 is in the tree if it's a positive odd integer
--
-- This tree is well-founded, so we CAN do structural induction!

Inductive InverseCollatz (n : Nat) :=
    | Root : Eq Nat n (Succ Zero) -> InverseCollatz n
    | FromDouble : InverseCollatz n -> InverseCollatz (double n)
    | FromTripleSucc : InverseCollatz (Succ (triple n)) -> InverseCollatz n.

-- ============================================
-- STRUCTURAL THEOREMS
-- ============================================
-- We can prove general facts about the inverse tree.

-- Theorem: 1 is in the inverse tree
Definition one_in_tree : InverseCollatz (Succ Zero) :=
    Root (Succ Zero) (refl Nat (Succ Zero)).

-- Theorem: 2 is in the tree (since 2 = double 1)
Definition two_in_tree : InverseCollatz (Succ (Succ Zero)) :=
    FromDouble (Succ Zero) one_in_tree.

-- Theorem: 4 is in the tree (since 4 = double 2)
Definition four_in_tree : InverseCollatz (Succ (Succ (Succ (Succ Zero)))) :=
    FromDouble (Succ (Succ Zero)) two_in_tree.

-- Verify the tree membership proofs
Check one_in_tree.
Check two_in_tree.
Check four_in_tree.

-- ============================================
-- THEOREM 1: All Powers of Two
-- ============================================
-- power_of_two computes 2^n: 2^0 = 1, 2^(n+1) = double(2^n)

Definition power_of_two : Nat -> Nat :=
    fix rec => fun n : Nat =>
    match n return Nat with
    | Zero => Succ Zero
    | Succ k => double (rec k)
    end.

-- Theorem: All 2^n are in the inverse tree (proof by induction on n)
-- Base: 2^0 = 1 is the Root
-- Step: If 2^k is in tree, then 2^(k+1) = double(2^k) is in tree via FromDouble
Definition all_powers_of_two : forall n : Nat, InverseCollatz (power_of_two n) :=
    fix proof => fun n : Nat =>
    match n return (fun k : Nat => InverseCollatz (power_of_two k)) with
    | Zero => Root (Succ Zero) (refl Nat (Succ Zero))
    | Succ k => FromDouble (power_of_two k) (proof k)
    end.

Check power_of_two.
Check all_powers_of_two.

-- Verify: power_of_two 3 = 8
Eval (power_of_two (Succ (Succ (Succ Zero)))).

-- ============================================
-- THEOREM 2: Grandchild Growth
-- ============================================
-- If n is in the tree, then 4n = double(double(n)) is also in the tree
-- This shows the tree has "depth" - we can always extend further

Definition grandchild_growth : forall n : Nat, InverseCollatz n -> InverseCollatz (double (double n)) :=
    fun n : Nat => fun pf : InverseCollatz n =>
    FromDouble (double n) (FromDouble n pf).

Check grandchild_growth.

-- ============================================
-- THEOREM 3: Odd Numbers via FromTripleSucc
-- ============================================
-- 5 is in the tree because 3*5+1 = 16 = 2^4 is in the tree
-- This demonstrates the "reverse odd step" rule

-- 5 is in tree: FromTripleSucc requires InverseCollatz (Succ (triple 5))
-- triple 5 = 15, so Succ (triple 5) = 16 = 2^4
Definition five_in_tree : InverseCollatz five :=
    FromTripleSucc five (all_powers_of_two four).

Check five_in_tree.

-- ============================================
-- WHAT WE PROVED AND DIDN'T PROVE
-- ============================================
-- PROVED:
--   - Specific numbers (1, 2, 4, 8) reach 1 via ReachesOne
--   - All powers of 2 are in the inverse tree (all_powers_of_two)
--   - If n is in tree, so is 4n (grandchild_growth)
--   - 5 is in the tree via the FromTripleSucc rule (five_in_tree)
--   - The inverse tree contains infinitely many numbers
--
-- DID NOT PROVE:
--   - That ALL positive integers reach 1 (the full conjecture)
--   - That the inverse tree covers ALL positive integers
--
-- The full conjecture remains open in mathematics!
-- But this demonstrates how proof assistants let us
-- verify partial results with absolute certainty.

-- ============================================
-- PART 3: TOPOLOGY OF THE INVERSE GRAPH
-- ============================================
-- The inverse Collatz graph has special "skeleton" structure.
-- "Skeleton nodes" (junctions) are nodes of the form 6k + 4.
-- These are the ONLY nodes that can spawn odd children!
--
-- Key insight: For (n-1)/3 to be a positive odd integer,
-- we need n = 6k + 4 for some k.

-- ============================================
-- SKELETON PREDICATE
-- ============================================
-- n is a skeleton node if n = 6k + 4 for some k

Inductive IsSkeleton (n : Nat) :=
    | Witness : forall k : Nat, Eq Nat n (add (mul six k) four) -> IsSkeleton n.

-- ============================================
-- ODDNESS PREDICATE
-- ============================================
-- n is odd if n = 2k + 1 for some k

Inductive IsOdd (n : Nat) :=
    | OddWitness : forall k : Nat, Eq Nat n (Succ (double k)) -> IsOdd n.

Check IsSkeleton.
Check IsOdd.

-- ============================================
-- CONCRETE SKELETON EXAMPLES
-- ============================================
-- Demonstrate specific skeleton nodes: 4, 10, 16, 22...
-- These are the nodes of the form 6k + 4.

-- 4 is skeleton: 4 = 6*0 + 4
Definition four_is_skeleton : IsSkeleton four :=
    Witness four Zero (refl Nat four).

Check four_is_skeleton.

-- 10 is skeleton: 10 = 6*1 + 4
-- First define 10
Definition ten : Nat := add six four.

Definition ten_is_skeleton : IsSkeleton ten :=
    Witness ten (Succ Zero) (refl Nat ten).

Check ten_is_skeleton.

-- ============================================
-- CONCRETE ODD EXAMPLES
-- ============================================
-- Demonstrate specific odd numbers and their skeleton mappings.

-- 1 is odd: 1 = 2*0 + 1
Definition one_is_odd : IsOdd (Succ Zero) :=
    OddWitness (Succ Zero) Zero (refl Nat (Succ Zero)).

Check one_is_odd.

-- 3 is odd: 3 = 2*1 + 1
Definition three : Nat := Succ (Succ (Succ Zero)).
Definition three_is_odd : IsOdd three :=
    OddWitness three (Succ Zero) (refl Nat three).

Check three_is_odd.

-- ============================================
-- SKELETON REDUCTION EXAMPLES
-- ============================================
-- Verify that 3m+1 lands on skeleton nodes for specific odd m.
--
-- For m=1: 3*1+1 = 4 = 6*0 + 4 (skeleton!)
-- For m=3: 3*3+1 = 10 = 6*1 + 4 (skeleton!)

-- Verify: take_collatz_step(1) = 4 (since 1 is odd)
Eval (take_collatz_step (Succ Zero)).

-- Verify: take_collatz_step(3) = 10 (since 3 is odd)
Eval (take_collatz_step three).

-- ============================================
-- THEOREM 5: GREEN HIGHWAY
-- ============================================
-- Powers of 4 provide an infinite highway of skeleton nodes.
--
-- Power of four function: 4^n

Definition power_of_four : Nat -> Nat :=
    fix rec => fun n : Nat =>
    match n return Nat with
    | Zero => Succ Zero
    | Succ k => double (double (rec k))
    end.

Check power_of_four.

-- Verify computations
Eval (power_of_four Zero).
Eval (power_of_four (Succ Zero)).
Eval (power_of_four (Succ (Succ Zero))).
Eval (power_of_four (Succ (Succ (Succ Zero)))).

-- Base case: 4^1 = 4 is skeleton (6*0 + 4)
Definition pow4_1_skeleton : IsSkeleton (power_of_four (Succ Zero)) :=
    Witness (power_of_four (Succ Zero)) Zero (refl Nat four).

Check pow4_1_skeleton.

-- 4^2 = 16 is skeleton: 16 = 6*2 + 4
Definition two : Nat := Succ (Succ Zero).
Definition pow4_2_skeleton : IsSkeleton (power_of_four (Succ (Succ Zero))) :=
    Witness (power_of_four (Succ (Succ Zero))) two (refl Nat sixteen).

Check pow4_2_skeleton.

-- ============================================
-- PART 3 SUMMARY
-- ============================================
-- We have demonstrated the "Skeleton Network" topology:
-- 1. Skeleton nodes are defined by n = 6k + 4 for some k.
-- 2. Examples: 4, 10, 16, 22... are all skeleton nodes.
-- 3. Odd numbers map to skeleton nodes via 3m+1 (verified for m=1, m=3).
-- 4. The Green Highway (4^n) provides skeleton nodes: 4, 16, 64...
-- 5. To solve Collatz, we only need to check if the Skeleton is connected.
"###;

// ============================================================
// LITERATE GÖDEL EXAMPLES (Phase 2)
// ============================================================

pub const MATH_GODEL_LITERATE: &str = r###"-- ============================================
-- GÖDEL SENTENCE CONSTRUCTION (Literate Mode)
-- ============================================
-- Building the self-referential sentence G that says "I am not provable"
--
-- This example demonstrates the fully Literate meta-logic syntax:
-- - "## To be Predicate" for predicate definitions
-- - "Let X be Y" for constant definitions
-- - "the Name X" for syntax names (maps to SName)
-- - "Variable N" for syntax variables (maps to SVar)
-- - "Apply(f, x)" for syntax application (maps to SApp)
-- - "the diagonalization of T" for diagonal lemma
-- - "there exists a d: T such that P" for existential quantification
-- - "X equals Y" for equality propositions

-- ============================================
-- 1. THE PROVABILITY PREDICATE
-- ============================================
-- "s is provable if there exists a derivation d that concludes s"

## To be Provable (s: Syntax) -> Prop:
    Yield there exists a d: Derivation such that (concludes(d) equals s).

-- ============================================
-- 2. THE TEMPLATE T
-- ============================================
-- T encodes "Not(Provable(x))" as syntax

Let Not_Name be the Name "Not".
Let Provable_Name be the Name "Provable".
Let T be Apply(Not_Name, Apply(Provable_Name, Variable 0)).

-- ============================================
-- 3. THE GÖDEL SENTENCE G
-- ============================================
-- G = T[code(T)/x] via the diagonal lemma
-- G says "I am not provable"

Let G be the diagonalization of T.

-- ============================================
-- VERIFICATION
-- ============================================

Check Provable.
Check T.
Check G.
Check Provable(G).
"###;

pub const MATH_INCOMPLETENESS_LITERATE: &str = r###"-- ============================================
-- GÖDEL'S FIRST INCOMPLETENESS THEOREM (Literate Mode)
-- ============================================
-- "If LOGOS is consistent, then G is not provable"
--
-- This example demonstrates fully Literate syntax:
-- - "## To be Predicate" for predicate definitions
-- - "## To be Consistent -> Prop:" for nullary predicates
-- - "## Theorem:" blocks with "Statement:"
-- - "X implies Y" for logical implication
-- - "X equals Y" for equality propositions

-- ============================================
-- 1. THE PROVABILITY PREDICATE
-- ============================================

## To be Provable (s: Syntax) -> Prop:
    Yield there exists a d: Derivation such that (concludes(d) equals s).

-- ============================================
-- 2. CONSISTENCY DEFINITION
-- ============================================
-- A system is consistent if it cannot prove False

Let False_Name be the Name "False".

## To be Consistent -> Prop:
    Yield Not(Provable(False_Name)).

-- ============================================
-- 3. THE GÖDEL SENTENCES
-- ============================================

Let T be Apply(the Name "Not", Apply(the Name "Provable", Variable 0)).
Let G be the diagonalization of T.

-- ============================================
-- 4. THE THEOREM STATEMENT
-- ============================================

## Theorem: Godel_First_Incompleteness
    Statement: Consistent implies Not(Provable(G)).

-- ============================================
-- VERIFICATION
-- ============================================

Check Godel_First_Incompleteness.
Check Consistent.
Check Provable(G).
Check Not(Provable(G)).
"###;

const MATH_RING: &str = r###"-- ============================================
-- RING TACTIC: Polynomial Equality by Normalization
-- ============================================
-- The ring tactic proves polynomial equalities automatically!
-- It works by normalizing both sides to canonical polynomial form
-- and checking if they're structurally equal.
--
-- Supported operations: add, sub, mul (no division)
-- This is a decision procedure - it either proves the equality or fails.

-- ============================================
-- BASIC SETUP
-- ============================================

-- Type annotation (for the Eq constructor)
Definition T : Syntax := SName "Int".

-- Variables using de Bruijn indices
Definition x : Syntax := SVar 0.
Definition y : Syntax := SVar 1.
Definition z : Syntax := SVar 2.

-- ============================================
-- EXAMPLE 1: REFLEXIVITY (x = x)
-- ============================================

Definition refl_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) x) x.
Definition refl_proof : Derivation := try_ring refl_goal.
Definition refl_result : Syntax := concludes refl_proof.

Check refl_proof.
Eval refl_result.

-- ============================================
-- EXAMPLE 2: COMMUTATIVITY OF ADDITION (x + y = y + x)
-- ============================================

Definition add_xy : Syntax := SApp (SApp (SName "add") x) y.
Definition add_yx : Syntax := SApp (SApp (SName "add") y) x.
Definition comm_add_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) add_xy) add_yx.

-- The ring tactic proves this automatically!
Definition comm_add_proof : Derivation := try_ring comm_add_goal.
Definition comm_add_result : Syntax := concludes comm_add_proof.

Check comm_add_proof.
Eval comm_add_result.

-- ============================================
-- EXAMPLE 3: COMMUTATIVITY OF MULTIPLICATION (x * y = y * x)
-- ============================================

Definition mul_xy : Syntax := SApp (SApp (SName "mul") x) y.
Definition mul_yx : Syntax := SApp (SApp (SName "mul") y) x.
Definition comm_mul_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) mul_xy) mul_yx.

Definition comm_mul_proof : Derivation := try_ring comm_mul_goal.
Definition comm_mul_result : Syntax := concludes comm_mul_proof.

Check comm_mul_proof.
Eval comm_mul_result.

-- ============================================
-- EXAMPLE 4: DISTRIBUTIVITY (x * (y + z) = x*y + x*z)
-- ============================================

-- LHS: x * (y + z)
Definition y_plus_z : Syntax := SApp (SApp (SName "add") y) z.
Definition dist_lhs : Syntax := SApp (SApp (SName "mul") x) y_plus_z.

-- RHS: x*y + x*z
Definition x_times_y : Syntax := SApp (SApp (SName "mul") x) y.
Definition x_times_z : Syntax := SApp (SApp (SName "mul") x) z.
Definition dist_rhs : Syntax := SApp (SApp (SName "add") x_times_y) x_times_z.

Definition dist_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) dist_lhs) dist_rhs.

Definition dist_proof : Derivation := try_ring dist_goal.
Definition dist_result : Syntax := concludes dist_proof.

Check dist_proof.
Eval dist_result.

-- ============================================
-- EXAMPLE 5: THE COLLATZ ALGEBRA STEP
-- ============================================
-- The key algebraic identity in Collatz analysis:
-- 3(2k+1) + 1 = 6k + 4
--
-- This proves that applying the Collatz odd step (3n+1)
-- to an odd number of the form 2k+1 yields 6k+4.

Definition k : Syntax := SVar 0.

-- Build LHS: 3 * (2*k + 1) + 1
Definition two_k : Syntax := SApp (SApp (SName "mul") (SLit 2)) k.
Definition two_k_plus_1 : Syntax := SApp (SApp (SName "add") two_k) (SLit 1).
Definition three_times : Syntax := SApp (SApp (SName "mul") (SLit 3)) two_k_plus_1.
Definition collatz_lhs : Syntax := SApp (SApp (SName "add") three_times) (SLit 1).

-- Build RHS: 6*k + 4
Definition six_k : Syntax := SApp (SApp (SName "mul") (SLit 6)) k.
Definition collatz_rhs : Syntax := SApp (SApp (SName "add") six_k) (SLit 4).

-- The equality goal
Definition collatz_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) collatz_lhs) collatz_rhs.

-- Ring proves it!
Definition collatz_proof : Derivation := try_ring collatz_goal.
Definition collatz_result : Syntax := concludes collatz_proof.

Check collatz_proof.
Eval collatz_result.

-- ============================================
-- EXAMPLE 6: ASSOCIATIVITY ((x + y) + z = x + (y + z))
-- ============================================

Definition xy_plus_z : Syntax := SApp (SApp (SName "add") (SApp (SApp (SName "add") x) y)) z.
Definition x_plus_yz : Syntax := SApp (SApp (SName "add") x) (SApp (SApp (SName "add") y) z).
Definition assoc_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) xy_plus_z) x_plus_yz.

Definition assoc_proof : Derivation := try_ring assoc_goal.
Definition assoc_result : Syntax := concludes assoc_proof.

Check assoc_proof.
Eval assoc_result.

-- ============================================
-- EXAMPLE 7: SUBTRACTION CANCELLATION (x - x = 0)
-- ============================================

Definition x_minus_x : Syntax := SApp (SApp (SName "sub") x) x.
Definition zero_lit : Syntax := SLit 0.
Definition cancel_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) x_minus_x) zero_lit.

Definition cancel_proof : Derivation := try_ring cancel_goal.
Definition cancel_result : Syntax := concludes cancel_proof.

Check cancel_proof.
Eval cancel_result.

-- ============================================
-- SUMMARY
-- ============================================
-- The ring tactic is a decision procedure for polynomial ring equalities.
-- It handles: constants, variables, addition, subtraction, multiplication.
-- It does NOT handle: division, modulo, or non-polynomial operations.
--
-- Key insight: Both sides are normalized to a canonical polynomial form
-- (sum of monomials with sorted variable indices), and compared structurally.
-- If they match, the equality is provable. If not, it fails.
"###;

const MATH_LIA: &str = r###"-- ============================================
-- LIA TACTIC: Linear Integer Arithmetic
-- ============================================
-- The lia tactic proves linear inequalities automatically!
-- It uses Fourier-Motzkin elimination to decide validity.
--
-- Supported: Lt (<), Le (<=), Gt (>), Ge (>=)
-- Expressions must be LINEAR: constants, variables, c*x (no x*y)

-- ============================================
-- BASIC SETUP
-- ============================================

-- Variables using de Bruijn indices
Definition x : Syntax := SVar 0.
Definition y : Syntax := SVar 1.
Definition z : Syntax := SVar 2.

-- ============================================
-- EXAMPLE 1: REFLEXIVITY (x <= x)
-- ============================================

Definition le_refl_goal : Syntax := SApp (SApp (SName "Le") x) x.
Definition le_refl_proof : Derivation := try_lia le_refl_goal.
Definition le_refl_result : Syntax := concludes le_refl_proof.

Check le_refl_proof.
Eval le_refl_result.

-- ============================================
-- EXAMPLE 2: CONSTANT INEQUALITY (2 < 5)
-- ============================================

Definition const_lt_goal : Syntax := SApp (SApp (SName "Lt") (SLit 2)) (SLit 5).
Definition const_lt_proof : Derivation := try_lia const_lt_goal.
Definition const_lt_result : Syntax := concludes const_lt_proof.

Check const_lt_proof.
Eval const_lt_result.

-- ============================================
-- EXAMPLE 3: SUCCESSOR (x < x + 1)
-- ============================================

Definition x_plus_1 : Syntax := SApp (SApp (SName "add") x) (SLit 1).
Definition succ_goal : Syntax := SApp (SApp (SName "Lt") x) x_plus_1.
Definition succ_proof : Derivation := try_lia succ_goal.
Definition succ_result : Syntax := concludes succ_proof.

Check succ_proof.
Eval succ_result.

-- ============================================
-- EXAMPLE 4: LINEAR COEFFICIENT (2*x <= 2*x)
-- ============================================

Definition two_x : Syntax := SApp (SApp (SName "mul") (SLit 2)) x.
Definition linear_goal : Syntax := SApp (SApp (SName "Le") two_x) two_x.
Definition linear_proof : Derivation := try_lia linear_goal.
Definition linear_result : Syntax := concludes linear_proof.

Check linear_proof.
Eval linear_result.

-- ============================================
-- EXAMPLE 5: PREDECESSOR (x - 1 < x)
-- ============================================

Definition x_minus_1 : Syntax := SApp (SApp (SName "sub") x) (SLit 1).
Definition pred_goal : Syntax := SApp (SApp (SName "Lt") x_minus_1) x.
Definition pred_proof : Derivation := try_lia pred_goal.
Definition pred_result : Syntax := concludes pred_proof.

Check pred_proof.
Eval pred_result.

-- ============================================
-- EXAMPLE 6: EQUALITY BOUND (5 <= 5)
-- ============================================

Definition eq_bound_goal : Syntax := SApp (SApp (SName "Le") (SLit 5)) (SLit 5).
Definition eq_bound_proof : Derivation := try_lia eq_bound_goal.
Definition eq_bound_result : Syntax := concludes eq_bound_proof.

Check eq_bound_proof.
Eval eq_bound_result.

-- ============================================
-- SUMMARY
-- ============================================
-- The lia tactic is a decision procedure for linear integer arithmetic.
-- It handles: constants, variables, addition, subtraction, c*x multiplication.
-- It does NOT handle: variable * variable (nonlinear), division, modulo.
--
-- Key insight: Fourier-Motzkin elimination projects out variables one by one,
-- combining lower and upper bounds until only constant constraints remain.
-- If these are contradictory, the negation is unsatisfiable, proving the goal.
"###;

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

pub const CODE_CRDT_TALLY: &str = r#"# Tally (Bidirectional Counter)
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

// ============================================================
// Temporal Types Example
// ============================================================

const CODE_TEMPORAL: &str = r#"## Main

Show "=== Duration Literals (SI Units) ===".
Let nano be 50ns.
Show nano.

Let micro be 100us.
Show micro.

Let milli be 500ms.
Show milli.

Let sec be 1s.
Show sec.

Show "".
Show "=== Sleep with Duration Variables ===".

Let short_pause be 200ms.
Let medium_pause be 500ms.

Show "Starting...".
Sleep short_pause.
Show "After 200ms pause".
Sleep medium_pause.
Show "After 500ms pause".
Sleep short_pause.
Show "Done with variable sleeps!".

Show "".
Show "=== Duration Math ===".
Let a be 500ms.
Let b be 500ms.
Let total be a + b.
Show "500ms + 500ms =".
Show total.

Let fast be 100ms.
Let doubled be fast + fast.
Show "100ms doubled =".
Show doubled.

Show "".
Show "=== Duration Comparisons ===".
Let quick be 100ms.
Let slow be 1s.

If quick < slow:
    Show "100ms is less than 1s".

If slow > quick:
    Show "1s is greater than 100ms".

Show "".
Show "=== Date Literals ===".
Let graduation be 2026-05-20.
Show graduation.

Let epoch be 1970-01-01.
Show epoch.

Let new_year be 2026-01-01.
Show new_year.

Show "".
Show "=== Date Comparisons ===".
If graduation > epoch:
    Show "Graduation is after the Unix epoch".

If new_year < graduation:
    Show "New Year comes before graduation".

Show "".
Show "=== Calendar Spans ===".
Let vacation be 2 weeks.
Show vacation.

Let project be 3 months.
Show project.

Let sprint be 2 weeks and 3 days.
Show sprint.

Let long_project be 1 year and 2 months and 5 days.
Show long_project.

Show "".
Show "=== Today Builtin ===".
Let current_date be today.
Show "Today's date:".
Show current_date.

Show "".
Show "=== Date + Span Arithmetic ===".
Let start be 2026-01-15.
Let deadline be start + 2 months.
Show "Start + 2 months =".
Show deadline.

Let exam be 2026-05-20.
Let reminder be exam - 3 days.
Show "Exam - 3 days =".
Show reminder.

Let project_start be 2026-01-10.
Let project_end be project_start + 1 month and 5 days.
Show "Project end:".
Show project_end.

Show "".
Show "=== Time-of-Day Literals ===".
Let morning be 9am.
Show morning.

Let afternoon be 4pm.
Show afternoon.

Let lunch be noon.
Show lunch.

Let late_night be midnight.
Show late_night.

Let meeting_time be 9:30am.
Show meeting_time.

Show "".
Show "=== Date + Time (Moments) ===".
Let meeting be 2026-05-20 at 4pm.
Show "Meeting moment:".
Show meeting.

Let conference be 2026-03-15 at 9:30am.
Show "Conference:".
Show conference.

Show "".
Show "=== Time Comparisons ===".
Let early be 9am.
Let late be 5pm.

If early < late:
    Show "9am is before 5pm".

If late > noon:
    Show "5pm is after noon".

Show "".
Show "All temporal tests complete!".
"#;

// ============================================================
// MATH_CC: Congruence Closure Tactic Example
// ============================================================

const MATH_CC: &str = r###"-- ============================================
-- CC TACTIC: Congruence Closure
-- ============================================
-- The cc tactic proves equalities over uninterpreted functions!
-- It uses the congruence rule: if a = b then f(a) = f(b)
--
-- Key insight: cc connects arithmetic proofs to function applications.
-- While ring proves "1 + 1 = 2", cc proves "f(1 + 1) = f(2)".

-- ============================================
-- EXAMPLE 1: REFLEXIVITY (f(x) = f(x))
-- ============================================
-- Any term equals itself

## Theorem: FxRefl
    Statement: (Eq (f x) (f x)).
    Proof: cc.

Check FxRefl.

-- ============================================
-- EXAMPLE 2: NESTED REFLEXIVITY (f(g(x)) = f(g(x)))
-- ============================================

## Theorem: FgxRefl
    Statement: (Eq (f (g x)) (f (g x))).
    Proof: cc.

Check FgxRefl.

-- ============================================
-- EXAMPLE 3: CONGRUENCE (x = y → f(x) = f(y))
-- ============================================
-- The core congruence rule: equal arguments give equal results

## Theorem: Congruence
    Statement: (implies (Eq x y) (Eq (f x) (f y))).
    Proof: cc.

Check Congruence.

-- ============================================
-- EXAMPLE 4: BINARY CONGRUENCE (a = b → add(a,c) = add(b,c))
-- ============================================
-- Congruence works for multi-argument functions too

## Theorem: BinaryCongruence
    Statement: (implies (Eq a b) (Eq (add a c) (add b c))).
    Proof: cc.

Check BinaryCongruence.

-- ============================================
-- EXAMPLE 5: TRANSITIVITY CHAIN (a = b → b = c → f(a) = f(c))
-- ============================================
-- Multiple hypotheses combine via transitivity

## Theorem: Transitivity
    Statement: (implies (Eq a b) (implies (Eq b c) (Eq (f a) (f c)))).
    Proof: cc.

Check Transitivity.

-- ============================================
-- SUMMARY
-- ============================================
-- The cc tactic proves equalities by:
-- 1. Building an E-graph from all subterms
-- 2. Merging equivalence classes from hypothesis equalities
-- 3. Propagating congruences: if a=b then f(a)=f(b)
-- 4. Checking if goal's LHS and RHS are equivalent
--
-- This completes the trinity of automated tactics:
-- - ring: polynomial equalities (normalization)
-- - lia: linear inequalities (Fourier-Motzkin)
-- - cc: function equalities (congruence closure)
"###;

const MATH_SIMP: &str = r###"-- ============================================
-- SIMP TACTIC: Term Rewriting
-- ============================================
-- The simp tactic normalizes goals by applying rewrite rules!
-- It unfolds definitions and simplifies arithmetic.
--
-- Key insight: simp turns complex terms into canonical forms,
-- making equalities trivially checkable by reflexivity.

-- ============================================
-- EXAMPLE 1: ARITHMETIC SIMPLIFICATION
-- ============================================
-- Constant expressions are evaluated

## Theorem: TwoPlusThree
    Statement: (Eq (add 2 3) 5).
    Proof: simp.

Check TwoPlusThree.

## Theorem: Nested
    Statement: (Eq (mul (add 1 1) 3) 6).
    Proof: simp.

Check Nested.

## Theorem: TenMinusFour
    Statement: (Eq (sub 10 4) 6).
    Proof: simp.

Check TenMinusFour.

-- ============================================
-- EXAMPLE 2: DEFINITION UNFOLDING
-- ============================================

## To double (n: Int) -> Int:
    Yield (add n n).

## Theorem: DoubleTwo
    Statement: (Eq (double 2) 4).
    Proof: simp.

Check DoubleTwo.

## To quadruple (n: Int) -> Int:
    Yield (double (double n)).

## Theorem: QuadTwo
    Statement: (Eq (quadruple 2) 8).
    Proof: simp.

Check QuadTwo.

## To zero_fn (n: Int) -> Int:
    Yield 0.

## Theorem: ZeroFnTest
    Statement: (Eq (zero_fn 42) 0).
    Proof: simp.

Check ZeroFnTest.

-- ============================================
-- EXAMPLE 3: WITH HYPOTHESES
-- ============================================
-- simp uses equalities from hypotheses as rewrite rules

## Theorem: SubstSimp
    Statement: (implies (Eq x 0) (Eq (add x 1) 1)).
    Proof: simp.

Check SubstSimp.

## Theorem: TwoHyps
    Statement: (implies (Eq x 1) (implies (Eq y 2) (Eq (add x y) 3))).
    Proof: simp.

Check TwoHyps.

-- ============================================
-- EXAMPLE 4: REFLEXIVE EQUALITIES
-- ============================================
-- simp handles reflexivity for free

## Theorem: XEqX
    Statement: (Eq x x).
    Proof: simp.

Check XEqX.

## Theorem: FxRefl
    Statement: (Eq (f x) (f x)).
    Proof: simp.

Check FxRefl.

-- ============================================
-- SUMMARY
-- ============================================
-- The simp tactic:
-- 1. Collects rewrite rules from definitions and hypotheses
-- 2. Applies rules bottom-up to both sides of equality
-- 3. Evaluates arithmetic on constants
-- 4. Checks if simplified terms are equal
--
-- Combined with ring, lia, and cc, this completes the core
-- automated reasoning toolkit!
--
-- - ring: polynomial equalities (normalization)
-- - lia: linear inequalities (Fourier-Motzkin)
-- - cc: function equalities (congruence closure)
-- - simp: term rewriting (bottom-up simplification)
"###;

const MATH_OMEGA: &str = r###"-- ============================================
-- OMEGA TACTIC: True Integer Arithmetic
-- ============================================
-- The omega tactic handles LINEAR INTEGER constraints!
-- Unlike lia (which uses rationals), omega knows that:
--   x > 1  means  x >= 2  for integers
--   2x = 3  has NO solution (3 is odd!)
--
-- This is essential for array bounds, loop indices,
-- and anything involving discrete counts.

-- ============================================
-- BASIC INEQUALITIES (same as lia)
-- ============================================

## Theorem: TwoLessThanFive
    Statement: (Lt 2 5).
    Proof: omega.

Check TwoLessThanFive.

## Theorem: XLessThanXPlusOne
    Statement: (Lt x (add x 1)).
    Proof: omega.

Check XLessThanXPlusOne.

## Theorem: XLeX
    Statement: (Le x x).
    Proof: omega.

Check XLeX.

-- ============================================
-- INTEGER-SPECIFIC REASONING
-- ============================================
-- These are IMPOSSIBLE with rational-based lia!

## Theorem: StrictToNonStrict
    Statement: (implies (Gt x 0) (Ge x 1)).
    Proof: omega.

Check StrictToNonStrict.

-- x > 0 in rationals allows x = 0.001
-- x > 0 in integers means x >= 1

## Theorem: LtConvertsToLe
    Statement: (implies (Lt x 5) (Le x 4)).
    Proof: omega.

Check LtConvertsToLe.

-- x < 5 in rationals allows x = 4.999
-- x < 5 in integers means x <= 4

## Theorem: CoeffBound
    Statement: (implies (Le (mul 3 x) 10) (Le x 3)).
    Proof: omega.

Check CoeffBound.

-- 3x <= 10 means x <= floor(10/3) = 3

## Theorem: TwoCoefficientBound
    Statement: (implies (Le (mul 2 x) 5) (Le x 2)).
    Proof: omega.

Check TwoCoefficientBound.

-- 2x <= 5 means x <= floor(5/2) = 2

-- ============================================
-- TRANSITIVITY AND CHAINS
-- ============================================

## Theorem: LtTrans
    Statement: (implies (Lt x y) (implies (Lt y z) (Lt x z))).
    Proof: omega.

Check LtTrans.

## Theorem: LeTrans
    Statement: (implies (Le x y) (implies (Le y z) (Le x z))).
    Proof: omega.

Check LeTrans.

-- ============================================
-- SUMMARY
-- ============================================
-- omega handles integer arithmetic properly:
-- 1. Strict-to-nonstrict conversion (x > n -> x >= n+1)
-- 2. Floor/ceil rounding in bounds
-- 3. Coefficient bounds with floor division
-- 4. Variable elimination via the Omega Test
--
-- Combined with ring, lia, cc, and simp, this completes
-- the core automated reasoning toolkit:
-- - ring: polynomial equalities (normalization)
-- - lia: linear rational inequalities (Fourier-Motzkin)
-- - cc: function equalities (congruence closure)
-- - simp: term rewriting (bottom-up simplification)
-- - omega: true integer arithmetic (Omega Test)
"###;

pub const MATH_AUTO: &str = r###"-- ============================================
-- AUTO TACTIC: The Infinity Gauntlet
-- ============================================
-- The auto tactic combines ALL decision procedures!
-- It tries each one in sequence until one succeeds:
--   1. True/False (trivial propositions)
--   2. simp  (simplification)
--   3. ring  (polynomial algebra)
--   4. cc    (congruence closure)
--   5. omega (integer arithmetic)
--   6. lia   (linear arithmetic)

-- ============================================
-- SIMPLIFICATION (auto -> simp)
-- ============================================

## Theorem: TrueIsTrue
    Statement: True.
    Proof: auto.

Check TrueIsTrue.

-- ============================================
-- RING ALGEBRA (auto -> ring)
-- ============================================

## Theorem: AddCommutative
    Statement: (Eq (add a b) (add b a)).
    Proof: auto.

Check AddCommutative.

## Theorem: AddAssociative
    Statement: (Eq (add (add a b) c) (add a (add b c))).
    Proof: auto.

Check AddAssociative.

## Theorem: MulDistributes
    Statement: (Eq (mul a (add b c)) (add (mul a b) (mul a c))).
    Proof: auto.

Check MulDistributes.

-- ============================================
-- CONGRUENCE CLOSURE (auto -> cc)
-- ============================================

## Theorem: FunctionReflexive
    Statement: (Eq (f x) (f x)).
    Proof: auto.

Check FunctionReflexive.

-- ============================================
-- INTEGER ARITHMETIC (auto -> omega)
-- ============================================

## Theorem: TwoLessThanFive
    Statement: (Lt 2 5).
    Proof: auto.

Check TwoLessThanFive.

## Theorem: StrictToNonStrict
    Statement: (implies (Gt x 0) (Ge x 1)).
    Proof: auto.

Check StrictToNonStrict.

## Theorem: XLessThanSucc
    Statement: (Lt x (add x 1)).
    Proof: auto.

Check XLessThanSucc.

-- ============================================
-- LINEAR ARITHMETIC (auto -> lia/omega)
-- ============================================

## Theorem: LeReflexive
    Statement: (Le x x).
    Proof: auto.

Check LeReflexive.

## Theorem: LeTransitive
    Statement: (implies (Le x y) (implies (Le y z) (Le x z))).
    Proof: auto.

Check LeTransitive.

-- ============================================
-- THE POWER OF AUTO
-- ============================================
-- With auto, you don't need to think about
-- which tactic to use. Just say:
--
--     Proof: auto.
--
-- And the system figures it out!
--
-- auto combines ALL five stones:
-- - ring: polynomial equalities
-- - lia: linear rational arithmetic
-- - cc: congruence closure
-- - simp: simplification
-- - omega: true integer arithmetic
--
-- This is the Infinity Gauntlet of tactics!
"###;

const MATH_INDUCTION: &str = r###"-- ============================================
-- INDUCTION TACTIC: The Time Machine
-- ============================================
-- Structural reasoning for inductive types.
-- Works for Nat, Bool, and any user-defined inductive.
--
-- The induction tactic automatically:
-- 1. Looks up constructors for the inductive type
-- 2. Generates one subgoal per constructor
-- 3. Provides induction hypotheses for recursive cases

-- ============================================
-- KERNEL INFRASTRUCTURE
-- ============================================
-- These are the building blocks for induction.

-- Check that induction helpers exist
Check try_induction.
Check induction_base_goal.
Check induction_step_goal.
Check induction_num_cases.

-- ============================================
-- NAT INDUCTION EXAMPLES
-- ============================================
-- Nat has 2 constructors: Zero, Succ

-- How many constructors does Nat have?
Definition nat_cases : Nat := induction_num_cases (SName "Nat").
Eval nat_cases.

-- The motive: what we're proving (λn:Nat. Le n n)
Definition le_motive : Syntax := SLam (SName "Nat") (SApp (SApp (SName "Le") (SVar 0)) (SVar 0)).

-- Base case goal: Le Zero Zero
Definition base_goal : Syntax := induction_base_goal (SName "Nat") le_motive.
Eval base_goal.

-- Step case goal: ∀k. P(k) → P(Succ k)
Definition step_goal : Syntax := induction_step_goal (SName "Nat") le_motive (Succ Zero).
Eval step_goal.

-- ============================================
-- BOOL INDUCTION
-- ============================================
-- Bool has 2 constructors: true, false

-- How many constructors does Bool have?
Definition bool_cases : Nat := induction_num_cases (SName "Bool").
Eval bool_cases.

-- Check the Bool type
Check Bool.
Check true.
Check false.

-- ============================================
-- BUILDING A COMPLETE PROOF
-- ============================================
-- Let's build an induction proof manually using the kernel.

-- Base case: Le Zero Zero (auto can solve this)
Definition base_proof : Derivation := try_auto (SApp (SApp (SName "Le") (SName "Zero")) (SName "Zero")).
Definition base_result : Syntax := concludes base_proof.
Eval base_result.

-- Step case: use axiom for now (step needs IH)
Definition step_proof : Derivation := DAxiom step_goal.

-- Combine into full induction proof
Definition full_proof : Derivation := try_induction (SName "Nat") le_motive (DCase base_proof (DCase step_proof DCaseEnd)).
Definition full_result : Syntax := concludes full_proof.
Eval full_result.

-- ============================================
-- ERROR HANDLING
-- ============================================
-- Wrong number of cases should error

Definition motive2 : Syntax := SLam (SName "Nat") (SName "True").
Definition single_case : Derivation := DAxiom (SName "True").

-- Only 1 case for 2-constructor Nat = error
Definition bad_proof : Derivation := try_induction (SName "Nat") motive2 (DCase single_case DCaseEnd).
Definition bad_result : Syntax := concludes bad_proof.
Eval bad_result.

-- ============================================
-- NON-INDUCTIVE TYPES
-- ============================================
-- Int is not an inductive type

Definition int_cases : Nat := induction_num_cases (SName "Int").
Eval int_cases.

-- ============================================
-- SUMMARY
-- ============================================
-- The induction infrastructure provides:
--
-- 1. induction_num_cases: Count constructors for a type
-- 2. induction_base_goal: Generate base case goal
-- 3. induction_step_goal: Generate step case goal
-- 4. try_induction: Build DElim from cases
--
-- This enables generic structural induction on:
-- - Nat (Zero, Succ)
-- - Bool (true, false)
-- - User-defined inductives
--
-- Bullet-point syntax in literate mode:
--   Proof:
--     induction n.
--     - auto.    # Base case
--     - auto.    # Step case
"###;

const MATH_HINTS: &str = r###"-- ============================================
-- HINT DATABASE: Teaching Auto New Tricks
-- ============================================
-- Register theorems as hints so auto can use them!
--
-- The hint system allows you to:
-- 1. Prove a theorem once
-- 2. Register it with "Attribute: hint."
-- 3. Auto will try to use it when other tactics fail

-- ============================================
-- BASIC HINT EXAMPLE
-- ============================================

-- First, let's define a simple property
Definition trivial_true : Syntax := SName "True".

-- Prove it (trivially)
Definition trivial_proof : Derivation := try_auto trivial_true.

-- Verify the proof
Eval concludes trivial_proof.

-- ============================================
-- HOW HINTS WORK
-- ============================================
-- When you write:
--
--   ## Theorem: my_lemma
--   Statement: <some_statement>
--   Proof: auto.
--   Attribute: hint.
--
-- The system:
-- 1. Proves the theorem
-- 2. Registers it in the hint database
-- 3. When auto runs later, it checks if any hint matches the goal

-- ============================================
-- HINT-AWARE AUTO
-- ============================================
-- Auto tries tactics in this order:
-- 1. Trivial (True/False)
-- 2. simp (simplification)
-- 3. ring (polynomial arithmetic)
-- 4. cc (congruence closure)
-- 5. omega (integer arithmetic)
-- 6. lia (linear arithmetic)
-- 7. HINTS (registered theorems) <-- NEW!

-- ============================================
-- LITERATE SYNTAX FOR HINTS
-- ============================================
-- In literate mode, you can write:
--
-- ## Theorem: plus_zero_right
-- Statement: For all (n: Nat), n + 0 = n.
-- Proof:
--   induction n.
--   - auto.
--   - auto.
-- Attribute: hint.
--
-- This registers plus_zero_right as a hint!
-- Now any proof with goal "n + 0 = n" can use auto.

-- ============================================
-- CHECKING HINTS
-- ============================================
-- You can inspect the hint database via the context.
-- Hints are stored as theorem names.

Check try_auto.

-- ============================================
-- SUMMARY
-- ============================================
-- The hint system extends auto with learned knowledge:
--
-- 1. Prove theorems normally
-- 2. Add "Attribute: hint." to register them
-- 3. Auto will try hints when built-in tactics fail
--
-- This creates a virtuous cycle:
--   Prove lemmas → Register as hints → Prove harder theorems
"###;

const MATH_INVERSION: &str = r###"-- ============================================
-- INVERSION: The Scalpel
-- ============================================
-- Derives contradictions by running constructors backwards.
--
-- If you claim something impossible (like Eq Nat 3 0),
-- inversion proves False by showing no constructor can build it.

-- ============================================
-- KERNEL INFRASTRUCTURE
-- ============================================
-- These are the building blocks for inversion.

-- Check that inversion helpers exist
Check try_inversion.
Check DInversion.

-- ============================================
-- DISCRIMINATE: DIFFERENT CONSTRUCTORS
-- ============================================
-- The Eq type has only one constructor: refl
-- refl : Π(A:Type). Π(x:A). Eq A x x
--
-- refl requires BOTH arguments to be the same!

-- Build hypothesis: Eq Nat 3 0 (impossible!)
Definition three : Syntax :=
    SApp (SName "Succ") (SApp (SName "Succ") (SApp (SName "Succ") (SName "Zero"))).

Definition eq_three_zero : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Nat")) three) (SName "Zero").

-- Try inversion: can refl build Eq Nat 3 0?
-- No! refl needs same args, but 3 ≠ 0
Definition discriminate_proof : Derivation := try_inversion eq_three_zero.
Definition discriminate_result : Syntax := concludes discriminate_proof.
Eval discriminate_result.

-- ============================================
-- REFLEXIVE: CONSTRUCTOR CAN MATCH
-- ============================================
-- Eq Nat Zero Zero CAN be built by refl Zero
-- So inversion should NOT derive False (returns error)

Definition eq_zero_zero : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero").

Definition reflexive_proof : Derivation := try_inversion eq_zero_zero.
Definition reflexive_result : Syntax := concludes reflexive_proof.
Eval reflexive_result.

-- ============================================
-- EMPTY INDUCTIVE: FALSE
-- ============================================
-- False has NO constructors at all.
-- Anything of type False is automatically contradictory.

Definition false_hyp : Syntax := SName "False".
Definition false_proof : Derivation := try_inversion false_hyp.
Definition false_result : Syntax := concludes false_proof.
Eval false_result.

-- ============================================
-- BOOL DISCRIMINATE: true ≠ false
-- ============================================
-- Eq Bool true false requires refl to make true = false
-- But true and false are different constructors!

Definition eq_true_false : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Bool")) (SName "true")) (SName "false").

Definition bool_proof : Derivation := try_inversion eq_true_false.
Definition bool_result : Syntax := concludes bool_proof.
Eval bool_result.

-- ============================================
-- NON-INDUCTIVE: ERROR
-- ============================================
-- Inversion only works on inductive types.
-- Variables or unknown types produce errors.

Definition var_hyp : Syntax := SVar 0.
Definition var_proof : Derivation := try_inversion var_hyp.
Definition var_result : Syntax := concludes var_proof.
Eval var_result.

-- ============================================
-- SUMMARY
-- ============================================
-- The inversion tactic:
--
-- 1. Extracts the inductive type from the hypothesis
-- 2. Checks if ANY constructor could produce the given args
-- 3. If no constructor matches → proves False
-- 4. If some constructor matches → returns error
--
-- Key insight: Inversion is the INVERSE of introduction.
-- Introduction builds terms; inversion checks if building is possible.
--
-- Common uses:
--   - Discriminate different constructors (Eq 3 0 → False)
--   - Empty inductives (False → False)
--   - Proof by contradiction
"###;

const MATH_OPERATOR: &str = r###"-- ============================================
-- THE OPERATOR: Manual Control Tactics
-- ============================================
-- When auto fails, you need precision tools.
--
-- rewrite  - The Sniper: targeted substitution
-- destruct - The Fork: case analysis without IH
-- apply    - The Arrow: backward chaining

-- ============================================
-- KERNEL INFRASTRUCTURE
-- ============================================

Check try_rewrite.
Check try_rewrite_rev.
Check try_destruct.
Check try_apply.
Check DRewrite.
Check DDestruct.
Check DApply.

-- ============================================
-- REWRITE: The Sniper
-- ============================================
-- Given a proof of Eq A x y and a goal containing x,
-- rewrite replaces x with y (or vice versa with rewrite_rev).
--
-- Use case: When you have an equality lemma and need to
-- substitute one term for another.

-- Example: Given Eq Nat x y, transform goal P(x) to P(y)

-- Build equality hypothesis: Eq Nat (SVar 0) (SVar 1)
-- This represents "x = y" where x is var 0, y is var 1
Definition eq_type : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 1).

-- Create a proof of this equality (as axiom for demo)
Definition eq_proof : Derivation := DAxiom eq_type.

-- Goal: P(x) = P(SVar 0)
Definition goal_px : Syntax := SApp (SName "P") (SVar 0).

-- Rewrite: Replace x with y to get P(y)
Definition rewritten : Derivation := try_rewrite eq_proof goal_px.
Eval (concludes rewritten).

-- Reverse rewrite: Given same equality, replace y with x
Definition goal_py : Syntax := SApp (SName "P") (SVar 1).
Definition rev_rewritten : Derivation := try_rewrite_rev eq_proof goal_py.
Eval (concludes rev_rewritten).

-- ============================================
-- DESTRUCT: The Fork
-- ============================================
-- Case analysis WITHOUT induction hypotheses.
--
-- For Bool: generates true and false cases
-- For Nat: generates Zero case and "forall k. P(Succ k)"
--          NOT "forall k. P(k) -> P(Succ k)" (that's induction)
--
-- Use case: When you need to split on cases but don't need
-- the induction hypothesis (enums, finite types).

-- Motive: λb:Bool. P(b)
Definition bool_motive : Syntax :=
    SLam (SName "Bool") (SApp (SName "P") (SVar 0)).

-- Case proofs: P(true) and P(false) as axioms
Definition case_true : Derivation := DAxiom (SApp (SName "P") (SName "true")).
Definition case_false : Derivation := DAxiom (SApp (SName "P") (SName "false")).

-- Build case list
Definition bool_cases : Derivation := DCase case_true (DCase case_false DCaseEnd).

-- Destruct Bool
Definition bool_destruct : Derivation :=
    try_destruct (SName "Bool") bool_motive bool_cases.
Eval (concludes bool_destruct).

-- ============================================
-- APPLY: The Arrow
-- ============================================
-- Manual backward chaining.
--
-- Given hypothesis H : P → Q and goal Q,
-- apply H transforms the goal to P.
--
-- Given hypothesis H : ∀x. P(x) and goal P(3),
-- apply H instantiates the forall.
--
-- Use case: When auto can't figure out which lemma to use.

-- Implication example: H : P → Q, goal: Q
Definition impl_type : Syntax := SPi (SName "P") (SName "Q").
Definition impl_proof : Derivation := DAxiom impl_type.
Definition goal_q : Syntax := SName "Q".

-- Apply H to goal Q → new goal P
Definition applied_impl : Derivation :=
    try_apply (SName "H") impl_proof goal_q.
Eval (concludes applied_impl).

-- Forall example: H : ∀x:Nat. P(x), goal: P(3)
Definition forall_type : Syntax :=
    SApp (SName "Forall")
        (SLam (SName "Nat") (SApp (SName "P") (SVar 0))).

Definition forall_proof : Derivation := DAxiom forall_type.

Definition three : Syntax :=
    SApp (SName "Succ") (SApp (SName "Succ") (SApp (SName "Succ") (SName "Zero"))).

Definition goal_p3 : Syntax := SApp (SName "P") three.

-- Apply forall to goal P(3)
Definition applied_forall : Derivation :=
    try_apply (SName "lemma") forall_proof goal_p3.
Eval (concludes applied_forall).

-- ============================================
-- SUMMARY
-- ============================================
-- The Operator tactics give you manual control:
--
-- rewrite eq_proof goal
--   - Given Eq A x y, replaces x with y in goal
--   - Surgical precision when you have the right equality
--
-- rewrite_rev eq_proof goal
--   - Same but replaces y with x (reverse direction)
--
-- destruct type motive cases
--   - Case analysis without induction hypothesis
--   - Simpler than induction for non-recursive proofs
--
-- apply hyp_name hyp_proof goal
--   - Backward chaining: uses hypothesis to transform goal
--   - Works with implications (P → Q) and foralls (∀x. P(x))
--
-- When to use:
--   - auto fails on complex goals
--   - Need specific control over proof steps
--   - Working with explicit equality proofs
"###;

const MATH_TACTICALS: &str = r###"-- ============================================
-- THE STRATEGIST: PROGRAMMABLE PROOFS
-- ============================================
--
-- Phase 10: Higher-Order Tactic Combinators
--
-- Tacticals turn proofs into programs. Instead of:
--   induction n.
--   auto.
--   auto.
--   auto.
--
-- Write:
--   induction n; repeat auto.
--
-- One line. Infinite power.

-- ============================================
-- TACT_TRY: THE SAFETY NET
-- ============================================
-- tact_try : (Syntax -> Derivation) -> Syntax -> Derivation
--
-- Attempts a tactic but never fails. If the tactic fails,
-- returns the goal unchanged (identity).
--
-- Use case: "Try to simplify, but don't crash if you can't"

-- Reflexive goal - try_refl succeeds
Definition goal_refl : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Nat"))
        (SName "Zero")) (SName "Zero").

-- Non-reflexive goal - try_refl would fail
Definition goal_hard : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Nat"))
        (SName "Zero")) (SApp (SName "Succ") (SName "Zero")).

-- tact_try always succeeds
Definition d_try_easy : Derivation := tact_try try_refl goal_refl.
Definition d_try_hard : Derivation := tact_try try_refl goal_hard.

-- Easy goal: proves it
Eval (concludes d_try_easy).

-- Hard goal: returns unchanged (identity) - NOT Error
Eval (concludes d_try_hard).

-- ============================================
-- TACT_REPEAT: THE LOOP
-- ============================================
-- tact_repeat : (Syntax -> Derivation) -> Syntax -> Derivation
--
-- Applies a tactic repeatedly until it fails.
-- Returns after the last successful application.
--
-- Use case: "Keep simplifying until you can't simplify anymore"

-- Identity tactic (always succeeds, does nothing)
Definition tact_id : Syntax -> Derivation := fun g : Syntax => DAxiom g.

-- tact_repeat stops when no progress is made
Definition d_repeat : Derivation := tact_repeat tact_id goal_refl.
Eval (concludes d_repeat).

-- ============================================
-- TACT_THEN: THE SEQUENCER (;)
-- ============================================
-- tact_then : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
--
-- Sequence two tactics: apply first, then apply second to result.
-- If either fails, the whole thing fails.
--
-- Use case: "First simplify, then prove by reflexivity"

-- Sequence: try (always succeeds) ; refl
Definition tact_combo : Syntax -> Derivation :=
    tact_then (tact_try tact_fail) try_refl.

Definition d_combo : Derivation := tact_combo goal_refl.
Eval (concludes d_combo).

-- ============================================
-- TACT_FIRST: THE MENU
-- ============================================
-- tact_first : TTactics -> Syntax -> Derivation
--
-- Try tactics from a list until one succeeds.
-- Returns Error if all fail.
--
-- TTactics = TList of (Syntax -> Derivation)
-- TacCons and TacNil are convenience wrappers

-- Build a tactic list: [tact_fail, tact_fail, try_refl]
Definition my_tactics : TTactics :=
    TacCons tact_fail
    (TacCons tact_fail
    (TacCons try_refl TacNil)).

-- First will skip the failures and use try_refl
Definition d_first : Derivation := tact_first my_tactics goal_refl.
Eval (concludes d_first).

-- All fail case
Definition fail_tactics : TTactics := TacCons tact_fail TacNil.
Definition d_all_fail : Derivation := tact_first fail_tactics goal_refl.
Eval (concludes d_all_fail).

-- ============================================
-- TACT_SOLVE: THE ENFORCER
-- ============================================
-- tact_solve : (Syntax -> Derivation) -> Syntax -> Derivation
--
-- Tactic MUST completely solve the goal.
-- If the tactic returns Error, fails.
-- If the tactic succeeds, returns its proof.
--
-- Use case: "Only use this tactic if it finishes the job"

-- try_refl completely solves reflexive goals
Definition d_solve : Derivation := tact_solve try_refl goal_refl.
Eval (concludes d_solve).

-- ============================================
-- THE NUCLEAR CODE
-- ============================================
-- Combine all tacticals into the ultimate tactic:
-- "Try everything we know how to do"

Definition nuclear : Syntax -> Derivation :=
    tact_first (TacCons try_refl
               (TacCons (tact_try try_simp)
               (TacCons try_lia
               (TacCons try_auto TacNil)))).

-- Test it on our reflexive goal
Definition d_nuclear : Derivation := nuclear goal_refl.
Eval (concludes d_nuclear).

-- ============================================
-- COMBINING TACTICALS
-- ============================================
-- Real power: nest them!

-- repeat (first [refl, simp]) - keep trying until nothing works
Definition solve_trivial : Syntax -> Derivation :=
    tact_repeat (tact_first (TacCons try_refl
                            (TacCons (tact_try try_simp) TacNil))).

Definition d_trivial : Derivation := solve_trivial goal_refl.
Eval (concludes d_trivial).

-- ============================================
-- SUMMARY
-- ============================================
-- tact_try t     - Try t, never fail (identity on failure)
-- tact_repeat t  - Apply t until failure
-- tact_then t1 t2 - Sequence: t1 then t2
-- tact_first ts  - Try list of tactics until one works
-- tact_solve t   - t must completely prove the goal
--
-- With tact_orelse from Phase 98:
-- tact_orelse t1 t2 - Try t1, if fails try t2
-- tact_fail        - Always fail
--
-- These form a complete tactical language for
-- programming proofs. God Mode achieved.
"###;

// ============================================================================
// Studio example registry — the single source of truth for every shipped
// example. `seed_examples` writes exactly these into the VFS, and the
// `example_health` test suite drives every one through the SAME pipeline the
// Studio uses and asserts its documented intended outcome. A new example must
// be added here with its `Expected`, so nothing can ship unlocked.
// ============================================================================

/// Which Studio surface an example belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Logic,
    Code,
    Math,
    Hardware,
}

/// The documented intended outcome an example is locked to. The test harness
/// dispatches on this to the same pipeline the Studio uses for that mode.
#[derive(Debug, Clone)]
pub enum Expected {
    /// Logic theorem: `compile_theorem_for_ui(src).verified` is true.
    Proves,
    /// Logic theorem that intentionally yields a derivation the kernel does NOT
    /// certify — the string-door-honesty demonstration (the Barber paradox): the
    /// backward chainer finds a derivation, but the system honestly reports it is
    /// not a certified proof (`derivation.is_some() && !verified`).
    DerivationNotCertified,
    /// Logic sentences: `compile_for_ui(src)` yields FOL (no error) whose BOTH
    /// the primary and the Simple views contain every content needle — so a
    /// silently-dropped modifier (e.g. an adverb lost when the Simple view
    /// flattens the event) fails the lock instead of passing unnoticed.
    CompilesToFol(&'static [&'static str]),
    /// Code: runs in the baseline interpreter with no error, printing each needle.
    OutputContains(&'static [&'static str]),
    /// Code that cannot run in the browser interpreter (real OS networking): it
    /// must still generate Rust cleanly.
    NativeOnlyCompiles,
    /// Math: every statement executes without error in a fresh kernel `Repl`.
    KernelAllStatementsOk,
    /// Math intentionally open: statements execute until a documented admitted
    /// goal (the marker string names the open point, e.g. a termination wall).
    KernelAdmitsAt(&'static str),
    /// Hardware English→SVA: synthesizes AND certifies equivalent to the spec.
    SvaSynthesizes,
    /// Hardware signal-design: yields a phase plan with at least one phase.
    SignalPlanSynthesizes,
    /// Verilog proven safe by k-induction (BMC fallback finds no counterexample).
    RtlProven,
    /// Verilog unsafe by design: the prover MUST find a counterexample.
    RefutesWithCounterexample,
    /// Register allocation that fits the register budget (certified valid).
    RegisterAllocFits,
    /// Register allocation that provably must spill (certified spill).
    SpillsRequired,
    /// Pigeonhole instance with a certified UNSAT (Hall) witness.
    UnsatCertified,
}

/// One shipped Studio example: where it seeds in the VFS, its mode, its source
/// text (the live const), and the outcome it is locked to.
pub struct ExampleSpec {
    pub vfs_path: &'static str,
    pub mode: Mode,
    pub source: &'static str,
    pub expected: Expected,
}

/// The 11 Logic-mode examples.
pub const ALL_LOGIC_EXAMPLES: &[ExampleSpec] = &[
    ExampleSpec { vfs_path: "/examples/logic/simple-sentences.logic", mode: Mode::Logic, source: LOGIC_SIMPLE, expected: Expected::CompilesToFol(&["Sleep", "Bark", "Loudly", "Love", "Fail"]) },
    ExampleSpec { vfs_path: "/examples/logic/quantifiers.logic", mode: Mode::Logic, source: LOGIC_QUANTIFIERS, expected: Expected::CompilesToFol(&["Student", "Book", "Professor", "Exam", "Cat"]) },
    ExampleSpec { vfs_path: "/examples/logic/tense-aspect.logic", mode: Mode::Logic, source: LOGIC_TENSE, expected: Expected::CompilesToFol(&["Run", "Eat", "Arrive", "Sleep", "Work"]) },
    ExampleSpec { vfs_path: "/examples/logic/prover-demo.logic", mode: Mode::Logic, source: LOGIC_PROVER, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/simon.logic", mode: Mode::Logic, source: LOGIC_SIMON, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/syllogism.logic", mode: Mode::Logic, source: LOGIC_SYLLOGISM, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/trivial-proof.logic", mode: Mode::Logic, source: LOGIC_TRIVIAL, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/disjunctive-syllogism.logic", mode: Mode::Logic, source: LOGIC_DISJUNCTIVE, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/modus-tollens.logic", mode: Mode::Logic, source: LOGIC_MODUS_TOLLENS, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/leibniz-identity.logic", mode: Mode::Logic, source: LOGIC_LEIBNIZ, expected: Expected::Proves },
    ExampleSpec { vfs_path: "/examples/logic/barber-paradox.logic", mode: Mode::Logic, source: LOGIC_BARBER, expected: Expected::DerivationNotCertified },
];

/// The 36 Code-mode examples. Output needles are provisional until the audit
/// run confirms the real interpreter output.
pub const ALL_CODE_EXAMPLES: &[ExampleSpec] = &[
    ExampleSpec { vfs_path: "/examples/code/hello-world.logos", mode: Mode::Code, source: CODE_HELLO, expected: Expected::OutputContains(&["Hello, LOGOS!", "30"]) },
    ExampleSpec { vfs_path: "/examples/code/hello-world2.logos", mode: Mode::Code, source: CODE_HELLO2, expected: Expected::OutputContains(&["Hello, World!"]) },
    ExampleSpec { vfs_path: "/examples/code/fibonacci.logos", mode: Mode::Code, source: CODE_FIBONACCI, expected: Expected::OutputContains(&["55"]) },
    ExampleSpec { vfs_path: "/examples/code/fizzbuzz.logos", mode: Mode::Code, source: CODE_FIZZBUZZ, expected: Expected::OutputContains(&["FizzBuzz", "Buzz"]) },
    ExampleSpec { vfs_path: "/examples/code/fizzbuzz2.logos", mode: Mode::Code, source: CODE_FIZZBUZZ2, expected: Expected::OutputContains(&["FizzBuzz", "Buzz"]) },
    ExampleSpec { vfs_path: "/examples/code/fizzbuzz3.logos", mode: Mode::Code, source: CODE_FIZZBUZZ3, expected: Expected::OutputContains(&["FizzBuzz", "Buzz"]) },
    ExampleSpec { vfs_path: "/examples/code/collections.logos", mode: Mode::Code, source: CODE_COLLECTIONS, expected: Expected::OutputContains(&["6"]) },
    ExampleSpec { vfs_path: "/examples/code/factorial.logos", mode: Mode::Code, source: CODE_FACTORIAL, expected: Expected::OutputContains(&["120"]) },
    ExampleSpec { vfs_path: "/examples/code/prime-check.logos", mode: Mode::Code, source: CODE_PRIME, expected: Expected::OutputContains(&["29"]) },
    ExampleSpec { vfs_path: "/examples/code/sum-list.logos", mode: Mode::Code, source: CODE_SUM_LIST, expected: Expected::OutputContains(&["150"]) },
    ExampleSpec { vfs_path: "/examples/code/bubble-sort.logos", mode: Mode::Code, source: CODE_BUBBLE_SORT, expected: Expected::OutputContains(&["90"]) },
    ExampleSpec { vfs_path: "/examples/code/struct-demo.logos", mode: Mode::Code, source: CODE_STRUCT, expected: Expected::OutputContains(&["Alice"]) },
    ExampleSpec { vfs_path: "/examples/code/types/enums.logos", mode: Mode::Code, source: CODE_ENUMS, expected: Expected::OutputContains(&["red"]) },
    ExampleSpec { vfs_path: "/examples/code/types/generics.logos", mode: Mode::Code, source: CODE_GENERICS, expected: Expected::OutputContains(&["100"]) },
    ExampleSpec { vfs_path: "/examples/code/collections/sets.logos", mode: Mode::Code, source: CODE_SETS, expected: Expected::OutputContains(&["3"]) },
    ExampleSpec { vfs_path: "/examples/code/collections/maps.logos", mode: Mode::Code, source: CODE_MAPS, expected: Expected::OutputContains(&["50"]) },
    ExampleSpec { vfs_path: "/examples/code/functions/higher-order.logos", mode: Mode::Code, source: CODE_HIGHER_ORDER, expected: Expected::OutputContains(&["42"]) },
    ExampleSpec { vfs_path: "/examples/code/distributed/counters.logos", mode: Mode::Code, source: CODE_CRDT_COUNTERS, expected: Expected::OutputContains(&["18"]) },
    ExampleSpec { vfs_path: "/examples/code/security/policies.logos", mode: Mode::Code, source: CODE_POLICIES, expected: Expected::OutputContains(&["Admin"]) },
    ExampleSpec { vfs_path: "/examples/code/memory/zones.logos", mode: Mode::Code, source: CODE_ZONES, expected: Expected::OutputContains(&["100"]) },
    ExampleSpec { vfs_path: "/examples/code/native/tasks.logos", mode: Mode::Code, source: CODE_TASKS, expected: Expected::OutputContains(&["worker"]) },
    ExampleSpec { vfs_path: "/examples/code/native/channels.logos", mode: Mode::Code, source: CODE_CHANNELS, expected: Expected::OutputContains(&["42"]) },
    ExampleSpec { vfs_path: "/examples/code/basics/variables.logos", mode: Mode::Code, source: CODE_BASICS_VARIABLES, expected: Expected::OutputContains(&["Alice"]) },
    ExampleSpec { vfs_path: "/examples/code/basics/operators.logos", mode: Mode::Code, source: CODE_BASICS_OPERATORS, expected: Expected::OutputContains(&["13"]) },
    ExampleSpec { vfs_path: "/examples/code/basics/control-flow.logos", mode: Mode::Code, source: CODE_BASICS_CONTROL_FLOW, expected: Expected::OutputContains(&["Grade"]) },
    ExampleSpec { vfs_path: "/examples/code/types/enums-patterns.logos", mode: Mode::Code, source: CODE_ENUMS_PATTERNS, expected: Expected::OutputContains(&["status"]) },
    ExampleSpec { vfs_path: "/examples/code/memory/ownership.logos", mode: Mode::Code, source: CODE_OWNERSHIP, expected: Expected::OutputContains(&["profile"]) },
    ExampleSpec { vfs_path: "/examples/code/concurrency/parallel.logos", mode: Mode::Code, source: CODE_CONCURRENCY_PARALLEL, expected: Expected::OutputContains(&["30"]) },
    ExampleSpec { vfs_path: "/examples/code/distributed/tally.logos", mode: Mode::Code, source: CODE_CRDT_TALLY, expected: Expected::OutputContains(&["80"]) },
    ExampleSpec { vfs_path: "/examples/code/distributed/merge.logos", mode: Mode::Code, source: CODE_CRDT_MERGE, expected: Expected::OutputContains(&["150"]) },
    ExampleSpec { vfs_path: "/examples/code/networking/server.logos", mode: Mode::Code, source: CODE_NETWORK_SERVER, expected: Expected::NativeOnlyCompiles },
    ExampleSpec { vfs_path: "/examples/code/networking/client.logos", mode: Mode::Code, source: CODE_NETWORK_CLIENT, expected: Expected::NativeOnlyCompiles },
    ExampleSpec { vfs_path: "/examples/code/error-handling.logos", mode: Mode::Code, source: CODE_ERROR_HANDLING, expected: Expected::OutputContains(&["5"]) },
    ExampleSpec { vfs_path: "/examples/code/advanced/refinement.logos", mode: Mode::Code, source: CODE_ADVANCED_REFINEMENT, expected: Expected::OutputContains(&["5"]) },
    ExampleSpec { vfs_path: "/examples/code/advanced/assertions.logos", mode: Mode::Code, source: CODE_ADVANCED_ASSERTIONS, expected: Expected::OutputContains(&["50"]) },
    ExampleSpec { vfs_path: "/examples/code/temporal/durations.logos", mode: Mode::Code, source: CODE_TEMPORAL, expected: Expected::OutputContains(&["ns"]) },
];

/// The 24 Math-mode examples.
pub const ALL_MATH_EXAMPLES: &[ExampleSpec] = &[
    ExampleSpec { vfs_path: "/examples/math/natural-numbers.logos", mode: Mode::Math, source: MATH_NAT, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/boolean-logic.logos", mode: Mode::Math, source: MATH_BOOL, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/godel-sentence.logos", mode: Mode::Math, source: MATH_GODEL, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/incompleteness.logos", mode: Mode::Math, source: MATH_INCOMPLETENESS, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/prop-logic.logos", mode: Mode::Math, source: MATH_PROP_LOGIC, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/functions.logos", mode: Mode::Math, source: MATH_FUNCTIONS, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/list-ops.logos", mode: Mode::Math, source: MATH_LIST_OPS, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/pairs.logos", mode: Mode::Math, source: MATH_PAIRS, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/logic-gates.logos", mode: Mode::Math, source: MATH_CIRCUIT, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/proven-property.logos", mode: Mode::Math, source: MATH_PROPERTY, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/collatz.logos", mode: Mode::Math, source: MATH_COLLATZ, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/godel-literate.logos", mode: Mode::Math, source: MATH_GODEL_LITERATE, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/incompleteness-literate.logos", mode: Mode::Math, source: MATH_INCOMPLETENESS_LITERATE, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/ring-tactic.logos", mode: Mode::Math, source: MATH_RING, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/lia-tactic.logos", mode: Mode::Math, source: MATH_LIA, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/cc-tactic.logos", mode: Mode::Math, source: MATH_CC, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/simp-tactic.logos", mode: Mode::Math, source: MATH_SIMP, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/omega-tactic.logos", mode: Mode::Math, source: MATH_OMEGA, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/auto-tactic.logos", mode: Mode::Math, source: MATH_AUTO, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/induction-tactic.logos", mode: Mode::Math, source: MATH_INDUCTION, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/hints.logos", mode: Mode::Math, source: MATH_HINTS, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/inversion-tactic.logos", mode: Mode::Math, source: MATH_INVERSION, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/operator-tactics.logos", mode: Mode::Math, source: MATH_OPERATOR, expected: Expected::KernelAllStatementsOk },
    ExampleSpec { vfs_path: "/examples/math/tacticals.logos", mode: Mode::Math, source: MATH_TACTICALS, expected: Expected::KernelAllStatementsOk },
];

/// The 18 Hardware-mode examples, sourced from the four hardware slices. The
/// deliberately-unsafe demos (`bad-arbiter`, `traffic-crash`, `queue-jam`,
/// `register-alloc-spill`) are locked to their intended failure/refutation.
pub const ALL_HARDWARE_SPECS: &[ExampleSpec] = &[
    ExampleSpec { vfs_path: "/examples/hardware/handshake.hw", mode: Mode::Hardware, source: HARDWARE_EXAMPLES[0].1, expected: Expected::SvaSynthesizes },
    ExampleSpec { vfs_path: "/examples/hardware/enable-ready.hw", mode: Mode::Hardware, source: HARDWARE_EXAMPLES[1].1, expected: Expected::SvaSynthesizes },
    ExampleSpec { vfs_path: "/examples/hardware/start-done.hw", mode: Mode::Hardware, source: HARDWARE_EXAMPLES[2].1, expected: Expected::SvaSynthesizes },
    ExampleSpec { vfs_path: "/examples/hardware/write-full.hw", mode: Mode::Hardware, source: HARDWARE_EXAMPLES[3].1, expected: Expected::SvaSynthesizes },
    ExampleSpec { vfs_path: "/examples/hardware/intersection-design.hw", mode: Mode::Hardware, source: HARDWARE_EXAMPLES[4].1, expected: Expected::SignalPlanSynthesizes },
    ExampleSpec { vfs_path: "/examples/hardware/arbiter.v", mode: Mode::Hardware, source: RTL_EXAMPLES[0].1, expected: Expected::RtlProven },
    ExampleSpec { vfs_path: "/examples/hardware/bad-arbiter.v", mode: Mode::Hardware, source: RTL_EXAMPLES[1].1, expected: Expected::RefutesWithCounterexample },
    ExampleSpec { vfs_path: "/examples/hardware/fifo.v", mode: Mode::Hardware, source: RTL_EXAMPLES[2].1, expected: Expected::RtlProven },
    ExampleSpec { vfs_path: "/examples/hardware/onehot.v", mode: Mode::Hardware, source: RTL_EXAMPLES[3].1, expected: Expected::RtlProven },
    ExampleSpec { vfs_path: "/examples/hardware/reset-mirror.v", mode: Mode::Hardware, source: RTL_EXAMPLES[4].1, expected: Expected::RtlProven },
    ExampleSpec { vfs_path: "/examples/hardware/traffic-safe.v", mode: Mode::Hardware, source: RTL_EXAMPLES[5].1, expected: Expected::RtlProven },
    ExampleSpec { vfs_path: "/examples/hardware/traffic-crash.v", mode: Mode::Hardware, source: RTL_EXAMPLES[6].1, expected: Expected::RefutesWithCounterexample },
    ExampleSpec { vfs_path: "/examples/hardware/queue-jam.v", mode: Mode::Hardware, source: RTL_EXAMPLES[7].1, expected: Expected::RefutesWithCounterexample },
    ExampleSpec { vfs_path: "/examples/hardware/queue-stable.v", mode: Mode::Hardware, source: RTL_EXAMPLES[8].1, expected: Expected::RtlProven },
    ExampleSpec { vfs_path: "/examples/hardware/register-alloc-fits.hw", mode: Mode::Hardware, source: REGALLOC_EXAMPLES[0].1, expected: Expected::RegisterAllocFits },
    ExampleSpec { vfs_path: "/examples/hardware/register-alloc-spill.hw", mode: Mode::Hardware, source: REGALLOC_EXAMPLES[1].1, expected: Expected::SpillsRequired },
    ExampleSpec { vfs_path: "/examples/hardware/pigeonhole.hw", mode: Mode::Hardware, source: PIGEONHOLE_EXAMPLES[0].1, expected: Expected::UnsatCertified },
    ExampleSpec { vfs_path: "/examples/hardware/pigeonhole-12.hw", mode: Mode::Hardware, source: PIGEONHOLE_EXAMPLES[1].1, expected: Expected::UnsatCertified },
];

#[cfg(test)]
mod example_health {
    //! Every shipped Studio example is locked to its documented intended outcome
    //! by driving it through the SAME pipeline the Studio uses for its mode.
    //! This is the single-source guard: the tests iterate the `ALL_*` registries
    //! (the same specs `seed_examples` writes to the VFS), so a broken example
    //! cannot ship unnoticed.
    use super::*;
    use logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec;
    use logicaffeine_compile::codegen_sva::hw_pipeline::prove_spec_sva_equivalence;
    use logicaffeine_compile::codegen_sva::rtl::parse_transition_system;
    use logicaffeine_compile::codegen_sva::signal_design::design_from_spec;
    use logicaffeine_compile::{
        compile_for_ui, compile_theorem_for_ui, generate_rust_code,
        interpret_for_ui_baseline_with_args,
    };
    use logicaffeine_kernel::interface::Repl;
    use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};
    use logicaffeine_proof::register_alloc::{
        allocate, is_spill_certificate, is_valid_allocation, Allocation,
    };

    /// Drive one example through its real Studio pipeline and check its
    /// intended outcome. Returns `Ok(())` on a pass, `Err(reason)` otherwise.
    fn check(spec: &ExampleSpec) -> Result<(), String> {
        match &spec.expected {
            Expected::CompilesToFol(needles) => {
                let r = compile_for_ui(spec.source);
                if let Some(e) = r.error {
                    return Err(format!("compile error: {e}"));
                }
                let logic = r.logic.unwrap_or_default();
                if logic.trim().is_empty() {
                    return Err("no FOL produced".to_string());
                }
                let simple = r.simple_logic.unwrap_or_default();
                for needle in *needles {
                    if !logic.contains(needle) {
                        return Err(format!("primary FOL missing {needle:?}:\n{logic}"));
                    }
                    if !simple.contains(needle) {
                        return Err(format!("Simple FOL missing {needle:?}:\n{simple}"));
                    }
                }
                Ok(())
            }
            Expected::Proves => {
                let r = compile_theorem_for_ui(spec.source);
                if r.verified {
                    Ok(())
                } else {
                    Err(format!(
                        "theorem '{}' not kernel-verified{}",
                        r.name,
                        r.verification_error
                            .map(|e| format!(": {e}"))
                            .unwrap_or_default()
                    ))
                }
            }
            Expected::DerivationNotCertified => {
                let r = compile_theorem_for_ui(spec.source);
                match (r.derivation.is_some(), r.verified) {
                    (true, false) => Ok(()),
                    (true, true) => Err(
                        "expected an uncertified derivation (honesty demo) but it certified"
                            .to_string(),
                    ),
                    (false, _) => Err("expected a derivation to be found, none was".to_string()),
                }
            }
            Expected::OutputContains(needles) => {
                // Both Studio Code surfaces must work: it generates Rust (the
                // 🦀 Compile view) AND it runs (the interpreter view).
                match generate_rust_code(spec.source) {
                    Ok(rust) if !rust.trim().is_empty() => {}
                    Ok(_) => return Err("generated empty Rust".to_string()),
                    Err(e) => return Err(format!("codegen error: {e:?}")),
                }
                let result = run_code(spec.source);
                if let Some(e) = result.error {
                    return Err(format!("interpreter error: {e}"));
                }
                let out = result.lines.join("\n");
                for needle in *needles {
                    if !out.contains(needle) {
                        return Err(format!(
                            "output missing {needle:?}; actual output:\n{out}"
                        ));
                    }
                }
                Ok(())
            }
            Expected::NativeOnlyCompiles => match generate_rust_code(spec.source) {
                Ok(rust) if !rust.trim().is_empty() => Ok(()),
                Ok(_) => Err("generated empty Rust".to_string()),
                Err(e) => Err(format!("codegen error: {e:?}")),
            },
            Expected::KernelAllStatementsOk => run_kernel(spec.source, None),
            Expected::KernelAdmitsAt(marker) => run_kernel(spec.source, Some(marker)),
            Expected::SvaSynthesizes => {
                let synth = synthesize_sva_from_spec(spec.source, "clk")
                    .map_err(|e| format!("SVA synthesis failed: {e}"))?;
                let equiv = prove_spec_sva_equivalence(spec.source, &synth.body, 8)
                    .map_err(|e| format!("equivalence check failed: {e:?}"))?;
                if equiv.equivalent {
                    Ok(())
                } else {
                    Err("synthesized SVA not certified equivalent to spec".to_string())
                }
            }
            Expected::SignalPlanSynthesizes => {
                let (_intersection, plan) = design_from_spec(spec.source)
                    .map_err(|e| format!("signal-design failed: {e}"))?;
                if plan.num_phases >= 1 {
                    Ok(())
                } else {
                    Err("signal-design produced no phases".to_string())
                }
            }
            Expected::RtlProven => {
                let ts = parse_transition_system(spec.source)
                    .map_err(|e| format!("RTL parse failed: {e:?}"))?;
                match ts.prove_invariant(4) {
                    InductionOutcome::Proven => Ok(()),
                    InductionOutcome::NotInductive => match ts.bmc(28) {
                        BmcOutcome::NoneWithin(_) => Ok(()),
                        other => Err(format!("expected safe, BMC found {other:?}")),
                    },
                    other => Err(format!("expected Proven, got {other:?}")),
                }
            }
            Expected::RefutesWithCounterexample => {
                let ts = parse_transition_system(spec.source)
                    .map_err(|e| format!("RTL parse failed: {e:?}"))?;
                let inv = ts.prove_invariant(4);
                if matches!(inv, InductionOutcome::CounterexampleAt { .. }) {
                    return Ok(());
                }
                if matches!(inv, InductionOutcome::NotInductive) {
                    if matches!(ts.bmc(28), BmcOutcome::CounterexampleAt { .. }) {
                        return Ok(());
                    }
                }
                Err(format!(
                    "expected a counterexample, prove_invariant gave {inv:?}"
                ))
            }
            Expected::RegisterAllocFits => {
                let regspec = crate::ui::pages::register_alloc_viz::parse_register_spec(spec.source)
                    .ok_or("could not parse register spec")?;
                match allocate(&regspec.ranges, regspec.registers) {
                    Allocation::Allocated(reg_of) => {
                        if is_valid_allocation(&regspec.ranges, regspec.registers, &reg_of) {
                            Ok(())
                        } else {
                            Err("allocation is not valid".to_string())
                        }
                    }
                    Allocation::Spill { .. } => {
                        Err("expected a fitting allocation, got a spill".to_string())
                    }
                }
            }
            Expected::SpillsRequired => {
                let regspec = crate::ui::pages::register_alloc_viz::parse_register_spec(spec.source)
                    .ok_or("could not parse register spec")?;
                match allocate(&regspec.ranges, regspec.registers) {
                    Allocation::Spill { must_spill, .. } => {
                        if is_spill_certificate(&regspec.ranges, regspec.registers, &must_spill) {
                            Ok(())
                        } else {
                            Err("spill certificate does not re-verify".to_string())
                        }
                    }
                    Allocation::Allocated(_) => {
                        Err("expected a required spill, got a fitting allocation".to_string())
                    }
                }
            }
            Expected::UnsatCertified => {
                let pspec = crate::ui::pages::pigeonhole_viz::parse_pigeonhole_spec(spec.source)
                    .ok_or("could not parse pigeonhole spec")?;
                let verdict = crate::ui::pages::pigeonhole_viz::solve(&pspec);
                if verdict.certified && !verdict.hall.slots.is_empty() {
                    Ok(())
                } else {
                    Err("pigeonhole UNSAT was not certified".to_string())
                }
            }
        }
    }

    /// Run a code example through the async baseline interpreter — the exact
    /// entry the Studio uses — inside a Tokio runtime so programs that sleep or
    /// drive the scheduler run for real (the sync entry has no reactor and would
    /// panic on `Sleep`).
    fn run_code(src: &str) -> logicaffeine_compile::interpreter::InterpreterResult {
        let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime for code example");
        rt.block_on(interpret_for_ui_baseline_with_args(src, &[]))
    }

    /// Run every statement of a math example through a fresh persistent kernel
    /// `Repl` (exactly as the Studio does). With `admit_marker`, statements are
    /// allowed to fail from the first statement containing the marker onward
    /// (the documented open/admitted point).
    fn run_kernel(source: &str, admit_marker: Option<&str>) -> Result<(), String> {
        let mut repl = Repl::new();
        let mut admitted = false;
        for stmt in crate::ui::pages::studio::parse_math_statements(source) {
            if let Some(marker) = admit_marker {
                if stmt.contains(marker) {
                    admitted = true;
                }
            }
            if let Err(e) = repl.execute(&stmt) {
                if admitted {
                    return Ok(());
                }
                return Err(format!("statement failed: {stmt}\n  -> {e}"));
            }
        }
        if admit_marker.is_some() && !admitted {
            return Err("admit marker never matched any statement".to_string());
        }
        Ok(())
    }

    /// Assert every spec in a registry passes `check`, collecting ALL failures so
    /// one run surfaces every broken example at once.
    fn lock(registry: &[ExampleSpec], mode: &str) {
        let mut failures = Vec::new();
        for spec in registry {
            if let Err(reason) = check(spec) {
                failures.push(format!("  {}: {reason}", spec.vfs_path));
            }
        }
        assert!(
            failures.is_empty(),
            "{} {mode} example(s) failed their lock:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn lock_logic_examples() {
        lock(ALL_LOGIC_EXAMPLES, "logic");
    }

    #[test]
    fn lock_code_examples() {
        lock(ALL_CODE_EXAMPLES, "code");
    }

    #[test]
    fn lock_math_examples() {
        lock(ALL_MATH_EXAMPLES, "math");
    }

    #[test]
    fn lock_hardware_examples() {
        lock(ALL_HARDWARE_SPECS, "hardware");
    }

    /// Every registry entry, in one iterator — the single source of truth that
    /// both `seed_examples` and the locks above consume.
    fn all_specs() -> impl Iterator<Item = &'static ExampleSpec> {
        ALL_LOGIC_EXAMPLES
            .iter()
            .chain(ALL_CODE_EXAMPLES)
            .chain(ALL_MATH_EXAMPLES)
            .chain(ALL_HARDWARE_SPECS)
    }

    /// Ratchet: the per-mode counts are locked, so an example cannot be added or
    /// removed without deliberately updating this test.
    #[test]
    fn registry_counts_are_locked() {
        assert_eq!(ALL_LOGIC_EXAMPLES.len(), 11, "logic example count changed");
        assert_eq!(ALL_CODE_EXAMPLES.len(), 36, "code example count changed");
        assert_eq!(ALL_MATH_EXAMPLES.len(), 24, "math example count changed");
        assert_eq!(ALL_HARDWARE_SPECS.len(), 18, "hardware example count changed");
    }

    /// Ratchet: every `vfs_path` is unique, lives under `/examples/<mode>/`, has the
    /// extension its mode requires, and carries non-empty source — so a copy-paste,
    /// a misfiled example, or a wrong-extension path fails the build.
    #[test]
    fn every_vfs_path_is_well_formed_and_unique() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for spec in all_specs() {
            assert!(seen.insert(spec.vfs_path), "duplicate vfs_path: {}", spec.vfs_path);
            let (dir, exts): (&str, &[&str]) = match spec.mode {
                Mode::Logic => ("/examples/logic/", &[".logic"]),
                Mode::Code => ("/examples/code/", &[".logos"]),
                Mode::Math => ("/examples/math/", &[".logos"]),
                Mode::Hardware => ("/examples/hardware/", &[".hw", ".v"]),
            };
            assert!(spec.vfs_path.starts_with(dir), "{} is not under {dir}", spec.vfs_path);
            assert!(
                exts.iter().any(|e| spec.vfs_path.ends_with(e)),
                "{} has the wrong extension for {:?}",
                spec.vfs_path,
                spec.mode
            );
            assert!(!spec.source.trim().is_empty(), "{} has empty source", spec.vfs_path);
        }
    }

    /// The pit of success: `seed_examples` writes EXACTLY the registry — no more, no
    /// less. Drive it through a real filesystem VFS and compare the seeded
    /// `/examples/**` files to the registry `vfs_path`s. A stray `vfs.write`, a
    /// forgotten example, or a path typo fails here.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn seeding_writes_exactly_the_registry() {
        use std::collections::HashSet;
        use std::path::Path;

        let root = std::env::temp_dir().join(format!("logos_seed_ratchet_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let vfs = logicaffeine_system::fs::NativeVfs::new(root.clone());
        let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        rt.block_on(super::seed_examples(&vfs)).expect("seed_examples failed");

        fn collect(dir: &Path, base: &Path, out: &mut HashSet<String>) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        collect(&p, base, out);
                    } else if let Ok(rel) = p.strip_prefix(base) {
                        out.insert(format!("/{}", rel.to_string_lossy().replace('\\', "/")));
                    }
                }
            }
        }
        let mut seeded = HashSet::new();
        collect(&root.join("examples"), &root, &mut seeded);
        let _ = std::fs::remove_dir_all(&root);

        let registry: HashSet<String> = all_specs().map(|s| s.vfs_path.to_string()).collect();
        let missing: Vec<&String> = registry.difference(&seeded).collect();
        let extra: Vec<&String> = seeded.difference(&registry).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "seed_examples drifted from the registry.\n  MISSING (in registry, not seeded): {missing:?}\n  EXTRA (seeded, not in registry): {extra:?}"
        );
    }

    /// AUDIT (host-only, print report): generate the Rust for every Logic + Math
    /// example and actually `rustc`-compile it, categorizing program vs note vs
    /// degraded (`unsupported`) stub. Answers "is anything not compiling to Rust?"
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn audit_logic_math_rust_compiles() {
        use logicaffeine_compile::{extract_logic_rust, extract_math_rust_from_source};
        use std::process::Command;

        fn first_error(stderr: &[u8]) -> String {
            String::from_utf8_lossy(stderr)
                .lines()
                .find(|l| l.contains("error"))
                .unwrap_or("(unknown)")
                .to_string()
        }

        fn rustc_check(name: &str, code: &str) -> Result<(), String> {
            // The name is a vfs_path; strip characters rustc rejects in a crate name.
            let safe = name.replace(['/', '.', '-'], "_");
            let src = std::env::temp_dir().join(format!("logos_audit_{safe}.rs"));
            std::fs::write(&src, code).map_err(|e| e.to_string())?;

            // A self-contained program with a `main` (the math "🦀 Compile" output
            // carries a self-verifying main) is compiled to a binary AND RUN, so its
            // kernel-checked asserts actually execute — the extraction's runtime
            // fidelity, not just that it type-checks. A bounded wait keeps a runaway
            // program from hanging the suite. Everything else is compile-checked as a
            // library.
            if code.contains("fn main") {
                let bin = std::env::temp_dir().join(format!("logos_audit_{safe}_bin"));
                let o = Command::new("rustc")
                    .args(["--edition", "2021", "-A", "warnings", "-o"])
                    .arg(&bin)
                    .arg(&src)
                    .output()
                    .map_err(|e| e.to_string())?;
                if !o.status.success() {
                    return Err(first_error(&o.stderr));
                }
                let mut child = Command::new(&bin).spawn().map_err(|e| e.to_string())?;
                let mut waited = 0u32;
                let status = loop {
                    match child.try_wait().map_err(|e| e.to_string())? {
                        Some(s) => break Some(s),
                        None if waited >= 100 => {
                            let _ = child.kill();
                            break None;
                        }
                        None => {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            waited += 1;
                        }
                    }
                };
                let _ = std::fs::remove_file(&bin);
                match status {
                    Some(s) if s.success() => Ok(()),
                    Some(_) => Err("compiled but its self-checking main FAILED at runtime".to_string()),
                    None => Err("compiled but did not finish within 10s".to_string()),
                }
            } else {
                let out = std::env::temp_dir().join(format!("logos_audit_{safe}.rmeta"));
                let o = Command::new("rustc")
                    .args(["--edition", "2021", "--crate-type", "lib", "--emit=metadata", "-A", "warnings", "-o"])
                    .arg(&out)
                    .arg(&src)
                    .output()
                    .map_err(|e| e.to_string())?;
                if o.status.success() {
                    Ok(())
                } else {
                    Err(first_error(&o.stderr))
                }
            }
        }

        fn report(kind: &str, items: &[(&str, String)], failures: &mut Vec<String>) {
            println!("\n=== {kind} examples → Rust ===");
            for (name, code) in items {
                // A note is a comment-only output (the honest "not data" result).
                let is_program = code.contains("fn ") || code.contains("enum ");
                if !is_program {
                    println!("  {name:30} NOTE (not a program — by design)");
                    continue;
                }
                match rustc_check(name, code) {
                    Ok(()) => println!("  {name:30} compiles ✓"),
                    Err(e) => {
                        println!("  {name:30} DOES NOT COMPILE ✗ — {e}");
                        failures.push(format!("{kind}/{name}: {e}"));
                    }
                }
            }
        }

        let logic: Vec<(&str, String)> = ALL_LOGIC_EXAMPLES
            .iter()
            .map(|s| (s.vfs_path, extract_logic_rust(s.source).unwrap_or_else(|e| format!("// err: {e}"))))
            .collect();
        let math: Vec<(&str, String)> = ALL_MATH_EXAMPLES
            .iter()
            .map(|s| (s.vfs_path, extract_math_rust_from_source(s.source)))
            .collect();
        let mut failures = Vec::new();
        report("LOGIC", &logic, &mut failures);
        report("MATH", &math, &mut failures);
        // Every shipped example must compile to valid Rust OR be an honest note —
        // never broken Rust the user would hit on Compile.
        assert!(
            failures.is_empty(),
            "{} example(s) emit Rust that does not compile:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
