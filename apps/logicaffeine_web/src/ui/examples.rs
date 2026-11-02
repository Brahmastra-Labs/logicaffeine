//! Example files for the Studio playground.
//!
//! The Studio serves them read-only from the site itself (`studio_files`), so they never need
//! copying into the reader's storage and are there from the first frame.

use logicaffeine_system::fs::{Vfs, VfsResult};

/// Write every shipped example into a VFS, over the copy already there, and retire the ones the
/// registry dropped. The Studio itself never calls this: it serves the examples from the site
/// (`studio_files`).
pub async fn seed_examples<V: Vfs>(vfs: &V) -> VfsResult<()> {
    // Create directory structure (idempotent).
    vfs.create_dir_all("/examples/logic").await?;
    vfs.create_dir_all("/examples/code").await?;
    vfs.create_dir_all("/examples/math").await?;
    vfs.create_dir_all("/examples/hardware").await?;
    for sub in [
        "basics",
        "strings",
        "types",
        "collections",
        "functions",
        "algorithms",
        "distributed",
        "security",
        "memory",
        "concurrency",
        "networking",
        "crypto",
        "verification",
        "projects",
        "time-and-money",
        "web",
    ] {
        vfs.create_dir_all(&format!("/examples/code/{sub}")).await?;
    }

    // Retire earlier toy RTL examples replaced by the real-hardware set, plus the
    // three renamings of the handshake spec.
    for obsolete in [
        "toggle.v",
        "mutex.v",
        "counter.v",
        "enable-ready.hw",
        "start-done.hw",
        "write-full.hw",
    ] {
        let _ = vfs.remove(&format!("/examples/hardware/{obsolete}")).await;
    }

    // Retire code examples that were absorbed or rehomed by the catalog
    // restructure, so existing VFS trees converge.
    for obsolete in [
        "hello-world2.logos",
        "fizzbuzz.logos",
        "fizzbuzz2.logos",
        "fizzbuzz3.logos",
        "sum-list.logos",
        "collections.logos",
        "fibonacci.logos",
        "factorial.logos",
        "prime-check.logos",
        "bubble-sort.logos",
        "struct-demo.logos",
        "error-handling.logos",
        "types/enums-patterns.logos",
        "native/tasks.logos",
        "native/channels.logos",
        "advanced/refinement.logos",
        "advanced/assertions.logos",
        "temporal/durations.logos",
    ] {
        let _ = vfs.remove(&format!("/examples/code/{obsolete}")).await;
    }
    for empty_dir in ["native", "advanced", "temporal"] {
        let _ = vfs.remove(&format!("/examples/code/{empty_dir}")).await;
    }

    // Retire the trivial reflexive proof (subsumed by prover-demo; Wave L).
    let _ = vfs.remove("/examples/logic/trivial-proof.logic").await;

    for spec in seeded_examples() {
        vfs.write(spec.vfs_path, spec.source.as_bytes()).await?;
    }

    Ok(())
}

/// Every shipped Studio example, the four registries in one sequence. They are the single source
/// of truth: the Studio serves exactly them (`studio_files`) and opens on them (`studio_opening`),
/// and the `example_health` tests drive the same specs, so serving, opening and testing can never
/// drift.
pub fn seeded_examples() -> impl Iterator<Item = &'static ExampleSpec> {
    ALL_LOGIC_EXAMPLES
        .iter()
        .chain(ALL_CODE_EXAMPLES)
        .chain(ALL_MATH_EXAMPLES)
        .chain(ALL_HARDWARE_SPECS)
}

/// Hardware-mode examples: `(filename, English spec)`. Each is a single English hardware
/// specification that the Studio synthesizes to SystemVerilog Assertions and then certifies
/// (in-browser, no Z3) against the spec. Pure spec sentences — the loader feeds the whole
/// file to the synthesizer. The `hardware_examples` integration test verifies every one of
/// these synthesizes, round-trips as certified-equivalent, and has a reachable trigger.
pub const HARDWARE_EXAMPLES: &[(&str, &str)] = &[
    (
        "handshake.hw",
        "# Handshake\n\
         # Whenever request is high, acknowledge is high in the same cycle.\n\
         \n\
         Always, if request is high, then acknowledge is high.\n",
    ),
    (
        "only-if.hw",
        "# Grant Only If Requested\n\
         # A grant never appears in a cycle without a request.\n\
         \n\
         Always, grant is high only if request is high.\n",
    ),
    (
        "next-cycle.hw",
        "# Grant in the Next Cycle\n\
         # A request is answered with a grant one cycle later.\n\
         \n\
         Always, if request is high, then grant is high in the next cycle.\n",
    ),
    // Signal-DESIGN spec ("conflicts with"): the Studio synthesizes a conflict-free phase plan
    // with our own SAT solver and certifies it uses the fewest possible phases (no Z3).
    (
        "intersection-design.hw",
        "# Intersection Signal Plan\n\
         # Five movements and the pairs that cannot go at once; the Studio finds a plan with the\n\
         # fewest phases that keeps every such pair apart, and certifies no plan needs fewer.\n\
         \n\
         Movements: ns-through, ew-through, ns-left, ew-left, pedestrian.\n\
         ns-through conflicts with ew-through and ew-left.\n\
         ew-through conflicts with ns-left.\n\
         ns-left conflicts with ew-left.\n\
         pedestrian conflicts with ns-through, ew-through, ns-left and ew-left.\n",
    ),
    // Signal-design starter: three movements, everything pairwise conflicting.
    (
        "t-junction.hw",
        "# T-Junction Signal Plan\n\
         # Three movements, every pair of them at odds: each needs a phase of its own.\n\
         \n\
         Movements: main-through, side-left, pedestrian.\n\
         main-through conflicts with side-left.\n\
         pedestrian conflicts with main-through and side-left.\n",
    ),
];

/// Mutual exclusion as one English sentence. Certified equivalent to its SVA like
/// every spec above, but kept out of `HARDWARE_EXAMPLES` because the vacuity
/// check only applies to implication-shaped specs (this one has no trigger to
/// fire — "not both" is unconditional).
pub const HW_MUTEX: &str = "# Mutual Exclusion\n\
                            # The two grants are never high in the same cycle.\n\
                            \n\
                            Always, not both grant0 is high and grant1 is high.\n";

/// RTL examples: `(filename, Verilog)`. Synthesizable Verilog the Studio parses into a
/// transition system and bounded-model-checks / k-induction-proves in the browser (no Z3).
/// Verified by the `hardware_examples` integration test. Opened in Hardware mode (they live
/// under `/examples/hardware`); `load_hardware_spec` routes `module … endmodule` content to
/// the RTL BMC path.
pub const RTL_EXAMPLES: &[(&str, &str)] = &[
    (
        "arbiter.v",
        r#"// Round-Robin Arbiter
// Two masters share a bus, and grants are issued in mutually exclusive branches. Proven by
// k-induction for every request and turn sequence: the masters are never granted together.
module arbiter(input clk, input r0, input r1);
  reg g0;
  reg g1;
  reg turn;
  initial begin
    g0 = 0;
    g1 = 0;
    turn = 0;
  end
  always @(posedge clk) begin
    if (r0 && (!r1 || turn == 0)) begin
      g0 <= 1;
      g1 <= 0;
    end else if (r1) begin
      g0 <= 0;
      g1 <= 1;
    end else begin
      g0 <= 0;
      g1 <= 0;
    end
    turn <= ~turn;
  end
  assert property (~(g0 & g1));
endmodule
"#,
    ),
    (
        "bad-arbiter.v",
        r#"// Buggy Arbiter
// Each request is granted on its own, so when both masters ask in the same cycle, both are
// granted. BMC finds the violation at step 1: the waveform shows r0 = r1 = 1, then g0 = g1 = 1.
module bad_arbiter(input clk, input r0, input r1);
  reg g0;
  reg g1;
  initial begin
    g0 = 0;
    g1 = 0;
  end
  always @(posedge clk) begin
    if (r0)
      g0 <= 1;
    else
      g0 <= 0;
    if (r1)
      g1 <= 1;
    else
      g1 <= 0;
  end
  assert property (~(g0 & g1));
endmodule
"#,
    ),
    (
        "fifo.v",
        r#"// FIFO Occupancy Counter
// Depth 8, over free push and pop inputs: it counts up only when not full and down only when
// not empty. Proven for every push and pop sequence: the count never passes 8, although the
// 4-bit register could reach 15.
module fifo(input clk, input push, input pop);
  reg [3:0] count;
  initial count = 0;
  always @(posedge clk)
    if (push && (count < 4'd8) && !(pop && (count > 4'd0)))
      count <= count + 1;
    else if (pop && (count > 4'd0) && !(push && (count < 4'd8)))
      count <= count - 1;
    else
      count <= count;
  assert property (count <= 4'd8);
endmodule
"#,
    ),
    (
        "onehot.v",
        r#"// One-Hot Ring
// A 3-state one-hot ring: the rotation a <= c, b <= a, c <= b permutes the state bits.
// Proven: exactly one of a, b and c is ever high (at least one, and at most one).
module onehot(input clk);
  reg a;
  reg b;
  reg c;
  initial begin
    a = 1;
    b = 0;
    c = 0;
  end
  always @(posedge clk) begin
    a <= c;
    b <= a;
    c <= b;
  end
  assert property ((a | b | c) & ~(a & b) & ~(a & c) & ~(b & c));
endmodule
"#,
    ),
    (
        "reset-mirror.v",
        r#"// Reset Mirror
// Two registers with the same reset-and-toggle logic over a free reset input. Proven by
// k-induction for every reset sequence: a and b always stay equal.
module mirror(input clk, input rst);
  reg a;
  reg b;
  initial begin
    a = 0;
    b = 0;
  end
  always @(posedge clk) begin
    if (rst)
      a <= 0;
    else
      a <= ~a;
    if (rst)
      b <= 0;
    else
      b <= ~b;
  end
  assert property (a == b);
endmodule
"#,
    ),
    (
        "traffic-safe.v",
        r#"// Signalized Intersection
// North-south and east-west through movements, protected left turns (nsl, ewl) and a
// pedestrian phase, sequenced by a 9-state controller with a per-phase timer. Lights: 0 is
// red, 1 green, 2 yellow. Proven by k-induction: no two conflicting movements are ever active
// together, and pedestrians get WALK only while every vehicle movement is red.
module traffic(input clk);
  reg [1:0] ns;
  reg [1:0] ew;
  reg [1:0] nsl;
  reg [1:0] ewl;
  reg ped;
  reg [3:0] phase;
  reg [2:0] timer;
  initial begin
    ns = 2'd0;
    ew = 2'd0;
    nsl = 2'd1;
    ewl = 2'd0;
    ped = 1'd0;
    phase = 4'd0;
    timer = 3'd1;
  end
  always @(posedge clk)
    if (timer == 3'd0) begin
      timer <= 3'd1;
      if (phase == 4'd0) begin
        phase <= 4'd1;
        nsl <= 2'd2;
        ns <= 2'd0;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd1) begin
        phase <= 4'd2;
        nsl <= 2'd0;
        ns <= 2'd1;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd2) begin
        phase <= 4'd3;
        ns <= 2'd2;
        nsl <= 2'd0;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd3) begin
        phase <= 4'd4;
        ewl <= 2'd1;
        ns <= 2'd0;
        ew <= 2'd0;
        nsl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd4) begin
        phase <= 4'd5;
        ewl <= 2'd2;
        ns <= 2'd0;
        ew <= 2'd0;
        nsl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd5) begin
        phase <= 4'd6;
        ewl <= 2'd0;
        ew <= 2'd1;
        ns <= 2'd0;
        nsl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd6) begin
        phase <= 4'd7;
        ew <= 2'd2;
        ns <= 2'd0;
        nsl <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd7) begin
        phase <= 4'd8;
        ped <= 1'd1;
        ns <= 2'd0;
        ew <= 2'd0;
        nsl <= 2'd0;
        ewl <= 2'd0;
      end else begin
        phase <= 4'd0;
        nsl <= 2'd1;
        ns <= 2'd0;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end
    end else
      timer <= timer - 1;
  assert property (~((ns != 2'd0) & (ew != 2'd0))
    & ~((ped == 1'd1) & ((ns != 2'd0) | (ew != 2'd0) | (nsl != 2'd0) | (ewl != 2'd0))));
endmodule
"#,
    ),
    (
        "traffic-crash.v",
        r#"// Intersection With a Bug
// The same intersection with one dangerous bug: the pedestrian phase raises WALK but turns
// north-south green, sending pedestrians into live cross traffic. BMC finds the exact cycle;
// watch the lights step through the real trace, then flash CONFLICT.
module traffic(input clk);
  reg [1:0] ns;
  reg [1:0] ew;
  reg [1:0] nsl;
  reg [1:0] ewl;
  reg ped;
  reg [3:0] phase;
  reg [2:0] timer;
  initial begin
    ns = 2'd0;
    ew = 2'd0;
    nsl = 2'd1;
    ewl = 2'd0;
    ped = 1'd0;
    phase = 4'd0;
    timer = 3'd1;
  end
  always @(posedge clk)
    if (timer == 3'd0) begin
      timer <= 3'd1;
      if (phase == 4'd0) begin
        phase <= 4'd1;
        nsl <= 2'd2;
        ns <= 2'd0;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd1) begin
        phase <= 4'd2;
        nsl <= 2'd0;
        ns <= 2'd1;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd2) begin
        phase <= 4'd3;
        ns <= 2'd2;
        nsl <= 2'd0;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd3) begin
        phase <= 4'd4;
        ewl <= 2'd1;
        ns <= 2'd0;
        ew <= 2'd0;
        nsl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd4) begin
        phase <= 4'd5;
        ewl <= 2'd2;
        ns <= 2'd0;
        ew <= 2'd0;
        nsl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd5) begin
        phase <= 4'd6;
        ewl <= 2'd0;
        ew <= 2'd1;
        ns <= 2'd0;
        nsl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd6) begin
        phase <= 4'd7;
        ew <= 2'd2;
        ns <= 2'd0;
        nsl <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end else if (phase == 4'd7) begin
        phase <= 4'd8;
        ped <= 1'd1;
        ns <= 2'd1;
        ew <= 2'd0;
        nsl <= 2'd0;
        ewl <= 2'd0;
      end else begin
        phase <= 4'd0;
        nsl <= 2'd1;
        ns <= 2'd0;
        ew <= 2'd0;
        ewl <= 2'd0;
        ped <= 1'd0;
      end
    end else
      timer <= timer - 1;
  assert property (~((ns != 2'd0) & (ew != 2'd0))
    & ~((ped == 1'd1) & ((ns != 2'd0) | (ew != 2'd0) | (nsl != 2'd0) | (ewl != 2'd0))));
endmodule
"#,
    ),
    (
        "queue-jam.v",
        r#"// Queue That Jams
// An approach's queue is a counter. It is served only every other cycle while a car arrives
// every cycle, so demand beats capacity and the queue climbs until it jams. BMC finds the
// exact cycle it overflows: watch q ramp up in the waveform.
module flow(input clk);
  reg [2:0] q;
  reg phase;
  initial begin
    q = 3'd0;
    phase = 1'd0;
  end
  always @(posedge clk) begin
    phase <= ~phase;
    if (phase == 1'd1)
      q <= q + 3'd1;
  end
  assert property (q < 3'd7);
endmodule
"#,
    ),
    (
        "queue-stable.v",
        r#"// Queue That Drains
// The same queue, but service keeps up with demand, so a starting backlog only ever drains.
// Proven by k-induction never to jam: for all time, not just a bounded window.
module flow(input clk);
  reg [2:0] q;
  initial begin
    q = 3'd5;
  end
  always @(posedge clk)
    if (q != 3'd0)
      q <= q - 3'd1;
  assert property (q < 3'd7);
endmodule
"#,
    ),
    (
        "datapath-identity.v",
        r#"// Datapath Identity
// The bit-blaster at work: for every 4-bit value of x, x + x equals 2 * x. x starts free (no
// initial value) and holds, so the proof covers all 16 start values: a datapath identity
// proven by k-induction, not sampled by simulation.
module identity(input clk);
  reg [3:0] x;
  reg [3:0] a;
  reg [3:0] b;
  initial begin
    a = 0;
    b = 0;
  end
  always @(posedge clk) begin
    x <= x;
    a <= x + x;
    b <= 4'd2 * x;
  end
  assert property (a == b);
endmodule
"#,
    ),
    (
        "door-interlock.v",
        r#"// Elevator Door Interlock
// The door may open only while the car is stopped, and the car may move only while the door
// is shut. Proven by k-induction for every request sequence: moving and door-open are never
// true together.
module interlock(input clk, input req_open, input start);
  reg moving;
  reg door;
  initial begin
    moving = 0;
    door = 0;
  end
  always @(posedge clk) begin
    if (start && !door && !req_open)
      moving <= 1;
    else if (!start)
      moving <= 0;
    if (req_open && !moving && !start)
      door <= 1;
    else
      door <= 0;
  end
  assert property (~(moving & door));
endmodule
"#,
    ),
    (
        "door-interlock-bug.v",
        r#"// Interlock With a Dropped Guard
// The same interlock with one guard dropped: the door answers req_open without checking the
// car. BMC walks straight to the crash; the waveform shows the car moving with the door open
// at cycle 2.
module interlock(input clk, input req_open, input start);
  reg moving;
  reg door;
  initial begin
    moving = 0;
    door = 0;
  end
  always @(posedge clk) begin
    if (start && !door && !req_open)
      moving <= 1;
    else if (!start)
      moving <= 0;
    if (req_open)
      door <= 1;
    else
      door <= 0;
  end
  assert property (~(moving & door));
endmodule
"#,
    ),
];

/// Register-allocation examples: a basic block's variable live ranges + a register budget. Opened
/// in Hardware mode; `load_hardware_spec` routes a `registers:` spec to the certified linear-scan
/// allocator and renders the live-range timeline (coloured by register, spill clique flagged red).
pub const REGALLOC_EXAMPLES: &[(&str, &str)] = &[
    (
        "register-alloc-fits.hw",
        "# Register Allocation That Fits\n\
         # A basic block whose live ranges fit in 3 registers. Each line is\n\
         # `variable: firstInstr-lastInstr`; `registers:` is the physical budget.\n\
         \n\
         registers: 3\n\
         a: 0-4\n\
         b: 1-3\n\
         c: 2-6\n\
         d: 5-9\n\
         e: 7-10\n",
    ),
    (
        "register-alloc-spill.hw",
        "# Register Allocation Under Pressure\n\
         # Four variables are live at once but only 3 registers exist, so the allocator certifies\n\
         # (by the clique of mutually interfering ranges) that at least one must spill.\n\
         \n\
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
        "# Pigeonhole\n\
         # N pigeons into N-1 holes is impossible: watch the last pigeon find no home. Our prover\n\
         # certifies UNSAT in polynomial time (maximum matching and symmetry breaking); every\n\
         # resolution solver (Kissat, CaDiCaL, Z3) needs exponentially many steps here.\n\
         \n\
         pigeons: 6\n",
    ),
    (
        "pigeonhole-12.hw",
        "# Pigeonhole at 12\n\
         # PHP(12): far past where Z3 times out, our certified PR proof stays polynomial.\n\
         \n\
         pigeons: 12\n",
    ),
];

// ============================================================
// Logic Mode Examples (English -> FOL)
// ============================================================

// --- Wave L: FOL showcases ---

const LOGIC_GARDEN_PATH: &str = r#"# Garden-Path Sentences
# The classic reduced relatives: "raced past the barn" modifies the horse.

The horse raced past the barn fell.
The man pushed fell.
John said Mary ran.
"#;

const LOGIC_EVENTS_ADVERBS: &str = r#"# Event Semantics and Adverbs
# Verbs become events; adverbs attach to the event variable, not the subject.

John ran quickly.
Mary sliced the bread carefully.
Some dogs bark loudly.
"#;

const LOGIC_NPI: &str = r#"# Negation and "Any"
# "any" is licensed by negation and conditionals, and maps to the right quantifier.

John did not see any dogs.
If any dog barks, John runs.
No student saw any error.
"#;

const LOGIC_PLURALS: &str = r#"# Plurals and Collectives
# Pairing with "respectively", collective sums with "together", reciprocal "each other".

John and Mary helped Tom and Jerry respectively.
The boys gathered.
The boys lifted the piano together.
John and Mary love each other.
"#;

const LOGIC_COMPARATIVES: &str = r#"# Comparatives and Superlatives
# Degrees, measure phrases, and the superlative's hidden universal.

John is taller than Bill is.
John is 2 inches taller than Mary.
John climbed the highest mountain.
"#;

const LOGIC_MODALITY: &str = r#"# Modality
# Necessity ("must") and possibility ("can") under generalized quantifiers like "few birds".

All students must study.
Few birds can fly.
Some birds cannot fly.
"#;

const LOGIC_DONKEY: &str = r#"# Donkey Sentences
# The classic that breaks naive translation: "it" must reach back inside
# the relative clause. Discourse representation gets the binding right.

Every farmer who owns a donkey beats it.
If a farmer owns a donkey, he beats it.
No farmer who owns a donkey beats it.
"#;

const LOGIC_INTENSIONALITY: &str = r#"# Seeking Unicorns
# "seeks a unicorn" does not entail a unicorn exists: the theme stays
# intensional (^), unlike an ordinary object.

John seeks a unicorn.
Mary needs a doctor.
John believes a spy exists.
"#;

const LOGIC_COUNTERFACTUALS: &str = r#"# Counterfactuals
# "had studied / would have passed" becomes the Lewis conditional.

If John had studied, he would have passed.
If the glass had fallen, it would have broken.
"#;

// --- Wave L: prover showcases ---

const LOGIC_NEITHER_NOR: &str = r#"# Neither Nor
# "Neither A nor B" denies both.

## Theorem: Neither_Nor
Given: Neither Alice nor Bob is guilty.
Prove: Bob is not guilty.
Proof: Auto.
"#;

const LOGIC_EQUALITY_REWRITE: &str = r#"# Rewriting by Equality
# Once Alice is Bob, whatever is said of Bob can be said of Alice.

## Theorem: Equality_Rewrite
Given: Alice is equal to Bob.
Given: If Alice is happy then Alice is dancing.
Given: Bob is happy.

Prove: Bob is dancing.
Proof: Auto.
"#;

const LOGIC_PRESUPPOSITION: &str = r#"# What Regret Takes for Granted
# "Mary does not regret lying" still takes it for granted that she lied.

## Theorem: What_Regret_Takes_For_Granted
Given: Mary does not regret lying.
Prove: Mary lied.
Proof: Auto.
"#;

const LOGIC_GRID_STARTER: &str = r#"# A Mini Logic Grid
# Two trips, two years, two states: the exactly-one clues leave one answer.

## Theorem: Mini_Grid
Given: Alpha is a trip.
Given: Beta is a trip.
Given: Alpha is not Beta.
Given: Every trip is in 2003 or in 2004.
Given: Every trip is in Florida or in Maine.
Given: Exactly one trip is in 2003.
Given: Exactly one trip is in Florida.
Given: Alpha is in 2003.
Given: Alpha is in Florida.

Prove: Beta is in Maine.
Proof: Auto.
"#;

const LOGIC_GRID_QUESTION: &str = r#"# Asking the Puzzle
# The same grid, asked as a question: the prover names the trip that is in Maine.

## Theorem: Ask_The_Puzzle
Given: Alpha is a trip.
Given: Beta is a trip.
Given: Alpha is not Beta.
Given: Every trip is in 2003 or in 2004.
Given: Every trip is in Florida or in Maine.
Given: Exactly one trip is in 2003.
Given: Exactly one trip is in Florida.
Given: Alpha is in 2003.
Given: Alpha is in Florida.

Prove: Who is in Maine?
Proof: Auto.
"#;

const LOGIC_TACTIC_SCRIPT: &str = r#"# A Proof by Hand
# A labelled premise and a tactic script: take the conjunction apart, then rebuild it.

## Theorem: By_Hand
Given (h): Bob is happy and Bob is tall.
Prove: Bob is tall and Bob is happy.
Proof: cases h. split. assumption. assumption.
"#;

pub const LOGIC_SIMPLE: &str = r#"# Simple Sentences
# Plain sentences, each read into first-order logic.

Every cat sleeps.
Some dogs bark loudly.
John loves Mary.
The quick brown fox jumps.
No student failed.
"#;

pub const LOGIC_QUANTIFIERS: &str = r#"# Quantifier Scope
# Two quantifiers in one sentence: which one takes scope over the other.

Every student read a book.
A professor supervises every student.
No student failed every exam.
Some teacher praised every student.
Every dog chased some cat.
"#;

const LOGIC_TENSE: &str = r#"# Tense and Aspect
# Past, perfect, future and progressive, each read onto the event it describes.

John was running.
Mary has eaten.
The train will arrive.
She had been sleeping.
They have been working.
"#;

// ============================================================
// Logic Mode Examples (Prover/Theorem Proving)
// ============================================================

const LOGIC_PROVER: &str = r#"# Socrates Is Mortal
# The classic syllogism, proved by the kernel-certified prover.

## Theorem: Socrates_Mortality
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
const LOGIC_SIMON: &str = r#"# The Simon Logic Grid
# Four trips, each in a year and a state, with a friend and an activity, and six clues.
# The prover grounds the puzzle to its finite domain and derives where Beta went.

## Theorem: Simon
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

const LOGIC_SYLLOGISM: &str = r#"# A Chain of Syllogisms
# Two universal premises chained: men are mortal, and mortals are doomed.

## Theorem: Chain_Reasoning
Given: All men are mortal.
Given: All mortals are doomed.
Given: Plato is a man.

Prove: Plato is doomed.
Proof: Auto.
"#;

const LOGIC_DISJUNCTIVE: &str = r#"# Disjunctive Syllogism
# One of two is guilty; ruling one out convicts the other.

## Theorem: Disjunctive_Syllogism
Given: Either Alice or Bob is guilty.
Given: Alice is not guilty.
Prove: Bob is guilty.
Proof: Auto.
"#;

const LOGIC_MODUS_TOLLENS: &str = r#"# Modus Tollens
# Two conditionals, followed backwards from a consequent that did not happen.

## Theorem: Modus_Tollens_Chain
Given: If the butler did it, he was seen.
Given: If he was seen, he was caught.
Given: He was not caught.

Prove: The butler did not do it.
Proof: Auto.
"#;

pub const LOGIC_LEIBNIZ: &str = r#"# Leibniz's Law
# Identical things share their properties: what holds of Clark holds of Superman.

## Theorem: Leibniz_Identity
Given: Clark is Superman.
Given: Clark is mortal.
Prove: Superman is mortal.
Proof: Auto.
"#;

pub const LOGIC_BARBER: &str = r#"# The Barber Paradox
# A barber who shaves exactly the men who do not shave themselves cannot exist.

## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.

Prove: The barber does not exist.
Proof: Auto.
"#;

// ============================================================
// Code Mode Examples (Imperative LOGOS)
// ============================================================

pub const CODE_HELLO: &str = r#"# Hello, LOGOS
-- A program is numbered sections of English sentences.

## Main

Let greeting be "Hello, LOGOS!".
Show greeting.

Let x be 10.
Let y be 20.
Show "{x} + {y} = {x + y}".
"#;

/// THE deliberate variety showcase: every other example sticks to one
/// canonical style; this one tours the variants.
const CODE_SYNTAX_FLAVORS: &str = r#"# Many Surfaces, One Meaning
-- LOGOS accepts several spellings for the same idea. This example
-- tours the variants; the rest of the catalog sticks to one style.

## Main

Let classic be 20.
greeting = "bound with the = sign".
Show greeting.
Show "Let and = both bind values: {classic}".

Let n be 15.
If n is divisible by 15:
    Show "Otherwise If flavor: FizzBuzz".
Otherwise If n is divisible by 3:
    Show "Otherwise If flavor: Fizz".
Otherwise:
    Show "Otherwise If flavor: {n}".

If n is divisible by 15:
    Show "elif flavor: FizzBuzz".
elif n is divisible by 3:
    Show "elif flavor: Fizz".
Else:
    Show "elif flavor: {n}".

Let mut short_form be 1.
Let mutable long_form be 2.
Set short_form to short_form + 9.
Show "mut and mutable both work: {short_form + long_form}".

If n % 5 equals 0 and n is divisible by 5:
    Show "symbols and English agree: {n} is divisible by 5".
"#;

pub const CODE_FIBONACCI: &str = r#"# Fibonacci
-- Each number is the sum of the two before it.

## Main

Let n be 10.
Let mutable a be 0.
Let mutable b be 1.

Show "The first {n} Fibonacci numbers:".

Repeat for i from 1 to n:
    Show b.
    Let next be a + b.
    Set a to b.
    Set b to next.
"#;

const CODE_FIZZBUZZ: &str = r#"# FizzBuzz
-- The classic: `is divisible by` reads exactly like the rule.

## Main

Repeat for i from 1 to 20:
    If i is divisible by 15:
        Show "FizzBuzz".
    Otherwise If i is divisible by 3:
        Show "Fizz".
    Otherwise If i is divisible by 5:
        Show "Buzz".
    Otherwise:
        Show i.
"#;

const CODE_COLLECTIONS: &str = r#"# Lists
-- Lists are 1-indexed: item 1 is the first element.

## Main

Let mutable numbers be [10, 20, 30, 40, 50].
Show "Numbers: {numbers}".

Push 60 to numbers.
Show "After pushing 60: {numbers}".
Show "The list now holds {length(numbers)} items.".

Let first be item 1 of numbers.
Let last be item 6 of numbers.
Show "First: {first}, last: {last}.".

Let mutable total be 0.
Repeat for n in numbers:
    Set total to total + n.
Show "Sum of all items: {total}".

Pop from numbers into removed.
Show "Popped {removed}; {length(numbers)} items remain.".
"#;

const CODE_FACTORIAL: &str = r#"# Recursion
-- A function may call itself, as long as a base case stops it.

## To factorial (n: Int) -> Int:
    If n <= 1:
        Return 1.
    Return n * factorial(n - 1).

## To fib (n: Int) -> Int:
    If n <= 2:
        Return 1.
    Return fib(n - 1) + fib(n - 2).

## Main

Show "factorial(5) = {factorial(5)}".
Show "factorial(10) = {factorial(10)}".
Show "fib(10) = {fib(10)}".
"#;

const CODE_PRIME: &str = r#"# Prime Check
-- Trial division: prime means nothing divides it.

## To is_prime (n: Int) -> Bool:
    If n <= 1:
        Return false.
    Let mutable i be 2.
    While i * i <= n:
        If n is divisible by i:
            Return false.
        Set i to i + 1.
    Return true.

## Main

Show "Checking 2 through 30:".
Repeat for candidate from 2 to 30:
    If is_prime(candidate):
        Show "{candidate} is prime".
"#;

const CODE_BUBBLE_SORT: &str = r#"# Bubble Sort
-- Each pass bubbles the largest value to the end.
-- The `swapped` flag stops early once a pass changes nothing.

## Main

Let mutable numbers be [64, 34, 25, 12, 22, 11, 90].
Show "Before: {numbers}".

Let n be length(numbers).
Let mutable limit be n - 1.
Let mutable swapped be true.
While swapped:
    Set swapped to false.
    Repeat for j from 1 to limit:
        Let a be item j of numbers.
        Let b be item (j + 1) of numbers.
        If a > b:
            Set item j of numbers to b.
            Set item (j + 1) of numbers to a.
            Set swapped to true.
    Set limit to limit - 1.

Show "After: {numbers}".
"#;

const CODE_STRUCT: &str = r#"# Structs
-- A struct groups named fields; build one with `with`.
-- Fields may also be written as: a public name, which is Text.

## Definition

A Person has:
    name: Text.
    age: Int.

## Main

Let alice be a new Person with name "Alice" and age 30.
Let bob be a new Person with name "Bob" and age 25.

Show "{alice's name} is {alice's age}.".
Show "{bob's name} is {bob's age}.".

Set alice's age to 31.
Show "After a birthday, {alice's name} is {alice's age}.".
"#;

// ============================================================
// Advanced Code Mode Examples (organized by category)
// ============================================================

// --- Type System ---

const CODE_ENUMS: &str = r#"# Enums
-- An enum names a closed set of variants; Inspect matches them.

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

const CODE_GENERICS: &str = r#"# Generics
-- A type can take type parameters, written in brackets.

## A Box of [T] has:
    contents: T.

## A Pair of [A] and [B] has:
    first: A.
    second: B.

## Main

Let answer be a new Box of Int with contents 42.
Show "The Int box holds {answer's contents}.".

Let message be a new Box of Text with contents "hello".
Show "The Text box holds {message's contents}.".

Let entry be a new Pair of Text and Int with first "score" and second 97.
Show "Pair: {entry's first} = {entry's second}.".
"#;

// --- Collections ---

const CODE_SETS: &str = r#"# Sets
-- A set keeps one copy of each value.

## Main

Let names be a new Set of Text.
Add "Alice" to names.
Add "Bob" to names.
Add "Charlie" to names.
Add "Alice" to names.
Let count be length of names.
Show "After 4 adds (one duplicate): {count} members.".

If names contains "Bob":
    Show "Bob is in the set.".
Remove "Bob" from names.
Let remaining be length of names.
Show "After removing Bob: {remaining} members.".

Let evens be a new Set of Int.
Add 2 to evens.
Add 4 to evens.
Let small be a new Set of Int.
Add 1 to small.
Add 2 to small.

Let both be evens union small.
Show "union: {both}".
Let shared be evens intersection small.
Show "intersection: {shared}".
Let rest be evens without small.
Show "difference: {rest}".
"#;

const CODE_MAPS: &str = r#"# Maps
-- A map stores values by key; both access forms below are equivalent.

## Main

Let mutable inventory be a new Map of Text to Int.
Set inventory["iron"] to 50.
Set inventory["copper"] to 30.
Set item "gold" of inventory to 10.

Let iron be inventory["iron"].
Show "iron count: {iron}".

Set inventory["iron"] to 100.
Let updated be item "iron" of inventory.
Show "after restock: {updated}".

Show "Full inventory:".
Repeat for (name, count) in inventory:
    Show "  {name}: {count}".
"#;

// --- Functions ---

const CODE_HIGHER_ORDER: &str = r#"# Higher-Order Functions
-- Functions are values: pass them in, get new ones back.

## To twice (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(f(x)).

## To makeAdder (n: Int) -> fn(Int) -> Int:
    Return (x: Int) -> x + n.

## Main

Let double be (x: Int) -> x * 2.
Let doubled be double(21).
Show "double(21) = {doubled}".

Let quadrupled be twice(double, 5).
Show "twice(double, 5) = {quadrupled}".

Let add10 be makeAdder(10).
Let sum be add10(32).
Show "makeAdder(10) applied to 32 = {sum}".
"#;

// --- Distributed ---

pub const CODE_CRDT_COUNTERS: &str = r#"# Convergent Counter
-- A Shared type's fields are CRDTs: they merge without conflicts.

## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Increase c's points by 5.
Increase c's points by 3.
Show "Total points: {c's points}".
"#;

// --- Security ---

const CODE_POLICIES: &str = r#"# Policies & Capabilities
-- Policy rules read like a security document — and Check enforces them.

## Definition
A User has:
    a name: Text.
    a role: Text.

A Document has:
    an owner: Text.

## Policy
A User is admin if the user's role equals "admin".
A User can edit the Document if:
    The user is admin, OR
    The user's name equals the document's owner.

## Main

Let root be a new User with name "Root" and role "admin".
Check that root is admin.
Show "Admin check passed: Root is admin.".

Let alice be a new User with name "Alice" and role "editor".
Let doc be a new Document with owner "Alice".
Check that alice can edit doc.
Show "Capability check passed: Alice can edit her own document.".
"#;

// --- Memory ---

const CODE_ZONES: &str = r#"# Memory Zones
-- A zone is an arena: everything allocated inside is freed at the end.

## Main

Inside a zone called "Work":
    Let x be 42.
    Let y be 58.
    Show "Sum in zone: {x + y}".

Inside a zone called "Buffer" of size 1 MB:
    Let value be 100.
    Show "Value in the 1 MB zone: {value}".

Show "Zones cleaned up!".
"#;

// --- Native-only (Concurrency) ---

const CODE_TASKS: &str = r#"# Tasks
-- Launch runs a function concurrently; a handle can stop it.

## To worker:
    Show "worker done".

## To greet (name: Text):
    Show "Hello from {name}".

## Main

Launch a task to worker.
Show "main continues".

Launch a task to greet with "a task".

Let handle be Launch a task to worker.
Stop handle.
Show "second worker stopped".
"#;

const CODE_CHANNELS: &str = r#"# Pipes
-- A pipe carries values between tasks; Send blocks, Try does not.

## Main

Let ch be a Pipe of Int.
Send 42 into ch.
Receive x from ch.
Show "received: {x}".

Try to send 7 into ch.
Try to receive y from ch.
Show "try-receive got: {y}".
"#;

// --- Wave 3: basics / strings / collections / time-and-money ---

const CODE_INTERPOLATION: &str = r#"# String Interpolation
-- Any expression can sit in a {hole}; a colon adds a format spec.

## Main

Let name be "Ada".
Let age be 36.
Show "{name} is {age}.".
Show "Next year she will be {age + 1}.".

Let pi be 3.14159.
Show "pi to two places: {pi:.2}".

Let item be "socks".
Show "|{item:>10}|".
Show "|{item:<10}|".
Show "|{item:^10}|".

Let price be 19.99.
Show "that will be {price:$}".

Let x be 42.
Show "debugging: {x=}".
Show "literal braces: {{not a hole}}".
"#;

const CODE_NUMBERS: &str = r#"# Numbers Big & Small
-- Ints are exact and grow past 64 bits; Decimals never drift.

## Main

Let big be 2 ** 100.
Show "2 to the 100th = {big}".

Let third be 1.0 / 3.0.
Show "a third to 4 places: {third:.4}".

Let close be 0.1 + 0.2.
If close is approximately 0.3:
    Show "0.1 + 0.2 is approximately 0.3 (floats drift)".

Let exact be decimal("0.1") + decimal("0.2").
Show "with Decimal it is exactly {exact}".
"#;

const CODE_TEXT_BASICS: &str = r#"# Working with Text
-- Text is 1-indexed like every LOGOS collection.

## Main

Let word be "LOGOS".
Let letters be length of word.
Show "{word} has {letters} letters.".

Let first be item 1 of word.
Show "It starts with {first}.".

If word contains "GO":
    Show "There is GO in {word}.".

Let greeting be "Hello, " combined with "world!".
Show greeting.

Let mutable reversed be "".
Let mutable i be letters.
While i >= 1:
    Let c be item i of word.
    Set reversed to reversed combined with c.
    Set i to i - 1.
Show "Reversed: {reversed}".
"#;

const CODE_FORMATTING: &str = r#"# Formatted Output
-- Alignment and currency specs build tidy reports in the string itself.

## Main

Let header_item be "item".
Let header_cost be "cost".
Show "{header_item:<10}{header_cost:>8}".

Let apples be "apples".
Let apples_cost be 1.25.
Show "{apples:<10}{apples_cost:>8}".

Let bread be "bread".
Let bread_cost be 3.5.
Show "{bread:<10}{bread_cost:>8}".

Let total be apples_cost + bread_cost.
Show "total: {total:$}".
"#;

const CODE_FIND_AND_COUNT: &str = r#"# Scan & Count
-- Walk a Text by index to count what you care about.

## Main

Let sentence be "the cat sat on the mat".
Let target be "t".
Let mutable count be 0.
Let chars be length of sentence.
Repeat for i from 1 to chars:
    If item i of sentence equals target:
        Set count to count + 1.
Show "the letter {target} appears {count} times".

Let mutable words be 1.
Repeat for i from 1 to chars:
    If item i of sentence equals " ":
        Set words to words + 1.
Show "the sentence has {words} words".
"#;

const CODE_SLICING: &str = r#"# Slices & Copies
-- items M through N is an inclusive slice; copy of duplicates.

## Main

Let scores be [55, 70, 85, 90, 60].
Let middle be items 2 through 4 of scores.
Show "middle three: {middle}".

Let mutable working be copy of scores.
Set item 1 of working to 100.
Show "edited copy: {working}".
Show "original untouched: {scores}".
"#;

const CODE_TUPLES: &str = r#"# Tuples
-- A tuple holds a fixed group of mixed-type values, 1-indexed.

## Main

Let point be (10, 20).
Show "x = {point[1]}, y = {point[2]}".

Let city be ("Berlin", 3600000, true).
Show "{city[1]} has {city[2]} people".
Let slots be length of city.
Show "the tuple has {slots} slots".

Let mutable capitals be a new Map of Text to Text.
Set capitals["Germany"] to "Berlin".
Set capitals["France"] to "Paris".
Repeat for (country, capital) in capitals:
    Show "{capital} is the capital of {country}".
"#;

const CODE_FILTER_TRANSFORM: &str = r#"# Filter & Transform
-- The Repeat + Push pair is LOGOS's map/filter workhorse.

## Main

Let numbers be [1, 2, 3, 4, 5, 6].

Let mutable evens be a new Seq of Int.
Repeat for n in numbers:
    If n is even:
        Push n to evens.
Show "evens: {evens}".

Let mutable squares be a new Seq of Int.
Repeat for n in evens:
    Push n * n to squares.
Show "their squares: {squares}".

Let mutable total be 0.
Repeat for n in squares:
    Set total to total + n.
Show "sum of squares: {total}".
"#;

const CODE_DATES: &str = r#"# Dates & Moments
-- Dates, times of day, spans, and moments are all literal values.

## Main

Let liftoff be 2026-05-20.
Show "liftoff day: {liftoff}".

Let prep be liftoff - 3 days.
Show "prep starts: {prep}".

Let review be liftoff + 2 months.
Show "review lands: {review}".

Let sprint be 2 weeks and 3 days.
Show "sprint length: {sprint}".

Let standup be 9:30am.
Show "daily standup: {standup}".

Let kickoff be 2026-05-20 at 4pm.
Show "kickoff moment: {kickoff}".

If 9am < noon:
    Show "9am is before noon".
"#;

const CODE_MONEY: &str = r#"# Money
-- Decimal keeps cents exact; the :$ spec prints currency.

## Main

Let latte be decimal("4.50").
Let muffin be decimal("3.25").
Let subtotal be latte + muffin.
Show "subtotal: {subtotal:$}".

Let two_rounds be subtotal * 2.
Show "two rounds: {two_rounds:$}".

Let tip be decimal("1.55").
Let total be two_rounds + tip.
Show "with tip: {total:$}".

Let cents be decimal("0.1") + decimal("0.2").
Show "0.1 + 0.2 = {cents} exactly (no float drift)".
"#;

// --- Wave 4: functions / types ---

const CODE_DEFINING: &str = r#"# Defining Functions
-- Parameters are named and typed; prepositions read naturally.

## To area (width: Int) and (height: Int) -> Int:
    Return width * height.

## To withdraw (amount: Int) from (balance: Int) -> Int:
    Return balance - amount.

## To greet (name: Text):
    Show "Welcome, {name}!".

## Main

Let room be area(12, 10).
Show "a 12 x 10 room covers {room} square feet".

Let left be withdraw(40, 100).
Show "balance after withdrawing 40 from 100: {left}".

Let guest be "Ada".
Give guest to greet.
"#;

const CODE_CLOSURES: &str = r#"# Closures
-- A closure is an unnamed function value; it captures what it uses.

## Main

Let double be (x: Int) -> x * 2.
Let doubled be double(21).
Show "double(21) = {doubled}".

Let bonus be 100.
Let with_bonus be (score: Int) -> score + bonus.
Let final_score be with_bonus(42).
Show "42 with the captured bonus of {bonus}: {final_score}".

Let describe be (n: Int) ->:
    If n is even:
        Return "{n} is even".
    Return "{n} is odd".
Let verdict be describe(7).
Show verdict.
"#;

const CODE_UFCS: &str = r#"# Method-Style Calls
-- Any function can be called like a method: the receiver is argument one.

## To double (x: Int) -> Int:
    Return x * 2.

## To plus (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main

Let classic be double(5).
Show "function style: double(5) = {classic}".

Let method be (5).double().
Show "method style: (5).double() = {method}".

Let chained be (5).double().plus(3).double().
Show "chained: (5).double().plus(3).double() = {chained}".
"#;

const CODE_BYTES_WORDS: &str = r#"# Bits, Bytes & Words
-- Word types are fixed-width rings: arithmetic wraps, never traps.

## Main

Let max32 be word32(4294967295).
Let wrapped be max32 + word32(1).
Show "4294967295 + 1 wraps to {wrapped}".

Let low_bit be word32(1).
Let nudged be low_bit shifted left by 4.
Show "1 shifted left by 4 = {nudged}".
Let back be nudged shifted right by 2.
Show "16 shifted right by 2 = {back}".

Let bits be count_ones(255).
Show "count_ones(255) = {bits}".

Let mask be 255 & 170.
Show "255 AND 170 = {mask}".
Let mixed be 85 | 170.
Show "85 OR 170 = {mixed}".
Let flipped be 255 ^ 85.
Show "255 XOR 85 = {flipped}".

Let letter be `A`.
Show "a Char literal: {letter}".
"#;

const CODE_EXACT_NUMBERS: &str = r#"# Exact Arithmetic
-- Rationals stay fractions; Decimals keep scale; Complex is built in.

## Main

Let r: Rational be 1 / 3.
Let s: Rational be 1 / 6.
Let sum be r + s.
Show "1/3 + 1/6 = {sum}".

Let precise be decimal("19.99") * 3.
Show "19.99 * 3 = {precise} (no float drift)".

Let i be complex(0, 1).
Let squared be i * i.
Show "i * i = {squared}".
"#;

// --- Wave 5: algorithms ---

const CODE_SIEVE: &str = r#"# Sieve of Eratosthenes
-- Cross out every multiple; what survives is prime.

## Main

Let limit be 100.
Let mutable is_prime be a new Seq of Bool.
Repeat for i from 1 to limit:
    Push true to is_prime.

Set item 1 of is_prime to false.
Let mutable p be 2.
While p * p <= limit:
    If item p of is_prime:
        Let mutable multiple be p * p.
        While multiple <= limit:
            Set item multiple of is_prime to false.
            Set multiple to multiple + p.
    Set p to p + 1.

Let mutable count be 0.
Let mutable largest be 0.
Repeat for i from 1 to limit:
    If item i of is_prime:
        Set count to count + 1.
        Set largest to i.
Show "{count} primes at or below {limit}".
Show "the largest is {largest}".
"#;

const CODE_BINARY_SEARCH: &str = r#"# Binary Search
-- Halve the search space until the target is cornered.

## To find_index (target: Int) and (xs: Seq of Int) -> Int:
    Let mutable lo be 1.
    Let mutable hi be length of xs.
    While lo <= hi:
        Let mid be (lo + hi) / 2.
        Let candidate be item mid of xs.
        If candidate equals target:
            Return mid.
        If candidate < target:
            Set lo to mid + 1.
        Otherwise:
            Set hi to mid - 1.
    Return 0.

## Main

Let ordered be [3, 8, 15, 23, 42, 57, 91].
Let hit be find_index(42, ordered).
Show "found 42 at position {hit}".
Let miss be find_index(50, ordered).
If miss equals 0:
    Show "50 is not present".
"#;

const CODE_MERGE_SORT: &str = r#"# Merge Sort
-- Split in half, sort each side, merge the fronts.

## To merge_sorted (a: Seq of Int) and (b: Seq of Int) -> Seq of Int:
    Let mutable merged be a new Seq of Int.
    Let mutable i be 1.
    Let mutable j be 1.
    While i <= length of a and j <= length of b:
        Let from_a be item i of a.
        Let from_b be item j of b.
        If from_a <= from_b:
            Push from_a to merged.
            Set i to i + 1.
        Otherwise:
            Push from_b to merged.
            Set j to j + 1.
    While i <= length of a:
        Push item i of a to merged.
        Set i to i + 1.
    While j <= length of b:
        Push item j of b to merged.
        Set j to j + 1.
    Return merged.

## To merge_sort (xs: Seq of Int) -> Seq of Int:
    Let n be length of xs.
    If n <= 1:
        Return xs.
    Let half be n / 2.
    Let front be items 1 through half of xs.
    Let back be items (half + 1) through n of xs.
    Let sorted_front be merge_sort(front).
    Let sorted_back be merge_sort(back).
    Return merge_sorted(sorted_front, sorted_back).

## Main

Let messy be [38, 27, 43, 3, 9, 82, 10].
Let tidy be merge_sort(messy).
Show "before: {messy}".
Show "after: {tidy}".
"#;

const CODE_GCD: &str = r#"# Euclid's GCD
-- The oldest algorithm in the book, with a termination measure.

## To gcd (a: Int) and (b: Int) -> Int:
    Let mutable x be a.
    Let mutable y be b.
    While y > 0 (decreasing y):
        Let r be x % y.
        Set x to y.
        Set y to r.
    Return x.

## Main

Show "gcd(48, 18) = {gcd(48, 18)}".
Show "gcd(17, 5) = {gcd(17, 5)}".
Show "gcd(100, 75) = {gcd(100, 75)}".
"#;

const CODE_TWO_SUM: &str = r#"# Two Sum
-- One pass with a Set of seen values finds the pair.

## Main

Let numbers be [2, 7, 11, 15, 3].
Let target be 14.
Let mutable seen be a new Set of Int.
Let mutable found be false.
Repeat for n in numbers:
    Let complement be target - n.
    If seen contains complement and not found:
        Show "{complement} + {n} = {target}".
        Set found to true.
    Add n to seen.
If not found:
    Show "no pair sums to {target}".
"#;

const CODE_BFS: &str = r#"# Breadth-First Search
-- Explore level by level; the first arrival is the shortest path.

## Main

Let mutable graph be a new Map of Int to Seq of Int.
Set graph[1] to [2, 3].
Set graph[2] to [4].
Set graph[3] to [4, 5].
Set graph[4] to [6].
Set graph[5] to [6].
Set graph[6] to a new Seq of Int.

Let origin be 1.
Let goal be 6.
Let mutable frontier be [1].
Let mutable distance be a new Map of Int to Int.
Set distance[1] to 0.

While length of frontier > 0:
    Let mutable next_frontier be a new Seq of Int.
    Repeat for node in frontier:
        Repeat for neighbor in graph[node]:
            If not (distance contains neighbor):
                Set distance[neighbor] to distance[node] + 1.
                Push neighbor to next_frontier.
    Set frontier to next_frontier.

Show "shortest path from {origin} to {goal}: {distance[goal]} hops".
"#;

const CODE_COLLATZ: &str = r#"# Collatz
-- Halve if even; triple and add one if odd. Everyone reaches 1... probably.

## Main

Let mutable n be 27.
Let mutable steps be 0.
While n > 1:
    If n is even:
        Set n to n / 2.
    Otherwise:
        Set n to 3 * n + 1.
    Set steps to steps + 1.
Show "27 takes {steps} steps to reach 1".
"#;

// --- Wave 6: concurrency / distributed ---

const CODE_SELECT: &str = r#"# Select
-- Await the first of races receive arms against a timeout.

## Main

Let inbox be a Pipe of Int.
Send 99 into inbox.
Await the first of:
    Receive msg from inbox:
        Show "first arrival: {msg}".
    After 2 seconds:
        Show "timed out".

Let quiet be a Pipe of Int.
Await the first of:
    Receive msg from quiet:
        Show "got {msg}".
    After 1 seconds:
        Show "timed out waiting on the quiet pipe".
"#;

const CODE_CRDT_LWW: &str = r#"# Last-Write-Wins Register
-- A LastWriteWins field keeps whichever write happened latest.

## Definition
A Profile is Shared and has:
    status: LastWriteWins of Text.

## Main
Let mutable me be a new Profile.
Set me's status to "online".
Show "status: {me's status}".
Set me's status to "away".
Show "status now: {me's status}".
"#;

const CODE_CRDT_DIVERGENT: &str = r#"# Divergent Register
-- A Divergent field keeps both sides of a conflict until you Resolve it.

## Definition
A WikiPage is Shared and has:
    title: Divergent Text.

## Main
Let mutable page be a new WikiPage.
Set page's title to "Draft".
Show "title: {page's title}".
Resolve page's title to "Final".
Show "resolved title: {page's title}".
"#;

const CODE_CRDT_SHARED_SET: &str = r#"# Shared Sets
-- An add-wins set: when replicas disagree, the add survives the merge.

## Definition
A Team is Shared and has:
    members: a SharedSet of Text.

## Main
Let mutable local be a new Team.
Add "Ada" to local's members.
Add "Bob" to local's members.

Let mutable remote be a new Team.
Add "Bob" to remote's members.
Remove "Bob" from remote's members.

Merge remote into local.
If local's members contains "Bob":
    Show "Bob survives: concurrent add wins".
Let team_size be length of local's members.
Show "team size: {team_size}".
"#;

const CODE_CRDT_SHARED_SEQUENCE: &str = r#"# Shared Sequences
-- Replicated logs: appends from different replicas merge into one order.

## Definition
A Journal is Shared and has:
    lines: a SharedSequence of Text.

## Main
Let mutable mine be a new Journal.
Append "first entry" to mine's lines.
Append "second entry" to mine's lines.

Let mutable other be a new Journal.
Append "remote entry" to other's lines.

Merge other into mine.
Let total be length of mine's lines.
Show "entries after merge: {total}".
"#;

const CODE_CRDT_SHARED_MAP: &str = r#"# Shared Map
-- A key-value CRDT: each key keeps its latest write, and replicas that
-- merge converge on the same map.

## Definition
A Warehouse is Shared and has:
    stock: a SharedMap from Text to Int.

## Main
Let mutable w be a new Warehouse.
Set w's stock["wood"] to 50.
Set w's stock["stone"] to 30.
Show w's stock["wood"].
"#;

const CODE_CRDT_SYNC: &str = r#"# Network Sync
-- Sync joins a CRDT to a gossip topic. In the playground this runs in
-- offline single-node mode; compiled programs gossip with real peers.

## Definition
A Clicks is Shared and has:
    total: ConvergentCount.

## Main
Let mutable c be a new Clicks.
Sync c on "global-clicks".
Increase c's total by 1.
Increase c's total by 2.
Show "local clicks: {c's total}".
"#;

// --- Wave 7: crypto / verification / security ---

const CODE_HASHES: &str = r#"# Hash Functions
-- md5 and sha1 digest byte sequences right in the browser.

## Main

Let message be text_bytes("abc").
Let sha be sha1(message).
Let fingerprint be md5(message).

Show "sha1 of abc: first bytes {item 1 of sha}, {item 2 of sha}".
Show "md5 of abc: first byte {item 1 of fingerprint}".

Let sha_len be length of sha.
Let md5_len be length of fingerprint.
Show "digest lengths: sha1 {sha_len} bytes, md5 {md5_len} bytes".
"#;

const CODE_CHACHA20: &str = r#"# ChaCha20 in LOGOS
-- quarterRound below is stdlib code written in LOGOS itself (crypto.lg),
-- running on the RFC 8439 test vector.

## Main

Let mutable state be a new Seq of Word32.
Push word32(286331153) to state.
Push word32(16909060) to state.
Push word32(2609737539) to state.
Push word32(19088743) to state.

Let mixed be quarterRound(state, 0, 1, 2, 3).
Show "a = {item 1 of mixed}".
Show "b = {item 2 of mixed}".
Show "c = {item 3 of mixed}".
Show "d = {item 4 of mixed}".
"#;

const CODE_MLKEM: &str = r#"# Post-Quantum ML-KEM
-- ML-KEM-768 (FIPS 203): the lattice math is stdlib code written in
-- LOGOS (crypto.lg), proven bit-exact against the reference oracle.

## Main

Let mutable seed be a new Seq of Int.
Repeat for i from 1 to 32:
    Push i to seed.

Let ek be mlkem768Keygen(seed).
Let ek_size be length of ek.
Show "encapsulation key: {ek_size} bytes".

Let mutable msg be a new Seq of Int.
Repeat for i from 1 to 32:
    Push 42 to msg.
Let sealed be mlkem768Encaps(ek, msg).
Let sealed_size be length of sealed.
Let ct_size be sealed_size - 32.
Show "ciphertext: {ct_size} bytes, shared secret: 32 bytes".
"#;

const CODE_UUID: &str = r#"# UUIDs
-- RFC 9562 UUIDs, generated by stdlib code written in LOGOS (uuid.lg).

## Main

Let dns be uuidParse("6ba7b810-9dad-11d1-80b4-00c04fd430c8").
Let stable be uuidV5(dns, "www.example.com").
Show "v5 of example.com: {stable}".

Let mutable bytes be a new Seq of Int.
Repeat for i from 1 to 16:
    Push i * 11 to bytes.
Let tagged be uuidV4(bytes).
Show "v4 from fixed bytes: {tagged}".
"#;

const CODE_TERMINATION: &str = r#"# Proving Loops Halt
-- The (decreasing e) clause names a measure that must shrink every pass.

## To halvings (n: Int) -> Int:
    Let mutable remaining be n.
    Let mutable count be 0.
    While remaining > 1 (decreasing remaining):
        Set remaining to remaining / 2.
        Set count to count + 1.
    Return count.

## Main

Show "1024 halves to 1 in {halvings(1024)} steps".
Show "1000000 halves to 1 in {halvings(1000000)} steps".
"#;

const CODE_CHECK_GATE: &str = r#"# Check vs Assert
-- Assert guards logic errors; Check is a security gate that always
-- runs and always enforces its policy.

## Definition
An Account has:
    owner: Text.
    standing: Text.

## Policy
An Account is open if the account's standing equals "open".

## Main

Let acct be a new Account with owner "Ada" and standing "open".
Check that acct is open.
Show "gate passed: the account is open".

Let amount be 40.
Assert that amount is greater than 0.
Show "withdrawal of {amount} approved for {acct's owner}".
"#;

// --- Wave 8a: projects ---

const CODE_PROJ_TODO: &str = r#"# Todo List
-- Structs, an enum status, and a formatted checklist.

## A Status is one of:
    An Open.
    A Done.

## Definition
A Task has:
    label: Text.
    status: Status.

## Main

Let mutable chores be a new Seq of Task.
Let milk be a new Task with label "buy milk" and status a new Open.
Push milk to chores.
Let writing be a new Task with label "write examples" and status a new Done.
Push writing to chores.
Let plants be a new Task with label "water plants" and status a new Open.
Push plants to chores.

Show "TODO:".
Let mutable open_count be 0.
Repeat for chore in chores:
    Inspect chore's status:
        When Done: Show "  [x] {chore's label}".
        When Open:
            Show "  [ ] {chore's label}".
            Set open_count to open_count + 1.
Show "{open_count} remaining".
"#;

const CODE_PROJ_INVENTORY: &str = r#"# Inventory Manager
-- Map bookkeeping with a low-stock report.

## Main

Let mutable stock be a new Map of Text to Int.
Set stock["planks"] to 40.
Set stock["nails"] to 350.
Set stock["hinges"] to 6.
Set stock["varnish"] to 2.

Set stock["planks"] to stock["planks"] - 12.
Set stock["hinges"] to stock["hinges"] - 4.

Show "Inventory report:".
Repeat for (part, quantity) in stock:
    If quantity < 10:
        Show "  {part}: {quantity}  LOW - reorder".
    Otherwise:
        Show "  {part}: {quantity}".
"#;

const CODE_PROJ_DICE: &str = r#"# Dice Game
-- Dice from scratch: a linear congruential generator makes the rolls.
-- Same seed, same game — deterministic randomness you can replay.

## To next_seed (seed: Int) -> Int:
    Return (seed * 1103515245 + 12345) % 2147483648.

## Main

Let mutable rng be 20260712.
Let mutable player be 0.
Let mutable rival be 0.

Repeat for round in [1, 2, 3]:
    Set rng to next_seed(rng).
    Let yours be rng % 6 + 1.
    Set rng to next_seed(rng).
    Let theirs be rng % 6 + 1.
    Set player to player + yours.
    Set rival to rival + theirs.
    Show "round {round}: you rolled {yours}, rival rolled {theirs}".

Show "final score - you: {player}, rival: {rival}".
If player > rival:
    Show "you win!".
Otherwise If rival > player:
    Show "rival wins!".
Otherwise:
    Show "a tie!".
"#;

const CODE_PROJ_WEATHER: &str = r##"# Weather Stats
-- Min, max, mean, and an ASCII bar chart from a week of readings.

## Main

Let readings be [18, 21, 25, 24, 19, 16, 22].
Let mutable lowest be 999.
Let mutable highest be -999.
Let mutable total be 0.
Repeat for r in readings:
    Set total to total + r.
    If r < lowest:
        Set lowest to r.
    If r > highest:
        Set highest to r.
Let day_count be length of readings.
Let mean be total / day_count.

Show "week of readings: {readings}".
Show "low {lowest}, high {highest}, average {mean} (rounded down)".
Show "chart:".
Repeat for r in readings:
    Let mutable bar be "".
    Let stars be r - 15.
    Repeat for s from 1 to stars:
        Set bar to bar combined with "#".
    Show "  {r} {bar}".
"##;

const CODE_PROJ_LEDGER: &str = r#"# Bank Ledger
-- Exact Decimal money and a tidy statement.

## Main

Let mutable balance be decimal("250.00").
Show "opening balance: {balance:$}".

Let deposits be [decimal("75.50"), decimal("12.25")].
Repeat for d in deposits:
    Set balance to balance + d.
    Show "deposit {d:$} -> {balance:$}".

Let withdrawal be decimal("40.00").
Assert that withdrawal <= balance.
Set balance to balance - withdrawal.
Show "withdrew {withdrawal:$} -> {balance:$}".

Show "closing balance: {balance:$}".
"#;

// --- Wave 8b: projects ---

const CODE_PROJ_CAESAR: &str = r#"# Caesar Cipher
-- Shift each letter three places by scanning an alphabet string.

## To encode (message: Text) and (shift: Int) -> Text:
    Let alphabet be "abcdefghijklmnopqrstuvwxyz".
    Let mutable scrambled be "".
    Let message_len be length of message.
    Repeat for i from 1 to message_len:
        Let c be item i of message.
        Let mutable found_at be 0.
        Repeat for j from 1 to 26:
            If item j of alphabet equals c:
                Set found_at to j.
        If found_at equals 0:
            Set scrambled to scrambled combined with c.
        Otherwise:
            Let moved be (found_at - 1 + shift) % 26 + 1.
            Set scrambled to scrambled combined with item moved of alphabet.
    Return scrambled.

## Main

Let secret be encode("hello world", 3).
Show "encoded: {secret}".
Let restored be encode(secret, 23).
Show "decoded: {restored}".
"#;

const CODE_PROJ_WORDCOUNT: &str = r#"# Word Count
-- Count words and characters by scanning the text.

## Main

Let prose be "the quick brown fox jumps over the lazy dog".
Show "text: {prose}".

Let chars be length of prose.
Let mutable spaces be 0.
Repeat for i from 1 to chars:
    If item i of prose equals " ":
        Set spaces to spaces + 1.
Let words be spaces + 1.

Show "{chars} characters".
Show "{words} words".
"#;

const CODE_PROJ_MATRIX: &str = r#"# Matrix Multiply
-- A Seq of Seq is a matrix; three loops do the classic multiply.

## Main

Let mutable a be a new Seq of Seq of Int.
Push [1, 2] to a.
Push [3, 4] to a.
Let mutable b be a new Seq of Seq of Int.
Push [5, 6] to b.
Push [7, 8] to b.

Let mutable product be a new Seq of Seq of Int.
Repeat for i from 1 to 2:
    Let row_a be item i of a.
    Let mutable new_row be a new Seq of Int.
    Repeat for j from 1 to 2:
        Let mutable cell be 0.
        Repeat for k from 1 to 2:
            Let x be item k of row_a.
            Let row_b be item k of b.
            Let y be item j of row_b.
            Set cell to cell + x * y.
        Push cell to new_row.
    Push new_row to product.

Show "A x B = {product}".
"#;

const CODE_PROJ_SCHEDULER: &str = r#"# Event Scheduler
-- Moments and durations build an ordered agenda.

## Main

Let kickoff be 2026-06-01 at 9am.
Show "kickoff: {kickoff}".
Let standup be 2026-06-01 at 9:30am.
Show "standup: {standup}".
Let review be 2026-06-01 at 4pm.
Show "review: {review}".

If standup < review:
    Show "standup comes before review".

Let focus be 2h.
Show "each focus block runs {focus}".
"#;

const CODE_PROJ_VENDING: &str = r#"# Vending Machine
-- An enum state machine: coins move it, a purchase resets it.

## A MachineState is one of:
    An Idle.
    A HasCredit with cents Int.

## To insert (coins: Int) and (current: MachineState) -> MachineState:
    Inspect current:
        When Idle: Return a new HasCredit with cents coins.
        When HasCredit (c): Return a new HasCredit with cents (c + coins).

## Main

Let mutable machine be a new Idle.
Set machine to insert(50, machine).
Set machine to insert(25, machine).

Inspect machine:
    When Idle: Show "no credit".
    When HasCredit (c):
        Show "credit: {c} cents".
        If c >= 65:
            Show "dispensing soda".
            Let change be c - 65.
            Show "change: {change} cents".

Set machine to a new Idle.
Show "machine reset".
"#;

// --- Wave 8c: projects ---

const CODE_PROJ_ADVENTURE: &str = r#"# Tiny Text Adventure
-- Rooms live in a Map; a scripted walk drives the story.

## Main

Let mutable rooms be a new Map of Text to Text.
Set rooms["hall"] to "a dusty hall. Doors lead north and east.".
Set rooms["library"] to "a quiet library. A key glints on the desk.".
Set rooms["cellar"] to "a dark cellar. The exit door is locked.".

Let mutable here be "hall".
Let mutable has_key be false.

Let script be ["north", "take key", "south", "east"].
Repeat for command in script:
    Show "> {command}".
    If command equals "take key":
        Set has_key to true.
        Show "You found the key!".
    Otherwise:
        If command equals "north":
            Set here to "library".
        Otherwise If command equals "south":
            Set here to "hall".
        Otherwise If command equals "east":
            Set here to "cellar".
        Let room_desc be rooms[here].
        Show "You are in {room_desc}".

If has_key:
    Show "The key turns. You escape the cellar!".
"#;

const CODE_PROJ_CHAT: &str = r#"# Chat Room
-- Messages drain from the room pipe until a quiet timeout closes it.

## Main

Let room be a Pipe of Int.
Send 101 into room.
Send 102 into room.
Send 201 into room.

Let mutable open_room be true.
While open_room:
    Await the first of:
        Receive msg from room:
            If msg < 200:
                Show "<alice> message {msg}".
            Otherwise:
                Show "<bob> message {msg}".
        After 1 seconds:
            Show "room quiet - closing".
            Set open_room to false.
"#;

const CODE_PROJ_LEADERBOARD: &str = r#"# CRDT Leaderboard
-- Three replicas tally independently, then merge into one standings.

## Definition
A Board is Shared and has:
    alice: Tally.
    bob: Tally.

## Main
Let mutable phone be a new Board.
Increase phone's alice by 12.
Increase phone's bob by 7.

Let mutable laptop be a new Board.
Increase laptop's alice by 5.
Increase laptop's bob by 1.

Let mutable tablet be a new Board.
Increase tablet's alice by 2.
Increase tablet's bob by 10.
Decrease tablet's bob by 2.

Merge laptop into phone.
Merge tablet into phone.

Show "standings after merge:".
Show "  alice: {phone's alice}".
Show "  bob: {phone's bob}".
If phone's alice > phone's bob:
    Show "alice leads".
Otherwise:
    Show "bob leads".
"#;

const CODE_PROJ_HASHER: &str = r#"# Password Hasher
-- Salt the password, hash it, verify by rehashing. (A demo shape —
-- real systems use slow KDFs, not one hash round.)

## To hash_password (password: Text) and (salt: Text) -> Seq of Int:
    Let salted be salt combined with password.
    Return sha1(text_bytes(salted)).

## To verify (attempt: Text) and (salt: Text) and (expected: Seq of Int) -> Bool:
    Let candidate be hash_password(attempt, salt).
    Let hash_len be length of expected.
    Let mutable matching be true.
    Repeat for i from 1 to hash_len:
        If item i of candidate is not item i of expected:
            Set matching to false.
    Return matching.

## Main

Let stored be hash_password("hunter2", "s4lt").
Let stored_len be length of stored.
Show "stored digest: {stored_len} bytes".

If verify("hunter2", "s4lt", stored):
    Show "correct password accepted".
If not verify("wrong", "s4lt", stored):
    Show "wrong password rejected".
"#;

// ============================================================
// Math Mode Examples (Vernacular/Theorem Proving)
// ============================================================

// --- Wave M: kernel showcases ---

const MATH_PRELUDE_TOUR: &str = r#"-- What the kernel already knows
-- Every session starts with a typed prelude: naturals, equality,
-- logic, and exact integer arithmetic. Check asks for a type;
-- Eval computes a value.

Check Nat.
Check Zero.
Check Succ.
Check Eq.
Check refl.
Check True.
Check False.
Check Not.
Eval (add 20 22).
Eval (mul 6 7).
Eval (le 2 3).
"#;

const MATH_BIG_ARITHMETIC: &str = r#"-- Exact arithmetic past 2^63
-- These products overflow every machine integer. The kernel promotes
-- to exact big integers and keeps computing — so `refl` can PROVE an
-- equation by pure computation where bit-level tools must bit-blast.

Eval (mul 4000000000000000000 4).
Eval (add 9000000000000000000 9000000000000000000).
Eval (mul 123456789123456789 987654321987654321).

Definition big_eq : Eq Int (mul 4000000000000000000 4) (mul 8000000000000000000 2) :=
    refl Int (mul 4000000000000000000 4).
Check big_eq.
"#;

const MATH_TERMINATION_WALL: &str = r#"-- The wall is real: the kernel REJECTS recursion it cannot prove
-- terminates. Everything below builds up legally — parity, halving —
-- until `collatz_len` asks for the length of a Collatz descent.
-- Nobody has proven that descent finite. Watch the checker say no.

A Decision is either Yes or No.

## To negate (d: Decision) -> Decision:
    Consider d:
        When Yes: Yield No.
        When No: Yield Yes.

## To check_parity (n: Nat) -> Decision:
    Consider n:
        When Zero: Yield Yes.
        When Succ k: Yield negate(check_parity(k)).

## To is_odd (n: Nat) -> Decision:
    Yield negate(check_parity(n)).

## To halve (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k:
            Consider is_odd(k):
                When Yes: Yield Succ (halve(k)).
                When No: Yield halve(k).

-- Every definition above is certified total. This one cannot be:
-- the recursive call is not on a structurally smaller argument,
-- and the kernel refuses the axiom of wishful thinking.

## To collatz_len (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k: Yield Succ (collatz_len(halve(n))).
"#;

const MATH_TARSKI: &str = r#"## Theory Tarski

Axiom cong_pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom cong_inner_transitivity: for all a b c d e f,
    if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).
Axiom cong_identity: for all a b c, if Cong(a, b, c, c) then a = b.
Axiom segment_construction: for all q a b c, there exists x, Bet(q, a, x) and Cong(a, x, b, c).
Axiom bet_identity: for all a b, if Bet(a, b, a) then a = b.
Axiom pasch: for all a p c b q,
    if Bet(a, p, c) and Bet(b, q, c) then there exists x, Bet(p, x, b) and Bet(q, x, a).
Axiom five_segment: for all a b c d ap bp cp dp,
    if not (a = b) and Bet(a, b, c) and Bet(ap, bp, cp)
        and Cong(a, b, ap, bp) and Cong(b, c, bp, cp) and Cong(a, d, ap, dp) and Cong(b, d, bp, dp)
    then Cong(c, d, cp, dp).

Theorem cong_reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem cong_symmetry cites cong_reflexivity:
    prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
Theorem cong_transitivity cites cong_symmetry:
    prove for all a b c d e f, if Cong(a, b, c, d) and Cong(c, d, e, f) then Cong(a, b, e, f).
Theorem null_segment_identity: given Cong(P, Q, R, R); prove P = Q.
Theorem construct_point: prove there exists x, Bet(Q, A, x) and Cong(A, x, B, C).
Theorem cevians_meet: given Bet(A, P, C); given Bet(B, Q, C);
    prove there exists x, Bet(P, x, B) and Bet(Q, x, A).
Theorem degenerate_betweenness: given Bet(P, Q, P); prove P = Q.
Theorem outer_five_segment: given not (A = B); given Bet(A, B, C); given Bet(Ap, Bp, Cp);
    given Cong(A, B, Ap, Bp); given Cong(B, C, Bp, Cp); given Cong(A, D, Ap, Dp);
    given Cong(B, D, Bp, Dp); prove Cong(C, D, Cp, Dp).
"#;

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

pub const MATH_BOOL: &str = r#"-- Booleans
-- A two-constructor type, checked and evaluated by the kernel, and functions over it.

Inductive MyBool := Yes : MyBool | No : MyBool.

Check Yes.
Check No.
Eval Yes.
Eval No.

Definition id_bool : MyBool -> MyBool := fun b : MyBool => b.

Check id_bool.
Eval id_bool Yes.
Eval id_bool No.
"#;

const MATH_GODEL: &str = r#"-- Gödel Sentence Construction
-- Building the self-referential sentence G

-- The Provable predicate: "there exists a derivation concluding s"
Definition Provable : Syntax -> Prop :=
    fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).

-- The Gödel template T = "Not(Provable(x))"
-- When we apply the diagonal lemma, x becomes the code of T itself
Definition T : Syntax := SApp (SName "Not") (SApp (SName "Provable") (SVar 0)).

-- The Gödel sentence G = T[code(T)/x]
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

const MATH_INCOMPLETENESS: &str = r#"-- Gödel's First Incompleteness Theorem
-- If LOGOS is consistent, G is not provable

-- Setup: Provable predicate
Definition Provable : Syntax -> Prop :=
    fun s : Syntax => Ex Derivation (fun d : Derivation => Eq Syntax (concludes d) s).

-- Consistency: the system cannot prove False
Definition Consistent : Prop := Not (Provable (SName "False")).

-- The Gödel template and sentence
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

pub const MATH_GODEL_LITERATE: &str = r###"-- Gödel Sentence Construction (Literate Mode)
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

-- 1. The Provability Predicate
-- "s is provable if there exists a derivation d that concludes s"

## To be Provable (s: Syntax) -> Prop:
    Yield there exists a d: Derivation such that (concludes(d) equals s).

-- 2. The Template T
-- T encodes "Not(Provable(x))" as syntax

Let Not_Name be the Name "Not".
Let Provable_Name be the Name "Provable".
Let T be Apply(Not_Name, Apply(Provable_Name, Variable 0)).

-- 3. The Gödel Sentence G
-- G = T[code(T)/x] via the diagonal lemma
-- G says "I am not provable"

Let G be the diagonalization of T.

-- Verification

Check Provable.
Check T.
Check G.
Check Provable(G).
"###;

pub const MATH_INCOMPLETENESS_LITERATE: &str = r###"-- Gödel's First Incompleteness Theorem (Literate Mode)
-- "If LOGOS is consistent, then G is not provable"
--
-- This example demonstrates fully Literate syntax:
-- - "## To be Predicate" for predicate definitions
-- - "## To be Consistent -> Prop:" for nullary predicates
-- - "## Theorem:" blocks with "Statement:"
-- - "X implies Y" for logical implication
-- - "X equals Y" for equality propositions

-- 1. The Provability Predicate

## To be Provable (s: Syntax) -> Prop:
    Yield there exists a d: Derivation such that (concludes(d) equals s).

-- 2. Consistency Definition
-- A system is consistent if it cannot prove False

Let False_Name be the Name "False".

## To be Consistent -> Prop:
    Yield Not(Provable(False_Name)).

-- 3. The Gödel Sentences

Let T be Apply(the Name "Not", Apply(the Name "Provable", Variable 0)).
Let G be the diagonalization of T.

-- 4. The Theorem Statement

## Theorem: Godel_First_Incompleteness
    Statement: Consistent implies Not(Provable(G)).

-- Verification

Check Godel_First_Incompleteness.
Check Consistent.
Check Provable(G).
Check Not(Provable(G)).
"###;

const MATH_RING: &str = r###"-- The ring Tactic: Polynomial Equality by Normalization
-- The ring tactic proves polynomial equalities automatically!
-- It works by normalizing both sides to canonical polynomial form
-- and checking if they're structurally equal.
--
-- Supported operations: add, sub, mul (no division)
-- This is a decision procedure - it either proves the equality or fails.

-- Basic Setup

-- Type annotation (for the Eq constructor)
Definition T : Syntax := SName "Int".

-- Variables using de Bruijn indices
Definition x : Syntax := SVar 0.
Definition y : Syntax := SVar 1.
Definition z : Syntax := SVar 2.

-- Example 1: Reflexivity (x = x)

Definition refl_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) x) x.
Definition refl_proof : Derivation := try_ring refl_goal.
Definition refl_result : Syntax := concludes refl_proof.

Check refl_proof.
Eval refl_result.

-- Example 2: Commutativity of Addition (x + y = y + x)

Definition add_xy : Syntax := SApp (SApp (SName "add") x) y.
Definition add_yx : Syntax := SApp (SApp (SName "add") y) x.
Definition comm_add_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) add_xy) add_yx.

-- The ring tactic proves this automatically!
Definition comm_add_proof : Derivation := try_ring comm_add_goal.
Definition comm_add_result : Syntax := concludes comm_add_proof.

Check comm_add_proof.
Eval comm_add_result.

-- Example 3: Commutativity of Multiplication (x * y = y * x)

Definition mul_xy : Syntax := SApp (SApp (SName "mul") x) y.
Definition mul_yx : Syntax := SApp (SApp (SName "mul") y) x.
Definition comm_mul_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) mul_xy) mul_yx.

Definition comm_mul_proof : Derivation := try_ring comm_mul_goal.
Definition comm_mul_result : Syntax := concludes comm_mul_proof.

Check comm_mul_proof.
Eval comm_mul_result.

-- Example 4: Distributivity (x * (y + z) = x*y + x*z)

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

-- Example 5: The Collatz Algebra Step
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

-- Example 6: Associativity ((x + y) + z = x + (y + z))

Definition xy_plus_z : Syntax := SApp (SApp (SName "add") (SApp (SApp (SName "add") x) y)) z.
Definition x_plus_yz : Syntax := SApp (SApp (SName "add") x) (SApp (SApp (SName "add") y) z).
Definition assoc_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) xy_plus_z) x_plus_yz.

Definition assoc_proof : Derivation := try_ring assoc_goal.
Definition assoc_result : Syntax := concludes assoc_proof.

Check assoc_proof.
Eval assoc_result.

-- Example 7: Subtraction Cancellation (x - x = 0)

Definition x_minus_x : Syntax := SApp (SApp (SName "sub") x) x.
Definition zero_lit : Syntax := SLit 0.
Definition cancel_goal : Syntax := SApp (SApp (SApp (SName "Eq") T) x_minus_x) zero_lit.

Definition cancel_proof : Derivation := try_ring cancel_goal.
Definition cancel_result : Syntax := concludes cancel_proof.

Check cancel_proof.
Eval cancel_result.

-- Summary
-- The ring tactic is a decision procedure for polynomial ring equalities.
-- It handles: constants, variables, addition, subtraction, multiplication.
-- It does NOT handle: division, modulo, or non-polynomial operations.
--
-- Key insight: Both sides are normalized to a canonical polynomial form
-- (sum of monomials with sorted variable indices), and compared structurally.
-- If they match, the equality is provable. If not, it fails.
"###;

const MATH_LIA: &str = r###"-- The lia Tactic: Linear Integer Arithmetic
-- The lia tactic proves linear inequalities automatically!
-- It uses Fourier-Motzkin elimination to decide validity.
--
-- Supported: Lt (<), Le (<=), Gt (>), Ge (>=)
-- Expressions must be LINEAR: constants, variables, c*x (no x*y)

-- Basic Setup

-- Variables using de Bruijn indices
Definition x : Syntax := SVar 0.
Definition y : Syntax := SVar 1.
Definition z : Syntax := SVar 2.

-- Example 1: Reflexivity (x <= x)

Definition le_refl_goal : Syntax := SApp (SApp (SName "Le") x) x.
Definition le_refl_proof : Derivation := try_lia le_refl_goal.
Definition le_refl_result : Syntax := concludes le_refl_proof.

Check le_refl_proof.
Eval le_refl_result.

-- Example 2: Constant Inequality (2 < 5)

Definition const_lt_goal : Syntax := SApp (SApp (SName "Lt") (SLit 2)) (SLit 5).
Definition const_lt_proof : Derivation := try_lia const_lt_goal.
Definition const_lt_result : Syntax := concludes const_lt_proof.

Check const_lt_proof.
Eval const_lt_result.

-- Example 3: Successor (x < x + 1)

Definition x_plus_1 : Syntax := SApp (SApp (SName "add") x) (SLit 1).
Definition succ_goal : Syntax := SApp (SApp (SName "Lt") x) x_plus_1.
Definition succ_proof : Derivation := try_lia succ_goal.
Definition succ_result : Syntax := concludes succ_proof.

Check succ_proof.
Eval succ_result.

-- Example 4: Linear Coefficient (2*x <= 2*x)

Definition two_x : Syntax := SApp (SApp (SName "mul") (SLit 2)) x.
Definition linear_goal : Syntax := SApp (SApp (SName "Le") two_x) two_x.
Definition linear_proof : Derivation := try_lia linear_goal.
Definition linear_result : Syntax := concludes linear_proof.

Check linear_proof.
Eval linear_result.

-- Example 5: Predecessor (x - 1 < x)

Definition x_minus_1 : Syntax := SApp (SApp (SName "sub") x) (SLit 1).
Definition pred_goal : Syntax := SApp (SApp (SName "Lt") x_minus_1) x.
Definition pred_proof : Derivation := try_lia pred_goal.
Definition pred_result : Syntax := concludes pred_proof.

Check pred_proof.
Eval pred_result.

-- Example 6: Equality Bound (5 <= 5)

Definition eq_bound_goal : Syntax := SApp (SApp (SName "Le") (SLit 5)) (SLit 5).
Definition eq_bound_proof : Derivation := try_lia eq_bound_goal.
Definition eq_bound_result : Syntax := concludes eq_bound_proof.

Check eq_bound_proof.
Eval eq_bound_result.

-- Summary
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
-- Let binds a value; the type is inferred from it.

## Main

Let name be "Alice".
Let age be 25.
Let is_active be true.
Let price be 19.99.

Show "{name} is {age}, active: {is_active}".
Show "ticket price: {price:$}".

Let mutable count be 100.
Set count to count * 2.
Show "count doubled: {count}".
"#;

const CODE_BASICS_OPERATORS: &str = r#"# Operators
-- Symbols for arithmetic; English or symbols for comparisons.

## Main

Let a be 10.
Let b be 3.

Show "{a} + {b} = {a + b}".
Show "{a} - {b} = {a - b}".
Show "{a} * {b} = {a * b}".
Show "{a} / {b} = {a / b}".
Show "{a} % {b} = {a % b}".

If a is greater than b:
    Show "{a} is greater than {b}".
If 1 <= a <= 100:
    Show "{a} is between 1 and 100 (chained comparison)".
If a is between 5 and 15:
    Show "{a} is between 5 and 15".
If a is even:
    Show "{a} is even".
If b is odd:
    Show "{b} is odd".
If a is divisible by 5:
    Show "{a} is divisible by 5".

Let sunny be true.
Let warm be false.
Let both be sunny and warm.
Let either be sunny or warm.
Let flipped be not sunny.
Show "sunny and warm: {both}".
Show "sunny or warm: {either}".
Show "not sunny: {flipped}".
"#;

const CODE_BASICS_CONTROL_FLOW: &str = r#"# Control Flow
-- If chooses, While repeats, Repeat walks a range or a list.

## Main

Let score be 85.
If score is at least 90:
    Show "Grade: A".
Otherwise If score is at least 80:
    Show "Grade: B".
Otherwise:
    Show "Grade: C or below".

Show "Countdown:".
Let mutable count be 3.
While count >= 1:
    Show count.
    Set count to count - 1.
Show "Blastoff!".

Show "First multiple of 7 above 20:".
Repeat for n from 21 to 35:
    If n is divisible by 7:
        Show n.
        Break.
"#;

// ============================================================
// NEW: Enum Patterns Example (Guide Section 8)
// ============================================================

const CODE_ENUMS_PATTERNS: &str = r#"# Enums with Data
-- Variants can carry fields; Inspect binds them by position.

## A Shape is one of:
    A Circle with radius Int.
    A Rectangle with width Int and height Int.

## Main

Let round be a new Circle with radius 5.
Inspect round:
    When Circle (r): Show "Circle with radius {r}, area about {3 * r * r}".
    When Rectangle (w, h): Show "Rectangle {w} x {h}".

Let box be a new Rectangle with width 4 and height 6.
Inspect box:
    When Circle (r): Show "Circle with radius {r}".
    When Rectangle (w, h): Show "Rectangle {w} x {h}, area {w * h}".
"#;

// ============================================================
// NEW: Ownership Example (Guide Section 10)
// ============================================================

const CODE_OWNERSHIP: &str = r#"# Memory and Ownership
-- Show lends a value; Give moves it; copy of duplicates it.

## To display (data: Text):
    Show "Viewing: {data}".

## To consume (data: Text):
    Show "Consumed: {data}".

## Main

Let profile be "User Profile Data".

Show profile to display.
Show "Still usable after lending: {profile}".

Let duplicate be copy of profile.
Give duplicate to consume.

Show "Original intact: {profile}".
"#;

// ============================================================
// NEW: Concurrency Example (Guide Section 12)
// ============================================================

const CODE_CONCURRENCY_PARALLEL: &str = r#"# Parallel Blocks
-- Simultaneously runs statements on separate cores.
-- Attempt all overlaps async work and joins the results.

## Main

Simultaneously:
    Let squares be 25 * 25.
    Let cubes be 9 * 9 * 9.

Show "25^2 = {squares}, 9^3 = {cubes}".
Show "product: {squares * cubes}".

Attempt all of the following:
    Let x be 10.
    Let y be 20.

Show "x + y = {x + y}".
"#;

// ============================================================
// NEW: Additional CRDT Examples (Guide Section 13)
// ============================================================

pub const CODE_CRDT_TALLY: &str = r#"# Tally
-- A Tally counts both ways and still merges cleanly across replicas.

## Definition
A Score is Shared and has:
    points: Tally.

## Main
Let mutable s be a new Score.
Increase s's points by 100.
Show "after +100: {s's points}".

Decrease s's points by 30.
Show "after -30: {s's points}".

Increase s's points by 10.
Show "final: {s's points}".
"#;

const CODE_CRDT_MERGE: &str = r#"# Merging Replicas
-- Two replicas count independently, then merge without losing anything.

## Definition
A Stats is Shared and has:
    views: ConvergentCount.

## Main
Let mutable local be a new Stats.
Increase local's views by 100.
Show "local views: {local's views}".

Let mutable remote be a new Stats.
Increase remote's views by 50.
Show "remote views: {remote's views}".

Merge remote into local.
Show "after merge: {local's views}".
"#;

// ============================================================
// NEW: Networking Examples (Guide Section 15) - Native Only
// ============================================================

const CODE_NETWORK_SERVER: &str = r#"# P2P Server
-- Listen binds a libp2p multiaddr; LAN peers are found via mDNS.
-- Live networking needs the compiled runtime: build with `largo build`.

## Definition
A Message is Portable and has:
    content: Text.

## Main

Listen on "/ip4/0.0.0.0/tcp/8000".
Show "Server listening on port 8000".
Show "mDNS will auto-discover local peers".
"#;

const CODE_NETWORK_CLIENT: &str = r#"# P2P Client
-- Connect dials a peer; a PeerAgent is a handle for sending to it.
-- Live networking needs the compiled runtime: build with `largo build`.

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

const CODE_ERROR_HANDLING: &str = r#"# Outcomes, Not Crashes
-- A function that can fail returns an either-type; Inspect handles both arms.

## Definition
An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## To checked_divide (a: Int) and (b: Int) -> Outcome:
    If b equals 0:
        Return a new Failure with message "cannot divide {a} by zero".
    Return a new Success with value (a / b).

## Main

Let good be checked_divide(10, 2).
Inspect good:
    When Success (v): Show "10 / 2 = {v}".
    When Failure (m): Show "error: {m}".

Let bad be checked_divide(5, 0).
Inspect bad:
    When Success (v): Show "5 / 0 = {v}".
    When Failure (m): Show "error: {m}".
"#;

// ============================================================
// NEW: Advanced Examples (Guide Sections 17, 22-23)
// ============================================================

const CODE_ADVANCED_REFINEMENT: &str = r#"# Refinement Types
-- A type can carry a constraint; the value must satisfy it.

## Main

Let positive: Int where it > 0 be 5.
Let percentage: Int where it >= 0 and it <= 100 be 85.
Let dice: Int where it >= 1 and it <= 6 be 4.

Show "positive: {positive}".
Show "percentage: {percentage}".
Show "dice roll: {dice}".
"#;

const CODE_ADVANCED_ASSERTIONS: &str = r#"# Assert and Trust
-- Assert checks a claim at runtime; Trust records a justified assumption.

## To withdraw (amount: Int) from (balance: Int) -> Int:
    Assert that amount is greater than 0.
    Assert that amount is at most balance.
    Return balance - amount.

## To process (n: Int) -> Int:
    Trust that n is greater than 0 because "caller guarantees positive input".
    Return n * 2.

## Main

Let result be withdraw(50, 100).
Show "withdrew 50 from 100, leaving {result}".

Let doubled be process(5).
Show "process(5) = {doubled}".
"#;

// ============================================================
// Temporal Types Example
// ============================================================

const CODE_TEMPORAL: &str = r#"# Durations
-- Time literals are first-class values with real units.

## Main

Let blink be 50ns.
Let tick be 100us.
Let pause be 500ms.
Let breath be 1s.
Show "blink {blink}, tick {tick}, pause {pause}, breath {breath}".

Let total be pause + pause.
Show "500ms + 500ms = {total}".

If pause < breath:
    Show "500ms is less than 1s".

Show "Sleeping 200ms...".
Sleep 200ms.
Show "Awake!".
"#;

// ============================================================
// MATH_CC: Congruence Closure Tactic Example
// ============================================================

const MATH_CC: &str = r###"-- The cc Tactic: Congruence Closure
-- The cc tactic proves equalities over uninterpreted functions!
-- It uses the congruence rule: if a = b then f(a) = f(b)
--
-- Key insight: cc connects arithmetic proofs to function applications.
-- While ring proves "1 + 1 = 2", cc proves "f(1 + 1) = f(2)".

-- Example 1: Reflexivity (f(x) = f(x))
-- Any term equals itself

## Theorem: FxRefl
    Statement: (Eq (f x) (f x)).
    Proof: cc.

Check FxRefl.

-- Example 2: Nested Reflexivity (f(g(x)) = f(g(x)))

## Theorem: FgxRefl
    Statement: (Eq (f (g x)) (f (g x))).
    Proof: cc.

Check FgxRefl.

-- Example 3: Congruence (x = y → f(x) = f(y))
-- The core congruence rule: equal arguments give equal results

## Theorem: Congruence
    Statement: (implies (Eq x y) (Eq (f x) (f y))).
    Proof: cc.

Check Congruence.

-- Example 4: Binary Congruence (a = b → add(a,c) = add(b,c))
-- Congruence works for multi-argument functions too

## Theorem: BinaryCongruence
    Statement: (implies (Eq a b) (Eq (add a c) (add b c))).
    Proof: cc.

Check BinaryCongruence.

-- Example 5: Transitivity Chain (a = b → b = c → f(a) = f(c))
-- Multiple hypotheses combine via transitivity

## Theorem: Transitivity
    Statement: (implies (Eq a b) (implies (Eq b c) (Eq (f a) (f c)))).
    Proof: cc.

Check Transitivity.

-- Summary
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

const MATH_SIMP: &str = r###"-- The simp Tactic: Term Rewriting
-- The simp tactic normalizes goals by applying rewrite rules!
-- It unfolds definitions and simplifies arithmetic.
--
-- Key insight: simp turns complex terms into canonical forms,
-- making equalities trivially checkable by reflexivity.

-- Example 1: Arithmetic Simplification
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

-- Example 2: Definition Unfolding

## To double (n: Int) -> Int:
    Yield (add n n).

## Theorem: DoubleTwo
    Statement: (double 2) equals 4.
    Proof: simp.

Check DoubleTwo.

## To quadruple (n: Int) -> Int:
    Yield (double (double n)).

## Theorem: QuadTwo
    Statement: (quadruple 2) equals 8.
    Proof: simp.

Check QuadTwo.

## To zero_fn (n: Int) -> Int:
    Yield 0.

## Theorem: ZeroFnTest
    Statement: (zero_fn 42) equals 0.
    Proof: simp.

Check ZeroFnTest.

-- Example 3: With Hypotheses
-- simp uses equalities from hypotheses as rewrite rules

## Theorem: SubstSimp
    Statement: (implies (Eq Int x 0) (Eq Int (add x 1) 1)).
    Proof: simp.

Check SubstSimp.

## Theorem: TwoHyps
    Statement: (implies (Eq Int x 1) (implies (Eq Int y 2) (Eq Int (add x y) 3))).
    Proof: simp.

Check TwoHyps.

-- Example 4: Reflexive Equalities
-- simp handles reflexivity for free

## Theorem: XEqX
    Statement: (Eq x x).
    Proof: simp.

Check XEqX.

## Theorem: FxRefl
    Statement: (Eq (f x) (f x)).
    Proof: simp.

Check FxRefl.

-- Summary
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

const MATH_OMEGA: &str = r###"-- The omega Tactic: True Integer Arithmetic
-- The omega tactic handles LINEAR INTEGER constraints!
-- Unlike lia (which uses rationals), omega knows that:
--   x > 1  means  x >= 2  for integers
--   2x = 3  has NO solution (3 is odd!)
--
-- This is essential for array bounds, loop indices,
-- and anything involving discrete counts.

-- Basic Inequalities (same as lia)

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

-- Integer-Specific Reasoning
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

-- Transitivity and Chains

## Theorem: LtTrans
    Statement: (implies (Lt x y) (implies (Lt y z) (Lt x z))).
    Proof: omega.

Check LtTrans.

## Theorem: LeTrans
    Statement: (implies (Le x y) (implies (Le y z) (Le x z))).
    Proof: omega.

Check LeTrans.

-- Summary
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

pub const MATH_AUTO: &str = r###"-- The auto Tactic: The Infinity Gauntlet
-- The auto tactic combines ALL decision procedures!
-- It tries each one in sequence until one succeeds:
--   1. True/False (trivial propositions)
--   2. simp  (simplification)
--   3. ring  (polynomial algebra)
--   4. cc    (congruence closure)
--   5. omega (integer arithmetic)
--   6. lia   (linear arithmetic)

-- Simplification (auto -> simp)

## Theorem: TrueIsTrue
    Statement: True.
    Proof: auto.

Check TrueIsTrue.

-- Ring Algebra (auto -> ring)

## Theorem: AddCommutative
    Statement: (add a b) equals (add b a).
    Proof: auto.

Check AddCommutative.

## Theorem: AddAssociative
    Statement: (add (add a b) c) equals (add a (add b c)).
    Proof: auto.

Check AddAssociative.

## Theorem: MulDistributes
    Statement: (mul a (add b c)) equals (add (mul a b) (mul a c)).
    Proof: auto.

Check MulDistributes.

-- Congruence Closure (auto -> cc)

## Theorem: FunctionReflexive
    Statement: (Eq (f x) (f x)).
    Proof: auto.

Check FunctionReflexive.

-- Integer Arithmetic (auto -> omega)

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

-- Linear Arithmetic (auto -> lia/omega)

## Theorem: LeReflexive
    Statement: (Le x x).
    Proof: auto.

Check LeReflexive.

## Theorem: LeTransitive
    Statement: (implies (Le x y) (implies (Le y z) (Le x z))).
    Proof: auto.

Check LeTransitive.

-- The Power of auto
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

const MATH_INDUCTION: &str = r###"-- The induction Tactic: The Time Machine
-- Structural reasoning for inductive types.
-- Works for Nat, Bool, and any user-defined inductive.
--
-- The induction tactic automatically:
-- 1. Looks up constructors for the inductive type
-- 2. Generates one subgoal per constructor
-- 3. Provides induction hypotheses for recursive cases

-- Kernel Infrastructure
-- These are the building blocks for induction.

-- Check that induction helpers exist
Check try_induction.
Check induction_base_goal.
Check induction_step_goal.
Check induction_num_cases.

-- Nat Induction Examples
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

-- Bool Induction
-- Bool has 2 constructors: true, false

-- How many constructors does Bool have?
Definition bool_cases : Nat := induction_num_cases (SName "Bool").
Eval bool_cases.

-- Check the Bool type
Check Bool.
Check true.
Check false.

-- Building a Complete Proof
-- Let's build an induction proof manually using the kernel.

-- Base case: Le Zero Zero (auto can solve this)
Definition base_proof : Derivation :=
    try_auto (SApp (SApp (SName "Le") (SName "Zero")) (SName "Zero")).
Definition base_result : Syntax := concludes base_proof.
Eval base_result.

-- Step case: use axiom for now (step needs IH)
Definition step_proof : Derivation := DAxiom step_goal.

-- Combine into full induction proof
Definition full_proof : Derivation :=
    try_induction (SName "Nat") le_motive (DCase base_proof (DCase step_proof DCaseEnd)).
Definition full_result : Syntax := concludes full_proof.
Eval full_result.

-- Error Handling
-- Wrong number of cases should error

Definition motive2 : Syntax := SLam (SName "Nat") (SName "True").
Definition single_case : Derivation := DAxiom (SName "True").

-- Only 1 case for 2-constructor Nat = error
Definition bad_proof : Derivation :=
    try_induction (SName "Nat") motive2 (DCase single_case DCaseEnd).
Definition bad_result : Syntax := concludes bad_proof.
Eval bad_result.

-- Non-Inductive Types
-- Int is not an inductive type

Definition int_cases : Nat := induction_num_cases (SName "Int").
Eval int_cases.

-- Summary
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

const MATH_HINTS: &str = r###"-- The Hint Database: Teaching auto New Tricks
-- Register theorems as hints so auto can use them!
--
-- The hint system allows you to:
-- 1. Prove a theorem once
-- 2. Register it with "Attribute: hint."
-- 3. Auto will try to use it when other tactics fail

-- A Basic Hint

-- First, let's define a simple property
Definition trivial_true : Syntax := SName "True".

-- Prove it (trivially)
Definition trivial_proof : Derivation := try_auto trivial_true.

-- Verify the proof
Eval concludes trivial_proof.

-- How Hints Work
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

-- Hint-Aware auto
-- Auto tries tactics in this order:
-- 1. Trivial (True/False)
-- 2. simp (simplification)
-- 3. ring (polynomial arithmetic)
-- 4. cc (congruence closure)
-- 5. omega (integer arithmetic)
-- 6. lia (linear arithmetic)
-- 7. HINTS (registered theorems) <-- NEW!

-- Literate Syntax for Hints
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

-- Checking Hints
-- You can inspect the hint database via the context.
-- Hints are stored as theorem names.

Check try_auto.

-- Summary
-- The hint system extends auto with learned knowledge:
--
-- 1. Prove theorems normally
-- 2. Add "Attribute: hint." to register them
-- 3. Auto will try hints when built-in tactics fail
--
-- This creates a virtuous cycle:
--   Prove lemmas → Register as hints → Prove harder theorems
"###;

const MATH_INVERSION: &str = r###"-- Inversion: The Scalpel
-- Derives contradictions by running constructors backwards.
--
-- If you claim something impossible (like Eq Nat 3 0),
-- inversion proves False by showing no constructor can build it.

-- Kernel Infrastructure
-- These are the building blocks for inversion.

-- Check that inversion helpers exist
Check try_inversion.
Check DInversion.

-- Discriminate: Different Constructors
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

-- Reflexive: The Constructor Can Match
-- Eq Nat Zero Zero CAN be built by refl Zero
-- So inversion should NOT derive False (returns error)

Definition eq_zero_zero : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero").

Definition reflexive_proof : Derivation := try_inversion eq_zero_zero.
Definition reflexive_result : Syntax := concludes reflexive_proof.
Eval reflexive_result.

-- Empty Inductive: False
-- False has NO constructors at all.
-- Anything of type False is automatically contradictory.

Definition false_hyp : Syntax := SName "False".
Definition false_proof : Derivation := try_inversion false_hyp.
Definition false_result : Syntax := concludes false_proof.
Eval false_result.

-- Bool Discriminate: true ≠ false
-- Eq Bool true false requires refl to make true = false
-- But true and false are different constructors!

Definition eq_true_false : Syntax :=
    SApp (SApp (SApp (SName "Eq") (SName "Bool")) (SName "true")) (SName "false").

Definition bool_proof : Derivation := try_inversion eq_true_false.
Definition bool_result : Syntax := concludes bool_proof.
Eval bool_result.

-- Non-Inductive: An Error
-- Inversion only works on inductive types.
-- Variables or unknown types produce errors.

Definition var_hyp : Syntax := SVar 0.
Definition var_proof : Derivation := try_inversion var_hyp.
Definition var_result : Syntax := concludes var_proof.
Eval var_result.

-- Summary
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

const MATH_OPERATOR: &str = r###"-- The Operator: Manual Control Tactics
-- When auto fails, you need precision tools.
--
-- rewrite  - The Sniper: targeted substitution
-- destruct - The Fork: case analysis without IH
-- apply    - The Arrow: backward chaining

-- Kernel Infrastructure

Check try_rewrite.
Check try_rewrite_rev.
Check try_destruct.
Check try_apply.
Check DRewrite.
Check DDestruct.
Check DApply.

-- rewrite: The Sniper
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

-- destruct: The Fork
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

-- apply: The Arrow
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

-- Summary
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

const MATH_TACTICALS: &str = r###"-- The Strategist: Programmable Proofs
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

-- tact_try: The Safety Net
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

-- tact_repeat: The Loop
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

-- tact_then: The Sequencer (;)
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

-- tact_first: The Menu
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

-- tact_solve: The Enforcer
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

-- The Nuclear Code
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

-- Combining Tacticals
-- Real power: nest them!

-- repeat (first [refl, simp]) - keep trying until nothing works
Definition solve_trivial : Syntax -> Derivation :=
    tact_repeat (tact_first (TacCons try_refl
                            (TacCons (tact_try try_simp) TacNil))).

Definition d_trivial : Derivation := solve_trivial goal_refl.
Eval (concludes d_trivial).

-- Summary
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
// example. The Studio serves exactly these (`studio_files`), and the
// `example_health` test suite drives every one through the SAME pipeline the
// Studio uses and asserts its documented intended outcome. A new example must
// be added here with its `Expected`, so nothing can ship unlocked.
// ============================================================================

/// Web page: a counter. A `.logos` file that declares a `## Page` opens in the **Web** tab, where
/// it is compiled to WebAssembly and runs live.
pub const CODE_WEB_COUNTER: &str = r##"# Counter — a LOGOS page

A page owns a URL and mounts itself: no `## Main`, no `Mount`. One click is one TURN —
the action runs, the page repaints once.

## Page /

State:
    Let count be 0.

Actions:
    To bump:
        Set count to count + 1.

    To reset:
        Set count to 0.

View:
    <h1 id="count">Count: {count}</h1>
    <button id="bump" on-click={bump}>+1</button>
    <button id="reset" on-click={reset}>reset</button>

## Test: each click is one turn

Click "#bump".
Click "#bump".
Expect that the text of "#count" is equal to "Count: 2".
Click "#reset".
Expect that the text of "#count" is equal to "Count: 0".
"##;

/// Web page: a to-do list you add to, tick off and remove from — records in `State:`, a form
/// with `.prevent`, a keyed `{for each}` with an empty arm, and a derived count.
pub const CODE_WEB_TODO: &str = r##"# To-Do — add, finish, remove

`State:` holds an ordinary list of records. Each row is keyed by its id, so removing
one leaves the others' elements where they are. `left` is derived: it recomputes
whenever `todos` changes.

## A Todo has:
    an id, which is Int.
    a text, which is Text.
    a done, which is Bool.

## To openCount (todos: Seq of Todo) -> Int:
    Let mutable n be 0.
    Repeat for t in todos:
        If t's done is equal to false:
            Set n to n + 1.
    Return n.

## Page /

State:
    Let todos be a new Seq of Todo.
    Let draft be "".
    Let nextId be 1.
    Let left always be openCount(todos).

Actions:
    To add:
        If draft is equal to "":
            Return.
        Push (a new Todo with id nextId and text draft and done false) to todos.
        Set nextId to nextId + 1.
        Set draft to "".

    To toggle (id: Int):
        Let mutable next be a new Seq of Todo.
        Repeat for t in todos:
            If t's id is equal to id:
                Push (a new Todo with id t's id and text t's text and done (not t's done)) to next.
            Otherwise:
                Push t to next.
        Set todos to next.

    To remove (id: Int):
        Let mutable kept be a new Seq of Todo.
        Repeat for t in todos:
            If t's id is not equal to id:
                Push t to kept.
        Set todos to kept.

View:
    <h1>To-Do</h1>
    <form id="add" on-submit.prevent={add}>
      <input id="draft" type="text" aria-label="New task" bind={draft} placeholder="What needs doing?">
      <button type="submit">Add</button>
    </form>
    <ul>
      {for each t in todos, keyed by t's id:}
        <li class="todo" class:done={t's done}>
          <button id="toggle-{t's id}" on-click={toggle(t's id)}>{t's text}</button>
          <button id="remove-{t's id}" on-click={remove(t's id)}>remove</button>
        </li>
      {else:}
        <li class="empty">Nothing to do.</li>
      {end}
    </ul>
    <p id="left">{left} left</p>

Style:
    .done button:first-child { text-decoration: line-through; opacity: .6; }

## Test: todos are added, finished and removed

Type "milk" into "#draft".
Submit "#add".
Type "bread" into "#draft".
Submit "#add".
Expect that the count of ".todo" is equal to 2.
Click "#toggle-1".
Expect that the text of "#left" is equal to "1 left".
Click "#remove-1".
Expect that the count of ".todo" is equal to 1.
Expect that the text of "#toggle-2" is equal to "bread".
"##;

/// Web page: a form — two-way binding (`bind={…}`), `on-submit.prevent`, `forms.lg` validation,
/// and a scoped `Style:` block.
pub const CODE_WEB_FORM: &str = r##"# Form — binding, validation and scoped style

`bind={name}` binds an input to a field both ways. `forms.lg` collects what was
wrong with a submission, one message per field. `Style:` is scoped to this page, so
`.error` cannot collide with anything else on the site.

## Page /

State:
    Let name be "".
    Let password be "".
    Let errors be a new Seq of FieldError.
    Let sent be false.

Actions:
    To send:
        Let mutable e be noErrors().
        Set e to checkRequired(e, "name", name).
        Set e to checkMinLength(e, "password", password, 8).
        Set errors to e.
        Set sent to isValid(e).

View:
    <form id="signup" on-submit.prevent={send}>
      <label>Name <input id="name" type="text" bind={name}></label>
      <p class="error" id="name-error">{errorFor(errors, "name")}</p>
      <label>Password <input id="password" type="password" bind={password}></label>
      <p class="error" id="password-error">{errorFor(errors, "password")}</p>
      <button type="submit">Sign up</button>
    </form>
    {if sent:}<p class="greet" id="greet">Welcome, {name}!</p>{end}

Style:
    .error { color: #b3261e; }
    .greet { color: #cc785c; font-weight: 600; }

## Test: an empty form names each problem

Submit "#signup".
Expect that the text of "#name-error" is equal to "This is required.".
Expect that the text of "#password-error" is equal to "Use at least 8 characters.".

## Test: a complete form is welcomed

Type "Ada" into "#name".
Type "correct horse" into "#password".
Submit "#signup".
Expect that the text of "#greet" is equal to "Welcome, Ada!".
"##;

/// Web page: a website in a website, the front page's live example: a card with a list, a counted
/// button and a light switch. Kept in its own file so the front page shows exactly what ships.
pub const CODE_WEB_SITE: &str = include_str!("examples/website.logos");

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
    /// Reserved for examples whose output is genuinely nondeterministic (e.g.
    /// real randomness); every deterministic example uses [`Expected::OutputExact`].
    OutputContains(&'static [&'static str]),
    /// Code: runs in the baseline interpreter with no error and prints EXACTLY
    /// this output — every line, in order, byte-for-byte (the zero-drift lock).
    /// Codegen must also succeed, as with `OutputContains`.
    OutputExact(&'static str),
    /// Code that cannot run in the browser interpreter (real OS networking): it
    /// must still generate Rust cleanly.
    NativeOnlyCompiles,
    /// A web page (`## Page`): it compiles to a self-contained wasm-dom module — the path the
    /// Studio's Web preview runs — and every `## Test` it carries passes. A page drives the DOM
    /// rather than printing, so its own tests are its output lock.
    WebPage,
    /// Math: every statement executes without error in a fresh kernel `Repl`.
    KernelAllStatementsOk,
    /// Math intentionally open: statements execute until a documented admitted
    /// goal (the marker string names the open point, e.g. a termination wall).
    KernelAdmitsAt(&'static str),
    /// Math `## Theory` development: every theorem in the axiomatic development
    /// is independently kernel-certified by the multi-theorem driver.
    TheoryAllVerified,
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
    ExampleSpec {
        vfs_path: "/examples/logic/simple-sentences.logic",
        mode: Mode::Logic,
        source: LOGIC_SIMPLE,
        expected: Expected::CompilesToFol(&["Sleep", "Bark", "Loudly", "Love", "Fail"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/quantifiers.logic",
        mode: Mode::Logic,
        source: LOGIC_QUANTIFIERS,
        expected: Expected::CompilesToFol(&["Student", "Book", "Professor", "Exam", "Cat"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/tense-aspect.logic",
        mode: Mode::Logic,
        source: LOGIC_TENSE,
        expected: Expected::CompilesToFol(&["Run", "Eat", "Arrive", "Sleep", "Work"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/garden-path.logic",
        mode: Mode::Logic,
        source: LOGIC_GARDEN_PATH,
        expected: Expected::CompilesToFol(&["Horse", "Race", "Fall"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/events-adverbs.logic",
        mode: Mode::Logic,
        source: LOGIC_EVENTS_ADVERBS,
        expected: Expected::CompilesToFol(&["Quickly", "Carefully", "Loudly"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/negation-polarity.logic",
        mode: Mode::Logic,
        source: LOGIC_NPI,
        expected: Expected::CompilesToFol(&["Dog", "Bark", "Error"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/plurals-collectives.logic",
        mode: Mode::Logic,
        source: LOGIC_PLURALS,
        expected: Expected::CompilesToFol(&["σBoy", "Gather", "Love"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/comparatives.logic",
        mode: Mode::Logic,
        source: LOGIC_COMPARATIVES,
        expected: Expected::CompilesToFol(&["Taller", "2 inches", "Higher"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/modality.logic",
        mode: Mode::Logic,
        source: LOGIC_MODALITY,
        expected: Expected::CompilesToFol(&["Student", "FEW", "Bird"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/donkey-anaphora.logic",
        mode: Mode::Logic,
        source: LOGIC_DONKEY,
        expected: Expected::CompilesToFol(&["Farmer", "Donkey", "Beat"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/intensionality.logic",
        mode: Mode::Logic,
        source: LOGIC_INTENSIONALITY,
        expected: Expected::CompilesToFol(&["Seek", "^Unicorn", "Believe"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/counterfactuals.logic",
        mode: Mode::Logic,
        source: LOGIC_COUNTERFACTUALS,
        expected: Expected::CompilesToFol(&["□→", "Study", "Glass"]),
    },
    ExampleSpec {
        vfs_path: "/examples/logic/prover-demo.logic",
        mode: Mode::Logic,
        source: LOGIC_PROVER,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/simon.logic",
        mode: Mode::Logic,
        source: LOGIC_SIMON,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/syllogism.logic",
        mode: Mode::Logic,
        source: LOGIC_SYLLOGISM,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/disjunctive-syllogism.logic",
        mode: Mode::Logic,
        source: LOGIC_DISJUNCTIVE,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/modus-tollens.logic",
        mode: Mode::Logic,
        source: LOGIC_MODUS_TOLLENS,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/leibniz-identity.logic",
        mode: Mode::Logic,
        source: LOGIC_LEIBNIZ,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/neither-nor.logic",
        mode: Mode::Logic,
        source: LOGIC_NEITHER_NOR,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/equality-rewrite.logic",
        mode: Mode::Logic,
        source: LOGIC_EQUALITY_REWRITE,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/presupposition.logic",
        mode: Mode::Logic,
        source: LOGIC_PRESUPPOSITION,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/grid-starter.logic",
        mode: Mode::Logic,
        source: LOGIC_GRID_STARTER,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/grid-question.logic",
        mode: Mode::Logic,
        source: LOGIC_GRID_QUESTION,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/tactic-script.logic",
        mode: Mode::Logic,
        source: LOGIC_TACTIC_SCRIPT,
        expected: Expected::Proves,
    },
    ExampleSpec {
        vfs_path: "/examples/logic/barber-paradox.logic",
        mode: Mode::Logic,
        source: LOGIC_BARBER,
        expected: Expected::DerivationNotCertified,
    },
];

/// The 36 Code-mode examples. Output needles are provisional until the audit
/// run confirms the real interpreter output.
pub const ALL_CODE_EXAMPLES: &[ExampleSpec] = &[
    ExampleSpec { vfs_path: "/examples/code/hello-world.logos", mode: Mode::Code, source: CODE_HELLO, expected: Expected::OutputExact("Hello, LOGOS!\n10 + 20 = 30") },
    ExampleSpec { vfs_path: "/examples/code/basics/fizzbuzz.logos", mode: Mode::Code, source: CODE_FIZZBUZZ, expected: Expected::OutputExact("1\n2\nFizz\n4\nBuzz\nFizz\n7\n8\nFizz\nBuzz\n11\nFizz\n13\n14\nFizzBuzz\n16\n17\nFizz\n19\nBuzz") },
    ExampleSpec { vfs_path: "/examples/code/basics/syntax-flavors.logos", mode: Mode::Code, source: CODE_SYNTAX_FLAVORS, expected: Expected::OutputExact("bound with the = sign\nLet and = both bind values: 20\nOtherwise If flavor: FizzBuzz\nelif flavor: FizzBuzz\nmut and mutable both work: 12\nsymbols and English agree: 15 is divisible by 5") },
    ExampleSpec { vfs_path: "/examples/code/basics/interpolation.logos", mode: Mode::Code, source: CODE_INTERPOLATION, expected: Expected::OutputExact("Ada is 36.\nNext year she will be 37.\npi to two places: 3.14\n|     socks|\n|socks     |\n|  socks   |\nthat will be $19.99\ndebugging: x=42\nliteral braces: {not a hole}") },
    ExampleSpec { vfs_path: "/examples/code/basics/numbers.logos", mode: Mode::Code, source: CODE_NUMBERS, expected: Expected::OutputExact("2 to the 100th = 1267650600228229401496703205376\na third to 4 places: 0.3333\n0.1 + 0.2 is approximately 0.3 (floats drift)\nwith Decimal it is exactly 0.3") },
    ExampleSpec { vfs_path: "/examples/code/strings/text-basics.logos", mode: Mode::Code, source: CODE_TEXT_BASICS, expected: Expected::OutputExact("LOGOS has 5 letters.\nIt starts with L.\nThere is GO in LOGOS.\nHello, world!\nReversed: SOGOL") },
    ExampleSpec { vfs_path: "/examples/code/strings/formatting.logos", mode: Mode::Code, source: CODE_FORMATTING, expected: Expected::OutputExact("item          cost\napples        1.25\nbread          3.5\ntotal: $4.75") },
    ExampleSpec { vfs_path: "/examples/code/strings/find-and-count.logos", mode: Mode::Code, source: CODE_FIND_AND_COUNT, expected: Expected::OutputExact("the letter t appears 5 times\nthe sentence has 6 words") },
    ExampleSpec { vfs_path: "/examples/code/collections/slicing.logos", mode: Mode::Code, source: CODE_SLICING, expected: Expected::OutputExact("middle three: [70, 85, 90]\nedited copy: [100, 70, 85, 90, 60]\noriginal untouched: [55, 70, 85, 90, 60]") },
    ExampleSpec { vfs_path: "/examples/code/collections/tuples.logos", mode: Mode::Code, source: CODE_TUPLES, expected: Expected::OutputExact("x = 10, y = 20\nBerlin has 3600000 people\nthe tuple has 3 slots\nBerlin is the capital of Germany\nParis is the capital of France") },
    ExampleSpec { vfs_path: "/examples/code/collections/filter-transform.logos", mode: Mode::Code, source: CODE_FILTER_TRANSFORM, expected: Expected::OutputExact("evens: [2, 4, 6]\ntheir squares: [4, 16, 36]\nsum of squares: 56") },
    ExampleSpec { vfs_path: "/examples/code/time-and-money/dates.logos", mode: Mode::Code, source: CODE_DATES, expected: Expected::OutputExact("liftoff day: 2026-05-20\nprep starts: 2026-05-17\nreview lands: 2026-07-20\nsprint length: 17 days\ndaily standup: 09:30:00\nkickoff moment: 2026-05-20 16:00\n9am is before noon") },
    ExampleSpec { vfs_path: "/examples/code/time-and-money/money.logos", mode: Mode::Code, source: CODE_MONEY, expected: Expected::OutputExact("subtotal: $7.75\ntwo rounds: $15.50\nwith tip: $17.05\n0.1 + 0.2 = 0.3 exactly (no float drift)") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/fibonacci.logos", mode: Mode::Code, source: CODE_FIBONACCI, expected: Expected::OutputExact("The first 10 Fibonacci numbers:\n1\n1\n2\n3\n5\n8\n13\n21\n34\n55") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/primes.logos", mode: Mode::Code, source: CODE_PRIME, expected: Expected::OutputExact("Checking 2 through 30:\n2 is prime\n3 is prime\n5 is prime\n7 is prime\n11 is prime\n13 is prime\n17 is prime\n19 is prime\n23 is prime\n29 is prime") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/bubble-sort.logos", mode: Mode::Code, source: CODE_BUBBLE_SORT, expected: Expected::OutputExact("Before: [64, 34, 25, 12, 22, 11, 90]\nAfter: [11, 12, 22, 25, 34, 64, 90]") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/sieve.logos", mode: Mode::Code, source: CODE_SIEVE, expected: Expected::OutputExact("25 primes at or below 100\nthe largest is 97") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/binary-search.logos", mode: Mode::Code, source: CODE_BINARY_SEARCH, expected: Expected::OutputExact("found 42 at position 5\n50 is not present") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/merge-sort.logos", mode: Mode::Code, source: CODE_MERGE_SORT, expected: Expected::OutputExact("before: [38, 27, 43, 3, 9, 82, 10]\nafter: [3, 9, 10, 27, 38, 43, 82]") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/gcd.logos", mode: Mode::Code, source: CODE_GCD, expected: Expected::OutputExact("gcd(48, 18) = 6\ngcd(17, 5) = 1\ngcd(100, 75) = 25") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/two-sum.logos", mode: Mode::Code, source: CODE_TWO_SUM, expected: Expected::OutputExact("11 + 3 = 14") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/bfs.logos", mode: Mode::Code, source: CODE_BFS, expected: Expected::OutputExact("shortest path from 1 to 6: 3 hops") },
    ExampleSpec { vfs_path: "/examples/code/algorithms/collatz.logos", mode: Mode::Code, source: CODE_COLLATZ, expected: Expected::OutputExact("27 takes 111 steps to reach 1") },
    ExampleSpec { vfs_path: "/examples/code/collections/lists.logos", mode: Mode::Code, source: CODE_COLLECTIONS, expected: Expected::OutputExact("Numbers: [10, 20, 30, 40, 50]\nAfter pushing 60: [10, 20, 30, 40, 50, 60]\nThe list now holds 6 items.\nFirst: 10, last: 60.\nSum of all items: 210\nPopped 60; 5 items remain.") },
    ExampleSpec { vfs_path: "/examples/code/functions/recursion.logos", mode: Mode::Code, source: CODE_FACTORIAL, expected: Expected::OutputExact("factorial(5) = 120\nfactorial(10) = 3628800\nfib(10) = 55") },
    ExampleSpec { vfs_path: "/examples/code/functions/defining.logos", mode: Mode::Code, source: CODE_DEFINING, expected: Expected::OutputExact("a 12 x 10 room covers 120 square feet\nbalance after withdrawing 40 from 100: 60\nWelcome, Ada!") },
    ExampleSpec { vfs_path: "/examples/code/functions/closures.logos", mode: Mode::Code, source: CODE_CLOSURES, expected: Expected::OutputExact("double(21) = 42\n42 with the captured bonus of 100: 142\n7 is odd") },
    ExampleSpec { vfs_path: "/examples/code/functions/ufcs.logos", mode: Mode::Code, source: CODE_UFCS, expected: Expected::OutputExact("function style: double(5) = 10\nmethod style: (5).double() = 10\nchained: (5).double().plus(3).double() = 26") },
    ExampleSpec { vfs_path: "/examples/code/types/bytes-and-words.logos", mode: Mode::Code, source: CODE_BYTES_WORDS, expected: Expected::OutputExact("4294967295 + 1 wraps to 0\n1 shifted left by 4 = 16\n16 shifted right by 2 = 4\ncount_ones(255) = 8\n255 AND 170 = 170\n85 OR 170 = 255\n255 XOR 85 = 170\na Char literal: A") },
    ExampleSpec { vfs_path: "/examples/code/types/exact-numbers.logos", mode: Mode::Code, source: CODE_EXACT_NUMBERS, expected: Expected::OutputExact("1/3 + 1/6 = 1/2\n19.99 * 3 = 59.97 (no float drift)\ni * i = -1") },
    ExampleSpec { vfs_path: "/examples/code/types/structs.logos", mode: Mode::Code, source: CODE_STRUCT, expected: Expected::OutputExact("Alice is 30.\nBob is 25.\nAfter a birthday, Alice is 31.") },
    ExampleSpec { vfs_path: "/examples/code/types/enums.logos", mode: Mode::Code, source: CODE_ENUMS, expected: Expected::OutputExact("It's red!\nnot red") },
    ExampleSpec { vfs_path: "/examples/code/types/generics.logos", mode: Mode::Code, source: CODE_GENERICS, expected: Expected::OutputExact("The Int box holds 42.\nThe Text box holds hello.\nPair: score = 97.") },
    ExampleSpec { vfs_path: "/examples/code/collections/sets.logos", mode: Mode::Code, source: CODE_SETS, expected: Expected::OutputExact("After 4 adds (one duplicate): 3 members.\nBob is in the set.\nAfter removing Bob: 2 members.\nunion: {2, 4, 1}\nintersection: {2}\ndifference: {4}") },
    ExampleSpec { vfs_path: "/examples/code/collections/maps.logos", mode: Mode::Code, source: CODE_MAPS, expected: Expected::OutputExact("iron count: 50\nafter restock: 100\nFull inventory:\n  iron: 100\n  copper: 30\n  gold: 10") },
    ExampleSpec { vfs_path: "/examples/code/functions/higher-order.logos", mode: Mode::Code, source: CODE_HIGHER_ORDER, expected: Expected::OutputExact("double(21) = 42\ntwice(double, 5) = 20\nmakeAdder(10) applied to 32 = 42") },
    ExampleSpec { vfs_path: "/examples/code/distributed/counters.logos", mode: Mode::Code, source: CODE_CRDT_COUNTERS, expected: Expected::OutputExact("Total points: 18") },
    ExampleSpec { vfs_path: "/examples/code/security/policies.logos", mode: Mode::Code, source: CODE_POLICIES, expected: Expected::OutputExact("Admin check passed: Root is admin.\nCapability check passed: Alice can edit her own document.") },
    ExampleSpec { vfs_path: "/examples/code/memory/zones.logos", mode: Mode::Code, source: CODE_ZONES, expected: Expected::OutputExact("Sum in zone: 100\nValue in the 1 MB zone: 100\nZones cleaned up!") },
    ExampleSpec { vfs_path: "/examples/code/concurrency/tasks.logos", mode: Mode::Code, source: CODE_TASKS, expected: Expected::OutputExact("worker done\nmain continues\nHello from a task\nworker done\nsecond worker stopped") },
    ExampleSpec { vfs_path: "/examples/code/concurrency/pipes.logos", mode: Mode::Code, source: CODE_CHANNELS, expected: Expected::OutputExact("received: 42\ntry-receive got: 7") },
    ExampleSpec { vfs_path: "/examples/code/concurrency/select.logos", mode: Mode::Code, source: CODE_SELECT, expected: Expected::OutputExact("first arrival: 99\ntimed out waiting on the quiet pipe") },
    ExampleSpec { vfs_path: "/examples/code/distributed/last-write-wins.logos", mode: Mode::Code, source: CODE_CRDT_LWW, expected: Expected::OutputExact("status: online\nstatus now: away") },
    ExampleSpec { vfs_path: "/examples/code/distributed/divergent.logos", mode: Mode::Code, source: CODE_CRDT_DIVERGENT, expected: Expected::OutputExact("title: Draft\nresolved title: Final") },
    ExampleSpec { vfs_path: "/examples/code/distributed/shared-set.logos", mode: Mode::Code, source: CODE_CRDT_SHARED_SET, expected: Expected::OutputExact("Bob survives: concurrent add wins\nteam size: 2") },
    ExampleSpec { vfs_path: "/examples/code/distributed/shared-sequence.logos", mode: Mode::Code, source: CODE_CRDT_SHARED_SEQUENCE, expected: Expected::OutputExact("entries after merge: 3") },
    ExampleSpec { vfs_path: "/examples/code/distributed/shared-map.logos", mode: Mode::Code, source: CODE_CRDT_SHARED_MAP, expected: Expected::OutputExact("50") },
    ExampleSpec { vfs_path: "/examples/code/distributed/sync.logos", mode: Mode::Code, source: CODE_CRDT_SYNC, expected: Expected::OutputExact("local clicks: 3") },
    ExampleSpec { vfs_path: "/examples/code/basics/variables.logos", mode: Mode::Code, source: CODE_BASICS_VARIABLES, expected: Expected::OutputExact("Alice is 25, active: true\nticket price: $19.99\ncount doubled: 200") },
    ExampleSpec { vfs_path: "/examples/code/basics/operators.logos", mode: Mode::Code, source: CODE_BASICS_OPERATORS, expected: Expected::OutputExact("10 + 3 = 13\n10 - 3 = 7\n10 * 3 = 30\n10 / 3 = 3\n10 % 3 = 1\n10 is greater than 3\n10 is between 1 and 100 (chained comparison)\n10 is between 5 and 15\n10 is even\n3 is odd\n10 is divisible by 5\nsunny and warm: false\nsunny or warm: true\nnot sunny: false") },
    ExampleSpec { vfs_path: "/examples/code/basics/control-flow.logos", mode: Mode::Code, source: CODE_BASICS_CONTROL_FLOW, expected: Expected::OutputExact("Grade: B\nCountdown:\n3\n2\n1\nBlastoff!\nFirst multiple of 7 above 20:\n21") },
    ExampleSpec { vfs_path: "/examples/code/types/enum-payloads.logos", mode: Mode::Code, source: CODE_ENUMS_PATTERNS, expected: Expected::OutputExact("Circle with radius 5, area about 75\nRectangle 4 x 6, area 24") },
    ExampleSpec { vfs_path: "/examples/code/memory/ownership.logos", mode: Mode::Code, source: CODE_OWNERSHIP, expected: Expected::OutputExact("Viewing: User Profile Data\nStill usable after lending: User Profile Data\nConsumed: User Profile Data\nOriginal intact: User Profile Data") },
    ExampleSpec { vfs_path: "/examples/code/concurrency/parallel.logos", mode: Mode::Code, source: CODE_CONCURRENCY_PARALLEL, expected: Expected::OutputExact("25^2 = 625, 9^3 = 729\nproduct: 455625\nx + y = 30") },
    ExampleSpec { vfs_path: "/examples/code/distributed/tally.logos", mode: Mode::Code, source: CODE_CRDT_TALLY, expected: Expected::OutputExact("after +100: 100\nafter -30: 70\nfinal: 80") },
    ExampleSpec { vfs_path: "/examples/code/distributed/merge.logos", mode: Mode::Code, source: CODE_CRDT_MERGE, expected: Expected::OutputExact("local views: 100\nremote views: 50\nafter merge: 150") },
    ExampleSpec { vfs_path: "/examples/code/networking/server.logos", mode: Mode::Code, source: CODE_NETWORK_SERVER, expected: Expected::NativeOnlyCompiles },
    ExampleSpec { vfs_path: "/examples/code/networking/client.logos", mode: Mode::Code, source: CODE_NETWORK_CLIENT, expected: Expected::NativeOnlyCompiles },
    ExampleSpec { vfs_path: "/examples/code/types/option-result.logos", mode: Mode::Code, source: CODE_ERROR_HANDLING, expected: Expected::OutputExact("10 / 2 = 5\nerror: cannot divide 5 by zero") },
    ExampleSpec { vfs_path: "/examples/code/verification/refinement.logos", mode: Mode::Code, source: CODE_ADVANCED_REFINEMENT, expected: Expected::OutputExact("positive: 5\npercentage: 85\ndice roll: 4") },
    ExampleSpec { vfs_path: "/examples/code/verification/assertions.logos", mode: Mode::Code, source: CODE_ADVANCED_ASSERTIONS, expected: Expected::OutputExact("withdrew 50 from 100, leaving 50\nprocess(5) = 10") },
    ExampleSpec { vfs_path: "/examples/code/verification/termination.logos", mode: Mode::Code, source: CODE_TERMINATION, expected: Expected::OutputExact("1024 halves to 1 in 10 steps\n1000000 halves to 1 in 19 steps") },
    ExampleSpec { vfs_path: "/examples/code/security/check-gate.logos", mode: Mode::Code, source: CODE_CHECK_GATE, expected: Expected::OutputExact("gate passed: the account is open\nwithdrawal of 40 approved for Ada") },
    ExampleSpec { vfs_path: "/examples/code/crypto/hashes.logos", mode: Mode::Code, source: CODE_HASHES, expected: Expected::OutputExact("sha1 of abc: first bytes 169, 153\nmd5 of abc: first byte 144\ndigest lengths: sha1 20 bytes, md5 16 bytes") },
    ExampleSpec { vfs_path: "/examples/code/crypto/chacha20.logos", mode: Mode::Code, source: CODE_CHACHA20, expected: Expected::OutputExact("a = 3928658676\nb = 3407673550\nc = 1166100270\nd = 1484899515") },
    ExampleSpec { vfs_path: "/examples/code/crypto/ml-kem.logos", mode: Mode::Code, source: CODE_MLKEM, expected: Expected::OutputExact("encapsulation key: 1184 bytes\nciphertext: 1088 bytes, shared secret: 32 bytes") },
    ExampleSpec { vfs_path: "/examples/code/crypto/uuid.logos", mode: Mode::Code, source: CODE_UUID, expected: Expected::OutputExact("v5 of example.com: 2ed6657d-e927-568b-95e1-2665a8aea6a2\nv4 from fixed bytes: 0b16212c-3742-4d58-a36e-79848f9aa5b0") },
    ExampleSpec { vfs_path: "/examples/code/projects/todo-list.logos", mode: Mode::Code, source: CODE_PROJ_TODO, expected: Expected::OutputExact("TODO:\n  [ ] buy milk\n  [x] write examples\n  [ ] water plants\n2 remaining") },
    ExampleSpec { vfs_path: "/examples/code/projects/inventory.logos", mode: Mode::Code, source: CODE_PROJ_INVENTORY, expected: Expected::OutputExact("Inventory report:\n  planks: 28\n  nails: 350\n  hinges: 2  LOW - reorder\n  varnish: 2  LOW - reorder") },
    ExampleSpec { vfs_path: "/examples/code/projects/dice-game.logos", mode: Mode::Code, source: CODE_PROJ_DICE, expected: Expected::OutputExact("round 1: you rolled 4, rival rolled 1\nround 2: you rolled 6, rival rolled 5\nround 3: you rolled 6, rival rolled 3\nfinal score - you: 16, rival: 9\nyou win!") },
    ExampleSpec { vfs_path: "/examples/code/projects/temperature-stats.logos", mode: Mode::Code, source: CODE_PROJ_WEATHER, expected: Expected::OutputExact("week of readings: [18, 21, 25, 24, 19, 16, 22]\nlow 16, high 25, average 20 (rounded down)\nchart:\n  18 ###\n  21 ######\n  25 ##########\n  24 #########\n  19 ####\n  16 #\n  22 #######") },
    ExampleSpec { vfs_path: "/examples/code/projects/bank-ledger.logos", mode: Mode::Code, source: CODE_PROJ_LEDGER, expected: Expected::OutputExact("opening balance: $250.00\ndeposit $75.50 -> $325.50\ndeposit $12.25 -> $337.75\nwithdrew $40.00 -> $297.75\nclosing balance: $297.75") },
    ExampleSpec { vfs_path: "/examples/code/projects/caesar-cipher.logos", mode: Mode::Code, source: CODE_PROJ_CAESAR, expected: Expected::OutputExact("encoded: khoor zruog\ndecoded: hello world") },
    ExampleSpec { vfs_path: "/examples/code/projects/word-count.logos", mode: Mode::Code, source: CODE_PROJ_WORDCOUNT, expected: Expected::OutputExact("text: the quick brown fox jumps over the lazy dog\n43 characters\n9 words") },
    ExampleSpec { vfs_path: "/examples/code/projects/matrix-ops.logos", mode: Mode::Code, source: CODE_PROJ_MATRIX, expected: Expected::OutputExact("A x B = [[19, 22], [43, 50]]") },
    ExampleSpec { vfs_path: "/examples/code/projects/event-scheduler.logos", mode: Mode::Code, source: CODE_PROJ_SCHEDULER, expected: Expected::OutputExact("kickoff: 2026-06-01 09:00\nstandup: 2026-06-01 09:30\nreview: 2026-06-01 16:00\nstandup comes before review\neach focus block runs 2h") },
    ExampleSpec { vfs_path: "/examples/code/projects/vending-machine.logos", mode: Mode::Code, source: CODE_PROJ_VENDING, expected: Expected::OutputExact("credit: 75 cents\ndispensing soda\nchange: 10 cents\nmachine reset") },
    ExampleSpec { vfs_path: "/examples/code/projects/text-adventure.logos", mode: Mode::Code, source: CODE_PROJ_ADVENTURE, expected: Expected::OutputExact("> north\nYou are in a quiet library. A key glints on the desk.\n> take key\nYou found the key!\n> south\nYou are in a dusty hall. Doors lead north and east.\n> east\nYou are in a dark cellar. The exit door is locked.\nThe key turns. You escape the cellar!") },
    ExampleSpec { vfs_path: "/examples/code/projects/chat-simulation.logos", mode: Mode::Code, source: CODE_PROJ_CHAT, expected: Expected::OutputExact("<alice> message 101\n<alice> message 102\n<bob> message 201\nroom quiet - closing") },
    ExampleSpec { vfs_path: "/examples/code/projects/leaderboard.logos", mode: Mode::Code, source: CODE_PROJ_LEADERBOARD, expected: Expected::OutputExact("standings after merge:\n  alice: 19\n  bob: 16\nalice leads") },
    ExampleSpec { vfs_path: "/examples/code/projects/password-hasher.logos", mode: Mode::Code, source: CODE_PROJ_HASHER, expected: Expected::OutputExact("stored digest: 20 bytes\ncorrect password accepted\nwrong password rejected") },
    ExampleSpec { vfs_path: "/examples/code/time-and-money/durations.logos", mode: Mode::Code, source: CODE_TEMPORAL, expected: Expected::OutputExact("blink 50ns, tick 100\u{3bc}s, pause 500ms, breath 1s\n500ms + 500ms = 1s\n500ms is less than 1s\nSleeping 200ms...\nAwake!") },
    // Web pages — each opens in the Web tab and runs live.
    ExampleSpec { vfs_path: "/examples/code/web/counter.logos", mode: Mode::Code, source: CODE_WEB_COUNTER, expected: Expected::WebPage },
    ExampleSpec { vfs_path: "/examples/code/web/todo-list.logos", mode: Mode::Code, source: CODE_WEB_TODO, expected: Expected::WebPage },
    ExampleSpec { vfs_path: "/examples/code/web/form.logos", mode: Mode::Code, source: CODE_WEB_FORM, expected: Expected::WebPage },
    ExampleSpec { vfs_path: "/examples/code/web/website.logos", mode: Mode::Code, source: CODE_WEB_SITE, expected: Expected::WebPage },
];

/// The 24 Math-mode examples.
pub const ALL_MATH_EXAMPLES: &[ExampleSpec] = &[
    ExampleSpec {
        vfs_path: "/examples/math/prelude-tour.logos",
        mode: Mode::Math,
        source: MATH_PRELUDE_TOUR,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/big-arithmetic.logos",
        mode: Mode::Math,
        source: MATH_BIG_ARITHMETIC,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/termination-wall.logos",
        mode: Mode::Math,
        source: MATH_TERMINATION_WALL,
        expected: Expected::KernelAdmitsAt("collatz_len"),
    },
    ExampleSpec {
        vfs_path: "/examples/math/tarski-geometry.logos",
        mode: Mode::Math,
        source: MATH_TARSKI,
        expected: Expected::TheoryAllVerified,
    },
    ExampleSpec {
        vfs_path: "/examples/math/natural-numbers.logos",
        mode: Mode::Math,
        source: MATH_NAT,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/boolean-logic.logos",
        mode: Mode::Math,
        source: MATH_BOOL,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/godel-sentence.logos",
        mode: Mode::Math,
        source: MATH_GODEL,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/incompleteness.logos",
        mode: Mode::Math,
        source: MATH_INCOMPLETENESS,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/prop-logic.logos",
        mode: Mode::Math,
        source: MATH_PROP_LOGIC,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/functions.logos",
        mode: Mode::Math,
        source: MATH_FUNCTIONS,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/list-ops.logos",
        mode: Mode::Math,
        source: MATH_LIST_OPS,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/pairs.logos",
        mode: Mode::Math,
        source: MATH_PAIRS,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/logic-gates.logos",
        mode: Mode::Math,
        source: MATH_CIRCUIT,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/proven-property.logos",
        mode: Mode::Math,
        source: MATH_PROPERTY,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/collatz.logos",
        mode: Mode::Math,
        source: MATH_COLLATZ,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/godel-literate.logos",
        mode: Mode::Math,
        source: MATH_GODEL_LITERATE,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/incompleteness-literate.logos",
        mode: Mode::Math,
        source: MATH_INCOMPLETENESS_LITERATE,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/ring-tactic.logos",
        mode: Mode::Math,
        source: MATH_RING,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/lia-tactic.logos",
        mode: Mode::Math,
        source: MATH_LIA,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/cc-tactic.logos",
        mode: Mode::Math,
        source: MATH_CC,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/simp-tactic.logos",
        mode: Mode::Math,
        source: MATH_SIMP,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/omega-tactic.logos",
        mode: Mode::Math,
        source: MATH_OMEGA,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/auto-tactic.logos",
        mode: Mode::Math,
        source: MATH_AUTO,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/induction-tactic.logos",
        mode: Mode::Math,
        source: MATH_INDUCTION,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/hints.logos",
        mode: Mode::Math,
        source: MATH_HINTS,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/inversion-tactic.logos",
        mode: Mode::Math,
        source: MATH_INVERSION,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/operator-tactics.logos",
        mode: Mode::Math,
        source: MATH_OPERATOR,
        expected: Expected::KernelAllStatementsOk,
    },
    ExampleSpec {
        vfs_path: "/examples/math/tacticals.logos",
        mode: Mode::Math,
        source: MATH_TACTICALS,
        expected: Expected::KernelAllStatementsOk,
    },
];

/// The 18 Hardware-mode examples, sourced from the four hardware slices. The
/// deliberately-unsafe demos (`bad-arbiter`, `traffic-crash`, `queue-jam`,
/// `register-alloc-spill`) are locked to their intended failure/refutation.
pub const ALL_HARDWARE_SPECS: &[ExampleSpec] = &[
    ExampleSpec {
        vfs_path: "/examples/hardware/handshake.hw",
        mode: Mode::Hardware,
        source: HARDWARE_EXAMPLES[0].1,
        expected: Expected::SvaSynthesizes,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/only-if.hw",
        mode: Mode::Hardware,
        source: HARDWARE_EXAMPLES[1].1,
        expected: Expected::SvaSynthesizes,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/next-cycle.hw",
        mode: Mode::Hardware,
        source: HARDWARE_EXAMPLES[2].1,
        expected: Expected::SvaSynthesizes,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/mutex.hw",
        mode: Mode::Hardware,
        source: HW_MUTEX,
        expected: Expected::SvaSynthesizes,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/intersection-design.hw",
        mode: Mode::Hardware,
        source: HARDWARE_EXAMPLES[3].1,
        expected: Expected::SignalPlanSynthesizes,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/t-junction.hw",
        mode: Mode::Hardware,
        source: HARDWARE_EXAMPLES[4].1,
        expected: Expected::SignalPlanSynthesizes,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/arbiter.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[0].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/bad-arbiter.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[1].1,
        expected: Expected::RefutesWithCounterexample,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/fifo.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[2].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/onehot.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[3].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/reset-mirror.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[4].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/traffic-safe.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[5].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/traffic-crash.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[6].1,
        expected: Expected::RefutesWithCounterexample,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/queue-jam.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[7].1,
        expected: Expected::RefutesWithCounterexample,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/queue-stable.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[8].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/datapath-identity.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[9].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/door-interlock.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[10].1,
        expected: Expected::RtlProven,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/door-interlock-bug.v",
        mode: Mode::Hardware,
        source: RTL_EXAMPLES[11].1,
        expected: Expected::RefutesWithCounterexample,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/register-alloc-fits.hw",
        mode: Mode::Hardware,
        source: REGALLOC_EXAMPLES[0].1,
        expected: Expected::RegisterAllocFits,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/register-alloc-spill.hw",
        mode: Mode::Hardware,
        source: REGALLOC_EXAMPLES[1].1,
        expected: Expected::SpillsRequired,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/pigeonhole.hw",
        mode: Mode::Hardware,
        source: PIGEONHOLE_EXAMPLES[0].1,
        expected: Expected::UnsatCertified,
    },
    ExampleSpec {
        vfs_path: "/examples/hardware/pigeonhole-12.hw",
        mode: Mode::Hardware,
        source: PIGEONHOLE_EXAMPLES[1].1,
        expected: Expected::UnsatCertified,
    },
];

/// What each program the browser cannot run needs, the subject of the notice it opens to: one
/// row per `NativeOnlyCompiles` example (`compiled_runtime_notice_tests`).
const COMPILED_RUNTIME_NEEDS: &[(&str, &str)] = &[
    ("/examples/code/networking/server.logos", "Live networking"),
    ("/examples/code/networking/client.logos", "Live networking"),
];

/// Each planted bug beside the registry path of the same design without it: one row per
/// `RefutesWithCounterexample` and `SpillsRequired` example (`planted_bug_tests`).
const PLANTED_BUG_FIXES: &[(&str, &str)] = &[
    (
        "/examples/hardware/bad-arbiter.v",
        "/examples/hardware/arbiter.v",
    ),
    (
        "/examples/hardware/traffic-crash.v",
        "/examples/hardware/traffic-safe.v",
    ),
    (
        "/examples/hardware/queue-jam.v",
        "/examples/hardware/queue-stable.v",
    ),
    (
        "/examples/hardware/door-interlock-bug.v",
        "/examples/hardware/door-interlock.v",
    ),
    (
        "/examples/hardware/register-alloc-spill.hw",
        "/examples/hardware/register-alloc-fits.hw",
    ),
];

fn row(table: &[(&'static str, &'static str)], path: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(at, _)| *at == path)
        .map(|(_, value)| *value)
}

/// What a registry program the browser cannot run needs ("Live networking"); `None` for any other
/// file.
pub fn compiled_runtime_needs(path: &str) -> Option<&'static str> {
    let spec = ALL_CODE_EXAMPLES
        .iter()
        .find(|spec| spec.vfs_path == path)?;
    if !matches!(spec.expected, Expected::NativeOnlyCompiles) {
        return None;
    }
    row(COMPILED_RUNTIME_NEEDS, path)
}

/// The registry path of the design a planted bug's example is, fixed; `None` for any other file.
pub fn planted_bug_fix(path: &str) -> Option<&'static str> {
    let spec = ALL_HARDWARE_SPECS
        .iter()
        .find(|spec| spec.vfs_path == path)?;
    if !matches!(
        spec.expected,
        Expected::RefutesWithCounterexample | Expected::SpillsRequired
    ) {
        return None;
    }
    row(PLANTED_BUG_FIXES, path)
}

/// What a registry example with a planted bug says when it opens: that the failure is the point,
/// and which example is the same design fixed.
pub fn planted_bug_notice(path: &str) -> Option<String> {
    let fixed = planted_bug_fix(path)?;
    let name = fixed.rsplit('/').next().unwrap_or(fixed);
    Some(format!(
        "A planted bug: this design is meant to fail, and the prover shows exactly where. {name} is the same design, fixed."
    ))
}

/// What a registry example that cannot run in the browser shows when it opens, in place of an
/// interpreter run that would only pretend: what it needs, and where it does run.
pub fn compiled_runtime_notice(path: &str) -> Option<String> {
    let needs = compiled_runtime_needs(path)?;
    Some(format!(
        "{needs} needs the compiled runtime. Build it with `largo build`, and Compile shows the Rust it becomes."
    ))
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/examples/example_health.rs"]
mod example_health;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/examples/compiled_runtime_notice_tests.rs"]
mod compiled_runtime_notice_tests;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/examples/planted_bug_tests.rs"]
mod planted_bug_tests;
